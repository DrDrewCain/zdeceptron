//! Runs the JavaScript test suites against a minimal DOM.
//!
//! `reactivity.rs` covers the signal layer, which needs no document. These
//! cover the half a signal test cannot reach: keyed reconciliation, text
//! bindings updating in place, attribute effects, event handlers, and the
//! built-in elements. All of it runs under `cargo test` with no browser
//! and no JavaScript toolchain installed.
//!
//! **Two suites, two contexts.** `dom.test.js` tests `dom.js` and
//! `elements.test.js` tests `elements.js`, and they were one file in one
//! context until the element vocabulary grew. `boa` aborts the *process*
//! with a Rust-level `BorrowMutError` inside its own `Set` builtin once a
//! context's total allocation crosses a threshold — the defect
//! BENCHMARKS.md records as making signal fan-out unmeasurable here — and
//! the two together sat on it, deterministically, at a size the vocabulary
//! reached. The split is also the honest one: each suite now names the
//! module it is about.

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

/// Evaluate one suite in a context of its own and assert every case passed.
fn run_suite(name: &str, suite: &str, modules: &[(&str, String)], floor: usize) {
    let mut context = Context::default();
    let mut sources = vec![
        ("harness", HARNESS.to_string()),
        ("dom shim", include_str!("dom-shim.js").to_string()),
    ];
    sources.extend(modules.iter().map(|(what, source)| (*what, source.clone())));
    sources.push((name, flatten(suite)));
    for (what, source) in sources {
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
        "{name} reported nothing, so it did not run"
    );
    for line in &lines {
        println!("{line}");
    }

    let failures: Vec<&&str> = lines.iter().filter(|l| l.starts_with("FAIL")).collect();
    assert!(
        failures.is_empty(),
        "{} of {} cases in {name} failed:\n{}",
        failures.len(),
        lines.len(),
        failures
            .iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // A suite that stops running its cases still reports zero failures.
    assert!(
        lines.len() >= floor,
        "expected at least {floor} cases in {name}, found {}",
        lines.len()
    );
}

/// The renderer: `dom.js` against the shim.
#[test]
fn the_javascript_renderer_suite_passes() {
    run_suite(
        "dom.test.js",
        include_str!("../../../runtime/dom.test.js"),
        &[
            ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
            ("dom.js", flatten(zdc_runtime::DOM_JS)),
            ("markup.js", flatten(zdc_runtime::MARKUP_JS)),
        ],
        35,
    );
}

/// The element library: `elements.js` against the shim.
///
/// `element_parity.rs` checks the *trees* this module builds against the
/// compiler's own shape table. What it cannot see is behaviour after
/// construction: a two-way binding writing back, a reactive class staying
/// reactive, a script URL being filtered. That is what is here.
#[test]
fn the_element_library_suite_passes() {
    run_suite(
        "elements.test.js",
        include_str!("../../../runtime/elements.test.js"),
        &[
            ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
            ("dom.js", flatten(zdc_runtime::DOM_JS)),
            ("markup.js", flatten(zdc_runtime::MARKUP_JS)),
            ("elements.js", flatten(zdc_runtime::ELEMENTS_JS)),
        ],
        6,
    );
}
