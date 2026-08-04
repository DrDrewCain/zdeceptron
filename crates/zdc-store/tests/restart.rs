//! `durable` means it is still there afterwards.
//!
//! §5.5 makes `client` state ephemeral and `server` state ephemeral per
//! invocation, and §5.7 makes `durable` the one placement that outlives
//! both. §10 names the proof: the demo's vote counts "visibly surviving
//! reload". A reload only exercises the browser; these tests close the
//! database and reopen it, which is the stronger claim and the one that
//! catches a store that was only ever a hash map.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use zdc_store::{DurableStore, EmbeddedStore, Json, Number, Seq};

/// A path in the system temp directory that no other test is using.
///
/// Process id plus a counter: two tests in one run get different names,
/// and two runs of the suite do too.
fn scratch(name: &str) -> Scratch {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "zdc-store-{}-{}-{}.redb",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        name
    ));
    let _ = std::fs::remove_file(&path);
    Scratch { path }
}

/// Removes the file when the test ends, however it ends.
struct Scratch {
    path: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn a_count_survives_the_process_that_wrote_it() {
    let scratch = scratch("visits");

    {
        let store = EmbeddedStore::open(&scratch.path).expect("the store opens");
        store.incr("visits", Number::new(1.0)).expect("incr");
        store.incr("visits", Number::new(1.0)).expect("incr");
    }

    let reopened = EmbeddedStore::open(&scratch.path).expect("the store reopens");
    assert_eq!(
        reopened.get("visits").expect("get").map(Json::into_string),
        Some("2".to_string()),
        "durable state did not survive a restart"
    );
}

#[test]
fn incrementing_after_a_restart_continues_from_the_stored_value() {
    let scratch = scratch("continue");

    {
        let store = EmbeddedStore::open(&scratch.path).expect("the store opens");
        store.incr("visits", Number::new(41.0)).expect("incr");
    }

    let reopened = EmbeddedStore::open(&scratch.path).expect("the store reopens");
    let (value, _) = reopened.incr("visits", Number::new(1.0)).expect("incr");
    assert_eq!(value, Number::new(42.0));
}

#[test]
fn the_sequence_counter_survives_too() {
    // If the numbering restarted, a tab that reconnected across a restart
    // would resume from an id that now names a different write — which is
    // the duplicate §8.1 forbids, arriving as a silently wrong value.
    let scratch = scratch("seq");

    let before = {
        let store = EmbeddedStore::open(&scratch.path).expect("the store opens");
        store.set("a", Json::from_text("1")).expect("set");
        store.set("b", Json::from_text("2")).expect("set");
        store.latest()
    };
    assert_eq!(before, Seq(2));

    let reopened = EmbeddedStore::open(&scratch.path).expect("the store reopens");
    assert_eq!(reopened.latest(), before, "the counter restarted");
    assert_eq!(
        reopened.set("c", Json::from_text("3")).expect("set"),
        Seq(3),
        "the first write after a restart reused a sequence number"
    );
}

#[test]
fn a_deleted_key_stays_deleted() {
    let scratch = scratch("delete");

    {
        let store = EmbeddedStore::open(&scratch.path).expect("the store opens");
        store.set("visits", Json::from_text("7")).expect("set");
        store.delete("visits").expect("delete");
    }

    let reopened = EmbeddedStore::open(&scratch.path).expect("the store reopens");
    assert_eq!(reopened.get("visits").expect("get"), None);
    assert_eq!(reopened.keys().expect("keys"), Vec::<String>::new());
}

#[test]
fn concurrent_increments_are_all_counted() {
    // The two-window demo with the windows replaced by threads. §18.3
    // rejected provisional client-side writes on exactly this ground: two
    // visitors incrementing at once must both be counted, which is only
    // true if the read and the write are one operation in the store.
    let scratch = scratch("concurrent");
    let store = std::sync::Arc::new(EmbeddedStore::open(&scratch.path).expect("the store opens"));

    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = std::sync::Arc::clone(&store);
            std::thread::spawn(move || {
                for _ in 0..50 {
                    store.incr("visits", Number::new(1.0)).expect("incr");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a writer thread panicked");
    }

    assert_eq!(
        store.get("visits").expect("get").map(Json::into_string),
        Some("400".to_string()),
        "an increment was lost to a race"
    );
    assert_eq!(store.latest(), Seq(400), "a sequence number was reused");
}
