//! Runs the renderer's JavaScript test suite against a minimal DOM.
//!
//! `reactivity.rs` covers the signal layer, which needs no document. This
//! covers the half a signal test cannot reach: keyed reconciliation, text
//! bindings updating in place, attribute effects, event handlers, and the
//! built-in elements. Both run under `cargo test` with no browser and no
//! JavaScript toolchain installed.

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
  ok(value, note) {
    if (!value) throw new Error(note || 'expected a truthy value');
  },
};
"#;

const REPORT: &str = r#"
__results.map(r => (r.ok ? 'PASS ' : 'FAIL ') + r.name + (r.ok ? '' : ' :: ' + r.message)).join('\n')
"#;

/// Remove ES module syntax so modules can be evaluated as one script.
///
/// The runtime's modules import from each other; flattening them into a
/// single scope is what lets the exact shipped source run here without a
/// module loader or a bundler in the test path.
fn flatten(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_javascript_renderer_suite_passes() {
    let shim = include_str!("dom-shim.js");
    let suite = include_str!("../../../runtime/dom.test.js");

    let mut context = Context::default();
    for (what, source) in [
        ("harness", HARNESS.to_string()),
        ("dom shim", shim.to_string()),
        ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
        ("dom.js", flatten(zdc_runtime::DOM_JS)),
        ("elements.js", flatten(zdc_runtime::ELEMENTS_JS)),
        ("dom.test.js", flatten(suite)),
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
        "{} of {} renderer tests failed:\n{}",
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
        lines.len() >= 35,
        "expected at least 35 renderer tests, found {}",
        lines.len()
    );
}
