//! Live sync, driven through the emitted bundle.
//!
//! The transport is the seam. §8.1 assumed a held stream is available
//! everywhere; it is not — Lambda in buffered mode and Lambda behind an
//! ALB cannot stream at all, and where streaming works the ceiling runs
//! from 230 s (Azure, contested) to 900 s (Lambda) to unbounded
//! (Cloudflare Workers). So `subscribe` takes a transport, and the two
//! shipped implementations must be interchangeable: the tests below run
//! the *same* emitted bundle over a stream-shaped transport and a
//! poll-shaped one and require the same result.

mod support;

use support::{compile_source, live_context, run_settled};

const COUNTER: &str = "\
state visits is durable Whole starting 0

view
    Column
        when visits
            Loading       show Spinner
            Failed with e show ErrorBar message is e.message
            Ready with n  show Text n
        Button \"count\"
            on click
                add 1 to visits
";

fn drive(bundle_js: &str, setup: &str, driver: &str, report: &str) -> String {
    let mut context = live_context();
    run_settled(&mut context, setup, bundle_js, driver, report)
}

/// A transport that hands the page a scripted list of events, then stops.
/// Both shipped transports reduce to exactly this once decoded, which is
/// the property that makes them interchangeable.
const SCRIPTED: &str = r#"
setTransport((name, args) => Promise.resolve(name === 'visits' ? 0 : 1));
let $sent = [];
globalThis.$scripted = (events) => (keys, cursor, onEvent) => {
  $sent.push({ keys: keys.join(','), cursor });
  for (const event of events) onEvent(event);
  return () => {};
};
"#;

#[test]
fn a_durable_signal_is_bound_through_the_live_cell() {
    let bundle = compile_source(COUNTER);
    assert!(
        bundle
            .client_js
            .contains("const visits = $durable('visits', 'visits', []);"),
        "a durable read is not registered for live updates:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("$subscribe();"),
        "the cells are registered and nothing subscribes:\n{}",
        bundle.client_js
    );
}

#[test]
fn a_server_signal_is_not_bound_through_the_live_cell() {
    // Nothing else can move a `server` signal, so registering it would be
    // a subscription to writes that cannot happen.
    let bundle = compile_source(
        "\
state who is client Text starting \"\"
state greeting is server Text from echo with who

function echo with name
    give name

view
    Column
        Input who, hint is \"name\"
        when greeting
            Loading         show Spinner
            Failed with e   show ErrorBar message is e.message
            Ready with text show Text text
",
    );
    assert!(bundle.client_js.contains("$remote('greeting'"));
    assert!(
        !bundle.client_js.contains("$durable"),
        "a server signal was registered for live updates:\n{}",
        bundle.client_js
    );
    assert!(!bundle.client_js.contains("$subscribe"));
}

#[test]
fn an_announced_write_updates_the_page_with_no_round_trip() {
    // The second window's whole story. The update carries the value, so
    // the page re-renders without asking the server anything — checked by
    // counting the calls the transport saw.
    let bundle = compile_source(COUNTER);
    let rendered = drive(
        &bundle.client_js,
        &format!(
            "{SCRIPTED}\nlet $calls = 0;\nsetTransport((name) => {{ $calls += 1; return \
             Promise.resolve(0); }});"
        ),
        r#"
const $host = document.createElement('div');
main($host);
subscribe({ transport: $scripted([{ event: 'update', seq: 1, key: 'visits', value: 7 }]) });
"#,
        "serialize($host) + ' || calls=' + $calls",
    );
    assert!(
        rendered.contains(">7<"),
        "the announced value did not reach the page:\n{rendered}"
    );
    assert!(
        rendered.ends_with("calls=1"),
        "the page re-fetched instead of using the pushed value:\n{rendered}"
    );
}

#[test]
fn a_resync_makes_the_page_read_again() {
    // The server could not prove it had the whole tail. Continuing on the
    // last value it did send would be the dropped update §8.1 forbids, so
    // the only honest answer is another read.
    let bundle = compile_source(COUNTER);
    let rendered = drive(
        &bundle.client_js,
        &format!(
            "{SCRIPTED}\nlet $calls = 0;\nsetTransport((name) => {{ $calls += 1; return \
             Promise.resolve($calls); }});"
        ),
        r#"
const $host = document.createElement('div');
main($host);
subscribe({ transport: $scripted([{ event: 'resync', seq: 9 }]) });
"#,
        "'calls=' + $calls",
    );
    assert_eq!(rendered, "calls=2", "a resync did not produce a fresh read");
}

#[test]
fn a_subscription_asks_for_the_keys_the_program_declares() {
    // Not a prefix and not a wildcard: Deno KV's `watch()` takes a key
    // list, DynamoDB Streams take a shard, Cloudflare KV has no watch at
    // all. An explicit key set is the only shape every target can serve.
    let bundle = compile_source(COUNTER);
    let sent = drive(
        &bundle.client_js,
        SCRIPTED,
        r#"
const $host = document.createElement('div');
main($host);
subscribe({ transport: $scripted([]) });
"#,
        "JSON.stringify($sent)",
    );
    assert!(
        sent.contains("\"keys\":\"visits\""),
        "the subscription did not ask for `visits`: {sent}"
    );
}

#[test]
fn the_cursor_advances_so_a_reconnection_resumes() {
    // On Lambda the stream is cut at 900 s, on a timer, in normal
    // operation. The cursor is what makes the reconnection cost one round
    // trip instead of a full re-read.
    let bundle = compile_source(COUNTER);
    let cursor = drive(
        &bundle.client_js,
        SCRIPTED,
        r#"
const $host = document.createElement('div');
main($host);
let $at = null;
const $capture = (keys, cursor, onEvent) => {
  $at = onEvent({ event: 'update', seq: 4, key: 'visits', value: 1 });
  $at = onEvent({ event: 'update', seq: 5, key: 'visits', value: 2 });
  return () => {};
};
subscribe({ transport: $capture });
"#,
        "String($at)",
    );
    assert_eq!(cursor, "5", "the cursor did not follow the events");
}

#[test]
fn both_shipped_transports_build_the_same_request() {
    // The stream and the poll differ in where the cursor rides — a header
    // the browser manages versus a query parameter — and in nothing else.
    //
    // `wire=` is on both for the same reason it is on neither's header:
    // `EventSource` cannot set one, so the version #144 requires travels
    // in the query, and the poll spells it identically so that the two
    // stay one protocol at two stream lengths rather than two protocols.
    // It is present with and without a cursor, because a subscription
    // that omitted it on the first connection would be refused exactly
    // when a page first loads.
    let bundle = compile_source(COUNTER);
    let urls = drive(
        &bundle.client_js,
        SCRIPTED,
        "",
        "liveUrl(['visits'], 3) + ' | ' + pollUrl(['visits'], 3) + ' | ' + liveUrl(['visits'], null)",
    );
    assert_eq!(
        urls,
        "/_zd/live?keys=visits&since=3&wire=1 | /_zd/poll?keys=visits&since=3&wire=1 \
         | /_zd/live?keys=visits&wire=1"
    );
}

#[test]
fn the_poll_transport_delivers_the_same_events_a_stream_would() {
    // The fallback for the shapes that cannot stream at all. Same decoded
    // events in, same page out — which is what makes the transport a seam
    // rather than two implementations of live sync.
    let bundle = compile_source(COUNTER);
    let rendered = drive(
        &bundle.client_js,
        &format!("{SCRIPTED}\nsetTransport(() => Promise.resolve(0));"),
        r#"
const $host = document.createElement('div');
main($host);
let $asked = [];
const $fetch = (url) => {
  $asked.push(url);
  const body = $asked.length === 1
    ? [{ event: 'update', seq: 1, key: 'visits', value: 42 }]
    : [];
  // `ok` is not decoration: a poll's status line is how the retry policy
  // tells a refusal from an empty answer, so a double without one is a
  // double of a `Response` that cannot exist.
  return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(body) });
};
// One pass, then stop: `sleep` resolving never would spin the job queue
// for ever, and the loop's exit is `stop()`, not an empty answer.
const $stop = pollTransport(['visits'], null, (event) => receive(event, null), {
  fetch: $fetch,
  sleep: () => { $stop(); return Promise.resolve(); },
});
"#,
        "serialize($host) + ' || ' + JSON.stringify($asked)",
    );
    assert!(
        rendered.contains(">42<"),
        "the polled value did not reach the page:\n{rendered}"
    );
    assert!(
        rendered.contains("/_zd/poll?keys=visits"),
        "the poll did not ask for the declared key:\n{rendered}"
    );
}

#[test]
fn an_unknown_event_is_ignored_rather_than_fatal() {
    // A tab held open across a deploy will meet events a newer server
    // invented. Failing there would break a page that is merely behind.
    let bundle = compile_source(COUNTER);
    let cursor = drive(
        &bundle.client_js,
        SCRIPTED,
        r#"
const $host = document.createElement('div');
main($host);
const $at = receive({ event: 'something-new', seq: 12 }, 3);
"#,
        "String($at)",
    );
    assert_eq!(cursor, "12", "an unknown event lost the cursor");
}

#[test]
fn a_malformed_frame_does_not_take_the_stream_down() {
    let bundle = compile_source(COUNTER);
    let decoded = drive(
        &bundle.client_js,
        SCRIPTED,
        "",
        "JSON.stringify(decodeFrame('update', 'not json', '4'))",
    );
    assert!(
        decoded.contains("\"seq\":4"),
        "a malformed body lost the event id, so the cursor would stall: {decoded}"
    );
}

#[test]
fn a_durable_map_reaches_the_wire_as_a_map_and_not_as_an_empty_object() {
    // **The bug, from the browser's side.** `Map of K to V` compiles to a
    // JavaScript `Map`, and `JSON.stringify(new Map([['ada', 1]]))` is
    // `"{}"` — no throw, no warning. Every durable map used to leave the
    // browser as an empty object, so the store held nothing and the read
    // handed nothing back.
    //
    // Checked on the encoded body rather than on the argument, because the
    // argument was always right: it was the encoding that dropped it.
    let bundle = compile_source(
        "\
state tallies is durable Map of Text to Whole starting empty

view
    Column
        when tallies
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text \"held\"
        Button \"store\"
            on click
                set tallies to [\"ada\" to 1]
",
    );
    let body = drive(
        &bundle.client_js,
        r#"
let $body = 'never sent';
setTransport((name, args) => {
  // A handler's writes leave as one transaction, so what goes on the wire
  // is the batch — and the map has to survive being nested inside it.
  if (name === '~atomic') $body = stringify(args);
  return Promise.resolve(null);
});
"#,
        r#"
const $host = document.createElement('div');
main($host);
const $button = walk($host).filter((n) => n.tagName === 'button')[0];
$button.fire('click');
"#,
        "$body",
    );
    assert_eq!(
        body, "[[\"tallies.set\",[{\"$map\":[[\"ada\",1]]}]]]",
        "the map did not survive `stringify` — this is the silent `{{}}` bug"
    );
    assert!(
        !body.contains("[{}]"),
        "the map encoded as an empty object: {body}"
    );
}

/// **A pair reaches the durable store, so the wire format has to carry
/// one.** It does, and without a tag: a pair's runtime value is an object
/// with named fields, which is the shape `runtime/wire.js` already
/// encodes a record as. Nothing in that file changed for this.
///
/// Checked in both directions in one test, because the two halves are
/// what a round trip is: what leaves the browser, and what a value pushed
/// back down the stream rebuilds into.
#[test]
fn a_durable_pair_crosses_the_wire_as_an_object_and_needs_no_tag() {
    let bundle = compile_source(
        "\
state held is durable List of Pair of Text to Whole starting empty

view
    Column
        when held
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text \"held\"
        Button \"store\"
            on click
                set held to [(Pair with first is \"ada\", second is 7)]
",
    );
    let body = drive(
        &bundle.client_js,
        r#"
let $body = 'never sent';
setTransport((name, args) => {
  if (name === '~atomic') $body = stringify(args);
  return Promise.resolve(null);
});
"#,
        r#"
const $host = document.createElement('div');
main($host);
const $button = walk($host).filter((n) => n.tagName === 'button')[0];
$button.fire('click');
"#,
        "$body",
    );
    assert_eq!(
        body, "[[\"held.set\",[[{\"first\":\"ada\",\"second\":7}]]]]",
        "a pair left the browser as something other than its two named fields"
    );

    let restored = drive(
        &bundle.client_js,
        "setTransport(() => Promise.resolve(null));",
        r#"
const $host = document.createElement('div');
main($host);
const $event = decodeFrame('update', JSON.stringify({
  seq: 1,
  key: 'held',
  value: [{ first: 'bob', second: 9 }],
}), '1');
receive($event, null);
const $held = held().fields[0][0];
"#,
        "$held.first + ':' + String($held.second)",
    );
    assert_eq!(
        restored, "bob:9",
        "a pushed pair arrived as something `.first` and `.second` do not read"
    );
}

#[test]
fn a_map_pushed_down_the_stream_arrives_as_a_map() {
    // The other direction: an announcement carries the encoded form, and
    // the second window has to rebuild a `Map` from it — not the plain
    // object the marker rides as.
    let bundle = compile_source(
        "\
state tallies is durable Map of Text to Whole starting empty

view
    Column
        when tallies
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text \"held\"
        Button \"store\"
            on click
                set tallies to [\"ada\" to 1]
",
    );
    let shape = drive(
        &bundle.client_js,
        "setTransport(() => Promise.resolve(null));",
        r#"
const $host = document.createElement('div');
main($host);
const $event = decodeFrame('update', JSON.stringify({
  seq: 1,
  key: 'tallies',
  value: { $map: [['ada', 3]] },
}), '1');
receive($event, null);
const $held = tallies().fields[0];
"#,
        "($held instanceof Map) + ':' + String($held.get('ada'))",
    );
    assert_eq!(
        shape, "true:3",
        "a pushed map arrived as something other than a Map"
    );
}

/// **The same map, down the other transport.**
///
/// The two transports are a seam over one protocol, so a value that
/// arrives as a `Map` on the stream has to arrive as a `Map` on the poll.
/// Only the stream decoded: `streamTransport` went through `decodeFrame`,
/// which calls `wire.js`'s `decode`, and `pollTransport` handed
/// `response.json()` straight to `receive` — so a polled `Map` reached the
/// page as the `{"$map":[…]}` it travelled as, and `.get` on it is not a
/// function.
///
/// It is not a corner of the deployment matrix. The shapes that poll are
/// exactly the ones that cannot hold a stream — Lambda in buffered mode
/// and Lambda behind an ALB — so this is the only transport some
/// deployments ever use.
///
/// The value arrives through `response.json()` rather than as an
/// already-decoded event, because that is where the defect was:
/// `the_poll_transport_delivers_the_same_events_a_stream_would` polls a
/// scripted body too, and its `value: 42` cannot catch this — a number's
/// wire form is its JSON form.
#[test]
fn a_map_polled_from_the_server_arrives_as_a_map() {
    let bundle = compile_source(
        "\
state tallies is durable Map of Text to Whole starting empty

view
    Column
        when tallies
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text \"held\"
        Button \"store\"
            on click
                set tallies to [\"ada\" to 1]
",
    );
    let shape = drive(
        &bundle.client_js,
        "setTransport(() => Promise.resolve(null));",
        r#"
const $host = document.createElement('div');
main($host);
let $asked = 0;
const $fetch = (url) => {
  $asked += 1;
  // What the router sends: `once()` answers an array of `update` events
  // whose `value` is the wire form, and nothing between here and the cell
  // has decoded it.
  const body = $asked === 1
    ? [{ event: 'update', seq: 1, key: 'tallies', value: { $map: [['ada', 3]] } }]
    : [];
  return Promise.resolve({ json: () => Promise.resolve(body) });
};
// One pass, then stop, as in the transport-parity test above.
const $stop = pollTransport(['tallies'], null, (event) => receive(event, null), {
  fetch: $fetch,
  sleep: () => { $stop(); return Promise.resolve(); },
});
"#,
        // An expression rather than the driver's `const`, because the value
        // arrives on the job queue: the driver has only started the poll by
        // the time it returns. Reported as a shape and not asserted on `get`,
        // so the failure names what did arrive instead of throwing
        // `$held.get is not a function` at the reader.
        r#"
(() => {
  const $held = tallies().fields[0];
  return $held instanceof Map
    ? 'Map:' + String($held.get('ada'))
    : 'not a Map: ' + JSON.stringify($held);
})()
"#,
    );
    assert_eq!(
        shape, "Map:3",
        "a polled map arrived as the object it travelled as, so `Map of K to V` \
         is a different type on the two transports"
    );
}

/// Resume is not exact, so an event the client has already seen will
/// arrive again — and applying it replays a value that has since been
/// overwritten.
///
/// `Last-Event-ID` and `?since=` both mean "everything after N". A server
/// that cannot seek precisely is allowed to answer from a little earlier,
/// which is the whole reason the protocol carries a number. Before this,
/// `receive` applied every `update` it was handed and set the cursor to
/// whatever the event said — so a replayed frame put the old value back on
/// screen and *rewound* the cursor, asking for the same tail again on the
/// next reconnection.
#[test]
fn a_replayed_event_does_not_put_an_overwritten_value_back_on_screen() {
    let bundle = compile_source(COUNTER);
    let rendered = drive(
        &bundle.client_js,
        SCRIPTED,
        r#"
const $host = document.createElement('div');
main($host);
let $at = 0;
$at = receive({ event: 'update', seq: 1, key: 'visits', value: 1 }, $at);
$at = receive({ event: 'update', seq: 2, key: 'visits', value: 2 }, $at);
// The reconnection: the server replays from one frame too early.
$at = receive({ event: 'update', seq: 1, key: 'visits', value: 1 }, $at);
$at = receive({ event: 'update', seq: 2, key: 'visits', value: 2 }, $at);
"#,
        "serialize($host) + ' || cursor=' + $at",
    );
    assert!(
        rendered.contains(">2<"),
        "a replayed frame overwrote the current value:\n{rendered}"
    );
    assert!(
        rendered.contains("cursor=2"),
        "a replayed frame rewound the cursor, so the tail would be asked for again:\n{rendered}"
    );
}

/// The exception: `resync` is never skipped.
///
/// It is the server saying it cannot prove it has the tail this client
/// missed. Whether its sequence number moved says nothing about that, and
/// treating it as already-seen is the dropped update §8.1 forbids.
#[test]
fn a_resync_is_obeyed_even_when_its_sequence_number_did_not_advance() {
    let bundle = compile_source(COUNTER);
    let asked = drive(
        &bundle.client_js,
        r#"
let $calls = 0;
setTransport((name, args) => { $calls += 1; return Promise.resolve(0); });
"#,
        r#"
const $host = document.createElement('div');
main($host);
const $before = $calls;
receive({ event: 'resync', seq: 1 }, 7);
"#,
        "String($calls - $before)",
    );
    assert_eq!(asked, "1", "a resync behind the cursor did not re-read");
}

/// A live-sync frame this runtime cannot decode becomes a `resync`.
///
/// `wire.js` now refuses a `$map` payload it cannot read rather than
/// silently rebuilding an empty map. That throw must not escape into an
/// `EventSource` listener, where nothing catches it — and it must not
/// become a silently dropped update either. Asking again is the only
/// answer that is neither.
#[test]
fn a_frame_carrying_an_undecodable_value_becomes_a_resync() {
    let bundle = compile_source(COUNTER);
    let event = drive(
        &bundle.client_js,
        SCRIPTED,
        r#"
const $host = document.createElement('div');
main($host);
const $frame = decodeFrame(
  'update',
  JSON.stringify({ key: 'visits', seq: 5, value: { $map: 'not an array' } }),
  '5'
);
"#,
        "$frame.event + ':' + $frame.seq",
    );
    assert_eq!(
        event, "resync:5",
        "an undecodable frame must ask again rather than throw or vanish"
    );
}

// --- the retry bound (#143) ----------------------------------------------
//
// **Nothing below sleeps.** `sleep` and `random` are the two seams the
// policy in `runtime/store.js` is written against, and a test that waited
// for a real 30-second ceiling would be a slow test that still could not
// say what the schedule was. Handing in a recorder for one and a fixed
// roll for the other makes the whole schedule a value to compare, and
// every case here instant.

/// A fake `EventSource` that fails however the case asks it to.
///
/// Declared in the *driver* rather than the setup on purpose: the emitted
/// module calls `$subscribe()` at its own scope (§16.3.4), and a stream
/// that existed by then would open a connection this test did not ask for
/// and could not see. With no `EventSource` and no `fetch`, that call is
/// the poll transport finding neither and returning a no-op.
///
/// `$script(source, nth)` is what each case supplies: the frames the
/// `nth` connection delivers, in order, once its listeners are installed.
/// A real `EventSource` does not deliver anything inside its constructor
/// either.
/// A constructor function rather than a `class`, which is not a style
/// preference: `boa` panics — a Rust-level index-out-of-bounds inside its
/// own `define` opcode — on a class expression assigned to a global in an
/// evaluated script. The prototype form is the same object with the same
/// three methods, and it runs.
const STREAM: &str = r#"
const $opened = [];
function FakeSource(url) {
  $opened.push(url);
  this.listeners = {};
  this.closed = false;
  const self = this;
  const nth = $opened.length;
  // Nothing is delivered from inside the constructor, because the
  // listeners are installed after it returns. A real `EventSource` is no
  // different, and a fake that fired early would test an order that
  // cannot happen.
  Promise.resolve().then(() => {
    for (const frame of $script(nth)) self.fire(frame[0], frame[1]);
  });
}
FakeSource.prototype.addEventListener = function (name, fn) {
  this.listeners[name] = this.listeners[name] || [];
  this.listeners[name].push(fn);
};
FakeSource.prototype.close = function () {
  this.closed = true;
};
FakeSource.prototype.fire = function (name, message) {
  if (this.closed) return;
  for (const fn of (this.listeners[name] || [])) fn(message || {});
};
globalThis.EventSource = FakeSource;
"#;

/// A poll that never answers backs off, and then stops.
///
/// The two halves of the bound in one case: the *schedule* — 1 s doubling
/// to the 30 s ceiling, halved because the roll is fixed at 0.5 — and the
/// *end*, eight attempts and no ninth. Without the second half a tab open
/// when an outage began is a client of that outage until someone closes
/// it.
#[test]
fn a_poll_that_never_answers_backs_off_and_then_gives_up() {
    let bundle = compile_source(COUNTER);
    let report = drive(
        &bundle.client_js,
        &format!("{SCRIPTED}\nsetTransport(() => Promise.resolve(0));"),
        r#"
const $host = document.createElement('div');
main($host);
let $asked = 0;
const $slept = [];
pollTransport(['visits'], null, (event) => receive(event, null), {
  fetch: () => { $asked += 1; return Promise.reject(new Error('the server is down')); },
  sleep: (ms) => { $slept.push(ms); return Promise.resolve(); },
  random: () => 0.5,
});
"#,
        "'asked=' + $asked + ' slept=' + JSON.stringify($slept)",
    );
    assert_eq!(
        report, "asked=8 slept=[500,1000,2000,4000,8000,15000,15000]",
        "the poll did not follow the declared schedule, or did not stop"
    );
}

/// **What the program sees when it gives up.**
///
/// Not a console line and not a stall: the durable cell moves to `Failed`,
/// which is an arm the `when` already had, so a page that was written
/// against `Remote of T` renders the answer without being changed. The
/// message is the runtime's own — nothing a server sent chose it — and it
/// reaches `ErrorBar` because the program said it should.
#[test]
fn a_connection_that_has_given_up_reaches_the_page_as_failed() {
    let bundle = compile_source(COUNTER);
    let rendered = drive(
        &bundle.client_js,
        &format!("{SCRIPTED}\nsetTransport(() => Promise.resolve(3));"),
        r#"
const $host = document.createElement('div');
main($host);
pollTransport(['visits'], null, (event) => receive(event, null), {
  fetch: () => Promise.reject(new Error('the server is down')),
  sleep: () => Promise.resolve(),
  random: () => 0,
});
"#,
        "serialize($host)",
    );
    assert!(
        rendered.contains("gave up after 8 attempts"),
        "the page did not say that sync had stopped:\n{rendered}"
    );
    assert!(
        !rendered.contains(">3<"),
        "the page still shows the last value it was told, as though it were \
         still live:\n{rendered}"
    );
}

/// A poll that answers starts the count again.
///
/// The give-up counts *consecutive* failures, and that is the whole
/// difference between bounding an outage and punishing a flaky link. A
/// client on a bad connection that loses seven requests in eight goes on
/// working; it is only a run of eight that means the other end is gone.
#[test]
fn an_answered_poll_starts_the_failure_count_again() {
    let bundle = compile_source(COUNTER);
    let report = drive(
        &bundle.client_js,
        &format!("{SCRIPTED}\nsetTransport(() => Promise.resolve(0));"),
        r#"
const $host = document.createElement('div');
main($host);
let $asked = 0;
const $stop = pollTransport(['visits'], null, (event) => receive(event, null), {
  fetch: () => {
    $asked += 1;
    if ($asked >= 24) $stop();
    // Every eighth request answers, so the run of failures never reaches
    // eight. Twenty-four requests is three times what an unbroken run
    // would have been allowed.
    if ($asked % 8 === 0) {
      return Promise.resolve({
        ok: true,
        status: 200,
        json: () => Promise.resolve([{ event: 'update', seq: $asked, key: 'visits', value: 99 }]),
      });
    }
    return Promise.reject(new Error('dropped'));
  },
  sleep: () => Promise.resolve(),
  random: () => 0.5,
});
"#,
        "'asked=' + $asked + ' || ' + serialize($host)",
    );
    assert!(
        report.starts_with("asked=24"),
        "the poll gave up while it was still being answered:\n{report}"
    );
    assert!(
        report.contains(">99<") && !report.contains("gave up"),
        "a link that keeps answering was declared lost:\n{report}"
    );
}

/// A poll that is refused is failing, not answering.
///
/// A 5xx is a status line, and a status line is an answer to the request
/// and not to the question. Read as a successful poll it would be the
/// worst case there is: a server that refuses in two milliseconds, polled
/// as fast as the loop can run, for as long as it stays down.
#[test]
fn a_refused_poll_counts_as_a_failure_rather_than_an_empty_answer() {
    let bundle = compile_source(COUNTER);
    let report = drive(
        &bundle.client_js,
        &format!("{SCRIPTED}\nsetTransport(() => Promise.resolve(0));"),
        r#"
const $host = document.createElement('div');
main($host);
let $asked = 0;
const $slept = [];
pollTransport(['visits'], null, (event) => receive(event, null), {
  fetch: () => {
    $asked += 1;
    return Promise.resolve({ ok: false, status: 503, json: () => Promise.resolve([]) });
  },
  sleep: (ms) => { $slept.push(ms); return Promise.resolve(); },
  random: () => 0.5,
});
"#,
        "'asked=' + $asked + ' slept=' + JSON.stringify($slept)",
    );
    assert_eq!(
        report, "asked=8 slept=[500,1000,2000,4000,8000,15000,15000]",
        "a 503 was read as an empty poll, so the loop never backed off"
    );
}

/// The stream is bounded too, and by the same policy.
///
/// This is the case that needed the reconnection taken away from
/// `EventSource`. Left to the browser it retries at a fixed interval, with
/// no jitter, for ever — there is no ceiling to reach and no give-up to
/// arrive at, so the bound could not have been observed here at all.
#[test]
fn a_stream_that_cannot_reconnect_gives_up_after_the_same_eight_attempts() {
    let bundle = compile_source(COUNTER);
    let report = drive(
        &bundle.client_js,
        &format!("{SCRIPTED}\nsetTransport(() => Promise.resolve(3));"),
        &format!(
            r#"
const $host = document.createElement('div');
main($host);
{STREAM}
const $script = () => [['error', {{}}]];
const $slept = [];
subscribe({{ sleep: (ms) => {{ $slept.push(ms); return Promise.resolve(); }}, random: () => 0.5 }});
"#
        ),
        "'opened=' + $opened.length + ' slept=' + JSON.stringify($slept) + ' || ' + serialize($host)",
    );
    assert!(
        report.starts_with("opened=8 slept=[500,1000,2000,4000,8000,15000,15000]"),
        "the stream did not follow the declared schedule, or did not stop:\n{report}"
    );
    assert!(
        report.contains("gave up after 8 attempts"),
        "the stream stopped without telling the page:\n{report}"
    );
}

/// A reopened stream resumes at the cursor it reached.
///
/// `EventSource` reconnects to the URL it was constructed with, so the
/// `?since=` it resends is the cursor the session *started* from and the
/// server replays the whole tail on every attempt. Owning the reconnect is
/// what lets the second request say where this client actually got to.
#[test]
fn a_reopened_stream_asks_from_the_cursor_it_reached() {
    let bundle = compile_source(COUNTER);
    let opened = drive(
        &bundle.client_js,
        &format!("{SCRIPTED}\nsetTransport(() => Promise.resolve(0));"),
        &format!(
            r#"
const $host = document.createElement('div');
main($host);
{STREAM}
const $script = (nth) => nth === 1
  ? [
      ['update', {{ data: JSON.stringify({{ key: 'visits', value: 5, seq: 41 }}), lastEventId: '41' }}],
      ['error', {{}}],
    ]
  : [['error', {{}}]];
const $stop = subscribe({{
  sleep: () => {{ if ($opened.length >= 3) $stop(); return Promise.resolve(); }},
  random: () => 0.5,
}});
"#
        ),
        "JSON.stringify($opened)",
    );
    assert_eq!(
        opened,
        r#"["/_zd/live?keys=visits","/_zd/live?keys=visits&since=41","/_zd/live?keys=visits&since=41"]"#,
        "a reconnection asked from where the session began rather than from \
         where this client got to"
    );
}
