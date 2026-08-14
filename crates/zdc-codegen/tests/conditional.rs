//! `value if condition otherwise other` — the conditional expression.
//!
//! # What it replaces
//!
//! `if` is a statement, so a conditional *value* had nowhere to live: it
//! meant declaring a function whose whole body was the choice. The
//! portfolio accumulated a dozen of those — `oneUnless`, `detailAfter`,
//! `kindLabel`, `shadeFactor` — each taking a name in a flat module
//! namespace, each putting the question a screen away from where it is
//! asked, and none of them telling a reader anything.

mod support;

use support::{compile_source, refusals};

/// The shape, and the precedence that makes it read the way it looks.
#[test]
fn a_conditional_emits_a_javascript_conditional() {
    let bundle = compile_source(
        "state lit is client Truth starting yes\n\
         state shade is client Decimal from 0.74 if lit otherwise 1.0\n\
         view\n\
         \x20   Text (text of shade)\n",
    );
    assert!(
        bundle.client_js.contains("lit() ? 0.74 : 1"),
        "expected a ternary:\n{}",
        bundle.client_js
    );
}

/// **It binds looser than every operator**, so `a + b if p otherwise c`
/// is `(a + b) if p otherwise c` — the only reading anyone offers when
/// asked, and the reason the form is parsed outside the precedence
/// climber rather than inside it.
#[test]
fn a_conditional_binds_looser_than_arithmetic() {
    let bundle = compile_source(
        "state p is client Truth starting yes\n\
         state n is client Whole starting 2\n\
         state out is client Whole from n + 1 if p otherwise 0\n\
         view\n\
         \x20   Text (text of out)\n",
    );
    assert!(
        bundle.client_js.contains("p() ? n() + 1 : 0"),
        "the sum must be the taken arm, not the condition:\n{}",
        bundle.client_js
    );
}

/// Right-associative, so a chain needs no brackets to mean what it reads
/// as: each `otherwise` opens the next question.
#[test]
fn conditionals_chain_to_the_right() {
    let bundle = compile_source(
        "state n is client Whole starting 3\n\
         state band is client Text from \"low\" if n < 2 otherwise \"mid\" if n < 9 otherwise \"high\"\n\
         view\n\
         \x20   Text band\n",
    );
    assert!(
        bundle
            .client_js
            .contains("n() < 2 ? 'low' : n() < 9 ? 'mid' : 'high'"),
        "expected a right-nested chain:\n{}",
        bundle.client_js
    );
}

/// The condition is a `Truth` and nothing else. A `Whole` here is the
/// truthiness bug every language with an implicit conversion has, and
/// this one does not have it anywhere else either.
#[test]
fn a_condition_that_is_not_a_truth_is_refused() {
    let found = refusals(
        "state n is client Whole starting 1\n\
         state bad is client Text from \"yes\" if n otherwise \"no\"\n\
         view\n\
         \x20   Text bad\n",
    );
    assert!(
        found.iter().any(|message| message.contains("the condition is")),
        "expected the condition to be checked: {found:?}"
    );
}

/// Both arms give the answer, so both are the declared type. There is no
/// join and deliberately none: two arms of different types would need a
/// type that is either, and the language has no such type.
#[test]
fn arms_of_different_types_are_refused() {
    let found = refusals(
        "state p is client Truth starting yes\n\
         state bad is client Text from \"yes\" if p otherwise 3\n\
         view\n\
         \x20   Text bad\n",
    );
    assert!(
        found
            .iter()
            .any(|message| message.contains("the `otherwise` value is")),
        "expected the arms to be checked against each other: {found:?}"
    );
}

/// A conditional with no `otherwise` is refused where it is written: an
/// expression that is sometimes nothing has no type.
#[test]
fn a_conditional_without_an_otherwise_is_refused() {
    // Asked of the parser, because this is a syntax rule: `refusals`
    // collects what the checker and the emitter say, and a program that
    // does not parse never reaches either.
    let found = zdc_parser::parse(
        "state p is client Truth starting yes\n\
         state bad is client Text from \"yes\" if p\n\
         view\n\
         \x20   Text bad\n",
    );
    let error = found.expect_err("a missing `otherwise` must be refused");
    assert!(
        error.message.contains("otherwise"),
        "the refusal must name the missing word: {}",
        error.message
    );
}

/// The statement `if` is untouched. Both spellings exist because they
/// answer different questions — a statement may `give` from either arm
/// or from neither, and an expression must produce one value.
#[test]
fn the_statement_if_still_works() {
    let bundle = compile_source(
        "function sign of n\n\
         \x20   if n < 0\n\
         \x20       give \"down\"\n\
         \x20   give \"up\"\n\
         state n is client Whole starting 1\n\
         view\n\
         \x20   Text (sign of n)\n",
    );
    assert!(!bundle.client_js.is_empty());
}
