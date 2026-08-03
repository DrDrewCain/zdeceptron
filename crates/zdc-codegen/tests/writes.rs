//! Cross-region writes: awaited, ordered, and observable when they fail.
//!
//! # The bug these exist to keep fixed
//!
//! A handler with three durable writes used to emit
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
//! # What is still true
//!
//! There is no transaction. Awaiting makes the writes ordered, stops the
//! run at the first failure, and puts the failure somewhere reachable — it
//! does not roll back the writes that already committed. Atomicity across
//! a handler needs a single endpoint carrying the whole write set and a
//! store operation that applies a set at once, which of the surveyed
//! backends only Durable Objects and a local database provide.
//! `a_partial_application_is_reported_rather_than_silent` pins the
//! behaviour that actually ships.

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
fn every_cross_region_write_is_awaited() {
    let bundle = compile_source(THREE_WRITES);
    for endpoint in ["visits.incr", "votes.incr", "total.incr"] {
        let call = format!("await $call('{endpoint}', 1)");
        assert!(
            bundle.client_js.contains(&call),
            "`{endpoint}` is called without `await`:\n{}",
            bundle.client_js
        );
    }
}

#[test]
fn no_promise_is_created_and_discarded() {
    // The precise shape of the old bug: a `$call(` at the start of a
    // statement, with nothing waiting on the result.
    let bundle = compile_source(THREE_WRITES);
    let mut scanned = 0;
    for line in bundle.client_js.lines() {
        scanned += 1;
        let statement = line.trim_start();
        assert!(
            !statement.starts_with("$call("),
            "this promise is discarded: {statement}"
        );
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

/// Drive the emitted bundle with a transport that answers, or refuses.
///
/// The runtime modules are flattened into one scope because the engine has
/// no module loader; the source is otherwise exactly what ships.
fn drive(bundle_js: &str, setup: &str, driver: &str, report: &str) -> String {
    let mut context = live_context();
    run_settled(&mut context, setup, bundle_js, driver, report)
}

#[test]
fn three_writes_reach_the_server_in_the_order_they_were_written() {
    // Fire-and-forget gave no ordering at all. This is the property that
    // makes `set x to 0` followed by `add 1 to x` mean what it reads like.
    let bundle = compile_source(THREE_WRITES);
    let frames = drive(
        &bundle.client_js,
        r#"
// Only the writes. The `visits` value endpoint is also called, at
// module scope, because a `$remote` binding fetches on evaluation.
const $seen = [];
setTransport((name, args) => {
  if (name.includes('.')) $seen.push(name);
  return Promise.resolve(1);
});
"#,
        r#"
const $host = document.createElement('div');
main($host);
const $button = walk($host).filter((n) => n.tagName === 'button')[0];
$button.fire('click');
"#,
        "$seen.join(',')",
    );
    assert_eq!(frames, "visits.incr,votes.incr,total.incr");
}

#[test]
fn a_partial_application_is_reported_rather_than_silent() {
    // The second write fails. The third must not run, and the failure must
    // reach the sink — the first write has still committed, which is the
    // limit this test documents rather than hides.
    let bundle = compile_source(THREE_WRITES);
    let report = drive(
        &bundle.client_js,
        r#"
const $seen = [];
let $reported = 'none';
setFailureSink((error) => { $reported = String(error && error.message ? error.message : error); });
setTransport((name, args) => {
  if (name.includes('.')) $seen.push(name);
  if (name === 'votes.incr') return Promise.reject(new Error('the store refused'));
  return Promise.resolve(1);
});
"#,
        r#"
const $host = document.createElement('div');
main($host);
const $button = walk($host).filter((n) => n.tagName === 'button')[0];
$button.fire('click');
"#,
        "$seen.join(',') + ' | ' + $reported",
    );
    assert_eq!(
        report, "visits.incr,votes.incr | the store refused",
        "the run continued past a failed write, or the failure went nowhere"
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
