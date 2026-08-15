//! The gate for regressions a byte count cannot see.
//!
//! Every other gate in this repository weighs output. `benchmark.rs` counts
//! DOM crossings and effects, `scaling.rs` counts emitted bytes, and
//! `BENCHMARKS.md`'s generated region is matched exactly. All of it is
//! deterministic and none of it can flake, which is why the file says
//! plainly that "the counts are deterministic — there is no timing
//! anywhere".
//!
//! That sentence is also the hole. **A pass can become arbitrarily slower
//! without changing one byte it emits.** Issue #8 is the worked example:
//! the emitter's path scheduler ran a breadth-first search per named node,
//! over a set with a single element in it, and allocated two region-sized
//! vectors per call. Scheduling 1,025 view nodes took 1,866 ms where it now
//! takes 2.0 ms — 911× — and the walk it scheduled was byte-identical the
//! whole time. Every gate here passed, on every run, for as long as the
//! code was in the tree. Nothing in this repository measured time in a way
//! that could fail, so nothing failed.
//!
//! # What is asserted, and why it is not a time
//!
//! Not a duration. A wall-clock threshold on a GitHub runner is a coin
//! flip, and a gate that flakes is worse than no gate: it teaches everyone
//! to re-run until green, and then the run that was telling the truth gets
//! re-run too. This repository already has one such assertion —
//! `crates/zdc-lsp/tests/latency.rs` holds analysis of a six-kilobyte file
//! to ten milliseconds — and its own comment concedes the point ("a ratio
//! would fail on a loaded machine and teach everyone to ignore it"), then
//! guards the assertion with `if !release { return }`. CI runs `cargo test`
//! in debug. **That budget has therefore never once been enforced by CI**,
//! which is the honest fate of an absolute timing gate.
//!
//! What is asserted is a **shape**: over a geometric sweep of program
//! sizes, how much the cost *per unit of input* changes from the small end
//! to the large end. Call it the inflation. A pass linear in its input
//! holds its per-unit cost, so the inflation is about 1; an `n log n` pass
//! grows it by the log of the span; a quadratic one by the span; a cubic
//! one by the span squared. The sweep here spans 16×, so those readings are
//! 1, ~2.7, 16 and 256.
//!
//! This is robust to the thing that makes timing gates flaky. A runner that
//! is uniformly twice as slow multiplies every sample by two, and two
//! cancels in a ratio of two samples taken in the same process, on the same
//! machine, seconds apart. What does *not* cancel is a pass whose cost per
//! node grows with the program, which is exactly the class of defect #8 was
//! and exactly the class no count in this repository can see.
//!
//! # The numbers below are a ratchet, not a target
//!
//! Each axis records what the tree measures **today**, with a band around
//! it. Exceeding the band fails the build: that is the regression gate the
//! issue asks for. Falling well *under* it also fails the build, with a
//! different message: it means a pass got structurally faster and the
//! recorded number is now stale, so the gate has gone slack without anybody
//! deciding that. Lowering it is then part of the change that earned it,
//! the same discipline as regenerating `BENCHMARKS.md`.
//!
//! The emitter's recorded inflation is a disgrace and it is recorded
//! anyway, because that is what honesty costs here: #8 is open, the cubic
//! scheduler is still in `crates/zdc-codegen/src/view.rs` on this base, and
//! a gate that refused to admit the number would just be a red build. What
//! this gate buys today is that it cannot get *worse* unnoticed, and what
//! it buys the day #8 lands is a linearity gate, obtained by editing one
//! number down.
//!
//! # What was rejected
//!
//! * **An absolute per-pass time budget.** Flaky by construction on a
//!   shared runner; see `latency.rs` above for how that ends.
//! * **A statistical benchmark harness (criterion and friends).** It
//!   answers the same absolute question with better error bars, adds a
//!   dependency tree to a workspace that keeps `cargo test` as its only
//!   command, and needs a stored baseline from another machine to compare
//!   against. The baseline is the part that does not survive CI.
//! * **Operation counters inside the compiler.** Deterministic, which is
//!   attractive, and it was the first thing tried on paper. Two objections
//!   killed it: a counter only sees the pass it is soldered into, so it
//!   gates the last regression rather than the next one; and #8's own fix
//!   rewrites the very code that would carry the counter.
//! * **Counting allocations.** The cubic scheduler allocated two
//!   region-sized vectors per call, so an allocation count would have
//!   caught it exactly, deterministically, on every platform. It needs a
//!   `#[global_allocator]`, which needs `unsafe impl GlobalAlloc`, and
//!   every crate in this workspace forbids unsafe — `scripts/check-forbid-
//!   unsafe.sh` is a CI gate of its own. Worth revisiting only if that
//!   policy ever changes.
//!
//! # Running it
//!
//! `#[ignore]`d, in the pattern `crates/zdc-cli/tests/browser.rs` set: a
//! timed sweep needs a release build to mean anything and takes tens of
//! seconds, so it stays out of everybody's `cargo test` and is mandatory in
//! CI instead. `.github/workflows/ci.yml`'s `asymptotics` job is what makes
//! those two facts one fact.
//!
//! ```sh
//! cargo test -p zdc-bench --release --test asymptotics -- --ignored --nocapture
//! ```

use std::time::Duration;

use zdc_bench::{emitted_bytes, program_with_signals, time_emission, time_front_end, Curve};

/// The sweep. A 16× span, geometric so every step is the same question.
///
/// `program_with_signals(n)` is one `Column` holding `n` `Text` nodes, each
/// reading a signal of its own — so `n` is the number of view nodes in the
/// region *and* the number of named nodes the emitter has to schedule a
/// walk to, which is the axis #8's cubic lived on. It is also the generator
/// `survey_growth` in `scaling.rs` already sweeps, to 1,024, which is worth
/// saying out loud: the measurement was in the tree, on the right axis, at
/// the right size, for the whole life of the defect. It printed.
///
/// 64 at the small end rather than 8, so the smallest sample is comfortably
/// above the clock's resolution and the per-call overhead; 1,024 at the
/// large end because the emitter on this base costs about two seconds
/// there, and doubling it again would cost a quarter of an hour of CI for
/// no additional shape.
const SIZES: [usize; 5] = [64, 128, 256, 512, 1024];

/// How long each size may spend collecting samples before it settles for
/// the fewest [`zdc_bench::least`] will take.
const BUDGET: Duration = Duration::from_millis(400);

/// One measured axis, and the band its inflation has to stay inside.
struct Axis {
    label: &'static str,
    /// What this tree measures, on an unloaded machine.
    measured: f64,
    /// The multiple of `measured` at which the build fails.
    ///
    /// Not a confidence interval, and the distinction is the point. Five
    /// consecutive runs of this sweep on one machine gave the emitter
    /// 122, 130, 138, 115 and 140, and the front end 1.44, 1.43, 1.44,
    /// 1.49 and 1.25 — about ±12%, which the minimum-of-runs estimator
    /// keeps one-sided and small. Three times that spread is not where a
    /// three came from. It came from the readings themselves: linear is 1,
    /// `n log n` is 2.7, quadratic is 16. A factor of three is the
    /// smallest step that cannot be anything but a change in the order of
    /// growth, and a gate that fires on less than that is a gate that
    /// fires on a busy runner.
    ceiling: f64,
    /// Below `measured / slack`, the recorded number is stale.
    ///
    /// `None` where a legitimate change could lower the ratio without
    /// anything having got faster per node; the axis then says why.
    slack: Option<f64>,
    /// What the reader should do about a failure, in either direction.
    note: &'static str,
}

/// **The gate.**
#[test]
#[ignore = "a timed sweep; release only, and CI's `asymptotics` job is what runs it"]
fn no_pass_inflates_its_cost_per_node_beyond_what_is_recorded() {
    // Refused rather than skipped. A skip is how a gate becomes a thing
    // that has not run in a year — see `latency.rs`, whose budget returns
    // early on a debug build and is therefore never enforced by the CI that
    // builds in debug.
    if cfg!(debug_assertions) {
        panic!(
            "this sweep is meaningless without optimisation — a debug build measures the \
             compiler's own missing inlining, one to two orders of magnitude of it, and the \
             shape it reports is that build's rather than the emitter's. Run it with \
             `--release`; CI's `asymptotics` job does."
        );
    }

    let sources: Vec<String> = SIZES.iter().map(|&n| program_with_signals(n)).collect();

    // A shape gate divides one measurement by another, and division is the
    // operation that hides a generator which quietly stopped generating:
    // two sizes compiling the same program have a perfectly flat per-unit
    // cost and clear every threshold below. So the sweep proves it is a
    // sweep before anything is timed.
    let bytes: Vec<usize> = sources.iter().map(|source| emitted_bytes(source)).collect();
    assert_eq!(bytes.len(), SIZES.len());
    for window in bytes.windows(2) {
        assert!(
            window[1] > window[0] * 3 / 2,
            "the sweep emitted {bytes:?} bytes at sizes {SIZES:?}. Doubling the program has \
             to grow the emission or there is no sweep here to take a ratio over, and every \
             threshold in this file would pass over a constant."
        );
    }

    // The generator is deterministic and building the string is microseconds
    // against milliseconds of compiling it, so each axis makes its own copy
    // rather than the two sharing one and holding it across the sweep.
    let curves = [
        Curve::measure("front end", &SIZES, |n| {
            time_front_end(&program_with_signals(n), BUDGET)
        }),
        Curve::measure("emitter", &SIZES, |n| {
            time_emission(&program_with_signals(n), BUDGET)
        }),
    ];

    println!(
        "\n{:<12} {:>8} {:>12} {:>14}",
        "pass", "nodes", "least", "us/node"
    );
    for curve in &curves {
        for sample in &curve.samples {
            println!(
                "{:<12} {:>8} {:>10.3}ms {:>13.2}",
                curve.label,
                sample.size,
                sample.took.as_secs_f64() * 1e3,
                sample.per_unit() * 1e6
            );
        }
        println!(
            "{:<12} {:>8} {:>12} {:>14.2}   <- inflation over {:.0}x",
            curve.label,
            "",
            "",
            curve.inflation(),
            curve.span()
        );
    }

    // Written out here rather than beside the measurement so that the two
    // recorded numbers sit next to each other and can be read as a pair:
    // one pass is linear and one is not, on the same sweep, in the same
    // run.
    let axes = [
        Axis {
            label: "front end",
            // Essentially flat: 3.7 µs a node at 64 and 5.4 µs at 1,024.
            // What little rise there is comes from the resolver's name
            // tables growing, and 1.4× over 16× is not an order of growth.
            measured: 1.45,
            ceiling: 3.0,
            // No ratchet. §17.4.1's prelude — 150 definitions and 1,200
            // expressions resolved from nothing on every compile — is a
            // large *fixed* cost, and a fixed cost divided by a growing
            // node count falls, so landing it would push this ratio well
            // below 1 without one pass having got faster per node. A lower
            // bound here would fail on that change and be read as this
            // gate being wrong, which is how gates get deleted.
            slack: None,
            note: "parsing, resolving, splitting, flow-checking and typing. `split` is \
                   superlinear in the *product* of definitions and roots (§17.2), which this \
                   generator does not vary — every signal here is its own root and there are \
                   no shared definitions — so a product regression is not on this axis. \
                   `survey_compiler_asymptotics` in `scaling.rs` is where the product is \
                   measured, and `splitting_walks_the_product_of_definitions_and_roots` is \
                   the deterministic gate on its shape. The obvious next sweep to add here, \
                   and the one routing will make urgent.",
        },
        Axis {
            // 14 µs a node at 64 and 1,728 µs at 1,024. This is #8, and it
            // is between quadratic (16×) and cubic (256×) because the walk
            // has a linear part that still dominates at the small end.
            //
            // **This number is a defect, recorded.** #8 is open and the
            // breadth-first `Graph::route` is still in
            // `crates/zdc-codegen/src/view.rs` on this base. The gate that
            // could be written today is the one that stops it getting
            // worse; the gate that matters is the one this becomes when the
            // fix lands and this reads about 1. The ratchet below is what
            // makes that edit compulsory rather than optional.
            label: "emitter",
            measured: 125.0,
            ceiling: 3.0,
            slack: Some(4.0),
            note: "`zdc_codegen::compile` alone, with the front end run once and outside the \
                   timed region. This is issue #8's axis: a breadth-first search per named \
                   node, over a set with one element in it, run once per node already named. \
                   Emission is byte-identical however long it takes, which is why no other \
                   gate in this repository can see it.",
        },
    ];
    assert_eq!(axes.len(), curves.len(), "every curve measured is gated");

    for (axis, curve) in axes.iter().zip(&curves) {
        assert_eq!(axis.label, curve.label);
        let inflation = curve.inflation();
        assert!(
            inflation <= axis.measured * axis.ceiling,
            "`{}` cost {inflation:.2}× as much per node at {} nodes as at {} — recorded at \
             {:.2}×, and this gate fails past {:.2}×.\n\n{}\n\nA ratio is taken inside one \
             process on one machine, so a slow runner cancels out of it and re-running is \
             not the fix. What changed is the order of growth.",
            curve.label,
            SIZES[SIZES.len() - 1],
            SIZES[0],
            axis.measured,
            axis.measured * axis.ceiling,
            axis.note
        );
        let Some(slack) = axis.slack else { continue };
        assert!(
            inflation >= axis.measured / slack,
            "`{}` now costs {inflation:.2}× as much per node across the sweep, against the \
             {:.2}× recorded here. That is a large improvement and nothing is broken — but \
             the recorded number is the gate, and left where it is it now admits a \
             regression all the way back to where this pass used to be. Lower it to what \
             this run measured, in the change that earned it.\n\n{}",
            curve.label,
            axis.measured,
            axis.note
        );
    }
}
