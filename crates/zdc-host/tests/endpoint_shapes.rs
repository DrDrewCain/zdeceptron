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
//! | command, one path index, every verb     | [`each_mutation_verb_on_a_path_writes_inside_the_key`] |
//! | value, `starting empty` on a fresh store| [`a_starting_empty_durable_signal_reads_as_its_empty_value`] |
//!
//! A handler that names something nothing declares fails all of these the
//! same way, which is the point: the shape is what is under test, not the
//! particular program.
//!
//! # Store state, not call shape
//!
//! The tests that write assert **which key holds which value afterwards**,
//! read back through [`DurableStore::get`]. A path command that reached the
//! store and destroyed it satisfies "it reached the store"; only the
//! contents distinguish the two.

mod support;

use std::sync::Arc;

use support::{emit, endpoints, host, host_on};
use zdc_host::{Environment, Host};
use zdc_store::{DurableStore, EmbeddedStore, Json, Transaction, Write};

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
            Failed with e     show ErrorBar message is \"the greeting service did not answer\"
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

/// The same durable map and a durable map of lists, written through a path
/// by every one of §14B.2's five mutation verbs.
///
/// One program rather than five, because the interesting failure is a verb
/// that lands somewhere *other* than the place the program named, and that
/// is only visible against the entries it was supposed to leave alone.
const PATH_VERBS: &str = "\
state scores  is durable Map of Text to Whole        starting empty
state rosters is durable Map of Text to List of Text starting empty
state label   is client  Text                        starting \"\"

view
    Column
        Input label, hint is \"what\"
        when scores
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with counts show Text \"ok\"
        Button \"set\"
            on click
                set scores at label to 5
        Button \"add\"
            on click
                add 1 to scores at label
        Button \"take\"
            on click
                subtract 1 from scores at label
        Button \"join\"
            on click
                append label to rosters at label
        Button \"leave\"
            on click
                remove label from rosters at label
";

/// One durable signal per container the language has an empty value for,
/// each read directly so each is its own endpoint.
///
/// `empty` is a `List` or a `Map` and nothing else — `Constraint::Collection`
/// in `zdc-types` does not admit `Text`, so `durable Text starting empty` is
/// a type error and the empty `Text` is written `""`. All three are here
/// because all three must survive a store nobody has written to.
const EMPTY_DEFAULTS: &str = "\
state counts is durable Map of Text to Whole starting empty
state names  is durable List of Text         starting empty
state note   is durable Text                 starting \"\"

view
    Column
        when counts
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text \"ok\"
        when names
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text \"ok\"
        when note
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text value
";

/// An in-memory store with `key` already holding `json`.
///
/// Seeded through [`DurableStore::apply`] rather than through an endpoint,
/// so what a write lands on is measured against a state no handler produced.
fn seed(store: &Arc<dyn DurableStore>, key: &str, json: &str) {
    store
        .apply(&Transaction {
            reads: Vec::new(),
            writes: vec![Write::Set {
                key: key.to_string(),
                value: Json::from_text(json.to_string()),
            }],
        })
        .expect("the seed commits");
}

/// What a key holds, in the wire encoding the store actually keeps.
fn held(store: &Arc<dyn DurableStore>, key: &str) -> String {
    store
        .get(key)
        .expect("the key is readable")
        .map(Json::into_string)
        .unwrap_or_else(|| "<absent>".to_string())
}

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

    // The shape — the handler binds every name it uses and takes both wire
    // arguments — *and* where the write landed. Reaching the store is not
    // the property: `incr` that dropped the index reached the store too,
    // and left `scores` holding the number 1 where the map belonged.
    assert_eq!(
        held(&store, "scores"),
        "{\"$map\":[[\"ada\",1]]}",
        "the index did not select the entry the program named"
    );
}

#[test]
fn each_mutation_verb_on_a_path_writes_inside_the_key() {
    // §14B.2 closes the mutation verb set at five and §18.2 makes that verb
    // the wire contract, so all five are reachable through a path and all
    // five are checked. Every case seeds a second entry the write must not
    // touch, because "the key changed" is satisfied equally by the correct
    // write and by one that overwrote the whole map.
    let map = |entries: &str| format!("{{\"$map\":[{entries}]}}");

    for (endpoint, arguments, key, before, after) in [
        (
            "scores.set.at",
            "[5, \"ada\"]",
            "scores",
            map("[\"ada\",1],[\"bob\",2]"),
            map("[\"ada\",5],[\"bob\",2]"),
        ),
        (
            "scores.incr.at",
            "[3, \"ada\"]",
            "scores",
            map("[\"ada\",1],[\"bob\",2]"),
            map("[\"ada\",4],[\"bob\",2]"),
        ),
        (
            "scores.decr.at",
            "[1, \"bob\"]",
            "scores",
            map("[\"ada\",1],[\"bob\",2]"),
            map("[\"ada\",1],[\"bob\",1]"),
        ),
        (
            "rosters.append.at",
            "[\"cy\", \"reds\"]",
            "rosters",
            map("[\"reds\",[\"ada\"]],[\"blues\",[\"bob\"]]"),
            map("[\"reds\",[\"ada\",\"cy\"]],[\"blues\",[\"bob\"]]"),
        ),
        (
            "rosters.remove.at",
            "[\"ada\", \"reds\"]",
            "rosters",
            map("[\"reds\",[\"ada\",\"cy\"]],[\"blues\",[\"bob\"]]"),
            map("[\"reds\",[\"cy\"]],[\"blues\",[\"bob\"]]"),
        ),
    ] {
        let store: Arc<dyn DurableStore> =
            Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
        seed(&store, key, &before);
        // The key the write must leave exactly as it found it.
        seed(&store, "untouched", "7");
        let host = host_on(PATH_VERBS, Arc::clone(&store), Environment::empty());

        host.invoke(endpoint, arguments)
            .unwrap_or_else(|error| panic!("{endpoint} did not run: {error}"));

        assert_eq!(
            held(&store, key),
            after,
            "`{endpoint}` did not write where the program said"
        );
        assert_eq!(
            held(&store, "untouched"),
            "7",
            "`{endpoint}` wrote to a key the program never named"
        );
    }
}

#[test]
fn a_path_command_on_a_key_nobody_has_written_builds_the_declared_container() {
    // `examples/voting-board.zd`'s first vote. `votes` is `starting empty`,
    // so the first `add 1 to votes at candidate` has no container to write
    // inside and has to make the one the declaration named — a `Map`, not a
    // list and not a bare number.
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(PATH_VERBS, Arc::clone(&store), Environment::empty());

    host.invoke("scores.incr.at", "[1, \"ada\"]")
        .expect("the first vote runs");
    host.invoke("scores.incr.at", "[1, \"bob\"]")
        .expect("the second vote runs");
    host.invoke("scores.incr.at", "[1, \"ada\"]")
        .expect("the third vote runs");

    assert_eq!(
        held(&store, "scores"),
        "{\"$map\":[[\"ada\",2],[\"bob\",1]]}",
        "three votes for two candidates did not produce two counts"
    );
}

/// `remove` from a durable `Map` with no path at all — the whole key, by
/// key rather than by element.
const REMOVE_FROM_MAP: &str = "\
state tallies is durable Map of Text to Whole starting empty
state label   is client  Text                 starting \"\"

view
    Column
        Input label, hint is \"what\"
        when tallies
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text \"ok\"
        Button \"drop\"
            on click
                remove label from tallies
";

#[test]
fn remove_takes_an_entry_out_of_a_durable_map() {
    // §14B.2 admits `remove` on either collection and `zdc-codegen` emits
    // both arms for a client-side one, so the store façade owes both too.
    // It had only the list arm, and `remove label from tallies` threw
    // "`tallies` does not hold a list" on a map the program declared.
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    seed(&store, "tallies", "{\"$map\":[[\"ada\",1],[\"bob\",2]]}");
    let host = host_on(REMOVE_FROM_MAP, Arc::clone(&store), Environment::empty());

    host.invoke("tallies.remove", "[\"ada\"]")
        .expect("the removal runs");

    assert_eq!(
        held(&store, "tallies"),
        "{\"$map\":[[\"bob\",2]]}",
        "`remove` did not take the entry the program named out of the map"
    );
}

#[test]
fn a_starting_empty_durable_signal_reads_as_its_empty_value() {
    // A fresh store answers every read with "absent", and the declaration
    // is what says what absent means. `null` is not an empty map: reading
    // one is `cannot convert 'null' or 'undefined' to object`, which is
    // what `examples/voting-board.zd`'s `ranked` threw.
    let host = host_on(
        EMPTY_DEFAULTS,
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens")),
        Environment::empty(),
    );

    assert_eq!(
        host.invoke("counts", "[]").expect("counts runs"),
        "{\"$map\":[]}",
        "a `Map … starting empty` did not read as an empty map"
    );
    assert_eq!(
        host.invoke("names", "[]").expect("names runs"),
        "[]",
        "a `List … starting empty` did not read as an empty list"
    );
    assert_eq!(
        host.invoke("note", "[]").expect("note runs"),
        "\"\"",
        "a `Text … starting \"\"` did not read as the empty text"
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
        ("every verb on a path", PATH_VERBS),
        ("remove from a map", REMOVE_FROM_MAP),
        ("starting empty", EMPTY_DEFAULTS),
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
