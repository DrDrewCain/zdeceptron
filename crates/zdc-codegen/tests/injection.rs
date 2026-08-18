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
//!
//! The second half of this file (from "the shape checks" below) came from
//! `feature/apps`, which audited the same question over the constructs that
//! decide what markup is built at all. The two halves are one audit: the
//! first is about a value's *path* to the output, the second about whether
//! a node reaches a checked path in the first place.
//!
//! **One path is audited elsewhere and is named here so the index stays
//! complete.** `build parts` (#305) reads a *widget name* out of a `.md`
//! file, which is the first thing from a content file that both reaches
//! `client.js` as a literal and selects what is rendered. It is closed at
//! the source rather than escaped at the sink — a name that is not a
//! declaration name is refused, because it names nothing the program could
//! have declared — and that is asserted in `parts.rs`, by
//! `a_widget_name_that_could_close_a_string_never_becomes_one`, which
//! needs a project on disk that this file's helpers do not build.

mod support;

use support::{compile_source, context, refusals, run, try_compile};

fn assert_refused(source: &str, needle: &str) {
    let messages = refusals(source);
    assert!(
        messages.iter().any(|message| message.contains(needle)),
        "expected a diagnostic mentioning `{needle}`, got:\n{}",
        messages.join("\n")
    );
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
    //
    // This program has a `view`, so it has a page. Unwrapping rather than
    // defaulting to `""` is deliberate: a missing shell would otherwise
    // satisfy the "payload did not reach it" assertion vacuously.
    let index_html = bundle
        .index_html
        .as_deref()
        .expect("a program with a `view` emits a page");
    // **Escaped, not absent.** A document now ships with its first paint
    // in it, so a program's own text legitimately appears in the shell —
    // `Text note` is a request to show that string. What must never
    // appear is the *markup*: an unescaped `<script>` would be a tag the
    // browser runs rather than characters it draws, which is the whole
    // of the difference between a rendered page and an injection.
    assert!(
        !index_html.contains("<script>alert(1)</script>"),
        "the payload reached the page shell as live markup:\n{index_html}"
    );
    assert!(
        index_html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "the payload must appear escaped, or this test proves nothing:\n{index_html}"
    );
    // falsifiable: the two arms are the two spellings the shell may use to
    // reach the module — its own `<script src>` element, or the boot
    // module it loads instead (#146). Neither is unconditional: a shell
    // that inlined `main()`, which is the emission this test exists to
    // forbid, contains neither.
    let boot = bundle.boot_js.as_deref().unwrap_or("");
    assert!(
        index_html.contains("src=\"./client.js\"")
            || (index_html.contains("src=\"./boot.js\"") && boot.contains("from './client.js'")),
        "the shell must load the module rather than inline it:\n{index_html}\n{boot}"
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

// --- the routes a value takes that are not the module ---------------------
//
// The same audit, for the three paths that do not go through a JavaScript
// string literal at all: the base of a class getter as the DOM finally
// sees it, the printed stylesheet, and the shape checks that decide what
// markup is built in the first place.

/// The one place a program's own text was interpolated into JavaScript
/// source rather than into a value.
///
/// `class is <signal>` binds `() => '<base> ' + getter()`, and `<base>` is
/// the element's classes joined. A second `class` argument put the
/// program's own literal into that base, unescaped, so
/// `class is "a'+alert(1)+'b"` closed the string and wrote a call into the
/// module. Two things stop it now: the base goes through `js::string`, and
/// an argument given twice is refused outright.
#[test]
fn a_class_literal_cannot_close_the_getter_it_is_interpolated_into() {
    assert_refused(
        "state c is client Text starting \"red\"\n\
         view\n\
         \x20   Column class is \"a'+alert(1)+'b\", class is c\n\
         \x20       Text \"x\"\n",
        "given `class` twice",
    );
}

/// The escaping half of the same repair, tested where the refusal cannot
/// reach it: one `class` argument, and a base carrying a quote through a
/// path that has nothing to do with duplication.
#[test]
fn the_base_of_a_class_getter_is_a_quoted_string_rather_than_raw_text() {
    let client = compile_source(
        "state c is client Text starting \"red\"\n\
         view\n\
         \x20   Column class is c\n\
         \x20       Text \"x\"\n",
    )
    .client_js;
    assert!(
        client.contains("bindAttr($n0, 'class', () => 'zd-col ' + (c)());"),
        "{client}"
    );
}

/// The same program in the engine: the class the element ends up with is
/// the text the program wrote, and evaluating the module runs nothing the
/// program did not ask for.
#[test]
fn a_quote_in_a_class_reaches_the_dom_as_text_and_not_as_code() {
    let client = compile_source(
        "state c is client Text starting \"it's fine\"\n\
         view\n\
         \x20   Column class is c\n\
         \x20       Text \"x\"\n",
    )
    .client_js;
    let mut context = context(false);
    let found = run(
        &mut context,
        &client,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         html($host);",
    );
    assert_eq!(
        found,
        "<div><div class=\"zd-col it's fine\"><span>x</span></div></div>"
    );
}

/// The same property asserted as *behaviour* rather than as text: a class
/// literal shaped like an expression must not evaluate when the module
/// does.
///
/// A string-matching test can be satisfied by output that still runs the
/// payload under a different spelling, so the payload here writes to a
/// global and the test asks the engine whether it ever ran. This is the
/// one program shape that reached the getter's base with a single `class`
/// argument, so the duplicate-argument refusal cannot answer it and only
/// the escaping can.
#[test]
fn a_class_literal_shaped_like_an_expression_does_not_evaluate() {
    let client = compile_source(
        "state c is client Text starting \"a'+(globalThis.$pwned='ran')+'b\"\n\
         view\n\
         \x20   Column class is c\n\
         \x20       Text \"x\"\n",
    )
    .client_js;
    let mut engine = context(false);
    let pwned = run(
        &mut engine,
        &client,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         String(globalThis.$pwned);",
    );
    assert_eq!(
        pwned, "undefined",
        "the class literal executed as JavaScript:\n{client}"
    );

    // falsifiable: the class must still *be* what the program wrote, so a
    // compiler that satisfied the line above by dropping the value
    // entirely fails here.
    let mut fresh = context(false);
    let html = run(
        &mut fresh,
        &client,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         html($host);",
    );
    assert!(
        html.contains("a'+(globalThis.$pwned='ran')+'b"),
        "the class attribute lost the text the program wrote:\n{html}"
    );
}

// --- the generated stylesheet ---------------------------------------------

/// A folded style declaration is *printed* into `styles.css`, so a value
/// carrying `}` does not make a bad declaration — it ends the rule and
/// opens one for a selector the program never wrote. `bindStyle` goes
/// through `setProperty`, which parses one declaration and drops the rest,
/// which is why only the folded arm needs the check.
#[test]
fn a_style_value_may_not_end_the_rule_it_is_folded_into() {
    assert_refused(
        "view\n\
         \x20   Column weight is \"bold; } body { display: none } x {\"\n\
         \x20       Text \"x\"\n",
        "would end that rule",
    );
}

#[test]
fn a_style_value_may_not_smuggle_a_request_into_the_stylesheet() {
    assert_refused(
        "view\n\
         \x20   Column weight is \"url(https://example.com/x)\"\n\
         \x20       Text \"x\"\n",
        "would end that rule",
    );
}

/// The values a program actually writes still fold.
#[test]
fn an_ordinary_style_value_still_folds_into_a_class() {
    let bundle = compile_source(
        "view\n\
         \x20   Row padding is 8, weight is \"bold\"\n\
         \x20       Text \"a\"\n",
    );
    assert!(
        bundle
            .styles_css
            .contains(".zd-s0 { font-weight: bold; padding: 8px; }"),
        "{}",
        bundle.styles_css
    );
}

// --- the style vocabulary --------------------------------------------------
//
// One test per style argument, and the *same* exploit through every one of
// them, because this is an audit of paths and each argument is a path.
// `weight` above is the argument that predates the value grammars, and it
// is refused by the character allowlist; every argument below is refused by
// its own grammar instead, and the two are different code with the same
// obligation. A per-value audit that stopped at `weight` would have said
// the stylesheet was covered.
//
// The payload is the one that has always been the interesting one:
// `; } body { … } x {` closes the declaration, closes the rule, writes a
// rule for a selector the program never named, and reopens a rule so that
// what follows still parses. A value that can do that through any argument
// is a defacement of the whole page.

/// Every style argument, the payload, and the property it would have
/// written. The list is written out here rather than read from the
/// compiler's own table, deliberately: a table-driven version would go on
/// passing if an argument were deleted, and the point of this test is that
/// each argument that *exists* has been tried.
const PAYLOAD: &str = "; } body { display: none } x {";

#[test]
fn no_style_argument_can_write_a_rule_of_its_own() {
    let arguments = [
        ("color", "red"),
        ("background", "red"),
        ("backdrop", "/a.png"),
        ("margin", "8"),
        ("border", "1"),
        ("borderColor", "red"),
        ("borderStyle", "solid"),
        ("radius", "8"),
        ("display", "block"),
        ("grow", "1"),
        ("shrink", "1"),
        ("basis", "240"),
        ("justify", "center"),
        ("align", "center"),
        ("gap", "8"),
        ("width", "320"),
        ("height", "200"),
        ("minWidth", "200"),
        ("maxWidth", "720"),
        ("minHeight", "40"),
        ("maxHeight", "400"),
        ("font", "serif"),
        ("size", "large"),
        ("lineHeight", "1.6"),
        ("textAlign", "center"),
        ("decoration", "struck"),
        ("overflow", "scroll"),
        ("position", "sticky"),
        ("top", "0"),
        ("right", "0"),
        ("bottom", "0"),
        ("left", "0"),
        ("layer", "1"),
        ("opacity", "50"),
        ("shadow", "low"),
        ("cursor", "pointer"),
        ("transition", "fast"),
        ("weight", "bold"),
        ("padding", "8"),
    ];
    let mut tried = 0;
    for (argument, ordinary) in arguments {
        tried += 1;
        // The payload appended to a value the argument does admit, so a
        // grammar that merely checked the prefix would let it through.
        let source = format!(
            "view\n\
             \x20   Column {argument} is \"{ordinary}{PAYLOAD}\"\n\
             \x20       Text \"x\"\n"
        );
        match try_compile(&source, "test.zd") {
            Err(_) => {}
            Ok(bundle) => panic!(
                "`{argument}` emitted rather than refusing the payload:\n{}",
                bundle.styles_css
            ),
        }
    }
    assert_eq!(
        tried, 39,
        "every style argument must have been tried; the list holds {tried}"
    );
}

/// The same payload through every prefixed spelling of one argument. A
/// prefixed declaration is printed into a rule of its own, a `:hover`, or
/// one inside an `@media`, so it is a second printing site, and a check that
/// only guarded the unprefixed one would guard half of them.
#[test]
fn no_prefixed_style_argument_can_write_a_rule_of_its_own() {
    let mut tried = 0;
    for prefix in [
        "hover", "focus", "active", "disabled", "narrow", "wide", "dark",
    ] {
        tried += 1;
        let source = format!(
            "view\n\
             \x20   Column {prefix}Background is \"red{PAYLOAD}\"\n\
             \x20       Text \"x\"\n"
        );
        match try_compile(&source, "test.zd") {
            Err(_) => {}
            Ok(bundle) => panic!(
                "`{prefix}Background` emitted rather than refusing the payload:\n{}",
                bundle.styles_css
            ),
        }
    }
    assert_eq!(tried, 7, "every prefix must have been tried");
}

/// The other half of the audit: no `@media` or selector a program wrote
/// reaches the sheet, however the payload is shaped. A grammar that
/// refused `}` but admitted `{` would still let a program open a block.
#[test]
fn no_style_value_can_open_a_block_in_the_sheet() {
    let mut tried = 0;
    for payload in [
        "red { }",
        "red @media screen",
        "red/*",
        // `\\7d` in the source is the one backslash the CSS escape needs:
        // a literal escapes it since #16, so writing it raw would be a lex
        // error and the stylesheet rule would never be reached.
        "red\\\\7d",
        "red;color:blue",
        "url(https://evil.example/x)",
    ] {
        tried += 1;
        let source = format!(
            "view\n\
             \x20   Column color is \"{payload}\"\n\
             \x20       Text \"x\"\n"
        );
        match try_compile(&source, "test.zd") {
            Err(_) => {}
            Ok(bundle) => panic!("`{payload}` reached the sheet:\n{}", bundle.styles_css),
        }
    }
    assert_eq!(tried, 6, "every payload shape must have been tried");
}

/// A backdrop is the one argument whose value is printed inside a
/// function call, so it has a delimiter of its own to escape from. The
/// scheme filter does not catch this: the payload names no scheme.
#[test]
fn a_backdrop_cannot_close_the_url_and_name_another_host() {
    let source = "view\n\
                  \x20   Column backdrop is \"/a.png), url(https://evil.example/x\"\n\
                  \x20       Text \"x\"\n";
    match try_compile(source, "test.zd") {
        Err(_) => {}
        Ok(bundle) => panic!(
            "a second host reached the stylesheet:\n{}",
            bundle.styles_css
        ),
    }
}

/// Every rule the emitter prints balances its braces. An unbalanced sheet
/// is a sheet whose tail has been swallowed, which is what an unclosed
/// `@media` or an unclosed `url(` would produce without any value
/// containing a brace at all.
#[test]
fn the_generated_stylesheet_balances_whatever_it_was_given() {
    let bundle = compile_source(
        "view\n\
         \x20   Column background is \"surface\", hoverBackground is \"raised\", \
         narrowPadding is 8, widePadding is 32, darkColor is \"ink\", transition is \"fast\"\n\
         \x20       Text \"x\"\n",
    );
    assert_eq!(
        bundle.styles_css.matches('{').count(),
        bundle.styles_css.matches('}').count(),
        "the sheet does not balance:\n{}",
        bundle.styles_css
    );
    assert_eq!(
        bundle.styles_css.matches('(').count(),
        bundle.styles_css.matches(')').count(),
        "the sheet's parentheses do not balance:\n{}",
        bundle.styles_css
    );
}

// --- the shape checks -----------------------------------------------------

/// `only_children` is about what ends up a DOM child, and `each`, `if`,
/// `when` and a component's own scope all place their contents directly in
/// the parent. Checking only the direct `HirNode::Element` children let
/// every one of them through, so `List / each / Column` emitted a `<div>`
/// inside a `<ul>` — which is what the check exists to prevent.
#[test]
fn a_list_is_checked_through_the_constructs_that_place_nodes_in_it() {
    for (what, source) in [
        (
            "each",
            "state xs is client List of Text starting [\"a\"]\n\
             view\n\
             \x20   List\n\
             \x20       each x in xs\n\
             \x20           Column\n\
             \x20               Text x\n",
        ),
        (
            "if",
            "state open is client Truth starting yes\n\
             view\n\
             \x20   List\n\
             \x20       if open\n\
             \x20           Column\n\
             \x20               Text \"x\"\n",
        ),
        (
            "a component with state of its own",
            "component Bad\n\
             \x20   state n is client Whole starting 0\n\
             \x20   Column\n\
             \x20       Text n\n\
             view\n\
             \x20   List\n\
             \x20       Bad\n",
        ),
    ] {
        let messages = refusals(source);
        assert!(
            messages
                .iter()
                .any(|message| message.contains("takes only `Item`")),
            "`List` accepted a `Column` placed by {what}:\n{}",
            messages.join("\n")
        );
    }
}

/// The same constructs must not turn a legal list into a diagnostic.
#[test]
fn an_item_placed_by_each_is_still_an_item() {
    let bundle = compile_source(
        "state xs is client List of Text starting [\"a\"]\n\
         view\n\
         \x20   List\n\
         \x20       each x in xs\n\
         \x20           Item x\n",
    );
    assert!(bundle.client_js.contains("<ul>"), "{}", bundle.client_js);
}

// --- the two-way binding --------------------------------------------------

/// The binding is the first *positional* argument wherever it is written
/// among the named ones. The write analysis read `args.first()` instead,
/// so `Input hint is "…", name` left `name` with no setter and the emitter
/// refused itself with a message about its own internals.
#[test]
fn a_two_way_binding_is_found_after_a_named_argument() {
    let client = compile_source(
        "state name is client Text starting \"\"\n\
         view\n\
         \x20   Input hint is \"type\", name\n",
    )
    .client_js;
    assert!(
        client.contains("const [name, setName] = signal('');"),
        "{client}"
    );
    assert!(client.contains("bindAttr($n0, 'value', name);"), "{client}");
    assert!(
        client.contains("on($n0, 'input', (e) => setName(e.target.value));"),
        "{client}"
    );
}
