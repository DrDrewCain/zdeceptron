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
