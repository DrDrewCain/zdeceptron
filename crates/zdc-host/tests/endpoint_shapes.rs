//! One executed handler per endpoint shape the compiler can emit.
//!
//! `zdc-codegen`'s own suite asserts what emission *prints*. That is how
//! `voting-board.zd` shipped an endpoint reading two names nothing bound —
//! `rank(votes)` under `handler({  })`, with `items` free inside `rank` —
//! for as long as it did: every assertion about those bytes was about a
//! substring that was present, and no assertion ran them. `zdc build`
//! exited 0 and the endpoint was a `ReferenceError` on its first request.
//!
//! So this file is organised by *shape* rather than by feature, and every
//! test in it invokes the emitted handler with real arguments. The shapes
//! are the cross product of what a handler can be:
//!
//! | shape                                  | test |
//! |----------------------------------------|------|
//! | value, zero inputs                      | [`a_value_endpoint_with_no_inputs_runs`] |
//! | value, one input                        | [`a_value_endpoint_with_one_input_runs`] |
//! | value, several inputs                   | [`a_value_endpoint_with_several_inputs_runs`] |
//! | value, input read in a nested scope     | [`an_input_read_inside_a_closure_is_in_scope_there`] |
//! | value, durable key passed as an argument| [`a_durable_key_the_handler_reads_is_bound_by_the_handler`] |
//! | value, durable key read inside a helper | [`a_durable_key_read_inside_a_helper_is_in_scope_there`] |
//! | value, durable key as the endpoint itself| [`a_durable_signal_read_directly_is_its_own_endpoint`] |
//! | command, no path                        | [`a_command_endpoint_with_no_path_runs`] |
//! | command, one path index                 | [`a_command_endpoint_with_a_path_index_runs`] |
//!
//! A handler that names something nothing declares fails all of these the
//! same way, which is the point: the shape is what is under test, not the
//! particular program.

mod support;

use std::sync::Arc;

use support::{emit, endpoints, host, host_on};
use zdc_host::{Environment, Host};
use zdc_store::{DurableStore, EmbeddedStore, Json};

/// A server signal computed from nothing at all.
const NO_INPUTS: &str = "\
state motto is server Text from shout

function shout
    give \"steady\"

view
    Column
        when motto
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text
";

/// One client signal lifted to the server.
const ONE_INPUT: &str = "\
state who is client Text starting \"\"
state greeting is server Text from politeGreeting with who

function politeGreeting with name
    give \"Hello, \" + name + \".\"

view
    Column
        Input who, hint is \"name\"
        when greeting
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text
";

/// Two lifted client signals and an environment secret, so the wire order
/// of the inputs is something a handler can get wrong.
const SEVERAL_INPUTS: &str = "\
secret state apiKey is server Text from environment \"SHAPE_API_KEY\"
state who is client Text starting \"\"
state loud is client Truth starting no
state greeting is server Text from salute with who, loud, apiKey

function salute with name, shout, key
    if shout
        give \"HELLO \" + name
    give \"Hello, \" + name

view
    Column
        Input who, hint is \"name\"
        Checkbox loud, label is \"shout\"
        when greeting
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text
";

/// A lifted input read from inside a pipeline predicate — a closure the
/// handler's parameter list has to reach into.
const NESTED_SCOPE: &str = "\
record Item
    name  is Text
    score is Whole

state cutoff is client Whole starting 0
state pool is server List of Item from stock
state picked is server List of Text from pick with cutoff

function stock
    give [(Item with name is \"low\", score is 1), (Item with name is \"high\", score is 9)]

function pick with least
    from pool
    keep each item where item.score >= least
    map each item to item.name

view
    Column
        when picked
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with names  show Text \"ok\"
";

/// A durable key handed to a helper as an argument. The browser sends
/// nothing for it — the handler has to read it out of the store itself.
const DURABLE_ARGUMENT: &str = "\
state total is durable Whole starting 0
state doubled is server Whole from twice with total

view
    Column
        when total
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text value
        when doubled
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text value
        Button \"add\"
            on click
                add 1 to total

function twice with n
    give n + n
";

/// The same durable key, read inside the helper rather than passed to it.
/// The binding is an `await` inside `handler`, so the helper cannot be
/// emitted at module scope.
const DURABLE_IN_HELPER: &str = "\
state total is durable Whole starting 0
state doubled is server Whole from twice

view
    Column
        when doubled
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text value
        Button \"add\"
            on click
                add 1 to total

function twice
    give total + total
";

/// A durable map written through a path, which is the two-argument
/// command shape: the value and one index.
const PATH_COMMAND: &str = "\
state scores is durable Map of Text to Whole starting empty
state label is client Text starting \"\"

view
    Column
        Input label, hint is \"what\"
        when scores
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with counts show Text \"ok\"
        Button \"vote\"
            on click
                add 1 to scores at label
";

fn shape_host(source: &str) -> Host {
    host(
        source,
        Environment::from_pairs([("SHAPE_API_KEY", "sk-shape")]),
    )
}

#[test]
fn a_value_endpoint_with_no_inputs_runs() {
    assert_eq!(
        shape_host(NO_INPUTS)
            .invoke("motto", "[]")
            .expect("motto runs"),
        "\"steady\""
    );
}

#[test]
fn a_value_endpoint_with_one_input_runs() {
    assert_eq!(
        shape_host(ONE_INPUT)
            .invoke("greeting", "[\"Ada\"]")
            .expect("greeting runs"),
        "\"Hello, Ada.\""
    );
}

#[test]
fn a_value_endpoint_with_several_inputs_runs() {
    // Both arms, so a handler that ignored its second input would still
    // have to produce two different answers.
    let host = shape_host(SEVERAL_INPUTS);
    assert_eq!(
        host.invoke("greeting", "[\"Ada\", false]")
            .expect("greeting runs"),
        "\"Hello, Ada\""
    );
    assert_eq!(
        host.invoke("greeting", "[\"Ada\", true]")
            .expect("greeting runs"),
        "\"HELLO Ada\""
    );
}

#[test]
fn an_input_read_inside_a_closure_is_in_scope_there() {
    // `least` is named inside a predicate the pipeline builds, which is a
    // scope the parameter list does not obviously reach. Two thresholds,
    // so a handler that dropped the closure's reference and read a stale
    // constant would answer the same thing twice.
    let host = shape_host(NESTED_SCOPE);
    assert_eq!(
        host.invoke("picked", "[0]").expect("picked runs"),
        "[\"low\",\"high\"]"
    );
    assert_eq!(
        host.invoke("picked", "[5]").expect("picked runs"),
        "[\"high\"]"
    );
}

#[test]
fn a_durable_key_the_handler_reads_is_bound_by_the_handler() {
    // The regression. `doubled` takes no input from the browser and names
    // `total`, which lives in the store: the handler has to bind it, and
    // before it did the first request threw `total is not defined`.
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(DURABLE_ARGUMENT, Arc::clone(&store), Environment::empty());

    assert_eq!(
        host.invoke("doubled", "[]").expect("doubled runs"),
        "0",
        "the declared `starting 0` did not reach the first read"
    );
    host.invoke("total.incr", "[4]").expect("incr runs");
    assert_eq!(
        host.invoke("doubled", "[]").expect("doubled runs"),
        "8",
        "the handler did not re-read the key it was given"
    );
}

#[test]
fn a_durable_key_read_inside_a_helper_is_in_scope_there() {
    // The store binding is an `await` inside `handler`, so `twice` cannot
    // be emitted at module scope — a module-scope `twice` naming `total`
    // is the same `ReferenceError` one file over.
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(DURABLE_IN_HELPER, Arc::clone(&store), Environment::empty());

    host.invoke("total.incr", "[3]").expect("incr runs");
    assert_eq!(
        host.invoke("doubled", "[]").expect("doubled runs"),
        "6",
        "the helper could not see the key the handler bound"
    );
}

#[test]
fn a_durable_signal_read_directly_is_its_own_endpoint() {
    // The other durable shape: nothing derives from the key, so the key
    // *is* the endpoint and the whole body is one store read.
    let host = host_on(
        DURABLE_ARGUMENT,
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens")),
        Environment::empty(),
    );
    assert_eq!(host.invoke("total", "[]").expect("total runs"), "0");
}

#[test]
fn a_command_endpoint_with_no_path_runs() {
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(DURABLE_ARGUMENT, Arc::clone(&store), Environment::empty());

    assert_eq!(host.invoke("total.incr", "[2]").expect("incr runs"), "2");
    assert_eq!(
        store.get("total").expect("get").map(Json::into_string),
        Some("2".to_string())
    );
}

#[test]
fn a_command_endpoint_with_a_path_index_runs() {
    // Two arguments on the wire: the right-hand side and one index, both
    // evaluated in the region that asked (§17.2.7).
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(PATH_COMMAND, Arc::clone(&store), Environment::empty());

    host.invoke("scores.incr.at", "[1, \"ada\"]")
        .expect("the path command runs");

    // What is asserted here is the *shape*: the handler binds every name
    // it uses, takes both wire arguments, and reaches the store.
    //
    // What is deliberately **not** asserted is where in the map the write
    // landed, because it does not land in the map at all. The handler
    // emits `$store.incr('scores', $args[0], $args[1])` and the binding in
    // `zdc-host/src/bindings.rs` is `incr(key, delta)` — the index is
    // dropped, and the whole key becomes a number. That is a separate
    // defect from this file's, it is reported rather than encoded, and
    // asserting the wrong answer here would be the thing that keeps it.
    assert!(
        store.get("scores").expect("get").is_some(),
        "the path command reached no store key at all"
    );
}

#[test]
fn every_emitted_handler_binds_every_name_it_names() {
    // The property behind all of the above, asserted over every shape at
    // once: run each endpoint the compiler emitted, and refuse a
    // `ReferenceError`. A new shape added later gets this for free only if
    // its program is added here, which is why the list is explicit.
    let sources = [
        ("no inputs", NO_INPUTS),
        ("one input", ONE_INPUT),
        ("several inputs", SEVERAL_INPUTS),
        ("nested scope", NESTED_SCOPE),
        ("durable argument", DURABLE_ARGUMENT),
        ("durable in a helper", DURABLE_IN_HELPER),
        ("path command", PATH_COMMAND),
    ];
    for (label, source) in sources {
        let functions = emit(source, "shapes.zd");
        assert!(!functions.is_empty(), "{label} emitted no server function");
        let host = Host::new(
            endpoints(functions.clone()),
            Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens")),
            Environment::from_pairs([("SHAPE_API_KEY", "sk-shape")]),
        );
        for function in &functions {
            let arguments: Vec<&str> = match function.kind {
                zdc_codegen::FunctionKind::Value => {
                    function.inputs.iter().map(|_| "0").collect::<Vec<_>>()
                }
                // A command carries its right-hand side and one argument
                // per index; two covers both command shapes here.
                zdc_codegen::FunctionKind::Command => vec!["1", "\"k\""],
            };
            let body = format!("[{}]", arguments.join(", "));
            if let Err(error) = host.invoke(&function.name, &body) {
                let message = error.to_string();
                assert!(
                    !message.contains("is not defined"),
                    "{label}: `{}` names something nothing declares: {message}\n{}",
                    function.name,
                    function.source
                );
            }
        }
    }
}
