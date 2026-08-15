#![forbid(unsafe_code)]

//! The benchmark suite §14A.4 makes a deliverable.
//!
//! **What this measures.** Operation counts, not time. The workload runs in
//! a pure-Rust JavaScript interpreter embedded in a `cargo test`, and a
//! wall-clock number from there is not comparable to a browser — reporting
//! one as if it were would be dishonest. What *is* comparable is how many
//! times each arm crosses into the DOM, how many nodes it allocates, how
//! many effects it creates, and how many times those effects re-run. Those
//! counts are the same in this interpreter as in V8, because they are a
//! property of the emitted code rather than of the engine.
//!
//! **What it cannot measure.** React and SolidJS. §14A.4 asks for both, and
//! both need a package manager: CI has no network and §8 forbids a Node
//! dependency. The arms that stand in their place are a *direct-emission*
//! generator (the design §16.1 rejected) and two hand-written vanilla
//! implementations, one naive and one tuned. Nothing here should be read as
//! a measurement against React or Solid.
//!
//! **The gap.** `each` in the view is refused by this compiler (§16.5,
//! M5b), so the workload's list cannot be written in ZDeceptron today. The
//! row body in `js/benchmark.js` is the compiler's own emission for
//! `bench/row.zd` — `tests/fidelity.rs` proves it, and fails the build if
//! it drifts — but the `eachInto` around it is written by hand. See
//! `BENCHMARKS.md`.

use std::collections::BTreeMap;

use boa_engine::{Context, Source};

mod scaling;
mod shape;
mod sizes;
mod table;

pub use scaling::{
    build, code_lines, deepest_fold, linked_runtime_bytes, linked_runtime_bytes_in,
    linked_runtime_bytes_with_assertions, program_with_components, program_with_depth,
    program_with_roots, program_with_signals, program_without_components, runtime_js_bytes, survey,
    template_bytes, time_graph_passes, Emitted, GraphTimes, FOREIGN_VIEW_PROGRAM, NULL_PROGRAM,
    SMALLEST_PROGRAM, SWIFT_BYTES_PER_LINE, SWIFT_LARGEST_APP_JS, SWIFT_LARGEST_APP_LINES,
    SWIFT_NULL_PROGRAM_JS, SWIFT_NULL_PROGRAM_LINES,
};
pub use shape::{benchmark_row, emitted_row, RowShape};
pub use sizes::{
    bundle_sizes, compile, repository_path, runtime_sizes, try_compile, BundleSize, RuntimeSize,
};
pub use table::{generated_section, END_MARKER, START_MARKER};

/// Counting instrumentation layered over the runtime's DOM shim.
pub const INSTRUMENT_JS: &str = include_str!("../js/instrument.js");

/// The workload: five arms, ten operations, one DOM.
pub const BENCHMARK_JS: &str = include_str!("../js/benchmark.js");

/// Reordering, counted: two reconcilers, four shapes, three sizes.
///
/// Separate from the workload above because it answers a different
/// question. The workload asks what a list operation costs; this asks what
/// the cost is a *function of*, which is the only form a claim about a
/// reconciler's order of growth can take (§16.10, issue #207).
pub const REORDER_JS: &str = include_str!("../js/reorder.js");

/// The minimal DOM the runtime's own tests run against.
///
/// Re-exported from where it lives rather than copied. A second copy with
/// counters in it would drift, and the benchmark would then be measuring a
/// DOM nothing else in the repository runs against.
pub use zdc_runtime::DOM_SHIM_JS;

/// One js-framework-benchmark row, in ZDeceptron.
pub const ROW_ZD: &str = include_str!("../bench/row.zd");

/// One arm's counts for one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    pub arm: String,
    pub step: String,
    pub fields: BTreeMap<String, i64>,
}

impl Measurement {
    /// A counter's value, or zero — the report omits zeroes so a hundred
    /// columns of nothing do not drown the numbers that matter.
    pub fn get(&self, key: &str) -> i64 {
        self.fields.get(key).copied().unwrap_or(0)
    }
}

/// Every measurement, in the order the workload produced them.
pub struct Report(pub Vec<Measurement>);

impl Report {
    pub fn find(&self, arm: &str, step: &str) -> &Measurement {
        self.0
            .iter()
            .find(|m| m.arm == arm && m.step == step)
            .unwrap_or_else(|| panic!("no measurement for `{arm}` / `{step}`"))
    }

    pub fn arms(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for measurement in &self.0 {
            if !out.contains(&measurement.arm.as_str()) {
                out.push(&measurement.arm);
            }
        }
        out
    }

    pub fn steps(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for measurement in &self.0 {
            if !out.contains(&measurement.step.as_str()) {
                out.push(&measurement.step);
            }
        }
        out
    }
}

/// Remove ES module syntax so the shipped sources can be evaluated as one
/// script, exactly as the runtime's own tests do.
fn flatten(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Evaluate the runtime and the counters, then one measuring script.
fn measure(what: &str, script: &str) -> Report {
    let mut context = Context::default();
    let sources = [
        ("dom shim", DOM_SHIM_JS.to_string()),
        ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
        ("dom.js", flatten(zdc_runtime::DOM_JS)),
        ("markup.js", flatten(zdc_runtime::MARKUP_JS)),
        ("list.js", flatten(zdc_runtime::LIST_JS)),
        ("elements.js", flatten(zdc_runtime::ELEMENTS_JS)),
        ("instrument.js", INSTRUMENT_JS.to_string()),
    ];
    for (name, source) in sources {
        context
            .eval(Source::from_bytes(source.as_bytes()))
            .unwrap_or_else(|e| panic!("{name} failed to evaluate: {e}"));
    }

    let report = context
        .eval(Source::from_bytes(script.as_bytes()))
        .unwrap_or_else(|e| panic!("{what} failed: {e}"))
        .to_string(&mut context)
        .expect("a measuring script returns a string")
        .to_std_string_escaped();

    Report(parse(&report))
}

/// Run the workload and collect every arm's counts.
pub fn run() -> Report {
    measure("the workload", BENCHMARK_JS)
}

/// Count the moves a reorder costs, at three sizes and in four shapes.
///
/// Its own context rather than a further arm of [`run`]: the workload's
/// arms all render the same row and are compared against each other, and
/// an arm that reordered a different list at a different size would not be
/// comparable to any of them.
pub fn run_reorder() -> Report {
    measure("the reorder measurement", REORDER_JS)
}

fn parse(report: &str) -> Vec<Measurement> {
    let mut out = Vec::new();
    for line in report.lines() {
        let Some(rest) = line.strip_prefix("RESULT\t") else {
            continue;
        };
        let mut parts = rest.split('\t');
        let arm = parts.next().expect("a result line names its arm");
        let step = parts.next().expect("a result line names its step");
        let fields = parts.next().expect("a result line carries fields");

        let mut counts = BTreeMap::new();
        for field in fields.split(',') {
            let (key, value) = field
                .split_once('=')
                .unwrap_or_else(|| panic!("malformed field `{field}`"));
            let value: i64 = value
                .parse()
                .unwrap_or_else(|e| panic!("`{field}` is not a number: {e}"));
            counts.insert(key.to_string(), value);
        }
        out.push(Measurement {
            arm: arm.to_string(),
            step: step.to_string(),
            fields: counts,
        });
    }
    assert!(
        !out.is_empty(),
        "the workload produced no measurements — it probably did not run"
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_line_parses_into_counters() {
        let parsed = parse("RESULT\tzd\tcreate\trows=2,cross.cloneNode=2\nnoise\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].arm, "zd");
        assert_eq!(parsed[0].step, "create");
        assert_eq!(parsed[0].get("cross.cloneNode"), 2);
        assert_eq!(parsed[0].get("absent"), 0);
    }

    #[test]
    fn flatten_strips_module_syntax_and_nothing_else() {
        let flattened = flatten("import { a } from './b.js';\nexport function c() {}\n  indented");
        assert_eq!(flattened, "function c() {}\n  indented");
    }
}
