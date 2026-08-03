#![forbid(unsafe_code)]

//! The store behind `durable` placement.
//!
//! `durable` is the placement that makes the demo persuasive: §5.7 says a
//! durable signal is global and shared, and §10 says the proof is two
//! windows moving together and a value surviving a reload. Neither is a
//! property of the compiler — they are properties of this crate.
//!
//! # Five operations, and why they are five
//!
//! §7.4 gives the interface as `get`, `set`, `incr`, `delete`, `watch`, and
//! §8 item 5 says why it is that small: it has to be implementable on
//! DynamoDB, Cloudflare KV, Vercel KV, Redis **and** a local file, so the
//! narrowest interface that supports the language wins. A sixth operation
//! is a sixth thing every backing store must have.
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
//! here would be designing against a decision that was taken.
//!
//! **No per-visitor scoping.** §5.7 defers it beyond v1 for want of an
//! identity mechanism. [`watch`](DurableStore::watch) takes a key set
//! rather than a prefix — see [`watch`](crate::watch) for why a prefix is
//! not implementable on the stores this trait exists to abstract over.

pub mod embedded;
pub mod value;
pub mod watch;

pub use crate::embedded::EmbeddedStore;
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

    /// Write a key. Returns the position of the write.
    ///
    /// Idempotent, which §18.2 reads straight off the verb: `set visits to
    /// 0` becomes `$call("visits.set", 0)` with a constant on the wire, so
    /// a retry after a timeout costs nothing and needs no write id.
    fn set(&self, key: &str, value: Json) -> Result<Seq, StoreError>;

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
    fn incr(&self, key: &str, delta: Number) -> Result<(Number, Seq), StoreError>;

    /// Remove a key. Idempotent — deleting an absent key is a write that
    /// changes nothing, and reports the position anyway so a subscriber
    /// still learns the key is gone.
    fn delete(&self, key: &str) -> Result<Seq, StoreError>;

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
    /// therefore known at compile time. See [`watch`](crate::watch) for
    /// the full argument.
    fn watch(&self, keys: &Keys, since: Option<Seq>) -> Subscription;
}
