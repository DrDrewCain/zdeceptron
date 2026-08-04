use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use zdc_store::{
    DurableStore, EmbeddedStore, Json, Number, Read, Seq, StoreError, Transaction, Write,
};

static NEXT_STORE: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let serial = NEXT_STORE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "zdc-store-public-{}-{serial}-{name}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn store(name: &str) -> (Scratch, EmbeddedStore) {
    let scratch = Scratch::new(name);
    let store = EmbeddedStore::open(&scratch.0).expect("the store opens");
    (scratch, store)
}

#[test]
fn number_parsing_accepts_the_complete_json_number_grammar() {
    for (source, expected) in [
        ("0", 0.0),
        ("-0", -0.0),
        ("17", 17.0),
        ("-12.75", -12.75),
        ("1e3", 1_000.0),
        ("1E-3", 0.001),
        ("1e+3", 1_000.0),
        ("  4.5  ", 4.5),
    ] {
        assert_eq!(
            Number::parse(source),
            Some(Number::new(expected)),
            "{source:?}"
        );
    }
}

#[test]
fn number_parsing_rejects_rust_numbers_that_are_not_json_numbers() {
    for source in [
        "", "+1", ".5", "1.", "01", "-01", "1e", "1e+", "--1", "NaN", "inf", "Infinity",
    ] {
        assert_eq!(Number::parse(source), None, "{source:?} is not JSON");
    }
}

#[test]
fn transaction_keys_are_distinct_and_keep_first_write_order() {
    let transaction = Transaction {
        reads: Vec::new(),
        writes: vec![
            Write::Set {
                key: "b".to_string(),
                value: Json::from_text("1"),
            },
            Write::Incr {
                key: "a".to_string(),
                delta: Number::new(1.0),
            },
            Write::Delete {
                key: "b".to_string(),
            },
        ],
    };

    assert_eq!(transaction.keys(), ["b", "a"]);
    assert_eq!(transaction.writes[0].key(), "b");
    assert_eq!(transaction.writes[1].key(), "a");
    assert_eq!(transaction.writes[2].key(), "b");
}

#[test]
fn an_empty_transaction_spends_no_sequence_number() {
    let (_scratch, store) = store("empty");
    store.set("count", Json::from_text("1")).expect("seed");
    let before = store.latest();

    let applied = store
        .apply(&Transaction {
            reads: vec![Read {
                key: "count".to_string(),
                seen: Some(Json::from_text("1")),
            }],
            writes: Vec::new(),
        })
        .expect("a read-only transaction succeeds");

    assert_eq!(applied.seq, before);
    assert!(applied.values.is_empty());
    assert_eq!(store.latest(), before);
}

#[test]
fn writes_apply_in_order_and_report_each_intermediate_result() {
    let (_scratch, store) = store("ordered");
    let transaction = Transaction {
        reads: Vec::new(),
        writes: vec![
            Write::Set {
                key: "count".to_string(),
                value: Json::from_text("40"),
            },
            Write::Incr {
                key: "count".to_string(),
                delta: Number::new(2.0),
            },
        ],
    };

    let applied = store.apply(&transaction).expect("the transaction commits");
    assert_eq!(
        applied.seq,
        Seq(1),
        "one key written twice produces one observable update"
    );
    assert_eq!(
        applied.values,
        vec![Some(Json::from_text("40")), Some(Json::from_text("42"))]
    );
    assert_eq!(
        store.get("count").expect("get"),
        Some(Json::from_text("42"))
    );
}

#[test]
fn a_late_type_error_rolls_back_every_earlier_write() {
    let (_scratch, store) = store("rollback");
    store
        .set("text", Json::from_text("\"hello\""))
        .expect("seed text");
    let before = store.latest();
    let transaction = Transaction {
        reads: Vec::new(),
        writes: vec![
            Write::Set {
                key: "first".to_string(),
                value: Json::from_text("1"),
            },
            Write::Incr {
                key: "text".to_string(),
                delta: Number::new(1.0),
            },
        ],
    };

    assert!(matches!(
        store.apply(&transaction),
        Err(StoreError::NotANumber { ref key, .. }) if key == "text"
    ));
    assert_eq!(store.get("first").expect("get"), None);
    assert_eq!(store.latest(), before);
}

#[test]
fn a_stale_read_conflicts_without_applying_any_write() {
    let (_scratch, store) = store("conflict");
    store.set("version", Json::from_text("2")).expect("seed");
    let before = store.latest();
    let transaction = Transaction {
        reads: vec![Read {
            key: "version".to_string(),
            seen: Some(Json::from_text("1")),
        }],
        writes: vec![Write::Set {
            key: "result".to_string(),
            value: Json::from_text("99"),
        }],
    };

    assert_eq!(
        store.apply(&transaction),
        Err(StoreError::Conflict {
            key: "version".to_string()
        })
    );
    assert_eq!(store.get("result").expect("get"), None);
    assert_eq!(store.latest(), before);
}

#[test]
fn store_errors_explain_the_key_and_the_recovery_relevant_cause() {
    // Paired with what each one has to say rather than asserted as a
    // disjunction over both: only `Backend` has no key to name, and only it
    // carries the backend's own words, so a single `a || b` would have been
    // satisfied by every variant rendering the same half.
    let errors = [
        (
            StoreError::Backend {
                message: "disk full".to_string(),
            },
            "disk full",
        ),
        (
            StoreError::NotANumber {
                key: "visits".to_string(),
                found: "\"many\"".to_string(),
            },
            "visits",
        ),
        (
            StoreError::OutOfRange {
                key: "visits".to_string(),
            },
            "visits",
        ),
        (
            StoreError::Conflict {
                key: "visits".to_string(),
            },
            "visits",
        ),
    ];

    for (error, expected) in errors {
        let rendered = error.to_string();
        assert!(!rendered.is_empty());
        assert!(
            rendered.contains(expected),
            "expected {expected:?} in: {rendered}"
        );
    }
}
