//! Text a program wrote may not become syntax in an emitted file.
//!
//! Every case here is a literal from a `.zd` source that reaches a
//! generated file — a JavaScript expression, or a CSS declaration — and
//! the property is the same one §16.3.5 states for markup: what the
//! program wrote is a *value* in the output, never a token of it.
//!
//! The JavaScript half is asserted by running the emitted bundle in the
//! embedded engine rather than by grepping it, because the failure being
//! tested for is code that executes: a string-matching test can be
//! satisfied by output that still runs the payload under a different
//! spelling.

mod support;

use support::{compile_source, context, refusals, run};

/// A `class` argument that is not a literal is emitted as a getter which
/// concatenates the classes already collected. Those were interpolated
/// straight between two apostrophes, so a class literal containing one
/// closed the string and everything after it was JavaScript the page ran.
#[test]
fn a_class_literal_cannot_close_the_getter_it_is_written_into() {
    let bundle = compile_source(
        r#"
state theme is client Text starting "dark"

view
    Column
        Text "hi", class is "a'+(globalThis.$pwned='ran')+'b", class is theme
"#,
    );
    let mut engine = context(false);
    let pwned = run(
        &mut engine,
        &bundle.client_js,
        r#"
const $host = document.createElement('div');
main($host);
String(globalThis.$pwned)
"#,
    );
    assert_eq!(
        pwned, "undefined",
        "the class literal executed as JavaScript:\n{}",
        bundle.client_js
    );

    // And it is still the class it asked for, so the escape did not
    // change what the program means.
    let mut fresh = context(false);
    let classes = run(
        &mut fresh,
        &bundle.client_js,
        r#"
const $host = document.createElement('div');
main($host);
findTag($host, 'span').attributes['class']
"#,
    );
    assert!(
        classes.contains("a'+(globalThis.$pwned='ran')+'b") && classes.contains("dark"),
        "the class attribute lost the value the program wrote: {classes}"
    );
}
