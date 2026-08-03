//! `Markup`, `Prose`, and `build markdown` — the one path in the language
//! along which a value is parsed as HTML.
//!
//! §16.3.5 says every runtime value reaches the DOM through `nodeValue`,
//! `setAttribute`, `.value` or `.checked`, none of which parses HTML, and
//! that this is why template cloning adds no injection surface. Rendering a
//! post means narrowing that claim, and the narrowing is only sound if the
//! type carries it. So the tests here come in three groups:
//!
//! 1. **It works** — a real `.md` file on disk becomes real DOM: headings,
//!    paragraphs, links, code. Asserted against the parsed tree rather than
//!    against the emitted string, because a string containing `<h1>` proves
//!    nothing about what a browser makes of it.
//! 2. **Nothing else gets in** — `Text` is not `Markup`, no operator
//!    produces one, no literal spells one, and a `secret` cannot reach one.
//! 3. **The ordinary path is untouched** — every other element still writes
//!    `nodeValue`, and the emitter still has exactly one call that parses.

mod support;

use std::collections::BTreeMap;

use support::{compile_example, context, refusals, repository_path, run, try_compile_with_statics};

/// The program under test is the checked-in one, and the content is the
/// checked-in `examples/content/` — including the hostile file.
fn writing_bundle() -> zdc_codegen::Bundle {
    let source = std::fs::read_to_string(repository_path("examples/writing.zd"))
        .expect("examples/writing.zd");
    let module = support::build_module_of(&source, "examples/writing.zd")
        .expect("writing.zd declares `static` state, so it has a build root");
    let evaluated = zdc_codegen::evaluate(&module, repository_path("examples").as_path())
        .unwrap_or_else(|error| panic!("the build root did not run: {}", error.report()));
    try_compile_with_statics(&source, "examples/writing.zd", evaluated.values)
        .unwrap_or_else(|errors| panic!("writing.zd: {}", errors[0].message))
}

/// Mount the bundle in the embedded engine and ask the DOM a question.
///
/// `root` is the mounted tree and `all(node, tag)` collects every element
/// under it with that tag, so an assertion can be written about nodes.
fn ask(bundle: &zdc_codegen::Bundle, expression: &str) -> String {
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        &format!(
            "function all(node, tag, out) {{\n\
             \x20 if (node.tagName === tag) out.push(node);\n\
             \x20 const kids = node.childNodes || [];\n\
             \x20 for (let i = 0; i < kids.length; i += 1) all(kids[i], tag, out);\n\
             \x20 return out;\n\
             }}\n\
             function tags(node, tag) {{ return all(node, tag, []); }}\n\
             function textOf(node) {{\n\
             \x20 if (node.kind === 'text') return node.nodeValue;\n\
             \x20 const kids = node.childNodes || [];\n\
             \x20 let out = '';\n\
             \x20 for (let i = 0; i < kids.length; i += 1) out += textOf(kids[i]);\n\
             \x20 return out;\n\
             }}\n\
             function handlers(node, out) {{\n\
             \x20 const names = Object.keys(node.attributes || {{}});\n\
             \x20 for (let i = 0; i < names.length; i += 1) {{\n\
             \x20   if (names[i].toLowerCase().indexOf('on') === 0) out.push(names[i]);\n\
             \x20 }}\n\
             \x20 const kids = node.childNodes || [];\n\
             \x20 for (let i = 0; i < kids.length; i += 1) handlers(kids[i], out);\n\
             \x20 return out;\n\
             }}\n\
             function hrefs(node) {{\n\
             \x20 const found = tags(node, 'a');\n\
             \x20 let out = [];\n\
             \x20 for (let i = 0; i < found.length; i += 1) out.push(found[i].attributes.href);\n\
             \x20 return out;\n\
             }}\n\
             const root = document.createElement('div');\n\
             main(root);\n\
             String({expression});\n"
        ),
    )
}

// --- 1. it works ----------------------------------------------------------

/// The acceptance criterion: a markdown post renders as HTML.
///
/// `hello-world.md` is a real file with a heading, a paragraph and an
/// inline code span. Each assertion is about a **node**: the tag, its
/// position in the tree, and its text — not about a substring of
/// `client.js`, which would pass just as well if the markup were being
/// shown as literal text, which is the bug this whole branch exists to fix.
#[test]
fn a_markdown_post_becomes_dom_and_not_text() {
    let bundle = writing_bundle();

    let headings = ask(&bundle, "tags(root, 'h1').map(textOf).join('|')");
    assert!(
        headings.contains("Hello, world"),
        "the `# Hello, world` heading must be an h1 element, got: {headings}"
    );

    // A paragraph, from a blank-line-separated block.
    let paragraphs = ask(&bundle, "tags(root, 'p').length");
    assert!(
        paragraphs.parse::<usize>().expect("a count") >= 3,
        "the posts' paragraphs must be p elements, got {paragraphs}"
    );

    // An inline code span: `` `zdc build` `` in hello-world.md.
    let code = ask(&bundle, "tags(root, 'code').map(textOf).join('|')");
    assert!(
        code.contains("zdc build"),
        "an inline code span must be a code element, got: {code}"
    );

    // A list, from on-placement.md's `-` items.
    let items = ask(&bundle, "tags(root, 'li').length");
    assert!(
        items.parse::<usize>().expect("a count") >= 4,
        "the bullet list must be li elements, got {items}"
    );

    // A link, with its destination on the element rather than in text.
    let links = ask(&bundle, "hrefs(root).join('|')");
    assert!(
        links.contains("https://example.com/x"),
        "an ordinary link must keep its destination, got: {links}"
    );
}

/// The tags must be *inside* the `Prose` element, not siblings of it: the
/// document is the element's content.
#[test]
fn the_rendered_document_is_the_prose_elements_own_content() {
    let bundle = writing_bundle();
    let inside = ask(
        &bundle,
        "tags(root, 'div').filter((d) => (d.attributes.class || '')\
         .indexOf('zd-prose') >= 0).map((d) => tags(d, 'h1').length)\
         .reduce((a, b) => a + b, 0)",
    );
    assert!(
        inside.parse::<usize>().expect("a count") >= 1,
        "every heading must be inside the `Prose` element that rendered it, got {inside}"
    );
}

// --- 2. nothing else gets in ---------------------------------------------

/// **The acceptance criterion for `<script>` in a source file.**
///
/// `examples/content/untrusted-markdown.md` is checked in and contains all
/// four constructs that were measured passing straight through
/// `pulldown-cmark`. It is read by the same `build list` that reads every
/// other post, so this is the real pipeline and not a unit of it.
///
/// The assertion is about the **DOM**: after mounting, there is no script
/// element, no element carrying an event-handler attribute, and no anchor
/// whose destination is a script. A string assertion would not distinguish
/// an escaped `&lt;script&gt;` from a live one; a parsed tree does.
#[test]
fn a_script_in_a_checked_in_post_does_not_become_an_executing_script() {
    let bundle = writing_bundle();

    let scripts = ask(&bundle, "tags(root, 'script').length");
    assert_eq!(scripts, "0", "a post created a script element");

    let images = ask(&bundle, "tags(root, 'img').length");
    assert_eq!(images, "0", "a post created an img element");

    // No element anywhere carries an `on…` attribute.
    let handlers = ask(&bundle, "handlers(root, []).join('|')");
    assert_eq!(handlers, "", "a post attached an event handler: {handlers}");

    // And the one that is not raw HTML at all.
    let destinations = ask(&bundle, "hrefs(root).join('|')");
    assert!(
        !destinations.to_lowercase().contains("javascript:"),
        "a post kept a script destination: {destinations}"
    );
    assert!(
        destinations.contains("about:blank#blocked"),
        "the refused destination must still be a link, going nowhere: {destinations}"
    );

    // The global the fixture tries to set, from four directions, is unset.
    let owned = ask(&bundle, "globalThis.__zdOwned === undefined");
    assert_eq!(owned, "true", "a post ran script");
}

/// A `Text` cannot be rendered as markup.
#[test]
fn a_text_cannot_be_rendered_as_markup() {
    let messages =
        refusals("state note is client Text starting \"<b>hello</b>\"\nview\n    Prose note\n");
    assert!(
        messages
            .iter()
            .any(|m| m.contains("`Prose` renders") && m.contains("`Markup` is expected")),
        "a `Text` reached `Prose`: {messages:?}"
    );
}

/// And a `Markup` cannot be shown as text, so the two slots are disjoint in
/// both directions and neither element is a way round the other.
#[test]
fn a_markup_cannot_be_shown_as_text() {
    let messages = refusals(
        "state body is static Markup from render with source is \"*hi*\"\n\
         function render with source\n\
         \x20   give build markdown source\n\
         view\n\
         \x20   Text body\n",
    );
    assert!(
        messages.iter().any(|m| m.contains("shows text")),
        "a `Markup` was shown as text: {messages:?}"
    );
}

/// There is no way to *build* a markup value out of text, which is what
/// makes the producer set closed rather than merely small.
#[test]
fn no_operator_produces_markup_from_text() {
    for source in [
        // Concatenation.
        "state body is static Markup from render with source is \"*hi*\"\n\
         function render with source\n\
         \x20   give build markdown source + \"<b>x</b>\"\n\
         view\n\
         \x20   Prose body\n",
        // The other way round: markup into a text position.
        "state body is static Markup from render with source is \"*hi*\"\n\
         function render with source\n\
         \x20   give \"<i>\" + build markdown source\n\
         view\n\
         \x20   Text body\n",
    ] {
        let messages = refusals(source);
        assert!(
            !messages.is_empty(),
            "text and markup were joined:\n{source}"
        );
    }
}

/// A literal cannot spell one either: `starting` takes a literal, and every
/// literal in the language is `Text`, `Whole`, `Decimal` or `Truth`.
#[test]
fn markup_has_no_literal() {
    let messages =
        refusals("state body is client Markup starting \"<b>x</b>\"\nview\n    Prose body\n");
    assert!(!messages.is_empty(), "a markup literal was accepted");
}

/// A `secret` cannot reach markup — and it is refused twice over, by two
/// rules neither of which is about markup.
///
/// `build markdown` runs in `static` context and nowhere else, so the only
/// way a secret could reach one is through `static` state. Both routes are
/// closed by the placement lattice: a `static` secret is a contradiction
/// (E0313 — build-time state is readable by whoever it lives with), and a
/// `static` derivation cannot read a `server` secret (E0301 — build-time
/// state reads build-time state). **`Markup` needed no rule of its own**,
/// which is the result worth recording: the producer's placement is what
/// keeps secrets out of it.
#[test]
fn a_secret_cannot_reach_markup() {
    let statically_secret = refusals(
        "secret state shh is static Text starting \"the key\"\n\
         state body is static Markup from render with source is shh\n\
         function render with source\n\
         \x20   give build markdown source\n\
         view\n\
         \x20   Prose body\n",
    );
    assert!(
        statically_secret
            .iter()
            .any(|m| m.contains("declared `secret`")),
        "a `static` secret was accepted: {statically_secret:?}"
    );

    let from_the_server = refusals(
        "secret state key is server Text starting \"sk\"\n\
         state body is static Markup from render with source is key\n\
         function render with source\n\
         \x20   give build markdown source\n\
         view\n\
         \x20   Prose body\n",
    );
    assert!(
        from_the_server
            .iter()
            .any(|m| m.contains("build-time state reads")),
        "a build read a server secret: {from_the_server:?}"
    );
}

/// `build markdown` is build-time only, so no markup can be produced in the
/// browser — which is what stops a handler making one out of an input.
#[test]
fn markdown_cannot_be_rendered_in_the_browser() {
    let messages = refusals(
        "state body is client Text starting \"\"\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           set body to build markdown \"hi\"\n",
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("only readable while the build is running")),
        "`build markdown` ran in the browser: {messages:?}"
    );
}

// --- 3. the ordinary path is untouched ------------------------------------

/// The existing safety property, asserted directly: every ordinary text
/// element still writes `nodeValue`, and none of them gained an HTML parse.
///
/// This is the regression that would matter most and show up least — a
/// change that routed `Text` through `markup()` would make every test above
/// still pass while turning the whole language into an injection surface.
#[test]
fn every_ordinary_text_element_still_writes_node_value() {
    let bundle = compile_example("examples/counter.zd");
    assert!(
        bundle.client_js.contains("bindText("),
        "the ordinary text path must still be `bindText`:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("markup("),
        "a program with no `Prose` must emit no markup call:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("innerHTML"),
        "generated code must never name `innerHTML`:\n{}",
        bundle.client_js
    );
}

/// The one call that parses, in the one program that should have it.
///
/// `writing.zd` renders each post inside an `each`, so the value is a
/// reactive binder and the emitted call is `bindMarkup` rather than the
/// one-shot `markup`. Both are counted, because the property is *how many
/// places in a generated bundle can parse HTML*, not which of the two
/// spellings the scheduler picked.
#[test]
fn only_prose_emits_a_call_that_parses_html() {
    let bundle = writing_bundle();
    // `markup(` does not match `bindMarkup(` — the capital is what keeps
    // the two counts disjoint.
    let parses = bundle.client_js.matches("bindMarkup(").count()
        + bundle.client_js.matches("markup(").count();
    assert_eq!(
        parses, 1,
        "one `Prose` in the view, so one call that parses:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("bindText("),
        "the same program's ordinary text is still `bindText`"
    );
    // Generated code never names the property itself; it goes through the
    // runtime's one auditable function.
    assert!(
        !bundle.client_js.contains("innerHTML"),
        "generated code named `innerHTML`:\n{}",
        bundle.client_js
    );
}

/// `hello` and `counter` are the golden emissions, and adding an element to
/// the vocabulary must not move a byte of either.
#[test]
fn the_golden_emissions_are_untouched() {
    for example in ["examples/hello.zd", "examples/counter.zd"] {
        let bundle = compile_example(example);
        assert!(
            !bundle.client_js.contains("zd-prose"),
            "{example} gained markup machinery:\n{}",
            bundle.client_js
        );
    }
}

/// A `Prose` with no argument is refused rather than rendering an empty
/// element, because an empty document is almost always a missing one.
#[test]
fn prose_needs_the_markup_it_renders() {
    let messages = refusals("view\n    Prose\n");
    assert!(
        messages.iter().any(|m| m.contains("needs the markup")),
        "{messages:?}"
    );
}

/// The build root's own answers, for a source with no file behind it.
#[test]
fn build_markdown_gives_markup_and_build_read_gives_text() {
    let statics: BTreeMap<String, String> = BTreeMap::new();
    let Err(refused) = try_compile_with_statics(
        // `build read` gives `Text`, so it cannot be rendered.
        "state body is static Text from render with path is \"content/hello-world.md\"\n\
         function render with path\n\
         \x20   give build read path\n\
         view\n\
         \x20   Prose body\n",
        "test.zd",
        statics,
    ) else {
        panic!("`build read` gives `Text`, which `Prose` must refuse");
    };
    assert!(
        refused
            .iter()
            .any(|e| e.message.contains("`Prose` renders")),
        "{refused:?}"
    );
}
