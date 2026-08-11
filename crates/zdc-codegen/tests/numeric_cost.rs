//! What the numeric library costs, measured, so a quadratic one cannot
//! land quietly.
//!
//! `depth.rs` records the *stack* limit that index recursion imposes on
//! the collection library. This is the other axis and a different finding:
//! every operation in `prelude/number.zd` is O(1), and the two ways that
//! could stop being true are both ways a previous library has actually
//! gone wrong.
//!
//! * **A `mod` written by repeated subtraction** is O(value / divisor),
//!   which is invisible on `7 mod 3` and does not terminate on the values
//!   a hash function produces. The first test below asks for a remainder
//!   at a magnitude where a subtractive implementation cannot finish, so
//!   the guard is an answer rather than a clock.
//! * **A generator that is not O(1) per draw** turns a per-frame workload
//!   into a per-frame stall. The second test threads a seed through a
//!   realistic number of draws — a Life generation on a 32×32 torus is
//!   1,024 cells and a minesweeper board is a few hundred — and puts a
//!   budget on it that a linear implementation clears by orders of
//!   magnitude.
//!
//! The re-measurement of 2026-08-03 is the reason this file exists: it
//! found `rest of`-based folds running 7.3 s against 88 ms for the
//! index-based ones over the same 10,000 elements, an 83× gap that no
//! test would have caught. A budget that generous is still decisive.

mod support;

use std::time::{Duration, Instant};

use support::{compile_source, context, run};

/// Compile a program whose view shows one text signal and return what the
/// page says, the way `library.rs` does — the value has to survive the
/// whole compiler for this to be a measurement of the library at all.
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

/// `mod` and `quotient` are O(1), and this is the shape of the question
/// that proves it without a stopwatch.
///
/// 1,000,000,007 remainder 7 is one `floor`, one `*` and one `-`. Written
/// as repeated subtraction it is 142,857,143 iterations, which in the
/// embedded interpreter is not a slow answer but no answer at all.
///
/// Both are eliminated with `valueOr`: §14A.3 makes the
/// `Decimal`-to-`Whole` narrowing partial, so `mod` and `quotient` give an
/// `Option of Whole`. The elimination is one call and does not change what
/// is being measured — the cost under test is the arithmetic, and a
/// `valueOr` is O(1) whatever the magnitude.
#[test]
fn a_remainder_costs_the_same_at_every_magnitude() {
    let whole = |expr: &str| {
        text(&format!(
            "state answer is client Text from text of \
             (valueOr with maybe is ({expr}), fallback is 0 - 1)\n"
        ))
    };

    // 7 × 142,857,143 is 1,000,000,001, so the remainder is 6.
    assert_eq!(whole("mod with value is 1000000007, divisor is 7"), "6");
    assert_eq!(
        whole("quotient with value is 1000000007, divisor is 7"),
        "142857143"
    );
    // And at the top of the exact-integer range §14A.3 admits to, which is
    // where a loop would have given up long before.
    assert_eq!(
        whole("mod with value is 9007199254740991, divisor is 1000000"),
        "740991"
    );
}

/// Draw `count` numbers and return the last one, with the time the
/// *emitted JavaScript* took — compilation is outside the clock, because
/// a list literal of four thousand elements is a measurement of the
/// parser and this is not a test about the parser.
///
/// The workload is a pipeline rather than a recursion on purpose. A chain
/// of `nextSeed` calls costs three interpreter frames per draw and runs
/// the embedded engine out of stack at a few hundred, which is the finding
/// `depth.rs` already records about the *collection* library and would
/// only be re-measuring it here. `map each` emits a `.map`, so the stack
/// stays constant and what varies with `count` is only the arithmetic —
/// which is the thing under test.
fn draw(count: usize) -> (String, Duration) {
    let seeds: Vec<String> = (1..=count).map(|n| n.to_string()).collect();
    let source = format!(
        "function draws of seeds\n    \
         from seeds\n    \
         map each seed to randomBits of seed\n\
         state xs is client List of Whole starting [{}]\n\
         state answer is client Text from text of \
         (valueOr with maybe is (last of (draws of xs)), fallback is 0)\n\
         view\n    Text answer\n",
        seeds.join(", ")
    );
    let bundle = compile_source(&source);
    let mut context = context(false);
    let started = Instant::now();
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    let elapsed = started.elapsed();
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
    (out, elapsed)
}

/// A realistic number of draws, inside a budget a linear implementation
/// clears by orders of magnitude.
///
/// 4,096 is a Life generation on a 64×64 torus and rather more than a
/// minesweeper board or a dungeon floor — the K2 workloads that motivated
/// the whole numeric prelude. The budget is deliberately enormous: the
/// point is to separate "linear" from "quadratic", not to pin a number a
/// machine running eight other builds will fail.
#[test]
fn a_realistic_number_of_draws_stays_linear() {
    let (small, small_best) = best_of_three(2_048);
    let (large, large_best) = best_of_three(4_096);

    // Correct first. A fast wrong answer is not the property under test.
    let small_value: u32 = small.parse().unwrap_or_else(|_| panic!("{small}"));
    let large_value: u32 = large.parse().unwrap_or_else(|_| panic!("{large}"));
    assert_ne!(
        small_value, large_value,
        "the last draw of 2,048 seeds and of 4,096 must differ"
    );

    assert!(
        large_best < Duration::from_secs(30),
        "4,096 draws took {large_best:?}; every operation in `prelude/number.zd` is \
         documented O(1), so if this is slow one of them stopped being"
    );

    // Doubling the work roughly doubles the time — measured as the *best*
    // of three runs at each size rather than a single one.
    //
    // Noise on a shared runner is one-sided: another build stealing the
    // core makes a run slower and nothing makes it faster, so the minimum
    // is the closest thing to the cost with the machine to itself, and
    // the ratio of two minima is the ratio under test. A single pair is
    // the ratio of two samples from a distribution with a long right
    // tail, which is why this gate failed once at 4.1 on a Windows runner
    // — 55.6ms against 226.9ms — and passed on a re-run with no change.
    //
    // The 50ms threshold below is unchanged, and raising it was tried and
    // rejected: the runner that flaked measured 55.6ms, so anything above
    // that turns the gate off on the one platform where it actually runs.
    // On this repository's development machines the workload is ~15ms and
    // the ratio is never asserted at all — which is the threshold doing
    // its job, because at that size the clock's own resolution is a large
    // share of the measurement.
    if small_best > Duration::from_millis(50) {
        let ratio = large_best.as_secs_f64() / small_best.as_secs_f64();
        assert!(
            ratio < 3.0,
            "doubling the draws multiplied the time by {ratio:.1}; linear is about 2 and \
             quadratic is about 4 (best of three: {small_best:?} → {large_best:?})"
        );
    }
}

/// One size's answer, and the fastest of three runs at it.
///
/// Three and not more because the point is to drop an outlier, not to
/// build a benchmark: the whole test stays inside a couple of seconds,
/// and a gate that takes a minute is one somebody turns off.
///
/// The answer is the same every time — the draw is deterministic in its
/// seeds — so which run it comes from does not matter, and taking it here
/// keeps this to three compilations per size rather than four.
fn best_of_three(count: usize) -> (String, Duration) {
    let mut answer = None;
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let (drawn, elapsed) = draw(count);
        best = best.min(elapsed);
        answer.get_or_insert(drawn);
    }
    (answer.expect("three measurements"), best)
}
