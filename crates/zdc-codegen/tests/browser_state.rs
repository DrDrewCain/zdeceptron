//! The `remembered` placement and `media` queries, as emitted.
//!
//! Both features are decisions about the browser, and the browser is not
//! here. What *is* here is everything that has to be right before a
//! browser can be asked: which runtime files a bundle links, what the
//! declaration compiles to, and where the storage key comes from.
//!
//! The end-to-end claim — that a value written on one page load is there
//! on the next — is `zdc-cli`'s
//! `a_remembered_value_survives_a_reload_in_a_real_browser`, and it has to
//! be: this engine has no `localStorage`, no origin and no second load, so
//! a runtime that wrote nothing at all would pass every assertion below.
//! These tests and that one are halves of one claim, and neither is
//! sufficient.

mod support;

use support::try_compile;
use zdc_codegen::Bundle;

fn compile(source: &str) -> Bundle {
    try_compile(source, "browser-state.zd").expect("the fixture compiles")
}

fn linked(bundle: &Bundle) -> Vec<&str> {
    bundle.runtime.iter().copied().collect()
}

const REMEMBERED: &str = r#"
state visits is remembered Whole starting 0

view
    Column
        Text visits
        Button "more"
            on click
                add 1 to visits
"#;

/// A `remembered` signal is a cell like any other, built by a different
/// constructor.
///
/// The emitted shape is the whole of the placement's cost at the call
/// site: `remembered` returns the same `[read, write]` pair `signal` does,
/// so every binding, every `derived` and every handler downstream is
/// emitted unchanged. A placement that needed the *readers* to change
/// would be a much larger feature than this one, and the assertion that it
/// did not is here rather than in a comment.
#[test]
fn a_remembered_signal_is_a_cell_built_by_the_store() {
    let bundle = compile(REMEMBERED);
    assert!(
        bundle
            .client_js
            .contains("const [visits, setVisits] = remembered('visits', 0);"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle
            .client_js
            .contains("import { remembered } from './runtime/remembered.js';"),
        "{}",
        bundle.client_js
    );
}

/// **The storage key is the name in the source, not the name in the
/// output.**
///
/// Emitted names are renamed to dodge JavaScript's reserved words and the
/// setter convention, so a key taken from one would move when an unrelated
/// declaration was added — and a key that moves loses every value that had
/// survived a reload, which is the one thing this placement promises not
/// to do. `class` is a name the emitter must rename, so it is what the
/// fixture declares.
#[test]
fn the_storage_key_follows_the_source_name_through_a_rename() {
    let bundle = compile(
        r#"
state class is remembered Text starting "a"

view
    Column
        Text class
"#,
    );
    assert!(
        bundle.client_js.contains("remembered('class'"),
        "the key is the source name, whatever the emitted binding is called:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("const [class,"),
        "the fixture is only a test of the rename if the name is actually renamed:\n{}",
        bundle.client_js
    );
}

/// §16.3.1, both directions.
///
/// A program with a `remembered` signal links the store wrapper *and*
/// `wire.js`, because a ZD value has to survive as a string and
/// `JSON.stringify` turns a `Map` into `{}` — the bug `wire.js` exists to
/// fix, on the same trip a `durable` value makes. It does **not** link
/// `rpc.js` or `store.js`: nothing here crosses a boundary, and a
/// placement that quietly pulled in the RPC client would be a network
/// dependency nobody asked for.
#[test]
fn a_remembered_program_links_the_store_and_the_wire_format_and_no_rpc() {
    let bundle = compile(REMEMBERED);
    let linked = linked(&bundle);
    assert!(linked.contains(&"runtime/remembered.js"), "{linked:?}");
    assert!(linked.contains(&"runtime/wire.js"), "{linked:?}");
    assert!(!linked.contains(&"runtime/rpc.js"), "{linked:?}");
    assert!(!linked.contains(&"runtime/store.js"), "{linked:?}");
    assert!(!linked.contains(&"runtime/media.js"), "{linked:?}");
}

/// The other half of the split, and the half that makes it honest: a
/// program that declares no `remembered` state ships none of it.
#[test]
fn a_program_without_browser_state_ships_neither_module() {
    let bundle = compile(
        r#"
state count is client Whole starting 0

view
    Column
        Text count
"#,
    );
    let linked = linked(&bundle);
    assert!(!linked.contains(&"runtime/remembered.js"), "{linked:?}");
    assert!(!linked.contains(&"runtime/media.js"), "{linked:?}");
    assert!(!linked.contains(&"runtime/wire.js"), "{linked:?}");
}

/// **One cell per distinct query, however many times it is read.**
///
/// `matchMedia` returns a live `MediaQueryList`, so subscribing twice to
/// one query installs two listeners that always agree — wasted work, and
/// two sources of truth for one fact. The emitter hoists by query string
/// rather than by read site, and this is what pins that: three reads, two
/// queries, two cells.
#[test]
fn one_cell_is_hoisted_per_distinct_media_query() {
    let bundle = compile(
        r#"
state dark is client Truth from media "(prefers-color-scheme: dark)"
state calm is client Truth from media "(prefers-reduced-motion: reduce)"
state also is client Truth from media "(prefers-color-scheme: dark)"

view
    Column
        if dark
            Text "dark"
        if calm
            Text "calm"
        if also
            Text "also"
"#,
    );
    let js = &bundle.client_js;
    assert_eq!(
        js.matches("mediaMatch(").count(),
        2,
        "two calls for two distinct queries, and no more — the import names \
         the symbol without calling it:\n{js}"
    );
    assert!(
        js.contains("const $q0 = mediaMatch('(prefers-color-scheme: dark)');"),
        "{js}"
    );
    assert!(
        js.contains("const $q1 = mediaMatch('(prefers-reduced-motion: reduce)');"),
        "{js}"
    );
    assert!(
        !js.contains("$q2"),
        "the repeated query took a third cell:\n{js}"
    );
    assert!(linked(&bundle).contains(&"runtime/media.js"));
}

/// A media query is read through its cell, so a view that shows one is
/// reactive to it.
///
/// The read compiles to `$q0()` — a call — and that is not cosmetic: the
/// runtime discovers dependencies at read time, so an emission that
/// inlined the boolean instead would render once and never change. That is
/// the read-it-once bug this construct exists to make unwritable, and it
/// would be invisible in any test that only checked the page's first
/// paint.
#[test]
fn a_media_query_is_read_through_its_cell_rather_than_inlined() {
    let bundle = compile(
        r#"
state dark is client Truth from media "(prefers-color-scheme: dark)"

view
    Column
        if dark
            Text "dark"
"#,
    );
    assert!(
        bundle
            .client_js
            .contains("const dark = derived(() => $q0());"),
        "{}",
        bundle.client_js
    );
}
