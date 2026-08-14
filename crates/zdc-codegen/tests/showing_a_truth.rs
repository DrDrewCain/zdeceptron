//! A text node showing a `Truth` writes this language's word — issue #297.
//!
//! ```zd
//! state flag is client Truth starting yes
//!
//! view
//!     Column
//!         Text flag
//! ```
//!
//! used to render **`true`**, which is not a word in this language. Its
//! truth literals are `yes` and `no`, `zdc explain` says `yes` and `no`,
//! and the formatter writes `yes` and `no` — so a reader who typed `yes`
//! was shown JavaScript's spelling of their own literal. Nothing was wrong
//! with the value; only with the one place it becomes text for a person.
//!
//! **The words were not this fix's to choose.** §17.4.3's closed
//! dispatched set already sends `text of` a `Truth` to `textOfTruth`, and
//! §17.4.9 writes that function out as `if value / give "yes" / give
//! "no"`. `Text (text of flag)` has therefore said `yes` all along, and
//! the defect was that `Text flag` — the same value, the same text node,
//! one word shorter — said something else. So the assertions below are
//! mostly about **agreement**: what is tested is that the two spellings
//! cannot come apart again, not that `yes` is the right word.
//!
//! Run in the engine rather than read out of the emitted text wherever the
//! question is what a reader sees, because a conversion applied to the
//! wrong operand, or applied twice, would look right in the source.

mod support;

use support::{compile_source, context, page, run};

/// Drive a compiled module and hand back the serialised tree.
fn rendered(source: &str) -> String {
    let bundle = compile_source(source);
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         serialize($host)",
    )
}

/// **The issue's program, and the whole of the report.**
#[test]
fn a_text_node_showing_a_truth_says_yes() {
    let said = rendered(
        "state flag is client Truth starting yes\n\n\
         view\n    Column\n        Text flag\n",
    );
    assert!(
        said.contains(">yes<"),
        "a `Truth` reached the page in the host's word rather than this language's: {said}"
    );
    assert!(
        !said.contains(">true<"),
        "the JavaScript word survived somewhere in: {said}"
    );
}

/// **And `no` for the other one**, because a conversion that returned the
/// same word for both truths would pass the test above.
#[test]
fn a_false_truth_says_no() {
    let said = rendered(
        "state flag is client Truth starting no\n\n\
         view\n    Column\n        Text flag\n",
    );
    assert!(said.contains(">no<"), "{said}");
    assert!(!said.contains(">false<"), "{said}");
}

/// **`Text flag` and `Text (text of flag)` write the same word.**
///
/// The assertion the fix exists for. Written as one program showing the
/// same signal twice, so the two arrive at the page through both paths
/// under one set of conditions and neither can be changed alone.
#[test]
fn a_text_node_and_text_of_agree_about_a_truth() {
    let said = rendered(
        "state flag is client Truth starting yes\n\n\
         view\n    Column\n        Text flag\n        Text (text of flag)\n",
    );
    assert_eq!(
        said.matches(">yes<").count(),
        2,
        "the two ways of showing one `Truth` disagreed: {said}"
    );
}

/// **A written truth is folded into the markup and links nothing.**
///
/// `Text yes` is known at compile time, so the word belongs in the
/// template string beside every other baked literal rather than behind a
/// call made once at construction. Asserted on the emitted text because
/// *where* the word is written is the whole claim — a page whose first
/// paint carries `yes` and one that computes it a tick later serialise
/// identically.
#[test]
fn a_written_truth_is_baked_into_the_template() {
    let bundle = compile_source("view\n    Column\n        Text yes\n");
    assert!(
        bundle.client_js.contains("<span>yes</span>"),
        "a written `yes` was not baked into the template: {}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("$textOfTruth"),
        "a written truth linked a helper it does not need: {}",
        bundle.client_js
    );
}

/// **The conversion is emitted, so a program that shows no truth pays
/// nothing for it.**
///
/// This is the reason the fix is not the one line in `dom.js` the issue
/// sketched. `dom.js` ships with every program, and `zdc-bench`'s
/// null-program ceiling is the standing reason not to spend bytes there;
/// a preamble helper is paid for only by the bundle that calls it. Both
/// halves are asserted, because "only the program that needs it" is a
/// claim about the program that does *not*.
#[test]
fn the_conversion_is_in_the_preamble_and_not_in_the_runtime() {
    let shows_one = compile_source(
        "state flag is client Truth starting yes\n\n\
         view\n    Column\n        Text flag\n",
    );
    assert!(
        shows_one.client_js.contains("const $textOfTruth ="),
        "the helper was not declared in the preamble: {}",
        shows_one.client_js
    );
    assert!(
        !zdc_runtime::DOM_JS.contains("'yes'") && !zdc_runtime::DOM_JS.contains("\"yes\""),
        "the word reached the shipped runtime, which every program links"
    );

    let shows_none = compile_source(
        "state count is client Whole starting 1\n\n\
         view\n    Column\n        Text (text of count)\n",
    );
    assert!(
        !shows_none.client_js.contains("$textOfTruth"),
        "a program with no `Truth` in its view carried the helper anyway: {}",
        shows_none.client_js
    );
}

/// **The first paint says it too.**
///
/// The prerendered document is a second answer to the same question and
/// was wrong in the same way. It is produced by running the module rather
/// than by a second conversion, so this is a guard against the paths
/// diverging rather than a second implementation to check.
#[test]
fn the_prerendered_page_says_yes() {
    let bundle = compile_source(
        "state flag is client Truth starting yes\n\n\
         view\n    Column\n        Text flag\n",
    );
    let page = page(&bundle);
    assert!(page.contains(">yes<"), "{page}");
    assert!(!page.contains(">true<"), "{page}");
}

/// **An ARIA state still writes `true`, and that is not an inconsistency.**
///
/// The boundary this fix is drawn at. `aria-selected` takes one word from
/// an enumeration ARIA defines, and its two words are `true` and `false`;
/// a tab announcing `aria-selected="yes"` is announced as *selected*,
/// because every unrecognised token maps onto `true`. So the rule is not
/// "a `Truth` is spelled `yes`" but "a `Truth` shown **to a reader** is
/// spelled in this language's words" — and an ARIA token is not shown to
/// anybody, it is read by a screen reader that was promised ARIA's own
/// vocabulary.
#[test]
fn an_aria_state_keeps_the_word_aria_defines() {
    let bundle = compile_source(
        "state chosen is client Whole starting 0\n\
         view\n\
         \x20   Row role is \"tablist\"\n\
         \x20       Button \"Issues\", role is \"tab\", selected is chosen is 0\n",
    );
    let page = page(&bundle);
    assert!(
        page.contains(r#"aria-selected="true""#),
        "an ARIA state was given a word ARIA does not define: {page}"
    );
}
