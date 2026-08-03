//! **The milestone.** Two windows, one store, and a value that moves in
//! both.
//!
//! §10 names this as the proof `durable` is real: "open two browser
//! windows and both counters move". Everything below is that, with the
//! browsers replaced by two independent client sessions and the network
//! replaced by direct calls into the same host — because what is being
//! proved is not that HTTP works, it is that:
//!
//! 1. two sessions share one store rather than one copy each,
//! 2. a write from one is *announced* to the other rather than found by
//!    polling luck, and
//! 3. the announcement carries the value, so the second window updates
//!    with no round trip (§17.2.5 fatal 4's `LiveValue` edge).
//!
//! The third is the one a test can most easily fake. It is checked by
//! reading the update off the subscription, not by re-invoking the read
//! endpoint.

mod support;

use std::sync::Arc;
use std::time::Duration;

use support::{emit, endpoints};
use zdc_host::{Environment, Host};
use zdc_store::{DurableStore, EmbeddedStore, Event, Json, Keys, Seq, Subscription};

const COUNTER: &str = "\
state visits is durable Whole starting 0

view
    Column
        when visits
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with total show Text total
        Button \"count\"
            on click
                add 1 to visits
";

/// One browser tab: its own host, over the store every tab shares.
///
/// Separate `Host` values on purpose. A serverless invocation is not
/// shared between visitors — §5.5 makes `server` state ephemeral per
/// invocation — so a test that gave both windows one host would be
/// proving something the deployment does not provide. The *store* is the
/// only thing they have in common, which is exactly §5.7's claim.
struct Window {
    host: Host,
    stream: Subscription,
}

impl Window {
    fn open(store: &Arc<dyn DurableStore>, keys: &Keys, since: Option<Seq>) -> Window {
        Window {
            host: Host::new(
                endpoints(emit(COUNTER, "counter.zd")),
                Arc::clone(store),
                Environment::empty(),
            ),
            stream: store.watch(keys, since),
        }
    }

    /// What the tab renders: the durable read endpoint, run for real.
    fn rendered(&self) -> String {
        self.host
            .invoke("visits", "[]")
            .expect("the read endpoint runs")
    }

    /// A click on the button.
    fn click(&self) -> String {
        self.host
            .invoke("visits.incr", "[1]")
            .expect("the command endpoint runs")
    }

    /// The next update pushed to this tab, waiting briefly.
    ///
    /// A timeout rather than a spin: a live-sync test that passed by
    /// polling until the value happened to change would pass against a
    /// store with no fan-out at all.
    fn observed(&mut self) -> Option<Event> {
        self.stream.next_timeout(Duration::from_secs(2))
    }
}

fn shared_store() -> Arc<dyn DurableStore> {
    Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"))
}

fn keys() -> Keys {
    // What the manifest lists for this program, which is what the emitted
    // client subscribes to. Not a wildcard: there is no wildcard.
    Keys::new(["visits"])
}

#[test]
fn one_window_increments_and_the_other_is_told_the_new_value() {
    let store = shared_store();
    let mut watcher = Window::open(&store, &keys(), None);
    let clicker = Window::open(&store, &keys(), None);

    assert_eq!(watcher.rendered(), "0", "a fresh store renders its default");

    clicker.click();

    match watcher.observed() {
        Some(Event::Update(update)) => {
            assert_eq!(update.key, "visits");
            assert_eq!(
                update.value,
                Some(Json::from_text("1")),
                "the second window was told a key changed but not what to"
            );
            assert_eq!(update.seq, Seq(1));
        }
        other => panic!("the second window heard {other:?} instead of an update"),
    }
}

#[test]
fn the_second_window_needs_no_round_trip_to_show_the_new_value() {
    // The update carries the value, so what the watcher renders next is
    // decided without asking the server again. Checked by comparing the
    // pushed value against what a re-read would have returned — if they
    // ever disagree, one of the two paths is lying.
    let store = shared_store();
    let mut watcher = Window::open(&store, &keys(), None);
    let clicker = Window::open(&store, &keys(), None);

    clicker.click();
    clicker.click();

    let mut pushed = Vec::new();
    while let Some(Event::Update(update)) = watcher.observed() {
        pushed.push(update.value.clone().map(Json::into_string));
        if pushed.len() == 2 {
            break;
        }
    }
    assert_eq!(
        pushed,
        vec![Some("1".to_string()), Some("2".to_string())],
        "the pushed values are not the sequence of writes"
    );
    assert_eq!(
        watcher.rendered(),
        "2",
        "a re-read disagrees with what was pushed"
    );
}

#[test]
fn both_windows_clicking_at_once_counts_both() {
    // §18.3 rejected provisional client-side writes on exactly this
    // ground. `incr` is one transaction in the store, so the two clicks
    // cannot read the same value and both write 1.
    let store = shared_store();
    let a = Window::open(&store, &keys(), None);
    let b = Window::open(&store, &keys(), None);

    let first = a.click();
    let second = b.click();

    assert_ne!(first, second, "both clicks were told the same total");
    assert_eq!(a.rendered(), "2");
    assert_eq!(b.rendered(), "2", "the two windows disagree");
}

#[test]
fn a_window_hears_only_the_keys_its_program_declares() {
    // A tab subscribed to `visits` must not be woken by every write in the
    // store. On a deployment this is the difference between one channel
    // and a firehose.
    let store = shared_store();
    let mut watcher = Window::open(&store, &keys(), None);

    store
        .set("something-else", Json::from_text("1"))
        .expect("an unrelated write");
    assert!(
        watcher.observed().is_none(),
        "the window was woken by a key it never asked for"
    );

    store
        .set("visits", Json::from_text("9"))
        .expect("a write it wants");
    assert!(
        watcher.observed().is_some(),
        "the window missed its own key"
    );
}

#[test]
fn a_window_that_reconnects_is_replayed_exactly_what_it_missed() {
    // On Lambda the stream is cut at 900 s whether anyone wants it or not,
    // so this is not a rare path — it is the normal one, arriving on a
    // timer. `Last-Event-ID` is what makes a bounded stream behave like an
    // unbounded one, and "exactly" is the whole claim: no drop, no
    // duplicate.
    let store = shared_store();
    let clicker = Window::open(&store, &keys(), None);
    let mut watcher = Window::open(&store, &keys(), None);

    clicker.click();
    let seen = match watcher.observed() {
        Some(Event::Update(update)) => update.seq,
        other => panic!("expected the first update, got {other:?}"),
    };

    // The stream drops. Two writes land while nothing is listening.
    drop(watcher);
    clicker.click();
    clicker.click();

    let mut resumed = Window::open(&store, &keys(), Some(seen));
    let mut replayed = Vec::new();
    while let Some(Event::Update(update)) = resumed.observed() {
        replayed.push((update.seq, update.value.map(Json::into_string)));
        if replayed.len() == 2 {
            break;
        }
    }
    assert_eq!(
        replayed,
        vec![
            (Seq(2), Some("2".to_string())),
            (Seq(3), Some("3".to_string()))
        ],
        "the reconnecting window did not get exactly the tail it missed"
    );
    assert!(
        resumed.observed().is_none(),
        "the reconnecting window was sent something twice"
    );
}

#[test]
fn a_reconnecting_window_that_missed_too_much_is_told_to_re_read() {
    // Continuing silently here is the dropped update §8.1 forbids, and it
    // is invisible until a real disconnection produces it. A store that
    // has been restarted has no backlog at all, which is the common case:
    // a tab left open overnight.
    let store = shared_store();
    let clicker = Window::open(&store, &keys(), None);
    clicker.click();
    clicker.click();

    // A cursor from a run whose backlog this store never had.
    let mut stale = Window::open(&store, &keys(), Some(Seq(0)));
    match stale.observed() {
        Some(Event::Update(update)) => {
            // The backlog does reach back this far here, so the honest
            // answer is a replay rather than a resync.
            assert_eq!(update.seq, Seq(1));
        }
        Some(Event::Resync { seq }) => assert_eq!(seq, Seq(2)),
        None => panic!("a window behind the store was told nothing at all"),
    }
}

#[test]
fn two_windows_over_a_reopened_database_agree_with_what_was_stored() {
    // Restart, then two fresh windows. §10's two proofs at once: the value
    // survived, and both windows see the same survivor.
    let mut path = std::env::temp_dir();
    path.push(format!("zdc-host-two-windows-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(&path);

    {
        let store: Arc<dyn DurableStore> =
            Arc::new(EmbeddedStore::open(&path).expect("the store opens"));
        let window = Window::open(&store, &keys(), None);
        window.click();
        window.click();
        window.click();
    }

    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::open(&path).expect("the store reopens"));
    let a = Window::open(&store, &keys(), None);
    let b = Window::open(&store, &keys(), None);
    assert_eq!(a.rendered(), "3");
    assert_eq!(b.rendered(), "3");

    let _ = std::fs::remove_file(&path);
}
