//! Cross-region writes: one handler, one transaction.
//!
//! # The two bugs these exist to keep fixed
//!
//! A handler with three durable writes first emitted
//!
//! ```js
//! on($n, 'click', () => {
//!   $call('visits.incr', 1);
//!   $call('votes.incr', 1);
//!   $call('total.incr', 1);
//! });
//! ```
//!
//! Three promises created and thrown away. The requests could land in any
//! order, so `set x to 0` followed by `add 1 to x` was a race with itself.
//! Nothing could see any of them fail. And the second failing did not stop
//! the third, so a handler could half-apply and say nothing.
//!
//! Awaiting them fixed the order and the reporting and left the third
//! problem standing: three requests are three store operations, so the
//! second failing left the first committed with nothing to undo it. For a
//! vote spread over eight keys that is corrupt data, not a failed request.
//!
//! # What ships now
//!
//! Each write pushes `[endpoint, args]` into the handler's `$tx`, and one
//! `await $atomic($tx)` sends the whole list, which the server commits in a
//! single store transaction. **Every durable write one handler performs
//! commits together, in source order, or none of them does.** Three writes
//! are now one request, which is also why
//! `a_failed_transaction_sends_nothing_else` replaced the test that used to
//! pin the half-apply.
//!
//! It cost no syntax: the handler was already a syntactic unit.

mod support;

use support::{compile_source, live_context, run_settled};

/// Three durable writes from one click.
const THREE_WRITES: &str = "\
state visits is durable Whole starting 0
state votes  is durable Whole starting 0
state total  is durable Whole starting 0

view
    Column
        when visits
            Loading       show Spinner
            Failed with e show ErrorBar message is e.message
            Ready with n  show Text n
        Button \"all three\"
            on click
                add 1 to visits
                add 1 to votes
                add 1 to total
";

/// One durable write, and one purely local one beside it.
const MIXED: &str = "\
state visits is durable Whole starting 0
state clicks is client Whole starting 0

view
    Column
        Text clicks
        Button \"click\"
            on click
                add 1 to clicks
                add 1 to visits
";

#[test]
fn every_cross_region_write_joins_the_handlers_transaction() {
    let bundle = compile_source(THREE_WRITES);
    for endpoint in ["visits.incr", "votes.incr", "total.incr"] {
        let push = format!("$tx.push(['{endpoint}', [1]]);");
        assert!(
            bundle.client_js.contains(&push),
            "`{endpoint}` is not part of the handler's transaction:\n{}",
            bundle.client_js
        );
    }
    assert!(
        bundle.client_js.contains("const $tx = [];")
            && bundle.client_js.contains("await $atomic($tx);"),
        "the transaction is accumulated and never sent:\n{}",
        bundle.client_js
    );
}

#[test]
fn a_handler_sends_its_writes_exactly_once() {
    // One `$atomic` per handler. A second would be a second transaction,
    // and the writes it carried could commit while the first's did not.
    let bundle = compile_source(THREE_WRITES);
    assert_eq!(bundle.client_js.matches("await $atomic($tx);").count(), 1);
    assert_eq!(bundle.client_js.matches("const $tx = [];").count(), 1);
}

#[test]
fn no_promise_is_created_and_discarded() {
    // The precise shape of the first bug: a call at the start of a
    // statement with nothing waiting on the result.
    let bundle = compile_source(THREE_WRITES);
    let mut scanned = 0;
    for line in bundle.client_js.lines() {
        scanned += 1;
        let statement = line.trim_start();
        for discarded in ["$call(", "$atomic("] {
            assert!(
                !statement.starts_with(discarded),
                "this promise is discarded: {statement}"
            );
        }
    }
    // An empty bundle discards no promises and proves nothing. Three
    // writes cannot fit in fewer lines than this.
    assert!(scanned >= 10, "the bundle is only {scanned} lines long");
}

#[test]
fn a_handler_that_writes_across_a_boundary_is_async() {
    let bundle = compile_source(THREE_WRITES);
    assert!(
        bundle.client_js.contains("on($n2, 'click', async () => {"),
        "the handler is not async, so `await` inside it is a syntax error:\n{}",
        bundle.client_js
    );
}

#[test]
fn a_handler_that_writes_nothing_across_a_boundary_stays_synchronous() {
    // Making every handler `async` would cost a promise per click on
    // programs with no network at all, and `counter.zd` is the case §16.4
    // pins byte for byte.
    let bundle = compile_source(
        "\
state count is client Whole starting 0

view
    Column
        Text count
        Button \"plus\"
            on click
                add 1 to count
",
    );
    assert!(
        bundle
            .client_js
            .contains("on($n2, 'click', () => setCount(count() + 1))"),
        "a local write grew a promise:\n{}",
        bundle.client_js
    );
    assert!(!bundle.client_js.contains("async"));
}

#[test]
fn a_failure_has_somewhere_to_go() {
    let bundle = compile_source(THREE_WRITES);
    assert!(
        bundle.client_js.contains("catch ($e) {") && bundle.client_js.contains("$failed($e)"),
        "an async handler that rejects is an unhandled rejection:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("reportFailure as $failed"),
        "the failure sink is used but not imported:\n{}",
        bundle.client_js
    );
}

// --- what the compiler knows that a database client cannot ---------------

#[test]
fn the_manifest_carries_each_handlers_whole_write_set_in_source_order() {
    // **The compile-time payoff, made into an artefact.** A general
    // database client cannot know what a transaction will write until it
    // has run, so it needs an *interactive* transaction — which of the
    // surveyed backends only Durable Objects and a local database have.
    // Here the whole set is known before anything runs, so a
    // non-interactive atomic batch is enough, and Deno KV, DynamoDB and D1
    // all have one of those.
    //
    // In the manifest rather than only in the compiler, because the thing
    // that needs it is a deploy adapter checking its target's batch cap —
    // DynamoDB's on `TransactWriteItems`, Deno KV's 100 checks and 1000
    // mutations — before a user clicks rather than after.
    let manifest = compile_source(THREE_WRITES).manifest_json;
    assert!(
        manifest.contains(
            "\"transactions\":[{\"event\":\"click\",\"writes\":[\"visits.incr\",\"votes.incr\",\
             \"total.incr\"],\"bounded\":true}]"
        ),
        "the write set is not in the manifest, or is out of source order:\n{manifest}"
    );
}

#[test]
fn a_handler_with_no_durable_write_contributes_no_transaction() {
    let manifest = compile_source(
        "\
state count is client Whole starting 0

view
    Column
        Text count
        Button \"plus\"
            on click
                add 1 to count
",
    )
    .manifest_json;
    assert!(manifest.contains("\"transactions\":[]"), "{manifest}");
}

/// Drive the emitted bundle with a transport that answers, or refuses.
///
/// The runtime modules are flattened into one scope because the engine has
/// no module loader; the source is otherwise exactly what ships.
fn drive(bundle_js: &str, setup: &str, driver: &str, report: &str) -> String {
    let mut context = live_context();
    run_settled(&mut context, setup, bundle_js, driver, report)
}

#[test]
fn three_writes_reach_the_server_in_one_request_in_the_order_they_were_written() {
    // Fire-and-forget gave no ordering at all, and three requests gave no
    // atomicity. One request in source order gives both, and it is the
    // property that makes `set x to 0` followed by `add 1 to x` mean what
    // it reads like.
    let bundle = compile_source(THREE_WRITES);
    let frames = drive(
        &bundle.client_js,
        r#"
// The `visits` value endpoint is also called, at module scope, because a
// `$remote` binding fetches on evaluation. Only the transaction is
// recorded here.
const $requests = [];
setTransport((name, args) => {
  if (name === '~atomic') $requests.push(args.map((command) => command[0]).join(','));
  return Promise.resolve(1);
});
"#,
        r#"
const $host = document.createElement('div');
main($host);
const $button = walk($host).filter((n) => n.tagName === 'button')[0];
$button.fire('click');
"#,
        "$requests.length + ' | ' + $requests.join(' / ')",
    );
    assert_eq!(frames, "1 | visits.incr,votes.incr,total.incr");
}

#[test]
fn a_failed_transaction_sends_nothing_else_and_is_reported() {
    // The transaction is refused. There is nothing to stop after it and
    // nothing left committed before it — the writes never left as separate
    // requests — and the failure still has to reach the sink, because the
    // DOM layer discards what a listener returns.
    let bundle = compile_source(THREE_WRITES);
    let report = drive(
        &bundle.client_js,
        r#"
const $requests = [];
let $reported = 'none';
setFailureSink((error) => { $reported = String(error && error.message ? error.message : error); });
setTransport((name, args) => {
  if (name === '~atomic') {
    $requests.push(args.map((command) => command[0]).join(','));
    return Promise.reject(new Error('the store refused'));
  }
  return Promise.resolve(1);
});
"#,
        r#"
const $host = document.createElement('div');
main($host);
const $button = walk($host).filter((n) => n.tagName === 'button')[0];
$button.fire('click');
"#,
        "$requests.length + ' | ' + $reported",
    );
    assert_eq!(
        report, "1 | the store refused",
        "the handler sent more than one transaction, or the failure went nowhere"
    );
}

#[test]
fn a_local_write_and_a_remote_one_in_one_handler_both_happen() {
    // Wrapping the body in `try` must not change what a local write does.
    let bundle = compile_source(MIXED);
    let report = drive(
        &bundle.client_js,
        "setTransport(() => Promise.resolve(1));",
        r#"
const $host = document.createElement('div');
main($host);
const $button = walk($host).filter((n) => n.tagName === 'button')[0];
$button.fire('click');
"#,
        "serialize($host)",
    );
    assert!(
        report.contains('1'),
        "the local signal did not move: {report}"
    );
}
