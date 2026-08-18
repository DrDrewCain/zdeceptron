//! **The test that changes what green means.**
//!
//! Before this file, `zdc build examples/guestbook.zd` exited 0 having
//! written three server functions that nothing had ever executed. The
//! compiler's own suite checked the *text* of those files; `zdc dev`
//! served them as static assets; `POST /_zd/greeting` answered "not part
//! of this bundle". Every one of those signals was green.
//!
//! Everything below runs the emitted bytes. Not an equivalent, not a
//! re-implementation from the manifest — the exact source `zdc build`
//! writes into `dist/functions/`, through the platform adapter that binds
//! `$env` and `$store`.

mod support;

use std::sync::Arc;

use support::{emit_example, endpoints, host, host_on};
use zdc_host::{Environment, Host, HostError};
use zdc_store::{DurableStore, EmbeddedStore, Json};

/// A durable counter with a button that increments it.
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

/// A server signal computed from a client input and an environment secret.
const GREETING: &str = "\
secret state apiKey is server Text from environment \"GREETING_API_KEY\"
state who is client Text starting \"\"
state greeting is server Text from politeGreeting with who, apiKey

function politeGreeting with name, key
    if name is \"\"
        give \"Hello, stranger.\"
    give \"Hello, \" + name + \".\"

view
    Column
        Input who, hint is \"name\"
        when greeting
            Loading         show Spinner
            Failed with e   show ErrorBar message is \"the greeting service did not answer\"
            Ready with text show Text text
";

#[test]
fn a_value_endpoint_runs_and_returns_its_result() {
    let host = host(
        GREETING,
        Environment::from_pairs([("GREETING_API_KEY", "sk-test")]),
    );
    assert_eq!(
        host.invoke("greeting", "[\"Ada\"]").expect("greeting runs"),
        "\"Hello, Ada.\""
    );
}

#[test]
fn a_helper_emitted_beside_the_handler_is_the_one_that_runs() {
    // `politeGreeting` has a branch, and only one of its two arms is
    // reachable per input. A test that only checked the happy path would
    // pass against a handler that ignored its argument entirely.
    let host = host(
        GREETING,
        Environment::from_pairs([("GREETING_API_KEY", "sk-test")]),
    );
    assert_eq!(
        host.invoke("greeting", "[\"\"]").expect("greeting runs"),
        "\"Hello, stranger.\""
    );
}

#[test]
fn a_command_endpoint_writes_through_to_the_store() {
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(COUNTER, Arc::clone(&store), Environment::empty());

    assert_eq!(host.invoke("visits.incr", "[1]").expect("incr runs"), "1");
    assert_eq!(host.invoke("visits.incr", "[1]").expect("incr runs"), "2");

    // Read the store directly rather than through the endpoint: the point
    // is that the *store* moved, not that one JavaScript function agrees
    // with another.
    assert_eq!(
        store.get("visits").expect("get").map(Json::into_string),
        Some("2".to_string())
    );
}

#[test]
fn a_durable_read_endpoint_sees_what_a_command_endpoint_wrote() {
    // The round trip the browser makes: click, then re-read.
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(COUNTER, store, Environment::empty());

    host.invoke("visits.incr", "[3]").expect("incr runs");
    assert_eq!(host.invoke("visits", "[]").expect("read runs"), "3");
}

#[test]
fn a_key_nobody_has_written_reads_as_the_declared_starting_value() {
    // `starting 0`, not `null`. This is the very first thing a visitor
    // sees, and it is the emitted `?? 0` that decides it.
    let host = host(COUNTER, Environment::empty());
    assert_eq!(host.invoke("visits", "[]").expect("read runs"), "0");
}

#[test]
fn an_unset_environment_key_fails_loudly_rather_than_reading_as_empty() {
    // An empty API key produces a well-formed unauthorised request and the
    // upstream service gets blamed for it. This is the failure that has to
    // point at the deployment instead.
    //
    // In two halves, because the two readers are different machines.
    // `message` is what a browser renders and is bound by §16.3.12
    // assertion C, so it names no key. `detail` is what the server logs,
    // and it names the key — which is the part a developer can act on.
    let host = host(GREETING, Environment::empty());
    match host.invoke("greeting", "[\"Ada\"]") {
        Ok(value) => panic!("an unconfigured secret produced {value}"),
        Err(error @ HostError::Failed { .. }) => {
            let message = error.to_string();
            assert!(
                !message.contains("GREETING_API_KEY"),
                "the browser-visible failure names the environment key: {message}"
            );
            assert!(
                message.contains("environment"),
                "the failure says nothing a developer can act on: {message}"
            );
            assert!(
                error
                    .detail()
                    .is_some_and(|detail| detail.contains("GREETING_API_KEY")),
                "the server-side half does not name the key: {:?}",
                error.detail()
            );
        }
        Err(other) => panic!("expected a handler failure, got {other:?}"),
    }
}

/// §16.3.12 assertion C, on the one path that had been carrying the key
/// name across the boundary.
///
/// Separate from the test above because that one is about a *missing*
/// key. This is about the rendering: whatever a `HostError` is, the text
/// an adapter puts in a response body comes from `Display`, and `Display`
/// must not be able to reach `detail` at all.
#[test]
fn the_environment_key_name_is_not_in_any_failure_text_a_browser_can_read() {
    let host = host(GREETING, Environment::empty());
    let error = host
        .invoke("greeting", "[\"Ada\"]")
        .expect_err("an unconfigured secret must fail");

    // What every adapter writes into the body.
    assert!(
        !error.to_string().contains("GREETING_API_KEY"),
        "`Display` names the key: {error}"
    );
    // And the structural half: no field `Display` reads carries it.
    match &error {
        HostError::Failed {
            endpoint, message, ..
        } => {
            assert!(!endpoint.contains("GREETING_API_KEY"));
            assert!(
                !message.contains("GREETING_API_KEY"),
                "the browser-visible message names the key: {message}"
            );
        }
        other => panic!("expected a handler failure, got {other:?}"),
    }
}

#[test]
fn the_secret_reaches_the_handler_and_never_the_answer() {
    // §5.7's promise, checked on the wire rather than in the type system:
    // the value is available where the program asked for it, and the bytes
    // that go back to the browser do not contain it.
    let host = host(
        GREETING,
        Environment::from_pairs([("GREETING_API_KEY", "sk-do-not-leak")]),
    );
    let answer = host.invoke("greeting", "[\"Ada\"]").expect("greeting runs");
    assert!(
        !answer.contains("sk-do-not-leak"),
        "the secret came back: {answer}"
    );
}

#[test]
fn an_unknown_endpoint_is_a_404_rather_than_a_crash() {
    let host = host(COUNTER, Environment::empty());
    let error = host
        .invoke("visits.decr", "[1]")
        .expect_err("no such endpoint");
    assert_eq!(error.status(), 404);
}

#[test]
fn a_body_that_is_not_an_argument_array_is_the_callers_fault() {
    let host = host(COUNTER, Environment::empty());
    for body in ["{\"a\":1}", "not json", "7", ""] {
        match host.invoke("visits.incr", body) {
            Ok(value) => panic!("`{body}` was accepted and returned {value}"),
            // 400 and not 500: a malformed body is the caller's mistake,
            // and answering 500 would send a browser into a retry loop
            // against a request that can never succeed.
            Err(error) => assert_eq!(
                error.status(),
                400,
                "`{body}` was refused as {error:?}, which is not the caller's fault"
            ),
        }
    }
}

#[test]
fn a_value_endpoint_refuses_the_wrong_number_of_arguments() {
    // Passing too few binds every missing input to `undefined`, and the
    // handler then returns a plausible wrong answer instead of failing.
    let host = host(
        GREETING,
        Environment::from_pairs([("GREETING_API_KEY", "sk-test")]),
    );
    let error = host.invoke("greeting", "[]").expect_err("arity is checked");
    assert_eq!(error.status(), 400);
}

#[test]
fn a_store_failure_surfaces_as_a_failure_and_names_the_key() {
    // `incr` on a key holding text. Unreachable from a well-typed program,
    // which is exactly why the message has to be good when it does happen:
    // it means something outside the program wrote that key.
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    store
        .set("visits", Json::from_text("\"not a number\""))
        .expect("set");
    let host = host_on(COUNTER, store, Environment::empty());

    match host.invoke("visits.incr", "[1]") {
        Ok(value) => panic!("incrementing text produced {value}"),
        Err(HostError::Failed { message, .. }) => {
            assert!(
                message.contains("visits"),
                "the failure does not name the key: {message}"
            );
        }
        Err(other) => panic!("expected a handler failure, got {other:?}"),
    }
}

#[test]
fn the_guestbook_example_runs_end_to_end() {
    // The file the pitch is built around. Every endpoint `zdc build` emits
    // for it is invoked here, against a real store.
    let functions = emit_example("examples/guestbook.zd");
    assert_eq!(
        functions.len(),
        3,
        "guestbook.zd emitted {} functions, not the three it declares",
        functions.len()
    );

    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = Host::new(
        endpoints(functions),
        Arc::clone(&store),
        Environment::from_pairs([("GREETING_API_KEY", "sk-test")]),
    );

    assert_eq!(host.invoke("visits", "[]").expect("visits reads"), "0");
    assert_eq!(
        host.invoke("greeting", "[\"Ada\"]").expect("greeting runs"),
        "\"Hello, Ada.\""
    );
    host.invoke("visits.incr", "[1]").expect("the button works");
    assert_eq!(host.invoke("visits", "[]").expect("visits reads"), "1");
    assert_eq!(
        store.get("visits").expect("get").map(Json::into_string),
        Some("1".to_string()),
        "the click did not reach the store"
    );
}

#[test]
fn the_voting_board_example_runs_end_to_end() {
    // The example §18.3 argues from, driven rather than described. Two
    // things it needs and both were missing: `ranked` reads `votes` and
    // `items`, which are `starting empty` and absent on a fresh store; and
    // `votes.incr.at` writes *one candidate's* count, not the whole key.
    let functions = emit_example("examples/voting-board.zd");
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = Host::new(
        endpoints(functions),
        Arc::clone(&store),
        Environment::from_pairs([("STRIPE_KEY", "sk-test")]),
    );

    assert_eq!(
        host.invoke("ranked", "[]").expect("ranked runs"),
        "[]",
        "a board nobody has voted on is empty, not null"
    );

    host.invoke("votes.incr.at", "[1, \"ada\"]")
        .expect("a vote runs");
    host.invoke("votes.incr.at", "[1, \"bob\"]")
        .expect("a vote runs");
    host.invoke("votes.incr.at", "[1, \"ada\"]")
        .expect("a vote runs");

    assert_eq!(
        store.get("votes").expect("get").map(Json::into_string),
        Some("{\"$map\":[[\"ada\",2],[\"bob\",1]]}".to_string()),
        "the votes did not land on the candidates they were cast for"
    );
    assert_eq!(
        host.invoke("ranked", "[]").expect("ranked runs"),
        "[]",
        "no item was ever added, so the board is still empty"
    );
}

#[test]
fn the_tally_example_runs_end_to_end() {
    // `tallies` is a `Map … starting empty` read straight from the browser,
    // so its endpoint is one store read and the declared default is the
    // whole of what a first visitor sees. It answered `null`.
    let functions = emit_example("examples/tally.zd");
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = Host::new(
        endpoints(functions),
        Arc::clone(&store),
        Environment::empty(),
    );

    assert_eq!(
        host.invoke("tallies", "[]").expect("tallies runs"),
        "{\"$map\":[]}",
        "a store nobody has written answered with something other than an empty map"
    );

    host.invoke("tallies.set", "[{\"$map\":[[\"ada\",1]]}]")
        .expect("the button works");
    assert_eq!(
        host.invoke("tallies", "[]").expect("tallies runs"),
        "{\"$map\":[[\"ada\",1]]}",
        "the map did not survive the round trip"
    );

    // **The path form, which is a different endpoint.** `set m at k to v`
    // sends the key beside the value and mutates one entry; `set m to v`
    // sends the whole map. Until `tally.zd` wrote one, no example did, so
    // `tallies.set.at` was emitted by the compiler and executed by nobody
    // — which is the shape of gap that let a `release` ship a handler
    // calling a function it never defined (#357).
    host.invoke("tallies.set.at", "[2,\"grace\"]")
        .expect("a path write runs");
    assert_eq!(
        host.invoke("tallies", "[]").expect("tallies runs"),
        "{\"$map\":[[\"ada\",1],[\"grace\",2]]}",
        "a path write must add its key and leave the others alone"
    );
}

#[test]
fn durable_state_survives_the_process_that_wrote_it() {
    // §10's second proof, driven through the emitted endpoints rather than
    // through the store's own API: the database is closed and reopened
    // between the write and the read.
    let mut path = std::env::temp_dir();
    path.push(format!("zdc-host-restart-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(&path);

    {
        let store: Arc<dyn DurableStore> =
            Arc::new(EmbeddedStore::open(&path).expect("the store opens"));
        let host = host_on(COUNTER, store, Environment::empty());
        host.invoke("visits.incr", "[1]").expect("incr runs");
        host.invoke("visits.incr", "[1]").expect("incr runs");
    }

    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::open(&path).expect("the store reopens"));
    let host = host_on(COUNTER, store, Environment::empty());
    assert_eq!(
        host.invoke("visits", "[]").expect("read runs"),
        "2",
        "durable state did not survive a restart"
    );

    let _ = std::fs::remove_file(&path);
}
