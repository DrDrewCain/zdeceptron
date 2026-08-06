//! What the elements added to the vocabulary render, asserted against the
//! **parsed DOM** rather than against the emitted string.
//!
//! `element_parity.rs` already compares each built-in's template against
//! the tree `elements.js` builds, which pins the tag, the attributes and
//! the base class. That is a shape check and it is deliberately blind to
//! everything a program does with the element afterwards. This file is the
//! other half: a view is compiled, mounted in the engine, driven, and the
//! resulting tree is read back.

mod support;

use support::{compile_source, context, run};

/// Mount one view and serialise the tree it produced.
fn rendered(source: &str) -> String {
    let bundle = compile_source(source);
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    )
}

/// Fine print is its own element, not a styled span (#58).
#[test]
fn fine_print_renders_as_a_small_element() {
    let tree = rendered("view\n    Small \"terms apply\"\n");
    assert!(
        tree.contains("<small>terms apply</small>"),
        "fine print must carry its own semantics:\n{tree}"
    );
    assert!(
        !tree.contains("<span>terms apply</span>"),
        "a `Small` must not be emitted as a styled span:\n{tree}"
    );
}

/// A matched run of text is a `mark`, which is what a search result
/// highlights (#59). The term comes from a signal, because the whole
/// point of a mark is that what matched is not known when the page is
/// written.
#[test]
fn a_match_renders_as_a_mark_that_tracks_its_signal() {
    let tree = rendered(
        "state term is client Text starting \"parser\"\n\
         view\n\
         \x20   Paragraph \"write the\"\n\
         \x20       Mark term\n",
    );
    assert!(
        tree.contains("<mark>parser</mark>"),
        "a highlighted match must be a mark:\n{tree}"
    );
}

/// An abbreviation carries its expansion, and it carries it where both a
/// pointer and assistive technology look for it (#60).
#[test]
fn an_abbreviation_carries_its_expansion() {
    let tree = rendered(
        "view\n    Abbreviation \"HTML\", expansion is \"HyperText Markup Language\"\n",
    );
    assert!(
        tree.contains("<abbr title=\"HyperText Markup Language\">HTML</abbr>"),
        "the expansion must reach `title`:\n{tree}"
    );
}

/// The expansion is the whole reason the element exists, so an
/// abbreviation without one is refused rather than rendered as an
/// unexplained acronym. This follows `Image`'s `alt`.
#[test]
fn an_abbreviation_without_an_expansion_is_refused() {
    let refusals = support::refusals("view\n    Abbreviation \"HTML\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Abbreviation` needs `expansion is")),
        "an abbreviation with nothing to expand to must be refused: {refusals:?}"
    );
}
