//! The styling vocabulary: one value grammar per style argument.
//!
//! Every assertion here goes through the *mounted* DOM to find the class an
//! element actually carries, and then through `styles.css` to read what
//! that class declares. A test that read the emitted text instead would
//! pass for a class name printed into markup that no rule matches, which is
//! not styling.
//!
//! The organising rule of the whole vocabulary, and the reason each
//! argument is a separate decision rather than a row in one table: a style
//! value is **printed** into a folded stylesheet, so a value that can close
//! its own declaration writes CSS for the whole page. There is no CSS
//! escape that keeps a value meaning what it said, so each argument names
//! what it admits: a colour, a length, one word from a closed set. Every
//! other value is refused. `injection.rs` holds the matching refusals.

mod support;

use support::{compile_source, context, refusals, run};

/// The class list of the first `tag` in the mounted tree.
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
    text.split_whitespace().map(str::to_string).collect()
}

/// The generated class the first `tag` carries, or a panic naming what it
/// carried instead.
fn generated_class(bundle: &zdc_codegen::Bundle, tag: &str) -> String {
    let classes = classes(bundle, tag);
    classes
        .iter()
        .find(|class| class.starts_with("zd-s"))
        .unwrap_or_else(|| panic!("`{tag}` carries no generated class: {classes:?}"))
        .clone()
}

/// Every rule in `styles.css` whose selector begins with the element's
/// generated class, joined, so an assertion can name a declaration
/// without depending on how the sheet lays its rules out.
fn rules(bundle: &zdc_codegen::Bundle, tag: &str) -> String {
    let class = generated_class(bundle, tag);
    let needle = format!(".{class}");
    bundle
        .styles_css
        .lines()
        .filter(|line| line.contains(&needle))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Compile `source`, and return what the first `tag` in it is styled with.
fn styled(source: &str, tag: &str) -> String {
    let bundle = compile_source(source);
    rules(&bundle, tag)
}

/// A view holding one `Text` with the given arguments.
fn text_with(arguments: &str) -> String {
    format!("view\n    Column\n        Text \"x\", {arguments}\n")
}

fn assert_refused(source: &str, needle: &str) {
    let messages = refusals(source);
    assert!(
        messages.iter().any(|message| message.contains(needle)),
        "expected a diagnostic mentioning `{needle}`, got:\n{}",
        messages.join("\n")
    );
}

// ---------------------------------------------------------------------
// #64 Colour.
//
// A colour is a hex triple or one of the plain colour words. It is
// deliberately not "any CSS colour": `rgb(…)` and `color-mix(…)` are
// function calls, and a function call in a printed declaration is a
// parenthesis a value can walk out of.
// ---------------------------------------------------------------------

#[test]
fn a_named_colour_folds_into_the_generated_class() {
    let rules = styled(&text_with("color is \"red\""), "span");
    assert!(rules.contains("color: red;"), "{rules}");
}

#[test]
fn a_hex_colour_folds_into_the_generated_class() {
    let rules = styled(&text_with("color is \"#b3151c\""), "span");
    assert!(rules.contains("color: #b3151c;"), "{rules}");
}

#[test]
fn a_short_hex_colour_folds_into_the_generated_class() {
    let rules = styled(&text_with("color is \"#abc\""), "span");
    assert!(rules.contains("color: #abc;"), "{rules}");
}

#[test]
fn a_colour_that_is_not_a_colour_is_refused() {
    assert_refused(&text_with("color is \"reddish\""), "is a colour");
}

#[test]
fn a_colour_may_not_be_a_function_call() {
    assert_refused(&text_with("color is \"rgb(1,2,3)\""), "is a colour");
}

#[test]
fn a_hex_colour_of_the_wrong_length_is_refused() {
    assert_refused(&text_with("color is \"#ab\""), "is a colour");
}
