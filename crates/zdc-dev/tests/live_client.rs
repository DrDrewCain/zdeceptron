//! The live-reload client is JavaScript, so it is tested by running it.
//!
//! A syntax error or a wrong event name in the injected script would break
//! reload silently: the page would load, the app would work, and edits
//! would simply never appear. Nothing in Rust catches that, so the script
//! is executed here against stubbed browser globals — with a pure-Rust
//! engine, so checking it needs no Node and no browser (spec §7).

use boa_engine::{Context, Source};

/// Stand-ins for the three browser globals the client touches, each
/// recording what it was asked to do.
const BROWSER: &str = r#"
    var opened = [];
    var listeners = {};
    var reloads = 0;
    var location = { reload: function () { reloads += 1; } };
    function EventSource(url) {
        opened.push(url);
        this.addEventListener = function (name, handler) { listeners[name] = handler; };
    }
"#;

/// Pull the script body out of the `<script>` element the server injects,
/// so the test runs the exact text a browser would.
fn client_source() -> String {
    let script = zdc_dev::page::live_script();
    let body = script
        .split_once("<script>")
        .and_then(|(_, rest)| rest.split_once("</script>"))
        .map(|(body, _)| body)
        .unwrap_or_else(|| panic!("no script element in:\n{script}"));
    body.to_string()
}

/// Load the stubs and then the client, then evaluate `after` and report
/// what it came to.
fn run(after: &str) -> String {
    let mut context = Context::default();
    let client = client_source();
    for source in [BROWSER, client.as_str()] {
        context
            .eval(Source::from_bytes(source.as_bytes()))
            .unwrap_or_else(|e| panic!("evaluating\n{source}\nfailed: {e}"));
    }
    context
        .eval(Source::from_bytes(after.as_bytes()))
        .unwrap_or_else(|e| panic!("evaluating\n{after}\nfailed: {e}"))
        .display()
        .to_string()
}

#[test]
fn the_client_is_valid_javascript_and_subscribes_on_load() {
    assert_eq!(run("opened.join(',')"), "\"/__zdc/live\"");
}

/// The server's event names, not a copy of them.
///
/// This compared against the literal `"ready,reload"`, so a third event
/// added to `sse` — the exact drift the test's name claims to catch —
/// could not fail it. The expectation is now built from the constants the
/// server frames its events with, so adding one without teaching the
/// client about it fails here.
#[test]
fn the_client_registers_a_handler_for_every_event_the_server_sends() {
    let mut sent = zdc_dev::sse::EVENTS;
    sent.sort_unstable();
    let expected = format!("\"{}\"", sent.join(","));

    assert_eq!(sent.len(), 2, "the server sends two events today");
    assert_eq!(run("Object.keys(listeners).sort().join(',')"), expected);
}

#[test]
fn a_reload_event_reloads_the_page() {
    assert_eq!(run("listeners.reload({ data: '2' }); reloads"), "1");
}

#[test]
fn the_first_ready_event_does_not_reload() {
    // The `ready` that arrives on the connection the page itself opened
    // must not bounce the page it just finished loading.
    assert_eq!(run("listeners.ready({ data: '3' }); reloads"), "0");
}

#[test]
fn a_ready_event_from_a_different_generation_reloads() {
    // The dev server was restarted, or the machine was asleep across an
    // edit: the tab is stale and nothing else will tell it so.
    assert_eq!(
        run("listeners.ready({ data: '3' }); listeners.ready({ data: '4' }); reloads"),
        "1"
    );
}

#[test]
fn a_reconnect_at_the_same_generation_does_not_reload() {
    // Reconnections are routine — the stream is closed and reopened
    // whenever the network hiccups. Reloading on each would make the page
    // unusable.
    assert_eq!(
        run("listeners.ready({ data: '3' }); listeners.ready({ data: '3' }); reloads"),
        "0"
    );
}

#[test]
fn the_client_leaks_no_globals_of_its_own() {
    // It is injected into the developer's page; a stray global could
    // collide with the program being developed.
    assert_eq!(run("typeof seen"), "\"undefined\"");
    assert_eq!(run("typeof source"), "\"undefined\"");
}
