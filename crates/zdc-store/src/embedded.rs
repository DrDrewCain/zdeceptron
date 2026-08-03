//! The local implementation: an embedded database in one file.
//!
//! # Why `redb` and not SQLite
//!
//! §7.5 puts `#![forbid(unsafe_code)]` in every crate root and calls the
//! commitment "mechanical, not aspirational", and §7 promises a developer
//! installs one binary. `rusqlite` with the `bundled` feature compiles
//! ~250k lines of C into that binary, and without it links a system
//! library that has to already be there — the first breaks "no C in the
//! tree", the second breaks "one binary". Neither is a small compromise
//! for a component whose whole interface is five key-value operations.
//!
//! `turso` (the pure-Rust SQLite rewrite, formerly `limbo`) was the
//! obvious candidate and was rejected on three counts: it is a
//! pre-release (`0.8.0-pre.2`), its default features pull `mimalloc`,
//! which is C, and its API is async, which would put a runtime into a
//! blocking dev server that deliberately has none.
//!
//! `redb` is pure Rust with **no required dependencies at all**, is ACID
//! with MVCC, and stores a whole database in one file. What it does not
//! have is SQL — and this interface has no use for SQL. §7.4's five
//! operations are a key-value interface, §14G.5 sent relational
//! persistence back, and §14C.3a records that `durable` being key-value is
//! a structural limit of v1. A relational engine here would be a
//! dependency bought to serve a feature the language does not have.
//!
//! # Two backends, one implementation
//!
//! A test runs against the same code as a developer's `zdc dev`, on an
//! in-memory backend instead of a file. A separate `MemoryStore` would be
//! a second implementation of the interesting parts — the sequence
//! counter and the atomic increment — and the one under test would not be
//! the one that ships.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::txn::{Applied, Transaction, Write};
use crate::value::{Json, Number};
use crate::watch::{Fanout, Keys, Seq, Subscription, Update};
use crate::{DurableStore, StoreError};

/// Every `durable` key, as JSON text.
const VALUES: TableDefinition<&str, &str> = TableDefinition::new("values");

/// The store's own bookkeeping. One row today: the sequence counter.
///
/// Persisted rather than reset on start, because `Last-Event-ID` is
/// meaningless if the numbering restarts — a tab open across a server
/// restart would resume from an id that now names a different write.
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");

const SEQ: &str = "seq";

/// A durable store in one file, or in memory.
pub struct EmbeddedStore {
    db: Arc<Database>,
    fanout: Fanout,
}

impl EmbeddedStore {
    /// Open, or create, the database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<EmbeddedStore, StoreError> {
        let db = Database::create(path).map_err(backend)?;
        EmbeddedStore::wrap(db)
    }

    /// A store with no file behind it, for tests and for a `zdc dev` run
    /// that should leave nothing on disk.
    pub fn in_memory() -> Result<EmbeddedStore, StoreError> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(backend)?;
        EmbeddedStore::wrap(db)
    }

    fn wrap(db: Database) -> Result<EmbeddedStore, StoreError> {
        // Both tables are created up front so a read transaction never has
        // to cope with a table that does not exist yet. `get` on a store
        // nobody has written to is an ordinary `None`, not a special case.
        let txn = db.begin_write().map_err(backend)?;
        {
            txn.open_table(VALUES).map_err(backend)?;
            txn.open_table(META).map_err(backend)?;
        }
        txn.commit().map_err(backend)?;

        let latest = {
            let txn = db.begin_read().map_err(backend)?;
            let meta = txn.open_table(META).map_err(backend)?;
            let seq = meta.get(SEQ).map_err(backend)?.map(|slot| slot.value());
            Seq(seq.unwrap_or(0))
        };

        Ok(EmbeddedStore {
            db: Arc::new(db),
            fanout: Fanout::new(latest),
        })
    }

    /// Every key currently set, in key order.
    ///
    /// Not part of [`DurableStore`] — a listing is not one of §7.4's five
    /// and not every backing store can do it cheaply. It exists so a test
    /// can assert what survived a restart without knowing what to ask for.
    pub fn keys(&self) -> Result<Vec<String>, StoreError> {
        let txn = self.db.begin_read().map_err(backend)?;
        let values = txn.open_table(VALUES).map_err(backend)?;
        let mut out = Vec::new();
        for entry in values.iter().map_err(backend)? {
            let (key, _) = entry.map_err(backend)?;
            out.push(key.value().to_string());
        }
        Ok(out)
    }

    /// The position of the most recent write.
    pub fn latest(&self) -> Seq {
        self.fanout.latest()
    }

    pub fn subscribers(&self) -> usize {
        self.fanout.subscribers()
    }

    /// The next value of `key`, given what the transaction has already put
    /// there and what one write asks for.
    ///
    /// Split out so the whole batch is computed *before* any of it is
    /// written. A `NotANumber` halfway through must leave the store
    /// untouched, and the cheapest way to be sure of that is for the
    /// fallible part to finish before the writing part starts.
    fn next_value(write: &Write, current: Option<Json>) -> Result<Option<Json>, StoreError> {
        match write {
            Write::Set { value, .. } => Ok(Some(value.clone())),
            Write::Delete { .. } => Ok(None),
            Write::Incr { key, delta } => {
                let base = match &current {
                    None => Number::ZERO,
                    Some(json) => match Number::parse(json.as_str()) {
                        Some(number) => number,
                        None => {
                            return Err(StoreError::NotANumber {
                                key: key.clone(),
                                found: json.as_str().to_string(),
                            })
                        }
                    },
                };
                match base.plus(*delta) {
                    Some(sum) => Ok(Some(sum.to_json())),
                    None => Err(StoreError::OutOfRange { key: key.clone() }),
                }
            }
        }
    }
}

impl DurableStore for EmbeddedStore {
    fn get(&self, key: &str) -> Result<Option<Json>, StoreError> {
        let txn = self.db.begin_read().map_err(backend)?;
        let values = txn.open_table(VALUES).map_err(backend)?;
        Ok(values
            .get(key)
            .map_err(backend)?
            .map(|slot| Json::from_text(slot.value())))
    }

    /// One `redb` write transaction, which is the whole implementation.
    ///
    /// `redb` is ACID with MVCC and serialises writers, so conditions 1
    /// through 3 of the trait's contract fall out of `begin_write` and
    /// `commit`: nothing is visible until `commit`, an early return drops
    /// the transaction and `redb` discards it, and no second writer can be
    /// inside this one. This is the implementation the contract was
    /// written against — a target that has less has to work harder, and
    /// [`crate::txn`] says which and how much.
    ///
    /// The three phases are separated deliberately. **Check** the reads,
    /// **compute** every next value, then **write**. Computing before
    /// writing is what makes a `NotANumber` in the middle of a batch leave
    /// the store as it was rather than as it was becoming.
    fn apply(&self, transaction: &Transaction) -> Result<Applied, StoreError> {
        // Condition 4: a read-only invocation takes no write lock and
        // spends no sequence number. Every value-endpoint invocation lands
        // here, so this is the common case rather than an optimisation.
        if transaction.is_empty() {
            return Ok(Applied {
                seq: self.fanout.latest(),
                values: Vec::new(),
            });
        }

        let txn = self.db.begin_write().map_err(backend)?;
        let last;
        let mut updates: Vec<Update> = Vec::new();
        let mut values_out: Vec<Option<Json>> = Vec::new();
        {
            let mut values = txn.open_table(VALUES).map_err(backend)?;

            // Phase 1 — the reads still say what the handler was told they
            // said. Inside the write transaction, so nothing can slip
            // between the check and the write.
            for read in &transaction.reads {
                let current = values
                    .get(read.key.as_str())
                    .map_err(backend)?
                    .map(|slot| Json::from_text(slot.value()));
                if current != read.seen {
                    return Err(StoreError::Conflict {
                        key: read.key.clone(),
                    });
                }
            }

            // Phase 2 — every next value, in order, against a view that
            // carries this transaction's own earlier writes. `working` is
            // what makes `set` then `incr` on one key see the `set`.
            let mut working: BTreeMap<String, Option<Json>> = BTreeMap::new();
            let mut order: Vec<String> = Vec::new();
            for write in &transaction.writes {
                let key = write.key();
                let current = match working.get(key) {
                    Some(pending) => pending.clone(),
                    None => values
                        .get(key)
                        .map_err(backend)?
                        .map(|slot| Json::from_text(slot.value())),
                };
                let next = EmbeddedStore::next_value(write, current)?;
                if !working.contains_key(key) {
                    order.push(key.to_string());
                }
                working.insert(key.to_string(), next.clone());
                values_out.push(next);
            }

            // Phase 3 — write, one sequence number per key that changed.
            // Per key and not per transaction, because `Seq` is what a
            // reconnecting client resumes from and two updates sharing a
            // position would make "everything after N" ambiguous. A key
            // written twice in one transaction is announced once, at its
            // final value: the intermediate value was never a state any
            // reader could observe.
            let mut meta = txn.open_table(META).map_err(backend)?;
            let mut seq = Seq(meta
                .get(SEQ)
                .map_err(backend)?
                .map_or(0, |slot| slot.value()));
            for key in &order {
                seq = seq.next();
                let value = working.get(key).cloned().unwrap_or(None);
                match &value {
                    Some(json) => {
                        values
                            .insert(key.as_str(), json.as_str())
                            .map_err(backend)?;
                    }
                    None => {
                        values.remove(key.as_str()).map_err(backend)?;
                    }
                }
                updates.push(Update {
                    seq,
                    key: key.clone(),
                    value,
                });
            }
            meta.insert(SEQ, seq.0).map_err(backend)?;
            last = seq;
        }
        txn.commit().map_err(backend)?;

        // After the commit, never before: a subscriber told about a write
        // that then failed to commit would be showing a value no reader
        // can read. One call for the whole batch, so a transaction's
        // announcements reach a subscriber contiguously.
        self.fanout.publish_all(updates);
        Ok(Applied {
            seq: last,
            values: values_out,
        })
    }

    fn watch(&self, keys: &Keys, since: Option<Seq>) -> Subscription {
        self.fanout.subscribe(keys, since)
    }
}

fn backend(error: impl std::fmt::Display) -> StoreError {
    StoreError::Backend {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::Event;

    fn store() -> EmbeddedStore {
        EmbeddedStore::in_memory().expect("an in-memory store opens")
    }

    #[test]
    fn a_key_that_was_never_written_reads_as_absent() {
        assert_eq!(store().get("visits").expect("get"), None);
    }

    #[test]
    fn a_write_is_readable() {
        let store = store();
        store.set("visits", Json::from_text("7")).expect("set");
        assert_eq!(
            store.get("visits").expect("get").map(Json::into_string),
            Some("7".to_string())
        );
    }

    #[test]
    fn incrementing_an_absent_key_starts_from_zero() {
        // A `durable` signal always has a `starting` value, so the first
        // click before any write is ordinary rather than an error.
        let store = store();
        let (value, _) = store.incr("visits", Number::new(1.0)).expect("incr");
        assert_eq!(value, Number::new(1.0));
    }

    #[test]
    fn increments_accumulate() {
        let store = store();
        store.incr("visits", Number::new(1.0)).expect("incr");
        store.incr("visits", Number::new(1.0)).expect("incr");
        let (value, _) = store.incr("visits", Number::new(1.0)).expect("incr");
        assert_eq!(value, Number::new(3.0));
        assert_eq!(
            store.get("visits").expect("get").map(Json::into_string),
            Some("3".to_string()),
            "the stored text is what the browser will render"
        );
    }

    #[test]
    fn a_negative_delta_is_how_decr_is_spelled() {
        // §7.4 has five operations and `decr` is not one of them. The
        // adapter negates; the store does not grow a sixth method.
        let store = store();
        store.set("visits", Json::from_text("5")).expect("set");
        let (value, _) = store.incr("visits", Number::new(-2.0)).expect("incr");
        assert_eq!(value, Number::new(3.0));
    }

    #[test]
    fn incrementing_text_names_the_key_and_what_it_found() {
        let store = store();
        store.set("name", Json::from_text("\"ada\"")).expect("set");
        assert_eq!(
            store.incr("name", Number::new(1.0)),
            Err(StoreError::NotANumber {
                key: "name".to_string(),
                found: "\"ada\"".to_string()
            })
        );
    }

    #[test]
    fn a_refused_increment_leaves_the_value_and_the_counter_alone() {
        // The transaction is not committed, so neither the value nor the
        // sequence number moves. A counter that advanced on a failed write
        // would leave a hole that `Last-Event-ID` reads as a lost update.
        let store = store();
        store.set("name", Json::from_text("\"ada\"")).expect("set");
        let before = store.latest();
        assert!(store.incr("name", Number::new(1.0)).is_err());
        assert_eq!(store.latest(), before);
        assert_eq!(
            store.get("name").expect("get").map(Json::into_string),
            Some("\"ada\"".to_string())
        );
    }

    #[test]
    fn deleting_removes_the_key() {
        let store = store();
        store.set("visits", Json::from_text("7")).expect("set");
        store.delete("visits").expect("delete");
        assert_eq!(store.get("visits").expect("get"), None);
    }

    #[test]
    fn every_write_advances_the_sequence_by_one() {
        let store = store();
        assert_eq!(store.latest(), Seq(0));
        assert_eq!(store.set("a", Json::from_text("1")).expect("set"), Seq(1));
        assert_eq!(store.set("b", Json::from_text("2")).expect("set"), Seq(2));
        assert_eq!(store.incr("a", Number::new(1.0)).expect("incr").1, Seq(3));
        assert_eq!(store.delete("b").expect("delete"), Seq(4));
    }

    #[test]
    fn a_watcher_is_sent_the_value_rather_than_only_the_key() {
        // §17.2.5 fatal 4's `LiveValue` edge, which is what lets a second
        // window update without a round trip.
        let store = store();
        let mut window = store.watch(&Keys::new(["visits"]), None);
        store.incr("visits", Number::new(1.0)).expect("incr");
        assert_eq!(
            window.try_next(),
            Some(Event::Update(Update {
                seq: Seq(1),
                key: "visits".to_string(),
                value: Some(Json::from_text("1")),
            }))
        );
    }

    // --- the transaction ---------------------------------------------

    use crate::txn::{Read, Transaction};

    fn set(key: &str, json: &str) -> Write {
        Write::Set {
            key: key.to_string(),
            value: Json::from_text(json),
        }
    }

    fn incr(key: &str, delta: f64) -> Write {
        Write::Incr {
            key: key.to_string(),
            delta: Number::new(delta),
        }
    }

    fn held(store: &EmbeddedStore, key: &str) -> Option<String> {
        store.get(key).expect("get").map(Json::into_string)
    }

    #[test]
    fn a_batch_that_fails_part_way_leaves_every_earlier_write_unapplied() {
        // **The point of the whole feature.** Three writes, the third
        // impossible. Before the transaction the first two were committed
        // and stayed committed, which for a vote spread over eight keys is
        // corrupt data rather than a failed request.
        let store = store();
        store.set("name", Json::from_text("\"ada\"")).expect("set");
        let before = store.latest();

        let outcome = store.apply(&Transaction {
            reads: Vec::new(),
            writes: vec![
                incr("votes", 1.0),
                set("last", "\"ada\""),
                // `name` holds text, so this cannot be incremented.
                incr("name", 1.0),
            ],
        });

        assert_eq!(
            outcome,
            Err(StoreError::NotANumber {
                key: "name".to_string(),
                found: "\"ada\"".to_string()
            })
        );
        assert_eq!(held(&store, "votes"), None, "the first write was applied");
        assert_eq!(held(&store, "last"), None, "the second write was applied");
        assert_eq!(
            held(&store, "name"),
            Some("\"ada\"".to_string()),
            "the failing write changed the key it failed on"
        );
        assert_eq!(
            store.latest(),
            before,
            "a failed transaction spent a sequence number, leaving a hole a \
             reconnecting client reads as a lost update"
        );
    }

    #[test]
    fn a_watcher_hears_nothing_at_all_from_a_transaction_that_failed() {
        // Worse than a half-applied store: a second window told about a
        // write no reader can read.
        let store = store();
        store.set("name", Json::from_text("\"ada\"")).expect("set");
        // Subscribed after the setup write, so anything this window hears
        // came from the transaction below.
        let mut window = store.watch(&Keys::new(["votes", "last", "name"]), None);

        assert!(store
            .apply(&Transaction {
                reads: Vec::new(),
                writes: vec![
                    incr("votes", 1.0),
                    set("last", "\"ada\""),
                    incr("name", 1.0)
                ],
            })
            .is_err());

        assert_eq!(window.try_next(), None);
    }

    #[test]
    fn every_write_in_a_batch_lands_when_none_of_them_fails() {
        let store = store();
        let applied = store
            .apply(&Transaction {
                reads: Vec::new(),
                writes: vec![
                    incr("votes", 1.0),
                    set("last", "\"ada\""),
                    incr("total", 5.0),
                ],
            })
            .expect("the batch commits");
        assert_eq!(held(&store, "votes"), Some("1".to_string()));
        assert_eq!(held(&store, "last"), Some("\"ada\"".to_string()));
        assert_eq!(held(&store, "total"), Some("5".to_string()));
        assert_eq!(
            applied.values,
            vec![
                Some(Json::from_text("1")),
                Some(Json::from_text("\"ada\"")),
                Some(Json::from_text("5"))
            ],
            "the committed value of each write is what a command answers with"
        );
        assert_eq!(applied.seq, Seq(3), "one position per key that changed");
    }

    #[test]
    fn writes_in_a_batch_see_the_earlier_writes_of_the_same_batch() {
        // Contract condition 2. `set` then `incr` on one key must read the
        // `set`, not the value the key held before the transaction.
        let store = store();
        store.set("total", Json::from_text("100")).expect("set");
        store
            .apply(&Transaction {
                reads: Vec::new(),
                writes: vec![set("total", "0"), incr("total", 1.0), incr("total", 1.0)],
            })
            .expect("the batch commits");
        assert_eq!(held(&store, "total"), Some("2".to_string()));
    }

    #[test]
    fn a_key_written_twice_in_one_batch_is_announced_once_at_its_final_value() {
        // The intermediate value was never a state a reader could observe,
        // so announcing it would be announcing a state that never existed.
        let store = store();
        let mut window = store.watch(&Keys::new(["total"]), None);
        store
            .apply(&Transaction {
                reads: Vec::new(),
                writes: vec![set("total", "1"), set("total", "2"), set("total", "3")],
            })
            .expect("the batch commits");
        assert_eq!(
            window.try_next(),
            Some(Event::Update(Update {
                seq: Seq(1),
                key: "total".to_string(),
                value: Some(Json::from_text("3")),
            }))
        );
        assert_eq!(window.try_next(), None);
    }

    #[test]
    fn a_read_that_no_longer_holds_refuses_the_transaction_and_names_the_key() {
        // The compare-and-set half. This is what `append` and `remove`
        // stand on: they read a list, compute a new one, and must not
        // write it over somebody else's list.
        let store = store();
        store.set("names", Json::from_text("[]")).expect("set");
        let seen = store.get("names").expect("get");

        // Somebody else appends first.
        store
            .set("names", Json::from_text("[\"ada\"]"))
            .expect("set");

        assert_eq!(
            store.apply(&Transaction {
                reads: vec![Read {
                    key: "names".to_string(),
                    seen,
                }],
                writes: vec![set("names", "[\"grace\"]")],
            }),
            Err(StoreError::Conflict {
                key: "names".to_string()
            })
        );
        assert_eq!(
            held(&store, "names"),
            Some("[\"ada\"]".to_string()),
            "the stale write landed anyway and one append was lost"
        );
    }

    #[test]
    fn a_read_that_still_holds_lets_the_transaction_through() {
        let store = store();
        store.set("names", Json::from_text("[]")).expect("set");
        let seen = store.get("names").expect("get");
        store
            .apply(&Transaction {
                reads: vec![Read {
                    key: "names".to_string(),
                    seen,
                }],
                writes: vec![set("names", "[\"ada\"]")],
            })
            .expect("nothing changed underneath it");
        assert_eq!(held(&store, "names"), Some("[\"ada\"]".to_string()));
    }

    #[test]
    fn a_read_of_a_key_that_was_absent_and_now_is_not_is_a_conflict() {
        // The absent case is the one an implementation forgets, and it is
        // the first append to a fresh key — the most common one there is.
        let store = store();
        store
            .apply(&Transaction {
                reads: vec![Read {
                    key: "names".to_string(),
                    seen: None,
                }],
                writes: vec![set("names", "[\"ada\"]")],
            })
            .expect("absent is what it saw");

        assert_eq!(
            store.apply(&Transaction {
                reads: vec![Read {
                    key: "names".to_string(),
                    seen: None,
                }],
                writes: vec![set("names", "[\"grace\"]")],
            }),
            Err(StoreError::Conflict {
                key: "names".to_string()
            })
        );
    }

    #[test]
    fn a_transaction_that_writes_nothing_costs_nothing() {
        // Contract condition 4. Every read-endpoint invocation records the
        // keys it read and writes none of them; taking a write lock and
        // spending a sequence number for each would make reads serialise
        // behind writes and fill the backlog with nothing.
        let store = store();
        store.set("visits", Json::from_text("7")).expect("set");
        let before = store.latest();
        let mut window = store.watch(&Keys::new(["visits"]), None);

        let applied = store
            .apply(&Transaction {
                reads: vec![Read {
                    key: "visits".to_string(),
                    // Deliberately stale: a read-only transaction has
                    // nothing to protect, so it must not be refused.
                    seen: Some(Json::from_text("1")),
                }],
                writes: Vec::new(),
            })
            .expect("a read-only transaction always commits");

        assert_eq!(applied.seq, before);
        assert!(applied.values.is_empty());
        assert_eq!(store.latest(), before);
        assert_eq!(window.try_next(), None);
    }

    #[test]
    fn the_derived_operations_go_through_apply_and_keep_their_answers() {
        // `set`, `incr` and `delete` are provided methods over `apply`
        // now. If the derivation were wrong the whole existing suite would
        // move, so this asserts the shape of the answers rather than the
        // values: `incr` still reports the new number.
        let store = store();
        assert_eq!(store.set("a", Json::from_text("1")).expect("set"), Seq(1));
        assert_eq!(
            store.incr("a", Number::new(2.0)).expect("incr"),
            (Number::new(3.0), Seq(2))
        );
        assert_eq!(store.delete("a").expect("delete"), Seq(3));
        assert_eq!(held(&store, "a"), None);
    }

    #[test]
    fn a_delete_reaches_a_watcher_as_an_absent_value() {
        let store = store();
        store.set("visits", Json::from_text("7")).expect("set");
        let mut window = store.watch(&Keys::new(["visits"]), None);
        store.delete("visits").expect("delete");
        assert_eq!(
            window.try_next(),
            Some(Event::Update(Update {
                seq: Seq(2),
                key: "visits".to_string(),
                value: None,
            }))
        );
    }
}
