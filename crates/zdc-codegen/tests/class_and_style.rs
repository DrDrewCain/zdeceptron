//! `class` and a folded style set on one element, in every combination.
//!
//! A style set that never reads a signal folds into one generated class
//! (§6, §16.3.11), and §16.2 R6 is why `Column` and `Row` carry `zd-col`
//! and `zd-row` rather than inline styles at all — so the `class`
//! attribute an element ends up with is the whole of its styling, not
//! decoration on top of it.
//!
//! A non-literal `class` is emitted as one assignment over the *whole*
//! attribute, `base + value`, where `base` is the element's classes joined
//! at the moment the argument was read. The generated style class is
//! pushed **after** the argument loop, so it was not in that base — and
//! the assignment then overwrote the markup's `class`, silently dropping
//! the styles. Nothing errors; the element renders unstyled.
//!
//! The assertions are on the mounted DOM's class list rather than on
//! emitted text, so they survive a change to how the fix spells its
//! output — and a defect that throws fails here with the engine's own
//! message.

mod support;

use std::collections::BTreeMap;

use support::{build_module_of, context, run, try_compile, try_compile_with_statics};

/// Compile a program with no `static` state.
fn compile(source: &str) -> zdc_codegen::Bundle {
    try_compile(source, "test.zd").unwrap_or_else(|errors| panic!("test.zd: {}", errors[0].message))
}

/// Compile a program with `static` state, running its build root first —
/// the same two steps `zdc build` takes.
fn compile_static(source: &str) -> zdc_codegen::Bundle {
    let module = build_module_of(source, "test.zd")
        .expect("this program declares `static` state, so it has a build root");
    let statics: BTreeMap<String, String> =
        zdc_codegen::evaluate(&module, std::path::Path::new("."))
            .unwrap_or_else(|error| panic!("the build root did not run: {}", error.report()))
            .values;
    try_compile_with_statics(source, "test.zd", statics)
        .unwrap_or_else(|errors| panic!("test.zd: {}", errors[0].message))
}

/// The class list of the first `tag` in the mounted tree, sorted.
///
/// Sorted because the order classes appear in is not a guarantee anything
/// here is about — a class list is a set, and asserting on its order would
/// make the test fail for a reordering that changes no rendering.
fn classes(bundle: &zdc_codegen::Bundle, tag: &str) -> Vec<String> {
    let mut context = context(false);
    let text = run(
        &mut context,
        &bundle.client_js,
        &format!(
            "const $host = document.createElement('div');\n\
             main($host);\n\
             String(walk($host).filter((n) => n !== $host)\
             .find((n) => n.tagName === '{tag}')\
             .attributes['class'] ?? '')"
        ),
    );
    let mut out: Vec<String> = text
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    out.sort();
    out
}

/// Every rule the stylesheet carries, so a generated class can be checked
/// to actually declare the padding the program asked for. A class name on
/// an element that no rule matches is not styling.
fn stylesheet(bundle: &zdc_codegen::Bundle) -> String {
    bundle.styles_css.clone()
}

// ---------------------------------------------------------------------
// A literal `class`, with and without a folded style.
// ---------------------------------------------------------------------

#[test]
fn a_literal_class_reaches_the_element() {
    let bundle = compile(
        "view\n\
         \x20   Column class is \"accent\"\n\
         \x20       Text \"hello\"\n",
    );
    assert_eq!(classes(&bundle, "div"), ["accent", "zd-col"]);
}

#[test]
fn a_literal_class_keeps_the_folded_style_class() {
    let bundle = compile(
        "view\n\
         \x20   Column class is \"accent\", padding is 8\n\
         \x20       Text \"hello\"\n",
    );
    let classes = classes(&bundle, "div");
    assert!(
        classes.contains(&"accent".to_string()),
        "the class the program asked for must survive: {classes:?}"
    );
    let generated = classes
        .iter()
        .find(|class| class.starts_with("zd-s"))
        .unwrap_or_else(|| panic!("the folded style class must reach the element: {classes:?}"));
    assert!(
        stylesheet(&bundle).contains(&format!(".{generated} {{ padding: 8px; }}")),
        "the generated class must declare the padding:\n{}",
        stylesheet(&bundle)
    );
}

// ---------------------------------------------------------------------
// A non-literal `class`, with and without a folded style. Both the
// `static` operand — inlined as a value, assigned once — and the reactive
// one, which goes through an effect.
// ---------------------------------------------------------------------

#[test]
fn a_reactive_class_reaches_the_element() {
    let bundle = compile(
        "state tone is client Text starting \"accent\"\n\
         view\n\
         \x20   Column class is tone\n\
         \x20       Text \"hello\"\n",
    );
    assert_eq!(classes(&bundle, "div"), ["accent", "zd-col"]);
}

#[test]
fn a_reactive_class_keeps_the_folded_style_class() {
    let bundle = compile(
        "state tone is client Text starting \"accent\"\n\
         view\n\
         \x20   Column class is tone, padding is 8\n\
         \x20       Text \"hello\"\n",
    );
    let classes = classes(&bundle, "div");
    assert!(
        classes.contains(&"accent".to_string()),
        "the class the program asked for must survive: {classes:?}"
    );
    let generated = classes
        .iter()
        .find(|class| class.starts_with("zd-s"))
        .unwrap_or_else(|| {
            panic!(
                "the folded style class must survive the `class` assignment, or the element \
                 renders unstyled: {classes:?}"
            )
        });
    assert!(
        stylesheet(&bundle).contains(&format!(".{generated} {{ padding: 8px; }}")),
        "the generated class must declare the padding:\n{}",
        stylesheet(&bundle)
    );
}

#[test]
fn a_static_class_reaches_the_element() {
    let bundle = compile_static(
        "state tone is static Text starting \"accent\"\n\
         view\n\
         \x20   Column class is tone\n\
         \x20       Text \"hello\"\n",
    );
    assert_eq!(classes(&bundle, "div"), ["accent", "zd-col"]);
}

#[test]
fn a_static_class_keeps_the_folded_style_class() {
    let bundle = compile_static(
        "state tone is static Text starting \"accent\"\n\
         view\n\
         \x20   Column class is tone, padding is 8\n\
         \x20       Text \"hello\"\n",
    );
    let classes = classes(&bundle, "div");
    assert!(
        classes.contains(&"accent".to_string()),
        "the class the program asked for must survive: {classes:?}"
    );
    let generated = classes
        .iter()
        .find(|class| class.starts_with("zd-s"))
        .unwrap_or_else(|| {
            panic!(
                "the folded style class must survive the `class` assignment, or the element \
                 renders unstyled: {classes:?}"
            )
        });
    assert!(
        stylesheet(&bundle).contains(&format!(".{generated} {{ padding: 8px; }}")),
        "the generated class must declare the padding:\n{}",
        stylesheet(&bundle)
    );
}

// ---------------------------------------------------------------------
// The same element one region deeper, reached through a component
// argument. A component body is lowered where it is instantiated, so its
// `class` argument is whatever the call site passed — which is where the
// `static` defect hid a second instance of itself.
// ---------------------------------------------------------------------

#[test]
fn a_class_through_a_component_argument_keeps_the_folded_style_class() {
    let bundle = compile(
        "component Badge with tone\n\
         \x20   Column class is tone, padding is 8\n\
         \x20       Text \"hello\"\n\
         \n\
         state chosen is client Text starting \"accent\"\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       Badge chosen\n",
    );
    let classes = classes(&bundle, "div");
    // The outer `Column` is first in document order and carries only its
    // base class, so the assertion is on the component's element.
    let inner = {
        let mut context = context(false);
        run(
            &mut context,
            &bundle.client_js,
            "const $host = document.createElement('div');\n\
             main($host);\n\
             String(walk($host).filter((n) => n !== $host)\
             .filter((n) => n.tagName === 'div')[1]\
             .attributes['class'] ?? '')",
        )
    };
    let mut inner: Vec<String> = inner.split_whitespace().map(str::to_string).collect();
    inner.sort();
    assert!(
        inner.contains(&"accent".to_string()),
        "the component's `class` argument must reach its element: {inner:?} (outer {classes:?})"
    );
    let generated = inner
        .iter()
        .find(|class| class.starts_with("zd-s"))
        .unwrap_or_else(|| {
            panic!(
                "a component's element loses its folded style class the same way an inline one \
                 does: {inner:?}"
            )
        });
    assert!(
        stylesheet(&bundle).contains(&format!(".{generated} {{ padding: 8px; }}")),
        "the generated class must declare the padding:\n{}",
        stylesheet(&bundle)
    );
}

#[test]
fn a_static_class_through_a_component_argument_keeps_the_folded_style_class() {
    let bundle = compile_static(
        "component Badge with tone\n\
         \x20   Column class is tone, padding is 8\n\
         \x20       Text \"hello\"\n\
         \n\
         state chosen is static Text starting \"accent\"\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       Badge chosen\n",
    );
    let inner = {
        let mut context = context(false);
        run(
            &mut context,
            &bundle.client_js,
            "const $host = document.createElement('div');\n\
             main($host);\n\
             String(walk($host).filter((n) => n !== $host)\
             .filter((n) => n.tagName === 'div')[1]\
             .attributes['class'] ?? '')",
        )
    };
    let mut inner: Vec<String> = inner.split_whitespace().map(str::to_string).collect();
    inner.sort();
    assert!(
        inner.contains(&"accent".to_string()),
        "the component's `class` argument must reach its element: {inner:?}"
    );
    let generated = inner
        .iter()
        .find(|class| class.starts_with("zd-s"))
        .unwrap_or_else(|| {
            panic!("a component's element must keep its folded style class: {inner:?}")
        });
    assert!(
        stylesheet(&bundle).contains(&format!(".{generated} {{ padding: 8px; }}")),
        "the generated class must declare the padding:\n{}",
        stylesheet(&bundle)
    );
}
