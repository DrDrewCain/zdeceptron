//! The anti-drift test, per spec §16.3.6.
//!
//! The compiler owns the DOM shape of every built-in, which duplicates
//! `elements.js` — §16.10 names that as a known cost, and this is the whole
//! mechanism that keeps it from becoming a bug. For each built-in, with
//! constant arguments, the tree `elements.js` builds must `isEqualNode` the
//! tree the compiler's markup parses into.
//!
//! `elements.js` was verified in a browser and is no longer what ships.
//! That verification is inherited only through this test.

mod support;

use support::{compile_source, context};

use boa_engine::{Context, Source};

/// One case: the ZDeceptron view, and the `elements.js` call that must
/// produce the identical tree.
struct Case {
    element: &'static str,
    view: &'static str,
    reference: &'static str,
    /// What the build host computed, for a view that reads `static` state.
    ///
    /// `Prose` is the only case that needs one: its argument's type is
    /// `Markup`, the only producer of a `Markup` is `build markdown`, and
    /// `build markdown` runs on the build host. There is no way to write a
    /// `Prose` whose value does not come from a build (§17.4.8) — which is
    /// the property under test elsewhere, and here is just plumbing.
    statics: &'static [(&'static str, &'static str)],
}

const NO_STATICS: &[(&str, &str)] = &[];

const CASES: &[Case] = &[
    Case {
        element: "Column",
        view: "view\n    Column\n        Text \"a\"\n",
        reference: "Column(undefined, {}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Row",
        view: "view\n    Row\n        Text \"a\"\n",
        reference: "Row(undefined, {}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    // The leading text slot §4.4 ratified: one text node, then the
    // children. This is the case the two tables most recently disagreed
    // about, so it is the one worth pinning on both sides.
    Case {
        element: "Row with a leading text",
        view: "view\n    Row \"who\"\n        Text \"a\"\n",
        reference: "Row(() => 'who', {}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Column with a leading text",
        view: "view\n    Column \"who\"\n        Text \"a\"\n",
        reference: "Column(() => 'who', {}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Text",
        view: "view\n    Text \"hello\"\n",
        reference: "Text(() => 'hello')",
        statics: NO_STATICS,
    },
    // A heading at the top of a document is `h1`, and the level is its
    // nesting depth rather than anything the program writes.
    Case {
        element: "Heading",
        view: "view\n    Heading \"Title\"\n",
        reference: "Heading(() => 'Title')",
        statics: NO_STATICS,
    },
    Case {
        element: "Main",
        view: "view\n    Main\n        Text \"a\"\n",
        reference: "Main({}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Section",
        view: "view\n    Section\n        Text \"a\"\n",
        reference: "Section({}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Article",
        view: "view\n    Article\n        Text \"a\"\n",
        reference: "Article({}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Aside",
        view: "view\n    Aside\n        Text \"a\"\n",
        reference: "Aside({}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Navigation",
        view: "view\n    Navigation\n        Text \"a\"\n",
        reference: "Navigation({}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Header",
        view: "view\n    Header\n        Text \"a\"\n",
        reference: "Header({}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Footer",
        view: "view\n    Footer\n        Text \"a\"\n",
        reference: "Footer({}, [Text(() => 'a')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Address",
        view: "view\n    Address\n        Text \"ada\"\n",
        reference: "Address({}, [Text(() => 'ada')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Divider",
        view: "view\n    Divider\n",
        reference: "Divider()",
        statics: NO_STATICS,
    },
    Case {
        element: "Paragraph",
        view: "view\n    Paragraph \"a sentence\"\n",
        reference: "Paragraph(() => 'a sentence')",
        statics: NO_STATICS,
    },
    Case {
        element: "Emphasis",
        view: "view\n    Emphasis \"lightly\"\n",
        reference: "Emphasis(() => 'lightly')",
        statics: NO_STATICS,
    },
    Case {
        element: "Strong",
        view: "view\n    Strong \"firmly\"\n",
        reference: "Strong(() => 'firmly')",
        statics: NO_STATICS,
    },
    Case {
        element: "CodeBlock",
        view: "view\n    CodeBlock\n        Code \"zdc build\"\n",
        reference: "CodeBlock(undefined, {}, [Code(() => 'zdc build')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Code",
        view: "view\n    Code \"zdc\"\n",
        reference: "Code(() => 'zdc')",
        statics: NO_STATICS,
    },
    Case {
        element: "Quote",
        view: "view\n    Quote\n        Paragraph \"said so\"\n",
        reference: "Quote({}, [Paragraph(() => 'said so')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Key",
        view: "view\n    Key \"Esc\"\n",
        reference: "Key(() => 'Esc')",
        statics: NO_STATICS,
    },
    Case {
        element: "Time",
        view: "view\n    Time \"3 August 2026\", exact is \"2026-08-03\"\n",
        reference: "Time(() => '3 August 2026', { exact: '2026-08-03' })",
        statics: NO_STATICS,
    },
    Case {
        element: "Small",
        view: "view\n    Small \"terms apply\"\n",
        reference: "Small(() => 'terms apply')",
        statics: NO_STATICS,
    },
    Case {
        element: "Mark",
        view: "view\n    Mark \"parser\"\n",
        reference: "Mark(() => 'parser')",
        statics: NO_STATICS,
    },
    Case {
        element: "Abbreviation",
        view: "view\n    Abbreviation \"HTML\", expansion is \"HyperText Markup Language\"\n",
        reference: "Abbreviation(() => 'HTML', { expansion: 'HyperText Markup Language' })",
        statics: NO_STATICS,
    },
    Case {
        element: "Superscript",
        view: "view\n    Superscript \"st\"\n",
        reference: "Superscript(() => 'st')",
        statics: NO_STATICS,
    },
    Case {
        element: "Subscript",
        view: "view\n    Subscript \"2\"\n",
        reference: "Subscript(() => '2')",
        statics: NO_STATICS,
    },
    Case {
        element: "List",
        view: "view\n    List\n        Item \"one\"\n",
        reference: "List({}, [Item(() => 'one')])",
        statics: NO_STATICS,
    },
    Case {
        element: "NumberedList",
        view: "view\n    NumberedList\n        Item \"one\"\n",
        reference: "NumberedList({}, [Item(() => 'one')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Item",
        view: "view\n    List\n        Item \"one\"\n",
        reference: "List({}, [Item(() => 'one')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Terms",
        view: "view\n    Terms\n        Term \"zdc\"\n        Description \"the compiler\"\n",
        reference: "Terms({}, [Term(() => 'zdc'), Description(() => 'the compiler')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Term",
        view: "view\n    Terms\n        Term \"zdc\"\n",
        reference: "Terms({}, [Term(() => 'zdc')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Description",
        view: "view\n    Terms\n        Description \"the compiler\"\n",
        reference: "Terms({}, [Description(() => 'the compiler')])",
        statics: NO_STATICS,
    },
    // The one element whose content is *parsed* rather than templated.
    //
    // This test compares the compiler's template markup against the tree
    // `elements.js` builds, and a `Prose`'s document is never in the
    // template: it arrives through `markup()` at construction, because a
    // rendered file is not a literal of the program (§16.3.5). So the two
    // sides are compared empty, which is exactly what this test is for —
    // the tag, the attributes and the base class, i.e. the shape table.
    // That the document then lands *inside* this element is asserted end
    // to end, against a mounted DOM, in `tests/markup.rs`.
    Case {
        element: "Prose",
        view: "state body is static Markup from render with source is \"*hi*\"\n\
               function render with source\n\
               \x20   give build markdown source\n\
               view\n\
               \x20   Prose body\n",
        reference: "Prose('')",
        statics: &[("body", "\"<p><em>hi</em></p>\\n\"")],
    },
    Case {
        element: "Link",
        view: "view\n    Link \"https://example.com\"\n        Text \"there\"\n",
        reference: "Link(() => 'https://example.com', {}, [Text(() => 'there')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Image",
        view: "view\n    Image source is \"/a.png\", alt is \"a thing\"\n",
        reference: "Image({ source: '/a.png', alt: 'a thing' })",
        statics: NO_STATICS,
    },
    Case {
        element: "Figure",
        view: "view\n    Figure\n        Caption \"below\"\n",
        reference: "Figure({}, [Caption(() => 'below')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Caption",
        view: "view\n    Figure\n        Caption \"below\"\n",
        reference: "Figure({}, [Caption(() => 'below')])",
        statics: NO_STATICS,
    },
    Case {
        element: "Canvas",
        view: "view\n    Canvas width is 300, height is 150\n",
        reference: "Canvas({ width: 300, height: 150 })",
        statics: NO_STATICS,
    },
    Case {
        element: "Button",
        view: "view\n    Button \"press\"\n",
        reference: "Button(() => 'press')",
        statics: NO_STATICS,
    },
    Case {
        element: "Input",
        view: "state name is client Text starting \"world\"\nview\n    Input name, hint is \"your name\"\n",
        reference: "Input(signal('world'), { hint: 'your name' })",
        statics: NO_STATICS,
    },
    Case {
        element: "Checkbox",
        view: "state done is client Truth starting no\nview\n    Checkbox done\n",
        reference: "Checkbox(signal(false))",
        statics: NO_STATICS,
    },
    Case {
        element: "Checkbox with a label",
        view: "state done is client Truth starting no\nview\n    Checkbox done, label is \"ready\"\n",
        reference: "Checkbox(signal(false), { label: 'ready' })",
        statics: NO_STATICS,
    },
    Case {
        element: "Spinner",
        view: "view\n    Spinner\n",
        reference: "Spinner()",
        statics: NO_STATICS,
    },
    Case {
        element: "ErrorBar",
        view: "view\n    ErrorBar message is \"boom\"\n",
        reference: "ErrorBar({ message: 'boom' })",
        statics: NO_STATICS,
    },
    // Routing's element. Its `href` is not written by the program: the
    // compiler renders it from the route value, which is what makes a
    // mistyped URL a name that does not resolve.
    Case {
        element: "Link",
        view: "route Site\n    Home is \"/\"\nview\n    Link Home\n        Text \"home\"\n",
        reference: "Link('/', {}, [Text(() => 'home')])",
        statics: NO_STATICS,
    },
];

/// The single `template('...')` literal out of an emitted module.
fn template_markup(client_js: &str) -> String {
    let start = client_js
        .find("template('")
        .unwrap_or_else(|| panic!("no template in:\n{client_js}"))
        + "template('".len();
    let rest = &client_js[start..];
    let end = rest
        .find("')")
        .unwrap_or_else(|| panic!("unterminated template in:\n{client_js}"));
    rest[..end].to_string()
}

fn assert_parity(context: &mut Context, case: &Case, markup: &str) {
    let script = format!(
        r#"
        (() => {{
          const built = {};
          const cloned = template({})();
          // `elements.js` returns one node; the compiler returns a fragment.
          const compiled = cloned.childNodes.length === 1 ? cloned.firstChild : cloned;
          if (built.isEqualNode(compiled)) return 'equal';
          return 'elements.js: ' + serialize(built) + '\ncompiler  : ' + serialize(compiled);
        }})()
        "#,
        case.reference,
        // The markup is already a JavaScript string literal's contents.
        format_args!("'{markup}'")
    );

    let verdict = context
        .eval(Source::from_bytes(script.as_bytes()))
        .unwrap_or_else(|e| panic!("{}: the parity script failed: {e}", case.element))
        .to_string(context)
        .expect("a string")
        .to_std_string_escaped();

    assert_eq!(
        verdict, "equal",
        "`{}` has drifted between the compiler's shape table and elements.js:\n{verdict}",
        case.element
    );
}

#[test]
fn every_built_in_renders_the_same_tree_through_both_strategies() {
    // One context holds both strategies: `elements.js` for the reference
    // tree, and `dom.js`'s `template` for the compiled one.
    let mut context = context(true);
    for case in CASES {
        let bundle = if case.statics.is_empty() {
            compile_source(case.view)
        } else {
            let statics = case
                .statics
                .iter()
                .map(|(name, json)| ((*name).to_string(), (*json).to_string()))
                .collect();
            support::try_compile_with_statics(case.view, "test.zd", statics)
                .unwrap_or_else(|errors| panic!("{}: {}", case.element, errors[0].message))
        };
        let markup = template_markup(&bundle.client_js);
        assert_parity(&mut context, case, &markup);
    }
}

/// A test that stopped running its cases would report no drift at all.
#[test]
fn the_parity_suite_covers_every_built_in() {
    for built_in in zdc_codegen::BUILT_INS {
        assert!(
            CASES.iter().any(|case| case.element == *built_in
                || case
                    .element
                    .strip_prefix(built_in)
                    .is_some_and(|rest| rest.starts_with(" with"))),
            "`{built_in}` has no parity case"
        );
    }
    assert_eq!(
        CASES.len(),
        zdc_codegen::BUILT_INS.len() + 4,
        "one case per built-in, plus `Checkbox with a label`, the leading text slot §4.4 gave \
         `Row` and `Column`, and the second kind of value `Link`'s destination slot takes — a \
         route value, whose URL the compiler renders (§14G.2 revision 1), beside a URL written \
         out. Both are one slot and one `href`, and this suite checks the tree each produces."
    );
}

/// The tags the vocabulary can produce, so a regression that quietly
/// dropped one is a failing test rather than a page that renders a `div`.
///
/// The 2026-08-02 portfolio gap analysis measured **five** distinct tags
/// against the thirty-four its target uses. Seven of the thirty-four are
/// still out of reach and each is out of reach for a stated reason:
/// `svg`, `path`, `g`, `circle` and `line` are foreign content with their
/// own namespace, their own case-sensitive attribute vocabulary, and a
/// parser mode `template()` would have to be trusted with; `form` waits on
/// the submit lifecycle §14G.6c sent back, and emitting one without it
/// gives a page that navigates away on Enter; and `script` is refused
/// permanently, because it is the sink the whole escaping design exists to
/// keep closed.
#[test]
fn the_vocabulary_reaches_the_tags_it_claims_to() {
    let mut tags: Vec<&str> = zdc_codegen::BUILT_INS
        .iter()
        .map(|name| zdc_codegen::tag_of(name).expect("a built-in has a tag"))
        .collect();
    // A heading is every level, and `Checkbox` with a label adds `label`.
    tags.extend(zdc_codegen::HEADING_TAGS);
    tags.push("label");
    tags.sort_unstable();
    tags.dedup();

    for expected in [
        "a",
        "abbr",
        "address",
        "article",
        "aside",
        "blockquote",
        "button",
        "canvas",
        "code",
        "dd",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "footer",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hr",
        "img",
        "input",
        "kbd",
        "label",
        "li",
        "main",
        "mark",
        "nav",
        "ol",
        "p",
        "pre",
        "section",
        "small",
        "span",
        "strong",
        "sub",
        "sup",
        "time",
        "ul",
    ] {
        assert!(tags.contains(&expected), "`{expected}` is not reachable");
    }
    assert_eq!(tags.len(), 44, "the reachable tags: {tags:?}");

    for refused in ["script", "svg", "path", "form", "table", "iframe", "style"] {
        assert!(
            !tags.contains(&refused),
            "`{refused}` must not be reachable"
        );
    }
}

/// The whole point of widening the vocabulary: a heading's level is its
/// nesting depth, so an outline can neither start below `h1` nor skip a
/// level, and nothing in the program names a level to get wrong.
///
/// This has no `elements.js` counterpart — the reference implementation has
/// no enclosing context to consult — so it is asserted against the markup
/// directly rather than through the parity harness.
#[test]
fn a_heading_takes_its_level_from_its_nesting() {
    let flat = compile_source("view\n    Heading \"one\"\n");
    assert!(
        template_markup(&flat.client_js).contains("<h1>one</h1>"),
        "a heading at the top of the document is `h1`"
    );

    let nested = compile_source(
        "view\n    Section\n        Heading \"two\"\n        Section\n            Heading \
         \"three\"\n",
    );
    let markup = template_markup(&nested.client_js);
    assert!(
        markup.contains("<h2>two</h2>"),
        "one section deep is `h2`:\n{markup}"
    );
    assert!(
        markup.contains("<h3>three</h3>"),
        "two sections deep is `h3`:\n{markup}"
    );
}

/// A `when` arm and an `each` body are separate regions, so the depth has
/// to be carried across a region boundary or a heading inside one restarts
/// at `h1` — which is exactly the outline break the design exists to stop.
#[test]
fn a_heading_keeps_its_level_across_a_region_boundary() {
    let bundle = compile_source(
        "state open is client Truth starting yes\nview\n    Section\n        if open\n            \
         Heading \"inside\"\n",
    );
    assert!(
        bundle.client_js.contains("<h2>inside</h2>"),
        "a heading inside a conditional inside a section is still `h2`:\n{}",
        bundle.client_js
    );
}
