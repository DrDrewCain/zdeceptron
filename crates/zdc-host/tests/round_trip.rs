//! **Every type the language can put in durable state, out and back.**
//!
//! # Why this file exists
//!
//! `state scores is durable Map of Text to Whole` compiled, ran, exited 0,
//! and stored `{}`. `JSON.stringify(new Map([['ada', 1]]))` is `"{}"` — no
//! throw, no warning, an empty object. Every durable map wrote nothing and
//! read nothing back, silently, which for a persistence bug is the worst
//! way to fail.
//!
//! It survived because nothing ever put a value into the store and took the
//! same value out. The emission tests checked the *text* of the generated
//! code; the store's tests used JSON text directly. Neither could see it.
//!
//! So: for every shape §14B.4 and §5.4 allow — number, text, truth, list,
//! map, record, choice, and nested combinations of them — write it through
//! the emitted command endpoint and read it back through the emitted value
//! endpoint. A wire format with no round-trip test is how this happened.

mod support;

use std::sync::Arc;

use support::{endpoints, host_on};
use zdc_host::{Environment, Host};
use zdc_store::{DurableStore, EmbeddedStore};

/// A durable cell of a declared type, with a `set` command and a read.
///
/// One fixture generator rather than eight fixtures, because what varies
/// is the *type*, and a copy per type is a copy per type to forget to
/// update.
fn program(ty: &str, literal: &str) -> String {
    format!(
        "\
state held is durable {ty} starting {literal}

view
    Column
        when held
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text \"held\"
        Button \"store\"
            on click
                set held to {literal}
"
    )
}

/// Write the declared literal through `held.set`, then read `held` back.
///
/// Both halves are the emitted endpoints, run by the adapter, against a
/// real store — so what is checked is the whole path a browser's click
/// takes, not a helper's idea of it.
fn round_trip(ty: &str, literal: &str) -> (String, String) {
    let source = program(ty, literal);
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(&source, Arc::clone(&store), Environment::empty());

    let written = host
        .invoke("held.set", "[__ARGS__]")
        .unwrap_or_else(|e| panic!("`{ty}` could not be written: {e}"));
    let read = host
        .invoke("held", "[]")
        .unwrap_or_else(|e| panic!("`{ty}` could not be read: {e}"));
    (written, read)
}

/// The same, but the argument is supplied as wire JSON rather than
/// re-derived — which is what a browser sends.
fn store_and_read(ty: &str, literal: &str, wire: &str) -> String {
    let source = program(ty, literal);
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host: Host = host_on(&source, Arc::clone(&store), Environment::empty());

    host.invoke("held.set", &format!("[{wire}]"))
        .unwrap_or_else(|e| panic!("`{ty}` could not be written: {e}"));
    host.invoke("held", "[]")
        .unwrap_or_else(|e| panic!("`{ty}` could not be read: {e}"))
}

#[test]
fn a_whole_round_trips() {
    assert_eq!(store_and_read("Whole", "0", "7"), "7");
}

#[test]
fn a_decimal_round_trips() {
    assert_eq!(store_and_read("Decimal", "0.0", "1.5"), "1.5");
}

#[test]
fn text_round_trips() {
    assert_eq!(
        store_and_read("Text", "\"\"", "\"ada\""),
        "\"ada\"",
        "text did not survive the store"
    );
}

#[test]
fn text_with_characters_json_has_to_escape_round_trips() {
    assert_eq!(
        store_and_read("Text", "\"\"", "\"a\\\"b\\\\c\\nd\""),
        "\"a\\\"b\\\\c\\nd\""
    );
}

#[test]
fn a_truth_round_trips() {
    assert_eq!(store_and_read("Truth", "yes", "true"), "true");
}

#[test]
fn a_list_round_trips() {
    assert_eq!(
        store_and_read("List of Whole", "empty", "[1,2,3]"),
        "[1,2,3]"
    );
}

#[test]
fn an_empty_list_round_trips_as_a_list_and_not_as_null() {
    assert_eq!(store_and_read("List of Whole", "empty", "[]"), "[]");
}

#[test]
fn a_map_round_trips() {
    // **The bug.** Before the tagged encoding this returned `{}` — the
    // write stored an empty object and the read handed it back, with
    // nothing anywhere reporting a problem.
    assert_eq!(
        store_and_read(
            "Map of Text to Whole",
            "empty",
            "{\"$map\":[[\"ada\",1],[\"grace\",2]]}"
        ),
        "{\"$map\":[[\"ada\",1],[\"grace\",2]]}",
        "a durable map did not survive the store"
    );
}

#[test]
fn an_empty_map_round_trips_as_a_map_and_not_as_a_record() {
    // `{}` and `{"$map":[]}` are different values, and the first is what
    // the bug produced for *every* map. If an empty map encoded as `{}`
    // the fix would be untested exactly where it is needed.
    assert_eq!(
        store_and_read("Map of Text to Whole", "empty", "{\"$map\":[]}"),
        "{\"$map\":[]}"
    );
}

#[test]
fn a_map_keeps_its_keys_distinct_from_a_records_fields() {
    // The reason a map cannot ride as a plain object: `Map of Whole to
    // Text` has number keys, and an object coerces every one of them to a
    // string, so `1` and `"1"` would collide.
    assert_eq!(
        store_and_read(
            "Map of Whole to Text",
            "empty",
            "{\"$map\":[[1,\"one\"],[2,\"two\"]]}"
        ),
        "{\"$map\":[[1,\"one\"],[2,\"two\"]]}",
        "a numeric map key was coerced to a string"
    );
}

#[test]
fn a_map_of_lists_round_trips() {
    assert_eq!(
        store_and_read(
            "Map of Text to List of Whole",
            "empty",
            "{\"$map\":[[\"ada\",[1,2]]]}"
        ),
        "{\"$map\":[[\"ada\",[1,2]]]}"
    );
}

#[test]
fn a_list_of_maps_round_trips() {
    // The recursion in the other direction: a marker nested inside an
    // array rather than an array inside a marker.
    assert_eq!(
        store_and_read(
            "List of Map of Text to Whole",
            "empty",
            "[{\"$map\":[[\"a\",1]]},{\"$map\":[]}]"
        ),
        "[{\"$map\":[[\"a\",1]]},{\"$map\":[]}]"
    );
}

#[test]
fn a_record_round_trips() {
    let source = "\
record Entry
    name is Text
    count is Whole

state held is durable Entry starting Entry with name is \"\", count is 0

view
    Column
        when held
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text \"held\"
        Button \"store\"
            on click
                set held to Entry with name is \"ada\", count is 1
";
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(source, Arc::clone(&store), Environment::empty());
    host.invoke("held.set", "[{\"name\":\"ada\",\"count\":1}]")
        .expect("a record could be written");
    assert_eq!(
        host.invoke("held", "[]").expect("a record could be read"),
        "{\"name\":\"ada\",\"count\":1}"
    );
}

#[test]
fn a_record_holding_a_map_round_trips() {
    // A marker nested inside a record's field: the case that proves the
    // encoding recurses through objects and not only through arrays.
    let source = "\
record Tally
    label is Text
    counts is Map of Text to Whole

state held is durable Tally starting Tally with label is \"\", counts is empty

view
    Column
        when held
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text \"held\"
        Button \"store\"
            on click
                set held to Tally with label is \"votes\", counts is [\"ada\" to 3]
";
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(source, Arc::clone(&store), Environment::empty());
    let wire = "{\"label\":\"votes\",\"counts\":{\"$map\":[[\"ada\",3]]}}";
    host.invoke("held.set", &format!("[{wire}]"))
        .expect("a record with a map could be written");
    assert_eq!(
        host.invoke("held", "[]")
            .expect("a record with a map could be read"),
        wire,
        "a map nested in a record did not survive"
    );
}

#[test]
fn a_choice_round_trips() {
    // A variant is `{ tag, fields }` — an ordinary object, so it rides as
    // one, and the fields recurse like any other array.
    let source = "\
choice Status
    Idle
    Counting with total is Whole

state held is durable Status starting Idle

view
    Column
        when held
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text \"held\"
        Button \"store\"
            on click
                set held to Counting with total is 5
";
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(source, Arc::clone(&store), Environment::empty());
    host.invoke("held.set", "[{\"tag\":\"Counting\",\"fields\":[5]}]")
        .expect("a choice could be written");
    assert_eq!(
        host.invoke("held", "[]").expect("a choice could be read"),
        "{\"tag\":\"Counting\",\"fields\":[5]}"
    );
}

#[test]
fn a_written_value_survives_a_restart_unchanged() {
    // Round-tripping through memory is not the claim; `durable` is. The
    // encoded bytes have to mean the same thing after the process that
    // wrote them is gone.
    let mut path = std::env::temp_dir();
    path.push(format!("zdc-round-trip-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let source = program("Map of Text to Whole", "empty");
    let wire = "{\"$map\":[[\"ada\",1],[\"grace\",2]]}";

    {
        let store: Arc<dyn DurableStore> =
            Arc::new(EmbeddedStore::open(&path).expect("the store opens"));
        let host = host_on(&source, store, Environment::empty());
        host.invoke("held.set", &format!("[{wire}]"))
            .expect("a map could be written");
    }

    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::open(&path).expect("the store reopens"));
    let host = host_on(&source, store, Environment::empty());
    assert_eq!(
        host.invoke("held", "[]").expect("a map could be read"),
        wire,
        "a durable map did not survive a restart"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_encoding_marker_cannot_collide_with_a_record_field() {
    // `$` is in neither `XID_Start` nor `XID_Continue`, so the lexer
    // cannot produce an identifier containing one and no record field can
    // ever be named `$map`. This asserts the property the encoding relies
    // on, at the front end, rather than trusting the claim.
    let source = "\
record Bad
    $map is Whole

view
    Column
        Text \"x\"
";
    assert!(
        zdc_parser::parse(source).is_err(),
        "a field named `$map` parsed, so the wire marker is ambiguous"
    );
}

#[test]
fn an_unrecognised_argument_shape_is_still_refused() {
    // The decoder is permissive by design — it must not fail a page over
    // an event a newer server invented — so the arity and array checks
    // stay the thing that refuses nonsense.
    let source = program("Whole", "0");
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = host_on(&source, store, Environment::empty());
    assert_eq!(
        host.invoke("held.set", "{\"$map\":[]}")
            .expect_err("a bare object is not an argument array")
            .status(),
        400
    );
}

/// Suppress the unused-helper warning: `round_trip` is kept because it is
/// the shape a future property test wants, and deleting it would mean
/// writing it again.
#[allow(dead_code)]
fn unused() {
    let _ = round_trip;
    let _ = endpoints;
}
