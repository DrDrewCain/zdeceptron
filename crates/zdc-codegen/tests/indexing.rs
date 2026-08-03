//! `at` is total, and it is defended at both ends.
//!
//! The defect. §14A.3 makes `Whole` an f64, so a fractional index is a
//! *representable* one, and `at` guarded only its range: `i >= 0 && i <
//! length` admits `1.5`, reads a property that is not there, and answers
//! `Some(undefined)` — an `Option of T` inhabited by a value of no type,
//! which `when` unwraps and hands to whatever follows.
//!
//! It was closed twice, independently, and both halves are kept because
//! they close it in different places.
//!
//! * **At the source.** `/` yields `Decimal` (§14B.2), so `xs at (3 / 2)`
//!   no longer typechecks at all: the index is `Decimal` where `Whole` is
//!   wanted. Integer division is written `floor of (a / b)`, which is an
//!   `Option` precisely because the narrowing can fail. This is the
//!   stronger half — it refuses the program rather than the value — and
//!   `a_fractional_index_is_refused_before_it_can_be_emitted` holds it.
//! * **At the sink.** `$listAt` and `$textAt` still test
//!   `Number.isInteger` before indexing. With the rule above there is no
//!   longer a source program that reaches it, and unreachable is not
//!   impossible — a future numeric rule, or a `foreign` handing back an
//!   f64, would reach it again with no diagnostic to say so. So the guard
//!   is asserted where it now lives, by driving the emitted helper
//!   directly.
//!
//! Run in the engine rather than read out of the text, because the wrong
//! answer is a value and not a token.

mod support;

use support::{compile_source, context, refusals, run};

/// A program that indexes both a list and a text, so the bundle carries
/// `$listAt` and `$textAt` for the drivers below to call.
const INDEXES_BOTH: &str = r#"
state xs is client List of Whole starting [10, 20]
state word is client Text starting "abc"
state pick is client Whole starting 1
state out is client Text from firstOf with xs, pick
state letter is client Text from charOf with word, pick

function firstOf with all, n
    when all at n
        Some with v
            give text of v
        None
            give "none"

function charOf with s, n
    when s at n
        Some with c
            give c
        None
            give "none"

view
    Column
        Text out
        Text letter
"#;

/// **The source half.** The index of an `at` is a position, and `/` does
/// not produce one. The program that used to emit a fractional index is
/// now refused, and the message names the type the index actually has.
#[test]
fn a_fractional_index_is_refused_before_it_can_be_emitted() {
    let mut checked = 0;
    for (what, source) in [
        (
            "a list",
            r#"
state xs is client List of Whole starting [10, 20]
state pick is client Whole starting 3
state out is client Text from firstOf with xs, pick

function firstOf with all, n
    when all at (n / 2)
        Some with v
            give text of v
        None
            give "none"

view
    Column
        Text out
"#,
        ),
        (
            "a text",
            r#"
state word is client Text starting "abc"
state pick is client Whole starting 3
state out is client Text from charOf with word, pick

function charOf with s, n
    when s at (n / 2)
        Some with c
            give c
        None
            give "none"

view
    Column
        Text out
"#,
        ),
    ] {
        let messages = refusals(source);
        assert!(
            messages
                .iter()
                .any(|m| m.contains("indexed by position") && m.contains("`Decimal`")),
            "{what} indexed by `n / 2` must be refused as a `Decimal` index, got: {messages:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 2, "a case was skipped");
}

/// **The sink half.** The emitted helpers answer `None` for an index that
/// is not a whole number, and `Some` for one that is — so the guard is a
/// statement about the *kind* of the index and not a narrowing of its
/// range.
///
/// The helpers are called directly because no source program can hand
/// them a fraction any more. That is the point: this asserts the defence
/// that is left when the type system's is taken away.
#[test]
fn the_emitted_helpers_answer_none_for_an_index_that_is_not_whole() {
    let bundle = compile_source(INDEXES_BOTH);
    // Both helpers must actually be in the bundle, or the drivers below
    // would fail for the wrong reason — and a bundle that emitted neither
    // would make every assertion here vacuous.
    assert!(
        bundle.client_js.contains("const $listAt ="),
        "the bundle does not define `$listAt`:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("const $textAt ="),
        "the bundle does not define `$textAt`:\n{}",
        bundle.client_js
    );

    let mut engine = context(false);
    let found = run(
        &mut engine,
        &bundle.client_js,
        "[$listAt([10, 20], 1.5).tag,\n\
         \x20$listAt([10, 20], 1).tag,\n\
         \x20$listAt([10, 20], 1).fields[0],\n\
         \x20$textAt('abc', 1.5).tag,\n\
         \x20$textAt('abc', 1).tag,\n\
         \x20$textAt('abc', 1).fields[0]].join(',')",
    );
    assert_eq!(
        found, "None,Some,20,None,Some,b",
        "1.5 must index nothing and 1 must index the second element"
    );
}

/// The guard is about the kind of the index, so a value that is not a
/// number at all is refused by it rather than by the range test happening
/// to be false — which is what made the original guard look sufficient.
#[test]
fn the_emitted_helpers_answer_none_for_a_nan_or_an_infinity() {
    let bundle = compile_source(INDEXES_BOTH);
    let mut engine = context(false);
    let found = run(
        &mut engine,
        &bundle.client_js,
        "[$listAt([10, 20], NaN).tag,\n\
         \x20$listAt([10, 20], Infinity).tag,\n\
         \x20$listAt([10, 20], -Infinity).tag,\n\
         \x20$textAt('abc', NaN).tag].join(',')",
    );
    assert_eq!(found, "None,None,None,None");
}
