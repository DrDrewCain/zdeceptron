//! One test per path a string a program wrote takes to emitted output.
//!
//! Every test here is named for the thing it prevents rather than for the
//! feature it exercises, because that is what a reader of a failure needs
//! to know. The programs are the exploits: each one was written first,
//! run against the compiler, and only then fixed.
//!
//! The organising idea is that this is an audit of *paths*, not of values.
//! The same style value is safe through `bindStyle` — the CSSOM parses one
//! declaration and drops the rest — and was a page-wide defacement through
//! the folded stylesheet, which prints its declarations. A per-value audit
//! sees one value and calls it done.

mod support;

use support::{compile_source, try_compile};

/// Whether the compiler will emit this program at all.
fn compiles(source: &str) -> bool {
    try_compile(source, "test.zd").is_ok()
}

// --- the `import` clause ---------------------------------------------------
//
// Not audited here, because this compiler has no such path: a `foreign`'s
// primitive layer is inlined into the module (§17.4.7) rather than
// imported, so no program-supplied module specifier or export name is
// written into an `import` clause at all. The four tests covering that
// path live with the work that creates it, on `feature/escape`; bringing
// them here would test an emission this branch does not perform.

// --- the generated `class` getter -----------------------------------------

/// `class is "…"` folds into the base of a generated getter, which used to
/// be built as `format!("() => '{base} ' + …")`. A literal closing that
/// apostrophe wrote expressions into the module.
#[test]
fn a_class_literal_cannot_close_the_getters_string() {
    let source = "state c is client Text starting \"red\"\n\
                  \n\
                  view\n\
                  \x20   Column class is \"a'+alert(1)+'b\", class is c\n\
                  \x20       Text \"x\"\n";
    let Ok(bundle) = try_compile(source, "test.zd") else {
        // Refusing the program is a correct answer too: an argument given
        // twice has no meaning. What must never happen is emission.
        return;
    };
    assert!(
        !bundle.client_js.contains("alert(1)+'"),
        "the literal escaped its string:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("\\'+alert(1)+\\'"),
        "expected the literal escaped in place:\n{}",
        bundle.client_js
    );
}

/// U+2028 is a line terminator in JavaScript source but not in `.zd`, so a
/// literal carrying one would end the getter's string mid-module.
#[test]
fn a_class_literal_carrying_a_line_terminator_is_escaped() {
    let source = "state c is client Text starting \"red\"\n\
                  \n\
                  view\n\
                  \x20   Column class is \"a\u{2028}b\", class is c\n\
                  \x20       Text \"x\"\n";
    if let Ok(bundle) = try_compile(source, "test.zd") {
        assert!(
            !bundle.client_js.contains('\u{2028}'),
            "a raw U+2028 reached the module:\n{}",
            bundle.client_js
        );
    }
}

// --- string literals in expression position -------------------------------

/// Every C0 control the `.zd` string rule admits — which is all of them
/// but the newline — must be escaped rather than written raw. A raw
/// U+001B in `client.js` is an ANSI escape for whatever reads it.
#[test]
fn a_text_literal_never_carries_a_raw_control_character_into_the_module() {
    let source = "state greeting is client Text starting \"a\u{1b}[2J\u{7}b\"\n\
                  \n\
                  view\n\
                  \x20   Column\n\
                  \x20       Text greeting\n";
    let bundle = compile_source(source);
    assert!(
        !bundle.client_js.contains('\u{1b}') && !bundle.client_js.contains('\u{7}'),
        "a raw control character reached the module:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("\\u001b[2J\\u0007"),
        "expected the controls escaped in place:\n{}",
        bundle.client_js
    );
}

// --- markup ----------------------------------------------------------------

/// The template is one HTML string parsed by the browser, so a literal in
/// text or attribute position must not be able to start a tag or close an
/// attribute.
#[test]
fn a_literal_in_markup_cannot_open_a_tag_or_close_an_attribute() {
    let source = "view\n\
                  \x20   Column\n\
                  \x20       Text \"<img src=x onerror=alert(1)>\"\n\
                  \x20       Column class is \"a b\"\n\
                  \x20           Text \"y\"\n";
    let bundle = compile_source(source);
    assert!(
        !bundle.client_js.contains("<img"),
        "markup was baked in raw:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("&lt;img"),
        "expected the markup escaped:\n{}",
        bundle.client_js
    );
}

// --- the folded stylesheet -------------------------------------------------

/// `styles.css` *prints* its declarations, so a value carrying `;` or `}`
/// is not a bad style — it is a new rule for a selector nothing in the
/// program wrote. The reactive arm is safe for the opposite reason: it
/// goes through `setProperty`, which parses one declaration and drops the
/// rest. Same value, two paths, one of them exploitable.
#[test]
fn a_folded_style_value_cannot_begin_a_rule_of_its_own() {
    let source = "view\n\
                  \x20   Column\n\
                  \x20       Text \"x\", weight is \"bold; } body { display: none } .x {\"\n";
    let Ok(bundle) = try_compile(source, "test.zd") else {
        return;
    };
    assert!(
        !bundle.styles_css.contains("body {"),
        "a rule for `body` was written into the stylesheet:\n{}",
        bundle.styles_css
    );
}

// --- the manifest ----------------------------------------------------------

/// `manifest.json` is parsed as JSON, and JSON's escapes are not
/// JavaScript's. The names in it are the program's own.
#[test]
fn the_manifest_is_json_a_parser_accepts() {
    let bundle = compile_source(
        "state count is client Whole starting 0\n\nview\n    Column\n        Text count\n",
    );
    assert!(
        bundle.manifest_json.contains("\"count\":\"client\""),
        "{}",
        bundle.manifest_json
    );
    assert!(
        !bundle.manifest_json.contains('\u{1b}'),
        "{}",
        bundle.manifest_json
    );
}

// --- names emitted as syntax ----------------------------------------------

/// A ZDeceptron name becoming a JavaScript name is the other half of the
/// class: `class`, `import` and `eval` are all things a program may call
/// its state, and all things JavaScript already means something by.
#[test]
fn a_name_javascript_reserves_is_renamed_rather_than_emitted() {
    let bundle = compile_source(
        "state class is client Whole starting 1\n\
         state import is client Whole starting 2\n\
         state eval is client Whole starting 3\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       Text class\n\
         \x20       Text import\n\
         \x20       Text eval\n",
    );
    for reserved in ["const class", "const import", "const eval"] {
        assert!(
            !bundle.client_js.contains(reserved),
            "`{reserved}` was emitted:\n{}",
            bundle.client_js
        );
    }
}

/// Every compiler-generated name begins with `$`, which is outside XID, so
/// a program cannot spell one. This is the property the whole naming
/// scheme rests on, and nothing tested it from the program's side.
#[test]
fn a_program_cannot_spell_a_compiler_generated_name() {
    let bundle = compile_source(
        "state t0 is client Whole starting 1\n\
         state n0 is client Whole starting 2\n\
         state f0 is client Whole starting 3\n\
         state byPosition is client Whole starting 4\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       Text t0\n\
         \x20       Text n0\n\
         \x20       Text f0\n\
         \x20       Text byPosition\n",
    );
    // The program's four names are emitted unsuffixed, which is only
    // possible because none of them collides with `$t0`, `$n0`, `$f0` or
    // `$byPosition`.
    for name in ["[t0]", "[n0]", "[f0]", "[byPosition]"] {
        assert!(
            bundle.client_js.contains(name),
            "`{name}` was renamed, so a program did reach the generated namespace:\n{}",
            bundle.client_js
        );
    }
}
