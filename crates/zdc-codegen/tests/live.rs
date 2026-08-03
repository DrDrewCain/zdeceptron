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
    let bundle = compile_source(COUNTER);
    let urls = drive(
        &bundle.client_js,
        SCRIPTED,
        "",
        "liveUrl(['visits'], 3) + ' | ' + pollUrl(['visits'], 3) + ' | ' + liveUrl(['visits'], null)",
    );
    assert_eq!(
        urls,
        "/_zd/live?keys=visits&since=3 | /_zd/poll?keys=visits&since=3 | /_zd/live?keys=visits"
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
  return Promise.resolve({ json: () => Promise.resolve(body) });
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
