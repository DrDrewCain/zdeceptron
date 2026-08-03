//! Runs the wire format's JavaScript test suite inside the embedded engine.
//!
//! `wire.js` is the one module three separate things depend on — the
//! browser encoding a request, the adapter decoding it, and the live-sync
//! stream carrying the encoded form straight through — and it was the one
//! module with no tests. A format that fails silently is the failure this
//! file exists to catch, in both directions: a value that does not survive
//! the trip, and a payload that is accepted as something it is not.
//!
//! Same shape as `reactivity.rs` and `render.rs`: `cargo test` is the only
//! command needed, with no Node and no browser installed.

use boa_engine::{Context, Source};

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

const REPORT: &str = r#"
__results.map(r => (r.ok ? 'PASS ' : 'FAIL ') + r.name + (r.ok ? '' : ' :: ' + r.message)).join('\n')
"#;

/// Remove ES module syntax so the module evaluates as one script.
fn flatten(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_javascript_wire_format_suite_passes() {
    let suite = include_str!("../../../runtime/wire.test.js");
    let mut context = Context::default();

    for (what, source) in [
        ("harness", HARNESS.to_string()),
        ("wire.js", flatten(zdc_runtime::WIRE_JS)),
        ("wire.test.js", flatten(suite)),
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
    for line in &lines {
        println!("{line}");
    }

    let failures: Vec<&&str> = lines.iter().filter(|l| l.starts_with("FAIL")).collect();
    assert!(
        failures.is_empty(),
        "{} of {} wire format tests failed:\n{}",
        failures.len(),
        lines.len(),
        failures
            .iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // A suite that stops running its tests still reports zero failures.
    assert!(
        lines.len() >= 13,
        "expected at least 13 wire format tests, found {}",
        lines.len()
    );
}
