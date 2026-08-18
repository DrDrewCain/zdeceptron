#![forbid(unsafe_code)]

//! The store behind `durable` placement.
//!
//! `durable` is the placement that makes the demo persuasive: §5.7 says a
//! durable signal is global and shared, and §10 says the proof is two
//! windows moving together and a value surviving a reload. Neither is a
//! property of the compiler — they are properties of this crate.
//!
//! # Three operations, and why the transaction made it three
//!
//! §7.4 gives the interface as `get`, `set`, `incr`, `delete`, `watch`, and
//! §8 item 5 says why it is that small: it has to be implementable on
//! DynamoDB, Cloudflare KV, Vercel KV, Redis **and** a local file, so the
//! narrowest interface that supports the language wins. A sixth operation
//! is a sixth thing every backing store must have.
//!
//! That rule is why [`DurableStore`] now requires **three**: `get`,
//! [`apply`](DurableStore::apply) and `watch`. A handler's writes have to
//! land together or not at all (§14G.7.4 puts the transaction boundary on
//! the handler; one vote in the milestone-12 target is ~25 writes across 8
//! tables, and a half-applied vote is corrupt data rather than a failed
//! request), and the only honest way to add that was to make the atomic
//! batch the primitive and derive `set`, `incr` and `delete` from it. Two
//! operations left the required surface, one arrived, and every backing
//! store now implements strictly less than it did — see [`crate::txn`] for
//! which of them can, and which cannot.
//!
//! The generated code's `$store` façade is wider — `zdc-codegen` emits
//! `set`, `incr`, `decr`, `append` and `remove`, because §14B.2 closed the
//! *mutation verb* set at five and §18.2 makes that verb the wire
//! contract. Those two fives are different fives, and conflating them
//! would widen this trait by three operations for no gain: `decr` is
//! `incr` of a negation, and `append`/`remove` are `get` and `set`. The
//! adapter derives them; the store does not carry them.
//!
//! # What is deliberately not here
//!
//! **No relational query.** §14G.5 sent relational persistence back, and
//! §14C.3a records that `durable` being a key-value store is a structural
//! limit of v1 rather than an implementation gap. Adding a query language
//! here would be designing against a decision that was taken — and #36
//! decided it again on 2026-08-16, against this crate rather than against
//! the spec: [`watch`](DurableStore::watch) takes a key set, a key set is
//! affordable only because every durable key is a declaration, and a query
//! is a set of keys that is not one. The language reference's *Querying
//! related data* carries the argument and names the two conditions that
//! reverse it.
//!
//! **No schema version, and that is now a named gap rather than an
//! unasked question.** Nothing here records what shape wrote a value —
//! [`Json`](crate::value::Json) is text this crate never parses, and
//! `META` holds one row. #37 decided the answer: a digest of the program's
//! durable shape, written under a reserved key, checked when a host is
//! built, refusing a mismatch by name. It is specified in the reference
//! and is not implemented here yet; until it is, a deploy that retypes a
//! `durable` declaration reads the old value at the new type.
//!
//! **No per-visitor scoping.** §5.7 defers it beyond v1 for want of an
//! identity mechanism. [`watch`](DurableStore::watch) takes a key set
//! rather than a prefix — see [`watch`] for why a prefix is
//! not implementable on the stores this trait exists to abstract over.

pub mod embedded;
pub mod txn;
pub mod value;
pub mod watch;

pub use crate::embedded::EmbeddedStore;
pub use crate::txn::{Applied, Read, Transaction, Write};
pub use crate::value::{Json, Number};
pub use crate::watch::{Event, Fanout, Keys, Seq, Subscription, Update};

/// Why an operation could not be performed.
///
/// Three variants, each one a fact about the request rather than a wrapped
/// backend string, so a handler can tell "you asked to increment a name"
/// apart from "the disk is full" without matching on a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The backing store refused. Its own words, because paraphrasing a
    /// disk error loses the only part a developer can act on.
    Backend { message: String },
    /// `incr` on a key that does not hold a number.
    ///
    /// Reachable only through a bug — the type checker knows a `durable`
    /// signal's type and `add` is only accepted on `Numeric` — so it names
    /// the key and shows what was found rather than saying "type error".
    NotANumber { key: String, found: String },
    /// The sum left the range JSON can carry (§14A.3's 2^53 bound is
    /// documented; infinity is not representable at all).
    OutOfRange { key: String },
    /// A key the handler read was changed by somebody else before the
    /// transaction could commit, so the values it computed from that key
    /// are stale and none of its writes were applied.
    ///
    /// Not a failure a caller should surface: the handler is a pure
    /// function of its arguments (§17.2.7 evaluated the right-hand side in
    /// the *caller's* region, so nothing in it depends on store state), and
    /// re-running it is therefore safe. `zdc-host` retries. It is a named
    /// variant rather than a `Backend` string because "run it again" and
    /// "the disk is full" are opposite instructions.
    Conflict { key: String },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Backend { message } => write!(f, "the durable store failed: {message}"),
            StoreError::NotANumber { key, found } => write!(
                f,
                "`{key}` holds {found}, which cannot be incremented; `add` needs a number"
            ),
            StoreError::OutOfRange { key } => write!(
                f,
                "incrementing `{key}` left the range a number can represent"
            ),
            StoreError::Conflict { key } => write!(
                f,
                "`{key}` was changed by another handler while this one was running, so none of \
                 its writes were applied"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

/// Persistent, shared state — and the notification that it changed.
///
/// Every method takes `&self`. A store is shared across concurrent
/// invocations by definition (§5.7: one value across all visitors), so an
/// interface that needed `&mut` would push a lock into every caller and
/// serialise reads behind writes.
pub trait DurableStore: Send + Sync {
    /// Read a key. `None` if it was never written or has been deleted.
    fn get(&self, key: &str) -> Result<Option<Json>, StoreError>;

    /// Commit one handler's whole effect, or none of it.
    ///
    /// **This is the only mutating operation a backing store implements.**
    /// [`set`](DurableStore::set), [`incr`](DurableStore::incr) and
    /// [`delete`](DurableStore::delete) are provided methods over it, so
    /// the required surface of this trait is three operations — `get`,
    /// `apply`, `watch` — where it was five. A transaction was added and
    /// the interface got *narrower*, which is the only shape in which it
    /// was worth adding.
    ///
    /// The contract, and it is the whole guarantee the language makes:
    ///
    /// 1. **All or nothing.** Every write in `transaction` lands, or none
    ///    does. A failure part way through leaves the store exactly as it
    ///    was — no key written, no sequence number spent, no announcement
    ///    published.
    /// 2. **In order.** The writes apply in the order given, so a `set`
    ///    followed by an `incr` on one key sees the `set`.
    /// 3. **Isolated for the keys it names.** No other handler observes a
    ///    state between the first write and the last, and if any key in
    ///    [`Transaction::reads`] changed since the handler read it, nothing
    ///    is applied and [`StoreError::Conflict`] says which key.
    /// 4. **Nothing at all for an empty transaction.** A read-only
    ///    invocation must not take a write lock or move the sequence.
    ///
    /// **Which targets can honour this, stated rather than assumed.**
    /// Durable Objects and a local database implement it with a real
    /// transaction. Deno KV implements it with `atomic()`: [`Read`] is
    /// `check()` and [`Write`] is the mutation list, within documented
    /// caps of 100 checks and 1000 mutations. DynamoDB implements it with
    /// `TransactWriteItems` plus a `ConditionExpression` per read, at
    /// double the write cost and inside a cap. **Cloudflare KV cannot
    /// implement it**, because one write per second per key rules out both
    /// the batch and the counter underneath it — that is a store this
    /// language cannot use for `durable`, and saying so is better than a
    /// runtime that silently downgrades the promise.
    ///
    /// **What it does not promise.** [`watch`](DurableStore::watch)
    /// announces one update per key, so a live subscriber sees a
    /// transaction's keys arrive in order rather than simultaneously.
    /// Atomicity is a property of the committed store, not of the fan-out;
    /// a subscriber that re-reads always sees a committed state, and one
    /// that renders each push as it lands may show an intermediate one.
    fn apply(&self, transaction: &Transaction) -> Result<Applied, StoreError>;

    /// Write a key. Returns the position of the write.
    ///
    /// Idempotent, which §18.2 reads straight off the verb: `set visits to
    /// 0` becomes `$call("visits.set", 0)` with a constant on the wire, so
    /// a retry after a timeout costs nothing and needs no write id.
    fn set(&self, key: &str, value: Json) -> Result<Seq, StoreError> {
        Ok(self
            .apply(&Transaction::of(Write::Set {
                key: key.to_string(),
                value,
            }))?
            .seq)
    }

    /// Add `delta` to a key, atomically. Returns the new value.
    ///
    /// Atomic rather than get-then-set because this is the operation the
    /// two-window demo exercises: §18.3 rejected provisional client-side
    /// writes precisely because two visitors incrementing at once must
    /// both be counted, and that is only true if the read and the write
    /// are one operation here.
    ///
    /// An absent key counts as zero rather than failing: a `durable`
    /// signal always has a `starting` value, so the first increment before
    /// any write is ordinary.
    ///
    /// **"Atomic" is a requirement on the result, not on the mechanism.**
    /// No surveyed backend can implement this with a native add:
    ///
    /// - **Deno KV** has `mutate({type:"sum"})`, but only on `Deno.KvU64`
    ///   — unsigned and wrapping, so it cannot hold a `Whole`, which is an
    ///   f64 that can be negative (§14A.3). Its usable primitive is
    ///   `check`/`set`, i.e. compare-and-set.
    /// - **DynamoDB** has `SET n = n + :v`, and AWS's own documentation
    ///   warns it is **not idempotent**, so a retried write over-counts.
    /// - **Cloudflare KV** allows one write per second per key and cannot
    ///   back a counter at all.
    /// - **Durable Objects** and a local database are the two that
    ///   genuinely serialise it.
    ///
    /// So an implementation is expected to reach this contract by whatever
    /// it has — a transaction here, a compare-and-set retry loop on Deno —
    /// and the contract is what the two-window demo depends on: two
    /// visitors incrementing at once are both counted.
    fn incr(&self, key: &str, delta: Number) -> Result<(Number, Seq), StoreError> {
        let applied = self.apply(&Transaction::of(Write::Incr {
            key: key.to_string(),
            delta,
        }))?;
        let value = applied
            .values
            .first()
            .and_then(|slot| slot.as_ref())
            .and_then(|json| Number::parse(json.as_str()))
            .ok_or_else(|| StoreError::Backend {
                message: format!("`{key}` was incremented and the store reported no number back"),
            })?;
        Ok((value, applied.seq))
    }

    /// Remove a key. Idempotent — deleting an absent key is a write that
    /// changes nothing, and reports the position anyway so a subscriber
    /// still learns the key is gone.
    fn delete(&self, key: &str) -> Result<Seq, StoreError> {
        Ok(self
            .apply(&Transaction::of(Write::Delete {
                key: key.to_string(),
            }))?
            .seq)
    }

    /// Subscribe to `keys`, resuming after `since`.
    ///
    /// This is the fan-out seam of §8.1 and the only operation in the
    /// interface that is not a plain read or write. `since` is where a
    /// reconnecting client's `Last-Event-ID` lands: the subscription
    /// replays exactly the tail that client missed, or opens with
    /// [`Event::Resync`] if it cannot prove it has all of it.
    ///
    /// **This is not §7.4's `watch(prefix)`.** The spec's signature cannot
    /// be implemented on the stores it was written for — Deno KV's
    /// `watch()` takes an explicit key list and has no prefix form,
    /// DynamoDB Streams are a pull-based feed with two readers per shard,
    /// and Cloudflare KV has no watch at all. A key set is the widest
    /// interface every target can honour, and it costs a ZDeceptron
    /// program nothing, because its durable keys are declarations and are
    /// therefore known at compile time. See [`watch`] for
    /// the full argument.
    fn watch(&self, keys: &Keys, since: Option<Seq>) -> Subscription;
}
