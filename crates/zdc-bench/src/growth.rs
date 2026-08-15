//! How a pass's cost grows with the program, in the one form a clock can
//! state on a machine nobody controls.
//!
//! Everything else in this crate counts. Counts are deterministic, so they
//! can be pinned exactly and a change to one is a build failure (§14A.4).
//! That is the right instrument for almost everything, and it has one blind
//! spot: **a pass can get arbitrarily slower without changing a single byte
//! it produces.** Issue #8 is the instance. The emitter's path scheduler ran
//! a breadth-first search per named node over a set with one element in it,
//! which made scheduling cubic in the size of a region — 1,866 ms to
//! schedule 1,025 view nodes — and the walk it scheduled came out
//! byte-identical throughout. Every gate in this repository passed, every
//! run of the day.
//!
//! So the thing that has to be measured here is time, and time on a shared
//! CI runner is not a number. What *is* a number is the **shape**: hold one
//! program generator fixed, sweep its size geometrically, and ask how the
//! cost per unit of input changes from one end of the sweep to the other. A
//! runner that is uniformly half as fast multiplies every point by the same
//! constant, and a ratio of two points taken in one process on one machine
//! divides it back out. That is the only claim about time this file makes,
//! and it is the one that would have caught #8: a linear pass has an
//! inflation of about 1 whatever the machine, a quadratic one has the span,
//! and a cubic one has the span squared.
//!
//! [`least`] is the estimator, and the choice matters more than it looks —
//! see its own comment.

use std::time::{Duration, Instant};

use crate::sizes::try_compile;

/// The least of several runs of `body`, within a time budget.
///
/// **A mean would be wrong here and a median would be weaker.**
/// Interference on a shared machine — another job, a migration between
/// cores, a frequency drop — only ever *adds* time to a sample. The
/// uncontended cost of the work is therefore the minimum, and every sample
/// above it is that sample's noise rather than the pass's cost. Taking the
/// least of several runs is what makes a ratio of two of them meaningful on
/// a runner that is doing other things, and it is why this file can assert
/// where `crates/zdc-lsp/tests/latency.rs`'s absolute budget cannot.
///
/// The budget bounds the sweep rather than the sample: a size that costs
/// two seconds is sampled the minimum number of times and a size that costs
/// a microsecond is sampled the maximum, which is the right way round.
pub fn least(budget: Duration, mut body: impl FnMut()) -> Duration {
    /// Enough that one unlucky sample cannot be the answer.
    const FEWEST: usize = 3;
    /// Past this, another run buys less than it costs.
    const MOST: usize = 9;

    let started = Instant::now();
    let mut least = Duration::MAX;
    for run in 0..MOST {
        let began = Instant::now();
        body();
        least = least.min(began.elapsed());
        if run + 1 >= FEWEST && started.elapsed() >= budget {
            break;
        }
    }
    least
}

/// One size on a sweep, and what the pass cost there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub size: usize,
    pub took: Duration,
}

impl Sample {
    /// Seconds per unit of input — the quantity whose *change* is the gate.
    pub fn per_unit(&self) -> f64 {
        self.took.as_secs_f64() / self.size.max(1) as f64
    }
}

/// A pass measured across a geometric sweep of input sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Curve {
    pub label: &'static str,
    pub samples: Vec<Sample>,
}

impl Curve {
    /// Measure `cost` at every size, smallest first.
    pub fn measure(
        label: &'static str,
        sizes: &[usize],
        mut cost: impl FnMut(usize) -> Duration,
    ) -> Curve {
        Curve {
            label,
            samples: sizes
                .iter()
                .map(|&size| Sample {
                    size,
                    took: cost(size),
                })
                .collect(),
        }
    }

    fn first(&self) -> Sample {
        *self.samples.first().expect("a sweep has a smallest size")
    }

    fn last(&self) -> Sample {
        *self.samples.last().expect("a sweep has a largest size")
    }

    /// How much larger the sweep's largest input is than its smallest.
    pub fn span(&self) -> f64 {
        self.last().size as f64 / self.first().size as f64
    }

    /// **The measured quantity.** Cost per unit of input at the largest
    /// size, over the same at the smallest.
    ///
    /// Read it against [`Curve::span`]: a pass linear in its input holds
    /// its per-unit cost, so the inflation is about 1 whatever the sweep;
    /// an `n log n` pass grows it by the log of the span; a quadratic one
    /// by the span; a cubic one by the span squared. Every constant factor
    /// the machine contributes — a slow runner, a debug build, a cold
    /// cache that is cold at both ends — is in both terms and cancels.
    pub fn inflation(&self) -> f64 {
        self.last().per_unit() / self.first().per_unit()
    }
}

/// The passes before the emitter, run on a source, timed once.
///
/// Written out rather than called through [`try_compile`] because the
/// point of separating the two halves is that a regression in one must not
/// be reported as a regression in the other — which is the mistake #310
/// found in `crates/zdc-lsp/tests/latency.rs`, where the two halves were
/// not compiling the same program and the whole difference came out
/// attributed to codegen.
fn run_front_end(source: &str) {
    let program = zdc_parser::parse(source).expect("the generated program parses");
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("the generated program resolves");
    let split = zdc_graph::split(&hir);
    assert!(!split.has_errors(), "the generated program splits");
    let verdict = zdc_graph::ifc(&hir, &split);
    std::hint::black_box(&verdict);
    let table = zdc_types::check(&hir, &split).expect("the generated program typechecks");
    std::hint::black_box(&table);
}

/// What parsing, resolving, splitting, flow-checking and typing cost.
pub fn time_front_end(source: &str, budget: Duration) -> Duration {
    least(budget, || run_front_end(source))
}

/// What the emitter alone costs, with the front end run once and outside.
///
/// This is the axis issue #8 lived on and the axis no byte count can see.
pub fn time_emission(source: &str, budget: Duration) -> Duration {
    let program = zdc_parser::parse(source).expect("the generated program parses");
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("the generated program resolves");
    let split = zdc_graph::split(&hir);
    assert!(!split.has_errors(), "the generated program splits");
    let verdict = zdc_graph::ifc(&hir, &split);
    let table = zdc_types::check(&hir, &split).expect("the generated program typechecks");
    let cleared = verdict
        .clearance()
        .expect("the generated program is cleared");

    let options = zdc_codegen::Options::new("growth.zd", "bench");
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
        cleared,
    };
    least(budget, || {
        let bundle = zdc_codegen::compile(&inputs, &options).expect("the generated program emits");
        std::hint::black_box(&bundle);
    })
}

/// The bytes a source emits, so a sweep can show that it really did grow.
///
/// A shape gate divides one measurement by another, and division is exactly
/// the operation that hides a generator which stopped generating: two sizes
/// that compile the same program have a perfectly flat per-unit cost and
/// pass every threshold in this file. This is what the gate checks first.
pub fn emitted_bytes(source: &str) -> usize {
    try_compile(source, "growth.zd")
        .expect("the generated program builds")
        .client_js
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_estimator_takes_the_least_run_and_not_the_last() {
        let mut run = 0usize;
        let least = least(Duration::ZERO, || {
            run += 1;
            // The third run is the quick one, and three runs is the floor,
            // so a `least` that returned the last sample would return the
            // fourth — which never happens — and one that averaged would
            // return something larger than any single run's floor.
            if run != 3 {
                std::thread::sleep(Duration::from_millis(8));
            }
        });
        assert_eq!(run, 3, "a zero budget still buys the floor of three runs");
        assert!(
            least < Duration::from_millis(8),
            "the least of three runs, one of which did not sleep, was {least:?}"
        );
    }

    #[test]
    fn inflation_is_one_for_a_pass_that_is_linear_in_its_input() {
        let linear = Curve::measure("linear", &[8, 64, 512], |size| {
            Duration::from_micros(size as u64)
        });
        assert_eq!(linear.span(), 64.0);
        assert!((linear.inflation() - 1.0).abs() < 1e-9);

        // And the span for a quadratic one, which is the reading that makes
        // the gate's thresholds legible.
        let quadratic = Curve::measure("quadratic", &[8, 64, 512], |size| {
            Duration::from_micros((size * size) as u64)
        });
        assert!((quadratic.inflation() - 64.0).abs() < 1e-6);
    }
}
