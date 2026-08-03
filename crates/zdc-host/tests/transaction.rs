//! **The gap this closes.** A handler that writes several durable keys
//! used to half-apply: the writes were awaited and ordered and the first
//! failure stopped the rest, but everything before the failure had already
//! committed.
//!
//! §14G.7.4 puts the transaction boundary on the handler, and the
//! milestone-12 target says why it matters concretely: one JudgeHuman vote
//! is roughly 25 operations across 8 tables. A vote that applied 11 of them
//! is not a failed request, it is corrupt data — and nothing in the
//! program can tell afterwards that it happened.
//!
//! # The guarantee these tests are the evidence for
//!
//! **Every durable write one event handler performs commits together, in
//! source order, or none of them does; and no concurrent handler observes
//! a state in between.**
//!
//! What that sentence does *not* say is as load-bearing as what it does.
//! It says nothing about client signals written in the same handler —
//! those are browser-local and nothing can roll them back. It says nothing
//! about two handlers' writes being one transaction; the unit is one
//! handler. And it says nothing about a live subscriber seeing the keys
//! arrive simultaneously — the store is atomic, the fan-out announces one
//! key at a time, and `DurableStore::apply` documents that seam rather
//! than glossing it.
//!
//! # Which targets can honour it
//!
//! Durable Objects and a local database: fully, with a real transaction.
//! Deno KV: fully, via `atomic()` — the recorded reads are `check()`s and
//! the writes are the mutation list, within documented caps of 100 checks
//! and 1000 mutations. DynamoDB: fully, via `TransactWriteItems` with a
//! `ConditionExpression` per read, at double the write cost and inside a
//! cap. **Cloudflare KV: not at all**, because one write per second per
//! key rules out the batch and the counter under it. That is a store this
//! language cannot back `durable` with, and it is written down rather than
//! silently downgraded.

mod support;

use std::sync::Arc;

use support::{emit, endpoints};
use zdc_host::{Environment, Host};
use zdc_store::{DurableStore, EmbeddedStore, Event, Json, Keys, Number, Seq};

/// A program with three durable keys and one handler that writes all
/// three, which is the shape a vote has and the shape that used to
/// half-apply.
///
/// Every command these tests send is an endpoint this program actually
/// emits. That is not pedantry: an unknown endpoint is refused before
/// anything runs, so a test that made its middle write fail by naming an
/// endpoint the build does not have would pass without the transaction
/// existing at all.
const BALLOT: &str = "\
state votes is durable Whole starting 0
state total is durable Whole starting 0
state winner is durable Text starting \"\"

view
    Column
        when votes
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with count show Text count
        Button \"vote\"
            on click
                add 1 to votes
                add 1 to total
                set winner to \"ada\"
";

const LIST: &str = "\
state names is durable List of Text starting []

view
    Column
        when names
            Loading         show Spinner
            Failed with e   show ErrorBar message is e.message
            Ready with held show Text \"ok\"
        Button \"sign\"
            on click
                append \"ada\" to names
";

/// Concrete rather than `Arc<dyn DurableStore>`, because `latest()` is not
/// one of the trait's operations — a sequence position is bookkeeping the
/// local store exposes for tests, not something every backing store has to
/// grow a method for.
fn store() -> Arc<EmbeddedStore> {
    Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"))
}

fn host_for(source: &str, store: &Arc<EmbeddedStore>) -> Host {
    Host::new(
        endpoints(emit(source, "transaction.zd")),
        Arc::clone(store) as Arc<dyn DurableStore>,
        Environment::empty(),
    )
}

/// The three commands one click on `BALLOT`'s button asks for, in source
/// order. This is the list the emitted handler builds and `$atomic` posts;
/// building it here rather than parsing the emitted JavaScript keeps the
/// test about the transaction rather than about the emitter, which
/// `zdc-codegen`'s own tests cover.
fn one_click(winner: &str) -> Vec<(String, String)> {
    vec![
        ("votes.incr".to_string(), "[1]".to_string()),
        ("total.incr".to_string(), "[1]".to_string()),
        ("winner.set".to_string(), format!("[{winner}]")),
    ]
}

fn held(store: &Arc<EmbeddedStore>, key: &str) -> Option<String> {
    store.get(key).expect("get").map(Json::into_string)
}

/// Push `total` to the largest double, so the next increment of the same
/// size leaves the range JSON can carry.
///
/// This is how a real store refusal is produced through real endpoints:
/// `total.incr` is emitted by `BALLOT`, the arguments are the ones
/// §17.2.7 says the browser evaluates and ships, and the failure comes
/// from the store rather than from the test.
fn saturate(host: &Host) {
    host.invoke("total.incr", "[1.7976931348623157e308]")
        .expect("one increment to the top of the range is fine");
}

/// One click whose second write cannot be applied.
fn overflowing_click() -> Vec<(String, String)> {
    vec![
        ("votes.incr".to_string(), "[1]".to_string()),
        (
            "total.incr".to_string(),
            "[1.7976931348623157e308]".to_string(),
        ),
        ("winner.set".to_string(), "[\"ada\"]".to_string()),
    ]
}

#[test]
fn a_handler_that_fails_part_way_applies_none_of_its_writes() {
    // **The acceptance test, and the whole point of the feature.** Three
    // writes; the second cannot be applied. The first is perfectly
    // ordinary and used to commit and stay committed, which for a vote
    // spread over eight keys is corrupt data rather than a failed request.
    let store = store();
    let host = host_for(BALLOT, &store);

    saturate(&host);
    let before = store.latest();
    let total_before = held(&store, "total");

    let outcome = host.invoke_all(&overflowing_click());

    assert!(outcome.is_err(), "the handler reported success");
    assert_eq!(
        held(&store, "votes"),
        None,
        "the write before the failure survived it"
    );
    assert_eq!(
        held(&store, "total"),
        total_before,
        "the failing write changed the key it failed on"
    );
    assert_eq!(
        held(&store, "winner"),
        None,
        "a write after the failure was applied"
    );
    assert_eq!(
        store.latest(),
        before,
        "a failed handler spent sequence numbers, so a reconnecting window \
         resumes past positions that name no write"
    );
}

#[test]
fn a_window_watching_hears_nothing_from_a_handler_that_failed() {
    // Worse than a half-applied store: a second window told a key changed
    // to a value no reader can read.
    let store = store();
    let host = host_for(BALLOT, &store);
    saturate(&host);

    // Subscribed after the setup, so anything heard came from the handler.
    let mut window = store.watch(&Keys::new(["votes", "total", "winner"]), None);

    assert!(host.invoke_all(&overflowing_click()).is_err());

    assert_eq!(window.try_next(), None);
}

#[test]
fn a_handler_whose_writes_all_succeed_applies_every_one_of_them() {
    // The other half of all-or-nothing, and the one it is easy to break
    // while fixing the first.
    let store = store();
    let host = host_for(BALLOT, &store);

    host.invoke_all(&one_click("\"ada\""))
        .expect("the handler commits");

    assert_eq!(held(&store, "votes"), Some("1".to_string()));
    assert_eq!(held(&store, "total"), Some("1".to_string()));
    assert_eq!(held(&store, "winner"), Some("\"ada\"".to_string()));
    assert_eq!(store.latest(), Seq(3), "one position per key written");
}

#[test]
fn a_single_write_is_the_same_path_and_answers_with_what_committed() {
    // No special case for one write: `$call` is a one-element transaction.
    // The answer is the committed value, which is what the two-window
    // milestone reads off a click.
    let store = store();
    let host = host_for(BALLOT, &store);
    assert_eq!(
        host.invoke_all(&[("votes.incr".to_string(), "[1]".to_string())])
            .expect("one write commits"),
        "1"
    );
    assert_eq!(
        host.invoke("votes.incr", "[1]").expect("and again"),
        "2",
        "`invoke` and `invoke_all` disagree about the same write"
    );
}

#[test]
fn an_unknown_endpoint_in_a_batch_applies_none_of_the_batch() {
    // Resolution happens before anything runs, so a stale tab posting a
    // renamed endpoint alongside three live ones does not get three
    // quarters of its handler.
    let store = store();
    let host = host_for(BALLOT, &store);

    let outcome = host.invoke_all(&[
        ("votes.incr".to_string(), "[1]".to_string()),
        ("gone.incr".to_string(), "[1]".to_string()),
    ]);

    assert!(matches!(outcome, Err(zdc_host::HostError::Unknown { .. })));
    assert_eq!(held(&store, "votes"), None);
}

#[test]
fn a_read_endpoint_cannot_be_smuggled_into_a_write_batch() {
    // The batch body is attacker-controlled. A value endpoint takes a
    // named object and a command takes a positional array, so calling one
    // through the other binds every input to `undefined` and returns a
    // plausible wrong answer — the failure `Shape` exists to prevent.
    let store = store();
    let host = host_for(BALLOT, &store);
    assert!(matches!(
        host.invoke_all(&[("votes".to_string(), "[]".to_string())]),
        Err(zdc_host::HostError::BadRequest { .. })
    ));
}

#[test]
fn two_handlers_incrementing_one_key_are_both_counted() {
    // The concurrent case, for the verb that must never conflict. `incr`
    // records no read, so it is a blind delta: two handlers writing the
    // same key do not race and neither is retried. §18.3 rejected
    // provisional client-side writes on exactly this ground, and a
    // transaction must not quietly reintroduce the problem by turning
    // every increment into a compare-and-set.
    let store = store();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                let host = host_for(BALLOT, &store);
                for _ in 0..5 {
                    host.invoke_all(&one_click("\"ada\""))
                        .expect("a click commits");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a window finished");
    }

    assert_eq!(held(&store, "votes"), Some("40".to_string()));
    assert_eq!(
        held(&store, "total"),
        Some("40".to_string()),
        "the two keys of one handler disagree, so some handler applied one \
         write and not the other"
    );
}

#[test]
fn two_handlers_appending_to_one_list_both_land() {
    // The concurrent case for the verb that *must* conflict. `append` is
    // read-modify-write, so without a check the two would read the same
    // list and one append would be lost — which is what the adapter
    // prelude used to admit in a comment. The recorded read makes the
    // stale one a conflict, and the invocation is re-run.
    let store = store();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                let host = host_for(LIST, &store);
                for _ in 0..5 {
                    host.invoke("names.append", "[\"ada\"]")
                        .expect("an append commits");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a window finished");
    }

    let held = held(&store, "names").expect("the list exists");
    assert_eq!(
        held.matches("\"ada\"").count(),
        20,
        "appends were lost to a lost update: {held}"
    );
}

#[test]
fn a_handler_reading_and_writing_one_key_twice_keeps_both_changes() {
    // Read-your-own-writes inside the transaction. Two appends in one
    // handler must not both read the pre-transaction list, or the second
    // silently discards the first — a half-apply inside a single
    // transaction.
    let store = store();
    let host = host_for(LIST, &store);
    host.invoke_all(&[
        ("names.append".to_string(), "[\"ada\"]".to_string()),
        ("names.append".to_string(), "[\"grace\"]".to_string()),
    ])
    .expect("the handler commits");
    assert_eq!(
        held(&store, "names"),
        Some("[\"ada\",\"grace\"]".to_string())
    );
}

#[test]
fn a_key_written_twice_by_one_handler_is_announced_once() {
    // A watcher must not be shown a value that was never a committed
    // state. The intermediate list existed only inside the transaction.
    let store = store();
    let host = host_for(LIST, &store);
    let mut window = store.watch(&Keys::new(["names"]), None);

    host.invoke_all(&[
        ("names.append".to_string(), "[\"ada\"]".to_string()),
        ("names.append".to_string(), "[\"grace\"]".to_string()),
    ])
    .expect("the handler commits");

    match window.try_next() {
        Some(Event::Update(update)) => assert_eq!(
            update.value,
            Some(Json::from_text("[\"ada\",\"grace\"]")),
            "an intermediate value reached a window"
        ),
        other => panic!("expected one update, got {other:?}"),
    }
    assert_eq!(window.try_next(), None, "the same key was announced twice");
}

#[test]
fn the_body_the_browser_posts_commits_or_does_not() {
    // End to end through the shape that actually crosses the wire. The
    // literal below is what the emitted handler's `$tx` stringifies to —
    // `$tx.push(['votes.incr', [1]])` three times — so this is the request
    // `POST /_zd/~atomic` carries.
    let store = store();
    let host = host_for(BALLOT, &store);

    host.invoke_batch("[[\"votes.incr\",[1]],[\"total.incr\",[1]],[\"winner.set\",[\"ada\"]]]")
        .expect("the transaction commits");
    assert_eq!(held(&store, "votes"), Some("1".to_string()));
    assert_eq!(held(&store, "winner"), Some("\"ada\"".to_string()));

    saturate(&host);
    let before = held(&store, "votes");
    assert!(host
        .invoke_batch("[[\"votes.incr\",[1]],[\"total.incr\",[1.7976931348623157e308]]]")
        .is_err());
    assert_eq!(
        held(&store, "votes"),
        before,
        "a write before the failure survived it"
    );
}

#[test]
fn a_body_that_is_not_a_transaction_is_a_bad_request_and_writes_nothing() {
    let store = store();
    let host = host_for(BALLOT, &store);
    for body in ["", "null", "[[\"votes.incr\"]]"] {
        assert!(
            matches!(
                host.invoke_batch(body),
                Err(zdc_host::HostError::BadRequest { .. })
            ),
            "`{body}` was accepted"
        );
    }
    assert_eq!(store.latest(), Seq(0));
}

#[test]
fn a_read_only_invocation_records_no_write_and_moves_nothing() {
    // Every value-endpoint request takes this path. If a read spent a
    // sequence number, a page with three durable signals would flood every
    // other window's backlog on load.
    let store = store();
    store
        .set("votes", Json::from_text("7"))
        .expect("a value to read");
    let before = store.latest();
    let host = host_for(BALLOT, &store);

    assert_eq!(host.invoke("votes", "[]").expect("the read runs"), "7");
    assert_eq!(store.latest(), before);
}

#[test]
fn an_empty_batch_is_not_an_error_and_writes_nothing() {
    // A handler whose only durable write is inside an `if` that did not
    // fire posts an empty list. Refusing it would turn a correct program
    // into a reported failure.
    let store = store();
    let host = host_for(BALLOT, &store);
    assert_eq!(
        host.invoke_all(&[]).expect("nothing is not a failure"),
        "null"
    );
    assert_eq!(store.latest(), Seq(0));
}

#[test]
fn the_failure_a_handler_reports_still_names_the_key_and_what_it_found() {
    // Deferring the writes must not flatten the diagnosis. A developer
    // reading the red bar needs the key and what went wrong with it, not
    // "the transaction failed".
    let store = store();
    let host = host_for(BALLOT, &store);
    saturate(&host);

    let message = host
        .invoke_all(&overflowing_click())
        .expect_err("the range cannot hold it")
        .to_string();
    assert!(
        message.contains("total") && message.contains("range"),
        "the diagnosis lost the key or the reason: {message}"
    );
}

#[test]
fn the_store_and_the_host_agree_about_what_one_click_did() {
    // Cross-check: the value the host answers with is the value a
    // subsequent read returns. If the projected answer were ever returned
    // instead of the committed one, these would disagree under contention
    // and agree in a single-threaded test — so this is checked against the
    // store rather than against a second projection.
    let store = store();
    let host = host_for(BALLOT, &store);
    for expected in 1..=3 {
        let answered = host
            .invoke("votes.incr", "[1]")
            .expect("the click commits")
            .parse::<f64>()
            .expect("a number came back");
        assert_eq!(answered, f64::from(expected));
        let (read, _) = store.incr("votes", Number::ZERO).expect("read it back");
        assert_eq!(read.as_f64(), answered);
    }
}
