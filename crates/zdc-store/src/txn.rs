//! One handler's whole effect on the store, as one value.
//!
//! # Why a declared batch and not an open transaction
//!
//! The obvious primitive is `begin`/`commit` with the handler running in
//! between. It is also the one primitive the surveyed backends mostly do
//! not have. Checked against vendor documentation on 2026-08-02:
//!
//! | store | interactive transaction | non-interactive atomic batch |
//! |---|---|---|
//! | Cloudflare Durable Objects | yes (`transactionSync`) | yes |
//! | a local database (`redb`, SQLite) | yes | yes |
//! | Deno KV | **no** | yes — `atomic()`, ≤100 `check`s, ≤1000 mutations |
//! | DynamoDB | **no** | yes — `TransactWriteItems`, capped, billed double |
//! | Cloudflare KV | **no** | **no** — 1 write/second/key |
//!
//! An interface only two of five can honour is not an interface; that was
//! the lesson `watch(prefix)` taught in [`crate::watch`], and repeating it
//! here would be repeating it knowingly. So the unit is a **[`Transaction`]
//! value**: the keys the handler read with the values it saw, and the
//! writes it wants, handed to the store all at once with nothing left to
//! discover.
//!
//! That shape is exactly what a non-interactive batch takes. [`Read`] is
//! Deno KV's `check()` and DynamoDB's `ConditionExpression`; [`Write`] is
//! their mutation list. A store that has a real transaction implements
//! [`DurableStore::apply`](crate::DurableStore::apply) with one; a store
//! that has only compare-and-set implements it with a retry loop; a store
//! that has neither cannot implement it at all, and Cloudflare KV is
//! named as that store rather than quietly downgraded.
//!
//! # What makes the batch knowable in advance
//!
//! This would not work for a general database client, which cannot know
//! what a transaction will write until it has run. It works here because
//! §17.2.7's Command rule already evaluates every right-hand side and
//! every index **in the caller's region** and ships them as arguments —
//! and §17.7 records the expressiveness that was spent to buy it. So no
//! value in a handler's write set depends on a durable read, and the whole
//! batch is determined before the first write lands. The compiler's
//! knowledge of the write set is not decoration here; it is the reason a
//! non-interactive primitive is sufficient.

use crate::value::{Json, Number};
use crate::watch::Seq;

/// A key a handler read, and what it saw.
///
/// The recorded value rather than a version stamp, because that is the
/// check every candidate backend actually offers: Deno KV's `check()`
/// compares a versionstamp it hands out, DynamoDB's `ConditionExpression`
/// compares the attribute itself, and a local database compares whatever
/// it likes. A value comparison is the one form all three can express, and
/// the ABA case it cannot distinguish is not a case that matters: if the
/// key holds the same bytes the handler read, the computation it did on
/// them is still correct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub key: String,
    /// `None` if the key was absent when the handler looked.
    pub seen: Option<Json>,
}

/// One write a handler asked for.
///
/// Three variants and not five. §14B.2's five mutation verbs are the
/// *wire* contract (§18.2); §7.4's operations are what a backing store
/// must implement, and `append` and `remove` are read-modify-write over
/// `get` and `set` — which is why they arrive here as a [`Write::Set`]
/// paired with the [`Read`] that justifies it. Widening this enum would
/// put list semantics into every backend for nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Write {
    Set {
        key: String,
        value: Json,
    },
    /// A blind delta, deliberately: it carries no [`Read`], so two
    /// handlers incrementing one key never conflict and both are counted.
    /// That is §18.3's rejection of provisional writes, kept true under a
    /// transaction.
    Incr {
        key: String,
        delta: Number,
    },
    Delete {
        key: String,
    },
}

impl Write {
    pub fn key(&self) -> &str {
        match self {
            Write::Set { key, .. } | Write::Incr { key, .. } | Write::Delete { key } => key,
        }
    }
}

/// Everything one handler did to the store.
///
/// Empty is meaningful and common: a read endpoint records the keys it
/// read and writes nothing, and [`DurableStore::apply`](crate::DurableStore::apply)
/// is required to do nothing at all for it — no write transaction, no
/// sequence number, no announcement.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transaction {
    pub reads: Vec<Read>,
    pub writes: Vec<Write>,
}

impl Transaction {
    /// The one write a single command performs, as a whole transaction.
    pub fn of(write: Write) -> Transaction {
        Transaction {
            reads: Vec::new(),
            writes: vec![write],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    /// Every distinct key this transaction writes, in first-write order.
    ///
    /// This is the lock set, and it is finite and known before the first
    /// write — which is what makes acquisition order total and therefore
    /// deadlock-free for any implementation that needs locks.
    pub fn keys(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for write in &self.writes {
            if !seen.contains(&write.key()) {
                seen.push(write.key());
            }
        }
        seen
    }
}

/// What committing a transaction produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// The position of the last write in the store's total order.
    pub seq: Seq,
    /// The value each write left behind, in write order. `None` for a
    /// delete. This is what a command endpoint answers with, and it is
    /// read from the commit rather than projected before it — under
    /// contention a projection and the committed value differ, and the
    /// browser must be told the truth.
    pub values: Vec<Option<Json>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_set_lists_each_key_once_in_first_write_order() {
        let transaction = Transaction {
            reads: Vec::new(),
            writes: vec![
                Write::Incr {
                    key: "votes".to_string(),
                    delta: Number::new(1.0),
                },
                Write::Set {
                    key: "last".to_string(),
                    value: Json::from_text("1"),
                },
                Write::Incr {
                    key: "votes".to_string(),
                    delta: Number::new(1.0),
                },
            ],
        };
        assert_eq!(transaction.keys(), vec!["votes", "last"]);
    }

    #[test]
    fn a_transaction_that_writes_nothing_is_empty_however_much_it_read() {
        let transaction = Transaction {
            reads: vec![Read {
                key: "visits".to_string(),
                seen: Some(Json::from_text("3")),
            }],
            writes: Vec::new(),
        };
        assert!(transaction.is_empty());
    }
}
