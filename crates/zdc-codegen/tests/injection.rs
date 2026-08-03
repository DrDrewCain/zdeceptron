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

use support::{compile_source, refusals, try_compile};

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

// --- the block text literal ------------------------------------------------
//
// Everything above this line was written when a text literal could not
// contain a quote or a newline. Several of the sites it covers were safe
// for exactly that reason and said so in their own comments. `"""` is the
// change that makes them reachable, so the same value — one carrying a
// quote, a line break and `</script>` at once — is run down all three
// paths separately, because a value that is safe on one is not thereby
// safe on another.

/// The payload: a double quote, an apostrophe, a real line break, and a
/// closing script tag. Written as a block literal, which is the only way
/// a `.zd` source can say it.
const HOSTILE: &str = "\"\"\"\n\
                       \x20   he said \"stop'\" </script><script>alert(1)</script>\n\
                       \x20   second line\n\
                       \x20   \"\"\"";

/// **The JavaScript emitter.** The literal becomes a single-quoted string
/// in `client.js`, so an apostrophe or a raw line break would end it and
/// leave the rest as program text.
#[test]
fn a_block_literal_cannot_end_the_javascript_string_it_becomes() {
    let source = format!(
        "state note is client Text starting {HOSTILE}\n\nview\n    Column\n        Text note\n"
    );
    let bundle = compile_source(&source);

    assert!(
        !bundle.client_js.contains("stop'\""),
        "the apostrophe reached `client.js` unescaped:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("stop\\'"),
        "expected the apostrophe escaped in place:\n{}",
        bundle.client_js
    );
    // The line break is the new one. A raw one inside a single-quoted
    // JavaScript string is a syntax error at best and a statement
    // boundary at worst.
    assert!(
        bundle.client_js.contains("\\nsecond line"),
        "expected the line break escaped in place:\n{}",
        bundle.client_js
    );
    // `client.js` is its own module file and is never inlined into a
    // `<script>` element, so `</script>` in it closes nothing — and the
    // shell is asserted to keep it that way rather than assumed to.
    assert!(
        !bundle.index_html.contains("alert(1)"),
        "the payload reached the page shell:\n{}",
        bundle.index_html
    );
    assert!(
        bundle.index_html.contains("src=\"./client.js\"")
            || bundle.index_html.contains("from './client.js'"),
        "the shell must load the module rather than inline it:\n{}",
        bundle.index_html
    );
}

/// **The HTML emitter.** The same value in markup text position, where it
/// is baked into a template string the browser parses as HTML.
#[test]
fn a_block_literal_in_markup_cannot_open_a_tag() {
    let source = format!("view\n    Column\n        Text {HOSTILE}\n");
    let bundle = compile_source(&source);

    assert!(
        !bundle.client_js.contains("</script>"),
        "a closing script tag was baked into the template:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("<script>"),
        "an opening script tag was baked into the template:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("&lt;/script&gt;"),
        "expected the tag escaped in place:\n{}",
        bundle.client_js
    );
}

/// And in attribute position, which escapes a different set: a `>` does
/// not end an attribute value and a `"` does.
#[test]
fn a_block_literal_in_an_attribute_cannot_close_it() {
    let source =
        format!("state who is client Text starting \"\"\n\nview\n    Column\n        Input who, hint is {HOSTILE}\n");
    let bundle = compile_source(&source);
    assert!(
        !bundle.client_js.contains("stop'\" <"),
        "the quote closed the attribute:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("</script>"),
        "`</script` ends a script element wherever it appears, an attribute \
         value included, so it must not reach the template raw:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("&quot;stop"),
        "expected the quote escaped in place:\n{}",
        bundle.client_js
    );
}

/// **The stylesheet emitter.** The same value again, and here it is
/// refused rather than escaped — `styles.css` prints its declarations, so
/// there is no escape that keeps a value inside its own rule.
#[test]
fn a_block_literal_cannot_be_folded_into_the_stylesheet() {
    let source = format!("view\n    Column\n        Text \"x\", weight is {HOSTILE}\n");
    let messages = refusals(&source);
    assert!(
        messages.iter().any(|m| m.contains("styles.css")),
        "a block literal was folded into a CSS rule: {messages:?}"
    );

    // And the line break alone, with nothing else hostile in it, because
    // that is the character `"""` newly makes reachable.
    let plain = "view\n    Column\n        Text \"x\", weight is \"\"\"\n            bold\n            normal\n            \"\"\"\n";
    let messages = refusals(plain);
    assert!(
        messages.iter().any(|m| m.contains("styles.css")),
        "a line break was folded into a CSS rule: {messages:?}"
    );
}

/// The multi-line template, run. `examples/terminal-help.zd` is the port
/// of the portfolio's `help` command — twenty-two lines that were an
/// array of strings there because the language had no other shape for
/// them — and what is checked is that it is *text*: that splitting it
/// gives back the lines, that the margin came off, and that the relative
/// indentation the command list lines up with did not.
#[test]
fn a_multi_line_template_is_text_the_library_can_take_apart() {
    let bundle = support::compile_example("examples/terminal-help.zd");
    let js = &bundle.client_js;
    assert!(
        js.contains("available commands:\\n  ls [projects]"),
        "the margin was not removed, or the line break was not kept:\n{js}"
    );
    assert!(
        js.contains("psst: dinosaurs once roamed this terminal"),
        "the last line did not survive:\n{js}"
    );
    assert!(
        !js.contains("\n    available commands"),
        "a raw line break reached the module:\n{js}"
    );
    // The quote the one-line rule could not carry at all.
    assert!(
        js.contains("you can\\'t put a quote"),
        "expected the apostrophe escaped in place:\n{js}"
    );
    // A double quote needs no escape inside a single-quoted JavaScript
    // string, and is carried through as itself. It is the *apostrophe*
    // that would have ended the literal, and that is escaped above.
    assert!(
        js.contains("said \"you can"),
        "expected the double quote carried through:\n{js}"
    );
}
