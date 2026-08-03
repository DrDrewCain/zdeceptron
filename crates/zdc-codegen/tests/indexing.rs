//! `at` is total, and the index it is total over is not the integers.
//!
//! §5.4 makes indexing bounds-checked and §14A.3 makes `Whole` an f64, so
//! `xs at (3 / 2)` is a program the checker accepts with an index of
//! `1.5`. A range comparison alone admits it and then reads a property
//! that is not there, so `at` answered `Some(undefined)` — an `Option of
//! Whole` inhabited by a value of no type, which `when` unwraps and hands
//! to whatever follows.
//!
//! Run in the engine rather than read out of the text, because the wrong
//! answer is a value and not a token.

mod support;

use support::{compile_source, context, run};

fn rendered(source: &str) -> String {
    let bundle = compile_source(source);
    let mut engine = context(false);
    run(
        &mut engine,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    )
}

/// `at` on a list, with an index that is a whole number and one that is
/// not. `3 / 2` is `1.5`, and there is no element there.
#[test]
fn a_list_index_that_is_not_a_whole_number_is_none() {
    let program = |divisor: &str| {
        format!(
            r#"
state xs is client List of Whole starting [10, 20]
state pick is client Whole starting 3
state out is client Text from firstOf with xs, pick

function firstOf with all, n
    when all at (n / {divisor})
        Some with v
            give text of v
        None
            give "none"

view
    Column
        Text out
"#
        )
    };

    assert!(
        rendered(&program("2")).contains(">none<"),
        "3 / 2 is 1.5, which indexes nothing: {}",
        rendered(&program("2"))
    );
    // The check is about the *kind* of the index, not about narrowing the
    // range: `3 / 3` is 1, and there is an element there.
    assert!(
        rendered(&program("3")).contains(">20<"),
        "3 / 3 is 1, which is the second element: {}",
        rendered(&program("3"))
    );
}

/// The same rule for `Text`, which indexes by code point and had the same
/// range-only guard.
#[test]
fn a_text_index_that_is_not_a_whole_number_is_none() {
    let program = |divisor: &str| {
        format!(
            r#"
state word is client Text starting "abc"
state pick is client Whole starting 3
state out is client Text from charOf with word, pick

function charOf with s, n
    when s at (n / {divisor})
        Some with c
            give c
        None
            give "none"

view
    Column
        Text out
"#
        )
    };

    assert!(rendered(&program("2")).contains(">none<"));
    assert!(rendered(&program("3")).contains(">b<"));
}
