//! Runs the runtime's own JavaScript test suite inside the embedded engine.
//!
//! The tests are written in JavaScript and live beside the code they cover
//! (`runtime/signal.test.js`), because reactivity assertions read far more
//! clearly in the language the code is written in. They execute here so
//! that `cargo test` is the only command needed — no Node, no npm, no
//! browser. Requiring a JavaScript toolchain to verify ZDeceptron would be
//! the first crack in the claim that a developer installs one binary
//! (spec §7).

use boa_engine::{Context, Source};

/// A minimal `test`/`assert` shim.
///
/// Deliberately tiny: a test harness with its own bugs is worse than none,
/// and everything here has to be obviously correct by reading it. Failures
/// accumulate rather than throwing, so one run reports every broken test
/// instead of only the first — the same rule the compiler's own
/// diagnostics follow.
const HARNESS: &str = r#"
const __results = [];
function test(name, fn) {
  try {
    fn();
    __results.push({ name, ok: true, message: '' });
  } catch (e) {
    __results.push({ name, ok: false, message: String(e && e.message ? e.message : e) });
  }
}
const assert = {
  equal(actual, expected, note) {
    if (!Object.is(actual, expected)) {
      throw new Error(
        (note ? note + ': ' : '') + 'expected ' + JSON.stringify(expected) +
        ', got ' + JSON.stringify(actual)
      );
    }
  },
  deepEqual(actual, expected, note) {
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b) {
      throw new Error((note ? note + ': ' : '') + 'expected ' + b + ', got ' + a);
    }
  },
  ok(value, note) {
    if (!value) throw new Error(note || 'expected a truthy value');
  },
};
"#;

/// Format the collected results as one line per test for the Rust side.
const REPORT: &str = r#"
__results.map(r => (r.ok ? 'PASS ' : 'FAIL ') + r.name + (r.ok ? '' : ' :: ' + r.message)).join('\n')
"#;

fn strip_exports(source: &str) -> String {
    source
        .lines()
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_javascript_reactivity_suite_passes() {
    let suite = include_str!("../runtime/signal.test.js");
    let mut context = Context::default();

    for (what, source) in [
        ("harness", HARNESS.to_string()),
        ("signal.js", strip_exports(zdc_runtime::SIGNAL_JS)),
        ("signal.test.js", strip_exports(suite)),
    ] {
        context
            .eval(Source::from_bytes(source.as_bytes()))
            .unwrap_or_else(|e| panic!("{what} failed to evaluate: {e}"));
    }

    let report = context
        .eval(Source::from_bytes(REPORT.as_bytes()))
        .expect("collecting results")
        .to_string(&mut context)
        .expect("report is a string")
        .to_std_string_escaped();

    let lines: Vec<&str> = report.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "the suite reported nothing — it probably did not run"
    );

    let failures: Vec<&&str> = lines.iter().filter(|l| l.starts_with("FAIL")).collect();
    for line in &lines {
        println!("{line}");
    }
    assert!(
        failures.is_empty(),
        "{} of {} reactivity tests failed:\n{}",
        failures.len(),
        lines.len(),
        failures
            .iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Guards against the suite silently shrinking — a file that stops
    // running its tests still reports zero failures.
    assert!(
        lines.len() >= 12,
        "expected at least 12 reactivity tests, found {}",
        lines.len()
    );
}
