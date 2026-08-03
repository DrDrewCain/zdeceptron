//! The fourth placement, end to end through emission — spec §14C.3b and
//! §17.4.8.
//!
//! The claim under test is narrow and checkable: a `static` signal costs
//! **nothing** at run time. Not a cheap request, not a cached one — none.
//! So every test here is about what is *absent* from the output as much as
//! what is present: no `$remote`, no getter, no cell, no function bundle.
//!
//! `zdc build` computes these values by running the build root under the
//! host's JavaScript runtime. These tests install none, so they do the same
//! thing in two halves: the printed module is evaluated in the embedded
//! engine every other test here uses, and the values it yields are then fed
//! back in as the build host's answers.

mod support;

use std::collections::BTreeMap;

use boa_engine::{Context, Source};
use support::{build_module_of, context, run, try_compile_with_statics};

const WRITING: &str = r#"record Post
    slug  is Text
    title is Text

state posts is static List of Post starting [(Post with slug is "one", title is "First"), (Post with slug is "two", title is "Second")]
state heading is static Text from headingFor with posts

function headingFor with all
    give "Writing"

state query is client Text starting ""

view
    Column
        Heading heading
        Input query, hint is "filter"
        each post in posts
            Text post.title
"#;

/// The build host's answers for [`WRITING`], obtained by running the module
/// the compiler printed rather than by writing them down. A hand-written
/// expectation here would test the inliner against a fiction.
fn evaluated(source: &str) -> BTreeMap<String, String> {
    let module =
        build_module_of(source, "test.zd").expect("a program with `static` has a build root");
    let mut context = Context::default();
    let driver = "\
        (() => { let out = []; \
         for (const k of Object.keys($values)) out.push(k + '\\u0001' + JSON.stringify($values[k])); \
         return out.join('\\u0002'); })()";
    let printed = run(&mut context, &module.source, driver);

    let mut values = BTreeMap::new();
    for entry in printed.split('\u{2}') {
        let (name, json) = entry.split_once('\u{1}').expect("name and value");
        values.insert(name.to_string(), json.to_string());
    }
    assert_eq!(
        values.keys().cloned().collect::<Vec<_>>(),
        module
            .statics
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>(),
        "the module must produce a value for every `static` it declares"
    );
    values
}

/// §17.4.8's named cost is paid only by programs that incur it: a program
/// with no `static` state must not need a JavaScript runtime on the build
/// host, so there is nothing to run.
#[test]
fn a_program_with_no_static_state_has_no_build_root_to_run() {
    let source = "state count is client Whole starting 0\n\nview\n    Text count\n";
    assert!(build_module_of(source, "test.zd").is_none());
}

#[test]
fn the_build_root_computes_every_static_signal() {
    let values = evaluated(WRITING);
    assert_eq!(
        values.get("posts").map(String::as_str),
        Some(r#"[{"slug":"one","title":"First"},{"slug":"two","title":"Second"}]"#)
    );
    // A *derived* `static` is computed at build time too, so what ships is
    // the answer and not the function that found it.
    assert_eq!(
        values.get("heading").map(String::as_str),
        Some("\"Writing\"")
    );
}

/// §14C.3b: "read once at build time and inlined". The literal is in the
/// bundle, and the bundle is all there is.
#[test]
fn a_static_value_is_inlined_into_the_client_bundle_as_a_literal() {
    let bundle = try_compile_with_statics(WRITING, "test.zd", evaluated(WRITING))
        .expect("the program compiles once the build host has answered");

    assert!(
        bundle
            .client_js
            .contains(r#"{"slug":"one","title":"First"}"#),
        "the content must appear as a literal:\n{}",
        bundle.client_js
    );
    assert!(bundle.client_js.contains(r#"String("Writing")"#));
}

/// The whole point, stated negatively. §14G.1.4's table gives `T` rather
/// than `Remote of T` for a `static` read from client context because no
/// boundary is crossed — so there must be no boundary in the output either.
#[test]
fn a_static_read_emits_no_remote_and_no_function_bundle() {
    let bundle = try_compile_with_statics(WRITING, "test.zd", evaluated(WRITING))
        .expect("the program compiles");

    assert!(
        !bundle.client_js.contains("$remote"),
        "a `static` read is not an RPC:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("rpc.js"),
        "nothing in this program talks to a network"
    );
    assert!(
        bundle.functions.is_empty(),
        "a `client` + `static` program emits no server function"
    );
}

/// A `static` signal has no cell and nothing that could ever change, so it
/// is neither `signal(...)` nor `derived(...)` and it is never read by
/// calling it. Emitting `heading()` would be a `ReferenceError` against a
/// name no root declares.
#[test]
fn a_static_signal_has_no_cell_and_no_getter() {
    let bundle = try_compile_with_statics(WRITING, "test.zd", evaluated(WRITING))
        .expect("the program compiles");

    assert!(!bundle.client_js.contains("const heading"));
    assert!(!bundle.client_js.contains("heading()"));
    assert!(!bundle.client_js.contains("const posts"));
    assert!(!bundle.client_js.contains("posts()"));
}

/// The manifest is client-readable, so it records the placement — which is
/// how a reader of `dist/` can tell that nothing here is fetched.
#[test]
fn the_manifest_records_the_fourth_placement() {
    let bundle = try_compile_with_statics(WRITING, "test.zd", evaluated(WRITING))
        .expect("the program compiles");
    assert!(bundle.manifest_json.contains(r#""posts":"static""#));
    assert!(bundle.manifest_json.contains(r#""functions":[]"#));
}

/// An inlined `undefined` is a blank page three layers from its cause, so a
/// missing answer is refused instead, naming the signal.
#[test]
fn a_static_read_with_no_computed_value_is_refused() {
    let Err(errors) = try_compile_with_statics(WRITING, "test.zd", BTreeMap::new()) else {
        panic!("nothing was computed, so there is nothing to inline");
    };
    let joined = errors
        .iter()
        .map(|e| e.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("`heading`"), "{joined}");
    assert!(joined.contains("§17.4.8"), "{joined}");
}

/// The emitted bundle has to *run*, not merely contain the right bytes: an
/// object literal in statement position parses as a block, which is what
/// `js::literal` parenthesises around.
#[test]
fn the_bundle_with_inlined_content_renders() {
    let bundle = try_compile_with_statics(WRITING, "test.zd", evaluated(WRITING))
        .expect("the program compiles");
    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div'); main($host); serialize($host)",
    );
    assert!(rendered.contains("First"), "{rendered}");
    assert!(rendered.contains("Second"), "{rendered}");
    assert!(rendered.contains("Writing"), "{rendered}");
}

/// A `static` value that reached the client through a record field must be
/// the field's value, not a getter call on a signal that does not exist.
#[test]
fn the_build_module_reads_static_state_as_a_plain_const() {
    let module = build_module_of(WRITING, "test.zd").expect("a build root");
    assert!(
        module.source.contains("const heading = headingFor(posts)"),
        "{}",
        module.source
    );
    assert!(!module.source.contains("posts()"), "{}", module.source);
    let _ = Source::from_bytes(module.source.as_bytes());
}

const EMITTING: &str = r#"state title is static Text starting "Writing"
state feed is static Text from feedFor with title emitting "rss.xml"

function feedFor with heading
    give "<rss><title>" + heading + "</title></rss>"

view
    Text title
"#;

/// §14C.3b's sub-requirement: a `static` value may be *written* to a path
/// at build time, not only read. `rss.xml` and `llms.txt` are the case.
#[test]
fn an_emitting_signal_declares_its_file_in_the_build_root() {
    let module = build_module_of(EMITTING, "test.zd").expect("a build root");
    assert_eq!(
        module.emits,
        vec![("rss.xml".to_string(), "feed".to_string())]
    );
    assert!(
        module.source.contains("export const $files = {"),
        "{}",
        module.source
    );
    assert!(
        module.source.contains("'rss.xml': feed"),
        "{}",
        module.source
    );
}

/// Always exported, empty or not: the driver reads one shape, so "emits
/// nothing" and "predates file emission" must not look the same to it.
#[test]
fn a_build_root_that_emits_nothing_still_declares_an_empty_file_set() {
    let module = build_module_of(WRITING, "test.zd").expect("a build root");
    assert!(module.emits.is_empty());
    assert!(
        module.source.contains("export const $files = {};"),
        "{}",
        module.source
    );
}

/// The file is a build-time output, so nothing about it reaches the
/// browser — not the text, and not the function that produced it.
#[test]
fn an_emitted_file_costs_the_client_bundle_nothing() {
    let bundle = try_compile_with_statics(EMITTING, "test.zd", evaluated(EMITTING))
        .expect("the program compiles");
    assert!(
        !bundle.client_js.contains("feedFor"),
        "{}",
        bundle.client_js
    );
    assert!(!bundle.client_js.contains("<rss>"), "{}", bundle.client_js);
    assert!(
        bundle.client_js.contains(r#"String("Writing")"#),
        "{}",
        bundle.client_js
    );
}
