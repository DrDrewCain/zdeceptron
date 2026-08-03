//! What crosses the boundary, and what must never cross it.
//!
//! Two separate claims live here.
//!
//! **`Remote of T` is honest.** §5.2 puts the network in the type, which
//! is only worth anything if the three states are all reachable: a call
//! that fails has to become `Failed`, not sit in `Loading` for ever. A
//! spinner that never stops is the single worst outcome of a boundary,
//! because it looks like slowness rather than breakage.
//!
//! **A secret is never in the bundle.** The information-flow pass refuses
//! a program that renders one. These tests check the complementary
//! property the pass cannot state: that a program the pass *accepts*
//! still ships no secret, no environment key name, and no value — because
//! a build artifact is committed, cached, and copied into images.

mod support;

use support::{compile_source, refusals, rpc_context, run_settled};

const GUESTBOOK: &str = "\
secret state apiKey is server Text from environment \"GREETING_API_KEY\"
state who is client Text starting \"\"
state greeting is server Text from politeGreeting with who, apiKey

function politeGreeting with name, key
    give \"Hello, \" + name + \".\"

view
    Column
        Input who, hint is \"name\"
        when greeting
            Loading         show Spinner
            Failed with e   show ErrorBar message is e.message
            Ready with text show Text text
";

fn drive(bundle_js: &str, setup: &str, driver: &str, report: &str) -> String {
    let mut context = rpc_context();
    run_settled(&mut context, setup, bundle_js, driver, report)
}

#[test]
fn a_failed_call_becomes_failed_rather_than_staying_in_loading() {
    // The acceptance criterion, driven through the emitted bundle: the
    // transport rejects, and what the page shows is the error bar.
    let bundle = compile_source(GUESTBOOK);
    let rendered = drive(
        &bundle.client_js,
        "setTransport(() => Promise.reject(new Error('the server is down')));",
        r#"
const $host = document.createElement('div');
main($host);
"#,
        "serialize($host)",
    );
    assert!(
        rendered.contains("the server is down"),
        "a failed call did not surface its message:\n{rendered}"
    );
    assert!(
        !rendered.contains("aria-busy"),
        "a failed call left the spinner on screen — this is the \"hangs in Loading\" failure:\n{rendered}"
    );
}

#[test]
fn a_call_that_succeeds_replaces_the_spinner_with_the_value() {
    let bundle = compile_source(GUESTBOOK);
    let rendered = drive(
        &bundle.client_js,
        "setTransport(() => Promise.resolve('Hello, Ada.'));",
        r#"
const $host = document.createElement('div');
main($host);
"#,
        "serialize($host)",
    );
    assert!(
        rendered.contains("Hello, Ada."),
        "the resolved value was not rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("aria-busy"),
        "the spinner outlived the answer"
    );
}

#[test]
fn a_call_that_has_not_answered_yet_is_loading() {
    // The third state has to be reachable too, or `Loading` is decoration.
    let bundle = compile_source(GUESTBOOK);
    let rendered = drive(
        &bundle.client_js,
        "setTransport(() => new Promise(() => {}));",
        r#"
const $host = document.createElement('div');
main($host);
"#,
        "serialize($host)",
    );
    assert!(
        rendered.contains("aria-busy"),
        "an unanswered call is not showing its loading state:\n{rendered}"
    );
}

#[test]
fn the_client_bundle_contains_no_environment_access() {
    // `$env` is bound by the platform adapter and exists only on the
    // server. Emitting a call to it into the browser would be a
    // `ReferenceError` at best and an exfiltrated key at worst.
    let bundle = compile_source(GUESTBOOK);
    assert!(
        !bundle.client_js.contains("$env"),
        "the client bundle reads the environment:\n{}",
        bundle.client_js
    );
}

#[test]
fn no_client_readable_file_carries_the_environment_key_name() {
    // §16.3.12 assertion C names the manifest specifically, because it is
    // the file most likely to grow a field "for the runtime's
    // convenience". The key name is a fact about the deployment, and it
    // belongs in the server file that reads it and nowhere else.
    let bundle = compile_source(GUESTBOOK);
    for (what, contents) in [
        ("client.js", &bundle.client_js),
        ("manifest.json", &bundle.manifest_json),
        ("index.html", &bundle.index_html),
        ("styles.css", &bundle.styles_css),
    ] {
        assert!(
            !contents.contains("GREETING_API_KEY"),
            "{what} names the environment key:\n{contents}"
        );
    }
}

#[test]
fn the_secret_signal_has_no_cell_in_the_browser() {
    // A `secret server` signal is not a client member, so there is nothing
    // to declare and nothing to set. If a binding for it appeared, the
    // value would have to arrive from somewhere.
    let bundle = compile_source(GUESTBOOK);
    assert!(
        !bundle.client_js.contains("apiKey"),
        "the secret is named in the browser:\n{}",
        bundle.client_js
    );
}

#[test]
fn rendering_a_secret_is_refused_before_anything_is_emitted() {
    // The pass that makes the tests above meaningful. Without it they
    // would only prove that the *accepted* programs are clean.
    let messages = refusals(
        "\
secret state apiKey is server Text from environment \"GREETING_API_KEY\"

view
    Column
        Text apiKey
",
    );
    assert!(
        !messages.is_empty(),
        "a program that renders a secret was compiled"
    );
}

#[test]
fn the_manifest_records_which_shape_each_endpoint_takes() {
    // A caller that guesses wrong sends an array to a destructured object
    // and every input silently becomes `undefined`.
    let bundle = compile_source(GUESTBOOK);
    assert!(
        bundle.manifest_json.contains("\"kind\":\"value\""),
        "the manifest does not say how to call `greeting`:\n{}",
        bundle.manifest_json
    );
}

#[test]
fn the_manifest_lists_the_durable_keys_a_client_must_subscribe_to() {
    // There is no prefix watch on the stores this has to run on, so the
    // client subscribes to an explicit key set — and this is where that
    // set comes from.
    let bundle = compile_source(
        "\
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
",
    );
    assert!(
        bundle.manifest_json.contains("\"durable\":[\"visits\"]"),
        "the durable key set is missing:\n{}",
        bundle.manifest_json
    );
}
