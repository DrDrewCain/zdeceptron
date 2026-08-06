//! **A record literal in an arrow body, executed.**
//!
//! A concise arrow body that begins with `{` is a *block*, not an object
//! literal, and a record literal is the only value form this emitter
//! produces that begins with one. The two ways that goes wrong are both
//! silent at build time:
//!
//!   * two or more fields, `(n) => { x: n, y: n }`, is a `SyntaxError`,
//!     so the bundle does not parse and the page is blank;
//!   * one field, `(n) => { x: n }`, is a block holding a labelled
//!     statement, so the arrow returns `undefined` for every element and
//!     the program computes with holes in it.
//!
//! `zdc check` and `zdc build` both exit 0 in both cases (#194).
//!
//! Nothing here reads emitted text. Every assertion is a number the
//! program computed after the bundle was evaluated, because the one-field
//! case emits source that *does* parse and only a run can see it.

mod support;

use support::{compile_source, context, run};

/// Compile a program whose view shows one text signal, run it, and return
/// what the page says with the markup taken out.
fn text(declarations: &str) -> String {
    let source = format!("{declarations}view\n    Text answer\n");
    let bundle = compile_source(&source);
    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    let mut out = String::new();
    let mut inside = false;
    for ch in rendered.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out
}

/// **The one-field case, which parses and is wrong.** `map each n to
/// (Point with x is n)` emitted `(n) => { x: n }`, so `points` held three
/// `undefined`s: `length of` still answered 3 and every field read off
/// them was `undefined`. The fields are read back and summed here for
/// exactly that reason: a count would have passed.
#[test]
fn a_one_field_record_built_by_map_each_keeps_its_field() {
    assert_eq!(
        text(
            "record Point\n\
             \x20   x is Whole\n\
             \n\
             function pointsOf of ns\n\
             \x20   from ns\n\
             \x20   map each n to (Point with x is n * 10)\n\
             \n\
             function widthsOf of ps\n\
             \x20   from ps\n\
             \x20   map each p to p.x\n\
             \n\
             state answer is client Text from text of (sumOf of (widthsOf of \
             (pointsOf of [1, 2, 3])))\n"
        ),
        "60"
    );
}

/// **The two-field case, which does not parse.** `(n) => { x: n, y: n }`
/// is a `SyntaxError`, so before the fix this failed by the module not
/// evaluating at all rather than by the sum being wrong.
#[test]
fn a_two_field_record_built_by_map_each_keeps_both_fields() {
    assert_eq!(
        text(
            "record Point\n\
             \x20   x is Whole\n\
             \x20   y is Whole\n\
             \n\
             function pointsOf of ns\n\
             \x20   from ns\n\
             \x20   map each n to (Point with x is n, y is n * 2)\n\
             \n\
             function widthsOf of ps\n\
             \x20   from ps\n\
             \x20   map each p to p.x + p.y\n\
             \n\
             state answer is client Text from text of (sumOf of (widthsOf of \
             (pointsOf of [1, 2, 3])))\n"
        ),
        "18"
    );
}

/// **`sort each … by` is the same emission site.** The comparator's key
/// extractor is `const $k1 = (n) => {key};`, so a record key was the same
/// `SyntaxError`. Two fields, because with one the block form parses and
/// every key ties, which no ordering can tell apart.
///
/// The elements are summed rather than checked in order on purpose: two
/// object keys are neither `<` nor `>`, so the comparator answers 0 for
/// every pair and the sort is a no-op. What is being checked is that the
/// bundle parses and the sequence survives it.
#[test]
fn a_record_used_as_a_sort_key_leaves_a_bundle_that_runs() {
    assert_eq!(
        text(
            "record Rank\n\
             \x20   low is Whole\n\
             \x20   high is Whole\n\
             \n\
             function rankedBy of ns\n\
             \x20   from ns\n\
             \x20   sort each n by (Rank with low is n, high is n)\n\
             \n\
             state answer is client Text from text of (sumOf of (rankedBy of [3, 1, 2]))\n"
        ),
        "6"
    );
}

/// **A top-level `derived` holding a record.** `state p is client Point
/// from (Point with …)` emitted `derived(() => { x: count() })`, which is
/// the same defect one emission site away from the pipeline.
#[test]
fn a_derived_signal_holding_a_record_keeps_its_fields() {
    assert_eq!(
        text(
            "record Point\n\
             \x20   x is Whole\n\
             \x20   y is Whole\n\
             \n\
             state count is client Whole starting 4\n\
             \n\
             state spot is client Point from (Point with x is count, y is count * 3)\n\
             \n\
             state answer is client Text from text of (spot.x + spot.y)\n"
        ),
        "16"
    );
}

/// **A component's own `state … from`, which is a different emitter.**
/// `zdc-codegen`'s region locals write their own `derived(() => …)`, so
/// fixing the top-level one leaves this one broken.
#[test]
fn a_component_local_derived_holding_a_record_keeps_its_fields() {
    let bundle = compile_source(
        "record Point\n\
         \x20   x is Whole\n\
         \x20   y is Whole\n\
         \n\
         state count is client Whole starting 4\n\
         \n\
         component Dot with label\n\
         \x20   state spot is client Point from (Point with x is count, y is count * 3)\n\
         \n\
         \x20   Column\n\
         \x20       Text label\n\
         \x20       Text (text of (spot.x + spot.y))\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       Dot label is \"total \"\n",
    );
    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    assert!(
        rendered.contains("total ") && rendered.contains("16"),
        "the component's own derived record lost its fields: {rendered}"
    );
}
