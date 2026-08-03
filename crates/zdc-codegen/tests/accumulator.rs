//! **An accumulator that survives across iterations, executed.**
//!
//! The gap analysis of 2026-08-03 (§3.4) put the dungeon generator's
//! placement loop on the short list of things this language had no way to
//! express: a set of claimed positions that is *built up across the
//! placements*, with each placement rejection-sampling against everything
//! the earlier ones took. It measured that against a compiler with
//! neither local bindings nor a tail-call rewrite.
//!
//! Both landed, and the conclusion did not survive them. `examples/dungeon.zd`
//! is the port, and what is checked here is the property the analysis said
//! could not hold: **no two entities land on the same tile**. A `taken`
//! that did not survive from one iteration to the next would still place
//! the right *number* of entities — it would just place some of them on
//! top of each other, which is why counting them is not enough and the
//! distinctness is asserted directly.
//!
//! Nothing here is a probe. The program under test is the example, read
//! off disk, compiled the way `zdc build` compiles it, and run.

mod support;

use support::{compile_example, context};

/// Run a bundle and return what one of its signals holds, as JSON.
///
/// Reading the signal rather than the rendered page is what lets a list of
/// 960 tiles be inspected without rendering 960 nodes.
fn signal_json(client_js: &str, expression: &str) -> String {
    let mut context = context(false);
    let module = support::flatten(client_js);
    context
        .eval(boa_engine::Source::from_bytes(module.as_bytes()))
        .expect("the module must evaluate");
    let driver = format!(
        "const $host = document.createElement('div');\nmain($host);\nJSON.stringify({expression})"
    );
    context
        .eval(boa_engine::Source::from_bytes(driver.as_bytes()))
        .expect("the driver must return")
        .to_string(&mut context)
        .expect("a string")
        .to_std_string_escaped()
}

fn numbers(json: &str) -> Vec<i64> {
    json.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .filter(|piece| !piece.trim().is_empty())
        .map(|piece| piece.trim().parse().expect("a whole number"))
        .collect()
}

/// **The property §3.4 said could not hold.** Eleven entities — `3 + depth`
/// enemies and `2 + depth` items at depth 3 — placed against one `taken`
/// that both passes share, and no tile claimed twice.
#[test]
fn no_two_placements_land_on_the_same_tile() {
    let bundle = compile_example("examples/dungeon.zd");
    let spots = numbers(&signal_json(&bundle.client_js, "level().spots"));

    assert_eq!(spots.len(), 11, "3 + depth enemies then 2 + depth items");

    let mut sorted = spots.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        spots.len(),
        "a placement landed on a tile an earlier one had already claimed, \
         which is what happens when `taken` does not survive the iteration: {spots:?}"
    );
}

/// And they land on floor, not through a wall — the other half of what
/// `emptyFloorTile` promises, and the half a `taken` bug would not break.
#[test]
fn every_placement_lands_on_a_floor_tile() {
    let bundle = compile_example("examples/dungeon.zd");
    let spots = numbers(&signal_json(&bundle.client_js, "level().spots"));
    let tiles = signal_json(&bundle.client_js, "tiles()");
    let glyphs: Vec<&str> = tiles
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|piece| piece.trim().trim_matches('"'))
        .collect();

    assert_eq!(glyphs.len(), 960, "40 x 24, built at a computed length");
    for spot in spots {
        assert_eq!(
            glyphs[spot as usize], ".",
            "entity placed at {spot}, which is a wall"
        );
    }
}

/// **The accumulator at a size that would show a stack.** The example
/// places eleven entities; this places two thousand, against a `taken`
/// that is two thousand long by the end, and it has to return.
///
/// Measured on this file: **6.02 s** for 2,000 placements, **1.48 s** for
/// 1,000 and **372 ms** for 500 — a ratio of 4.07 for each doubling, so
/// this loop is quadratic and honestly so. That is the cost of asking
/// `taken contains next` of a list rather than of a hash set, it is what
/// the original pays too against a `Set` only because a `Set` is O(1),
/// and it is *not* the thing this test rules out.
///
/// What it rules out is the stack. The same shape with the recursion out
/// of tail position cannot reach five hundred at all — the embedded
/// interpreter answers "exceeded maximum number of recursive calls", and
/// `tests/depth.rs` records that measurement against the same builder.
/// Two thousand here returns the right answer with no frame per
/// placement, which is the property that makes the loop writable at all.
#[test]
fn two_thousand_placements_thread_one_set_without_a_stack() {
    let source = "\
state rooms is client List of Whole starting [0]\n\
state claimed is client Whole from length of (claimAll with taken is empty, remaining is 2000, next is 0).taken\n\
record Bag\n\
\x20   taken is List of Whole\n\
function claimAll with taken, remaining, next\n\
\x20   if remaining < 1\n\
\x20       give Bag with taken is taken\n\
\x20   if taken contains next\n\
\x20       give claimAll with taken is taken, remaining is remaining, next is next + 1\n\
\x20   give claimAll with taken is (append next to taken), remaining is remaining - 1, next is next + 1\n\
state answer is client Text from text of claimed\n\
view\n    Text answer\n";

    let bundle = support::compile_source(source);
    let mut context = context(false);
    let module = support::flatten(&bundle.client_js);
    context
        .eval(boa_engine::Source::from_bytes(module.as_bytes()))
        .expect("the module must evaluate");
    let shown = context
        .eval(boa_engine::Source::from_bytes(
            "const $host = document.createElement('div');\nmain($host);\nserialize($host)"
                .as_bytes(),
        ))
        .expect("2,000 placements must return")
        .display()
        .to_string();
    assert!(
        shown.contains("2000"),
        "two thousand distinct claims, one set: {shown}"
    );
}
