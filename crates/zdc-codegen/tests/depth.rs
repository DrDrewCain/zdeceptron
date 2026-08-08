//! How deep the library's folds go, and how deep they used to go.
//!
//! This file used to record a defect. §17.4.10 observed that ZDeceptron
//! had no local bindings, so a fold over a collection could not carry an
//! accumulator and had to assemble its answer on the way back out —
//! `value + (sumFrom …)`. The stack depth of `sumOf`, `join`,
//! `listContains` and `slice` was therefore linear in the input, and
//! **four thousand elements exhausted the embedded interpreter**. Two
//! hundred were fine. The number in between was nobody's business until a
//! user's list got long.
//!
//! Two changes closed it, and both were needed:
//!
//! 1. §17.4.10's local binding, so a fold has somewhere to keep what it
//!    has computed and can end with a call and nothing else; and
//! 2. the emitter, which turns a call in tail position into a jump —
//!    because no JavaScript engine does. ES6 specified tail calls and no
//!    major engine shipped them, so "tail-shaped" on its own would have
//!    bought exactly nothing.
//!
//! What is pinned below is therefore the opposite of what used to be:
//! that the depth of a fold no longer depends on the length of what it
//! folds. The remaining limit is time and memory, linear in the input,
//! in the terms a program hits it in — how many elements.

mod support;

use support::{compile_source, context};

/// Compile a program whose view shows one text signal, run it, and report
/// what the page says, or the error the host raised trying.
fn run_fold(declarations: &str) -> Result<String, String> {
    let bundle = compile_source(&format!("{declarations}view\n    Text answer\n"));
    let mut context = context(false);
    let module = support::flatten(&bundle.client_js);
    context
        .eval(boa_engine::Source::from_bytes(module.as_bytes()))
        .map_err(|e| e.to_string())?;
    let driver = "const $host = document.createElement('div');\nmain($host);\nserialize($host)";
    context
        .eval(boa_engine::Source::from_bytes(driver.as_bytes()))
        .map(|value| value.display().to_string())
        .map_err(|e| e.to_string())
}

/// A literal list of `count` ones.
fn ones(count: usize) -> String {
    vec!["1"; count].join(", ")
}

fn sum_of(count: usize) -> Result<String, String> {
    run_fold(&format!(
        "state xs is client List of Whole starting [{}]\n\
         state answer is client Text from text of (sumOf of xs)\n",
        ones(count)
    ))
}

/// A list of a size an ordinary program has works, which is what makes
/// the library usable at all.
#[test]
fn a_fold_over_an_ordinary_list_is_fine() {
    let answer = sum_of(200).expect("200 elements must fold");
    assert!(answer.contains("200"), "{answer}");
}

/// **The number this file existed to record.** Four thousand elements
/// used to run the interpreter out of stack inside `sumFrom`; they now
/// return the right answer.
#[test]
fn four_thousand_elements_used_to_exhaust_the_stack_and_now_do_not() {
    let answer = sum_of(4_000).expect("4,000 elements must fold");
    assert!(answer.contains("4000"), "{answer}");
}

/// And the ceiling did not merely move: the depth is constant, so
/// twenty-five times the input costs twenty-five times the work and
/// nothing else. A hundred thousand is where this test stops because a
/// test suite has to finish, not because the fold does — a million
/// elements returns too, in about sixteen seconds, nearly all of it spent
/// compiling the literal rather than folding it.
#[test]
fn the_depth_of_a_fold_no_longer_grows_with_its_input() {
    let answer = sum_of(100_000).expect("100,000 elements must fold");
    assert!(answer.contains("100000"), "{answer}");
}

/// The other two list operations §17.4.10 named, at the size that used to
/// fail. `join` accumulates text; `listContains` walks to the end without
/// finding anything, which is its worst case and the one that used to
/// recurse deepest.
#[test]
fn join_and_contains_survive_the_size_that_used_to_fail() {
    let joined = run_fold(&format!(
        "state xs is client List of Text starting [{}]\n\
         state answer is client Text from text of (length of (join with parts is xs, \
         using is \"\"))\n",
        vec!["\"a\""; 4_000].join(", ")
    ))
    .expect("4,000 parts must join");
    assert!(joined.contains("4000"), "{joined}");

    let searched = run_fold(&format!(
        "state xs is client List of Whole starting [{}]\n\
         state answer is client Text from text of (xs contains 2)\n",
        ones(4_000)
    ))
    .expect("4,000 elements must be searched");
    assert!(searched.contains("no"), "{searched}");
}

/// One walk written two ways, and only one of them is a jump.
///
/// The rewrite turns a call in tail position into a jump when a function
/// calls **itself**. Two functions that call each other in tail position
/// are two ordinary calls, one frame each, so a walk split across a pair
/// has the linear depth this whole file exists to record the removal of.
///
/// Found by writing `examples/sorting.zd`: a merge needs an element from
/// each of two lists, `when` is a statement, and the obvious spelling is
/// therefore one `when` per side with the second in a helper. That
/// spelling is the `ping`/`pong` pair below. The example nests the second
/// `when` inside the first arm instead, which is the `alone` version, and
/// the difference is the difference between these two assertions.
///
/// **#198 is closed and this is where whoever closed it says so.** Both
/// spellings now return. A cycle of functions that give the result of
/// calling one another is emitted as a trampoline: each member's body
/// becomes a `$step$` function that returns a bounce marker instead of
/// calling across, and the wrapper that keeps the member's name drives
/// them in a loop. Depth is constant for the pair exactly as it already
/// was for the single function.
///
/// The self-call is still `continue $tail` — the cheaper rewrite, no
/// allocation — so `walkAlone` below is emitted exactly as it was before
/// the trampoline existed. Only a call that *crosses* to another member
/// of the cycle bounces, which is what keeps the cost on the programs
/// that need it.
#[test]
fn a_tail_call_between_two_functions_is_not_a_frame() {
    let alone = run_fold(
        "state answer is client Text from text of (walkAlone with index is 0, total is 0)\n\
         function walkAlone with index, total\n\
         \x20   if index >= 20000\n\
         \x20       give total\n\
         \x20   give walkAlone with index is index + 1, total is total + 1\n",
    )
    .expect("20,000 steps of a self call must return");
    assert!(alone.contains("20000"), "{alone}");

    let split = run_fold(
        "state answer is client Text from text of (ping with index is 0, total is 0)\n\
         function ping with index, total\n\
         \x20   if index >= 20000\n\
         \x20       give total\n\
         \x20   give pong with index is index, total is total + 1\n\
         function pong with index, total\n\
         \x20   give ping with index is index + 1, total is total\n",
    );
    let split = split.expect(
        "20,000 steps split across two functions must return: #198 is the \
         trampoline that makes a cycle of tail calls constant-depth",
    );
    assert!(split.contains("20000"), "{split}");

    // The two spellings are the same walk, so they must agree on the
    // answer and not merely both survive. A trampoline that dropped or
    // repeated a step would still return.
    assert_eq!(
        alone, split,
        "one walk written two ways must give one answer"
    );
}

/// **The spelling `examples/sorting.zd` is about**, which the pair above
/// does not reach: the crossing call is the body of a `when` arm rather
/// than of an `if`.
///
/// That distinction is the whole reason #198 existed. A merge needs an
/// element from each of two lists, `when` is a statement, so the obvious
/// spelling is one `when` per side with the second in a helper the first
/// tail-calls — and the tail-call walk has to see through an arm body to
/// find it. Written this way the merge died at 3200 elements; it now
/// merges a hundred thousand, which is what the nested one-function
/// spelling in the example already did.
#[test]
fn a_crossing_call_inside_a_when_arm_is_found_too() {
    let merged = run_fold(&format!(
        "state xs is client List of Whole starting [{}]\n\
         state answer is client Text from text of \
         (alpha with items is xs, index is 0, total is 0)\n\
         function alpha with items, index, total\n\
         \x20   when listAt with value is items, index is index\n\
         \x20       None\n\
         \x20           give total\n\
         \x20       Some with v\n\
         \x20           give beta with items is items, index is index, total is total + v\n\
         function beta with items, index, total\n\
         \x20   give alpha with items is items, index is index + 1, total is total\n",
        ones(20_000)
    ))
    .expect("20,000 elements folded across a `when` arm must return");
    assert!(merged.contains("20000"), "{merged}");
}

/// A cycle is a cycle at any length. `f` gives `g` gives `h` gives `f` is
/// the same shape as a pair, and a rule written for pairs would take the
/// pair and miss this for no reason a programmer could predict — which is
/// why the unit is the strongly connected component rather than the
/// mutually-recursive pair.
#[test]
fn a_cycle_of_three_functions_is_a_loop_as_much_as_a_pair_is() {
    let round = run_fold(
        "state answer is client Text from text of (one with n is 30000, total is 0)\n\
         function one with n, total\n\
         \x20   if n is 0\n\
         \x20       give total\n\
         \x20   give two with n is n - 1, total is total + 1\n\
         function two with n, total\n\
         \x20   give three with n is n, total is total\n\
         function three with n, total\n\
         \x20   give one with n is n, total is total\n",
    )
    .expect("a three-function cycle must return");
    assert!(round.contains("30000"), "{round}");
}

/// **The map half of the same finding.** A `Map` has no indexed access,
/// so `mapKeyAt` has to resolve a position against an array of the map's
/// keys — and a version that built that array per call would be O(n) per
/// step and O(n²) per fold, which is exactly the trap `rest of` set for
/// lists and which the key cache in `$mapKeyAt` is there to avoid.
///
/// Ten thousand entries is where the difference stops being academic.
/// Measured on this file: **179 ms** for ten thousand against 47 ms for
/// two and a half thousand — a ratio of 3.8 against the 4.0 the input
/// grew by. The uncached version was run against the same test to check
/// the difference is not a formality: 931 ms for 625 entries and 12.7 s
/// for 2,500, a ratio of 13.7 on the way to the 16 a quadratic walk
/// costs, and 271 times slower than the cache at 2,500. Ten thousand it
/// could not do at all — it took `Map`'s own iterator through ten
/// thousand full spreads and the host gave up.
///
/// What is asserted here is that the fold *returns the right answer* at
/// ten thousand. The property underneath it — one key array per map
/// however many positions are asked for — is counted exactly by the test
/// below, because a wall-clock ratio is a fact about a machine and about
/// what else that machine is running.
#[test]
fn a_fold_over_a_ten_thousand_entry_map_returns_rather_than_grinding() {
    let (small, _) = fold_a_map(2_500);
    let (large, _) = fold_a_map(10_000);
    assert!(small.contains("2500"), "{small}");
    assert!(large.contains("10000"), "{large}");
}

/// **Why that fold is linear, counted rather than timed.** `$mapKeyAt`
/// resolves a position against an array of the map's keys, and the whole
/// question is how often it builds one: once per map is linear, once per
/// call is quadratic.
///
/// So the map is asked. Its own `keys` is replaced with one that counts,
/// three positions are read, and the count has to be 1. A `$mapKeyAt`
/// that dropped the cache would report 3 here and would not need ten
/// thousand entries and a stopwatch to be caught.
#[test]
fn a_map_gives_up_its_keys_once_however_often_it_is_walked() {
    let bundle = compile_source(
        "state m is client Map of Text to Whole starting [\"a\" to 1, \"b\" to 2, \"c\" to 3]\n\
         state answer is client Text from text of (length of (keys of m))\n\
         view\n    Text answer\n",
    );
    let mut context = context(false);
    let module = support::flatten(&bundle.client_js);
    context
        .eval(boa_engine::Source::from_bytes(module.as_bytes()))
        .expect("the module must evaluate");
    let counted = context
        .eval(boa_engine::Source::from_bytes(
            "const $counted = new Map([['a', 1], ['b', 2], ['c', 3]]);\n\
             let $spreads = 0;\n\
             const $realKeys = $counted.keys.bind($counted);\n\
             $counted.keys = () => { $spreads += 1; return $realKeys(); };\n\
             let $walked = '';\n\
             for (let i = 0; i < 3; i += 1) { $walked += $mapKeyAt($counted, i).fields[0]; }\n\
             `${$walked} ${$spreads}`"
                .as_bytes(),
        ))
        .expect("the walk must return")
        .to_string(&mut context)
        .expect("a string")
        .to_std_string_escaped();
    assert_eq!(
        counted, "abc 1",
        "three positions read out of one map, and the key array built once"
    );
}

/// Sum the values of a map of `count` entries, and report how long the
/// emitted code took — the *fold*, not the compile, because a literal of
/// ten thousand entries takes longer to parse than to walk and would hide
/// the thing being measured.
fn fold_a_map(count: usize) -> (String, std::time::Duration) {
    let entries: Vec<String> = (0..count).map(|i| format!("\"k{i}\" to 1")).collect();
    let bundle = compile_source(&format!(
        "state m is client Map of Text to Whole starting [{}]\n\
         state answer is client Text from text of (sumOf of (values of m))\n\
         view\n    Text answer\n",
        entries.join(", ")
    ));
    let mut context = context(false);
    let module = support::flatten(&bundle.client_js);
    context
        .eval(boa_engine::Source::from_bytes(module.as_bytes()))
        .expect("the module must evaluate");
    let driver = "const $host = document.createElement('div');\nmain($host);\nserialize($host)";
    let started = std::time::Instant::now();
    let shown = context
        .eval(boa_engine::Source::from_bytes(driver.as_bytes()))
        .map(|value| value.display().to_string())
        .expect("the fold must return");
    (shown, started.elapsed())
}

/// **`indices` and `filled`, at a size no source could spell out.**
///
/// These build rather than fold, so they can fail two ways rather than
/// one: the recursion can go deep, and the *chain* `append` links can be
/// flattened once per element instead of once. The second is the one that
/// does not announce itself — a quadratic builder returns the right list
/// and merely takes forever — so it is measured, not asserted about.
///
/// The shape of the check is the ratio, not the wall clock: four times
/// the count costs about four times the time when the build is linear and
/// about sixteen when it is quadratic, and the two are far enough apart
/// that a loaded machine cannot turn one into the other. Measured on this
/// file with `indices`: **66.7 ms** at 25,000 against **17.4 ms** at
/// 6,250, a ratio of 3.83 against the 4.0 the input grew by, along a
/// ladder that is straight the whole way — 1.2 ms at 250, 2.9 ms at
/// 1,000, 11.3 ms at 4,000.
///
/// The other way round was measured too, by writing the same loop with
/// the call inside the `append` — `give append index to (indicesFrom …)`
/// — rather than in tail position, and it does not fail by being slow.
/// **It fails at five hundred elements**, with the embedded interpreter
/// reporting "exceeded maximum number of recursive calls": without the
/// tail position there is a frame per element, so the quadratic flatten
/// never gets a chance to be the thing that hurts. 250 returned, in
/// 1.34 ms against the tail version's 1.20 ms — indistinguishable, which
/// is exactly why a test at a realistic size is the only one worth
/// having.
#[test]
fn building_at_a_computed_length_is_linear_in_that_length() {
    let (small, small_flattens) = build_indices(6_250);
    let (large, large_flattens) = build_indices(25_000);
    assert!(small.contains("6250"), "{small}");
    assert!(large.contains("25000"), "{large}");

    // Non-vacuity first, because a counter that never fired would satisfy
    // the comparison below and prove nothing — which is how the wall-clock
    // version could pass while measuring the machine.
    assert!(
        small_flattens > 0,
        "no flatten was counted, so the instrumentation missed the path it watches"
    );
    // The claim, exactly: four times the elements does not buy more
    // flattens. A quadratic builder flattens once per element, so these
    // would be 6,250 and 25,000 rather than a handful.
    assert!(
        large_flattens <= small_flattens,
        "four times the elements flattened the chain more often \
         ({small_flattens} then {large_flattens}); a linear build flattens a \
         bounded number of times however long the list is"
    );
}

/// And the depth does not grow either: a hundred thousand elements is
/// past every stack this compiler has been run on, and the builder is in
/// tail position so it never uses one.
#[test]
fn building_a_hundred_thousand_elements_does_not_touch_the_stack() {
    let (shown, _) = build_indices(100_000);
    assert!(shown.contains("100000"), "{shown}");
}

/// Build `indices of count`, and report how long the emitted code took —
/// the build, not the compile. The count is a signal rather than a
/// literal so that nothing in the source is proportional to it, which is
/// what makes the timing a measurement of the builder.
fn build_indices(count: usize) -> (String, u64) {
    let bundle = compile_source(&format!(
        "state n is client Whole starting {count}\n\
         state xs is client List of Whole from indices of n\n\
         state answer is client Text from text of (length of xs)\n\
         view\n    Text answer\n"
    ));
    let mut context = context(false);
    let module = support::flatten(&bundle.client_js);
    // Counted, not timed (#192). `$append` makes an O(1) link and `$force`
    // flattens the chain the first time anything reads it, caching on the
    // node it was asked about — so a linear build flattens a bounded
    // number of times however long the list is, and the quadratic failure
    // this test exists to catch is one flatten per element.
    //
    // The increment sits past `if (!(xs instanceof $Ap))` and
    // `if (xs.flat)`, so cached and non-chain calls are not counted: only
    // flattens that copy.
    let module = module.replace(
        "const added = [];",
        "globalThis.$forced = (globalThis.$forced ?? 0) + 1; const added = [];",
    );
    assert!(
        module.contains("$forced"),
        "the flatten counter was not injected, so `$force` no longer looks the way this \
         test reads it and the count below would be meaningless"
    );
    context
        .eval(boa_engine::Source::from_bytes(module.as_bytes()))
        .expect("the module must evaluate");
    let driver = "const $host = document.createElement('div');\nmain($host);\nserialize($host)";
    let shown = context
        .eval(boa_engine::Source::from_bytes(driver.as_bytes()))
        .map(|value| value.display().to_string())
        .expect("the build must return");
    let forced = context
        .eval(boa_engine::Source::from_bytes(b"globalThis.$forced ?? 0"))
        .expect("the counter is readable")
        .to_number(&mut context)
        .expect("the counter is a number") as u64;
    (shown, forced)
}

/// `slice` is the text half of the same finding, measured in characters
/// rather than in elements. Four thousand of them were past the limit as
/// well.
#[test]
fn slicing_four_thousand_characters_returns_rather_than_running_out() {
    let text: String = std::iter::repeat_n('a', 4_000).collect();
    let answer = run_fold(&format!(
        "state s is client Text starting \"{text}\"\n\
         state answer is client Text from text of (length of (slice with value is s, \
         start is 0, stop is 4000))\n"
    ))
    .expect("4,000 characters must slice");
    assert!(answer.contains("4000"), "{answer}");
}
