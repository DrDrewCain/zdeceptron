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

use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::value::{Json, Number};
use crate::watch::{Fanout, Seq, Subscription, Update};
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

    /// One committed write: bump the counter, apply `change`, announce it.
    ///
    /// Every mutation goes through here so the counter cannot advance
    /// without a write landing, and a write cannot land without the
    /// counter advancing. Announcing happens after the commit — a
    /// subscriber told about a write that then failed to commit would be
    /// showing a value no reader can read.
    fn commit<F>(&self, key: &str, change: F) -> Result<(Option<Json>, Seq), StoreError>
    where
        F: FnOnce(Option<Json>) -> Result<Option<Json>, StoreError>,
    {
        let txn = self.db.begin_write().map_err(backend)?;
        let seq;
        let next;
        {
            let mut meta = txn.open_table(META).map_err(backend)?;
            let current = meta.get(SEQ).map_err(backend)?.map(|slot| slot.value());
            seq = Seq(current.unwrap_or(0)).next();
            meta.insert(SEQ, seq.0).map_err(backend)?;

            let mut values = txn.open_table(VALUES).map_err(backend)?;
            let before = values
                .get(key)
                .map_err(backend)?
                .map(|slot| Json::from_text(slot.value()));
            next = change(before)?;
            match &next {
                Some(value) => {
                    values.insert(key, value.as_str()).map_err(backend)?;
                }
                None => {
                    values.remove(key).map_err(backend)?;
                }
            }
        }
        txn.commit().map_err(backend)?;

        self.fanout.publish(Update {
            seq,
            key: key.to_string(),
            value: next.clone(),
        });
        Ok((next, seq))
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

    fn set(&self, key: &str, value: Json) -> Result<Seq, StoreError> {
        let (_, seq) = self.commit(key, |_| Ok(Some(value)))?;
        Ok(seq)
    }

    fn incr(&self, key: &str, delta: Number) -> Result<(Number, Seq), StoreError> {
        let mut result = Number::ZERO;
        let (_, seq) = self.commit(key, |before| {
            let current = match &before {
                None => Number::ZERO,
                Some(json) => match Number::parse(json.as_str()) {
                    Some(number) => number,
                    None => {
                        return Err(StoreError::NotANumber {
                            key: key.to_string(),
                            found: json.as_str().to_string(),
                        })
                    }
                },
            };
            match current.plus(delta) {
                Some(sum) => {
                    result = sum;
                    Ok(Some(sum.to_json()))
                }
                None => Err(StoreError::OutOfRange {
                    key: key.to_string(),
                }),
            }
        })?;
        Ok((result, seq))
    }

    fn delete(&self, key: &str) -> Result<Seq, StoreError> {
        let (_, seq) = self.commit(key, |_| Ok(None))?;
        Ok(seq)
    }

    fn watch(&self, prefix: &str, since: Option<Seq>) -> Subscription {
        self.fanout.subscribe(prefix, since)
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
        let mut window = store.watch("", None);
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

    #[test]
    fn a_delete_reaches_a_watcher_as_an_absent_value() {
        let store = store();
        store.set("visits", Json::from_text("7")).expect("set");
        let mut window = store.watch("", None);
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
