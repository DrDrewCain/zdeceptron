//! Runs the JavaScript test suites against a minimal DOM.
//!
//! `reactivity.rs` covers the signal layer, which needs no document. These
//! cover the half a signal test cannot reach: keyed reconciliation, text
//! bindings updating in place, attribute effects, event handlers, and the
//! built-in elements. All of it runs under `cargo test` with no browser
//! and no JavaScript toolchain installed.
//!
//! **A suite per module, a context per suite.** `dom.test.js` tests
//! `dom.js` and `elements.test.js` tests `elements.js`, and they were one
//! file in one context until the element vocabulary grew. `boa` aborts the
//! *process* with a Rust-level `BorrowMutError` inside its own `Set`
//! builtin once a context's total allocation crosses a threshold — the
//! defect BENCHMARKS.md records as making signal fan-out unmeasurable here
//! — and the two together sat on it, deterministically, at a size the
//! vocabulary reached. The split is also the honest one: each suite now
//! names the module it is about, and `foreign.test.js` joined them on the
//! same terms.

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
  // A whole sequence in one assertion. `clock.test.js` checks what a
  // binding *saw over time*, and comparing that element by element would
  // report the first difference and hide the shape of the rest.
  deepEqual(actual, expected, note) {
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b) {
      throw new Error((note ? note + ': ' : '') + 'expected ' + b + ', got ' + a);
    }
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
fn run_suite(
    name: &str,
    suite: &str,
    modules: &[(&str, String)],
    floor: usize,
    mode: zdc_runtime::Mode,
) {
    let mut context = Context::default();
    let release = mode == zdc_runtime::Mode::Release;
    let mut sources = vec![
        ("harness", HARNESS.to_string()),
        ("mode", format!("const RELEASE = {release};\n")),
        (
            "dom shim",
            include_str!("../runtime/dom-shim.js").to_string(),
        ),
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
    renderer_suite(zdc_runtime::Mode::Development);
}

/// The same suite against the source a release build actually ships.
///
/// Stripping `// $dev` blocks (#140) edits the module that reaches a
/// reader's browser, so what it produces has to be *run* and not merely
/// diffed. An assertion whose removal took a closing brace with it would
/// still make the file shorter, and every size gate in the repository would
/// go on passing while the emitted program threw on load.
#[test]
fn the_release_renderer_still_passes_the_suite() {
    renderer_suite(zdc_runtime::Mode::Release);
}

fn renderer_suite(mode: zdc_runtime::Mode) {
    run_suite(
        "dom.test.js",
        include_str!("../runtime/dom.test.js"),
        &[
            (
                "signal.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::SIGNAL_JS, mode)),
            ),
            (
                "dom.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::DOM_JS, mode)),
            ),
            (
                "markup.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::MARKUP_JS, mode)),
            ),
            (
                "list.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::LIST_JS, mode)),
            ),
            (
                "branch.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::BRANCH_JS, mode)),
            ),
        ],
        38,
        mode,
    );
}

/// The scene rasteriser's geometry: `scene.js` with no DOM at all.
///
/// `signal.js` is the only module it needs, which is the same claim
/// `foreign.js`'s suite makes and for the same reason: the canvas is
/// handed in. Nothing in the suite reaches a backend — the three that
/// need a GL context, a canvas or a GPU adapter cannot run here, and the
/// arithmetic they share is what this is about.
#[test]
fn the_scene_geometry_suite_passes() {
    run_suite(
        "scene.test.js",
        include_str!("../runtime/scene.test.js"),
        &[
            ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
            ("scene.js", flatten(zdc_runtime::SCENE_JS)),
        ],
        16,
        zdc_runtime::Mode::Development,
    );
}

/// The FFI lifecycle: `foreign.js` against the shim.
///
/// `crates/zdc-codegen/tests/foreign_view.rs` already drives this module
/// through a compiled program, which is where create/update/destroy
/// ordering belongs. This suite is for the contract check (#239), whose
/// cases are a matrix of malformed imports — a class, a handle missing one
/// method — and a compiled program can only carry one import at a time.
///
/// `dom.js` is deliberately absent: `foreign.js` imports `signal.js` and
/// nothing else, because the node is handed in. A suite that needed the
/// renderer would mean that had stopped being true.
#[test]
fn the_foreign_lifecycle_suite_passes() {
    run_suite(
        "foreign.test.js",
        include_str!("../runtime/foreign.test.js"),
        &[
            ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
            ("foreign.js", flatten(zdc_runtime::FOREIGN_JS)),
        ],
        7,
        // The unstripped source, as this suite has always run it: the
        // lifecycle contract is the same in either build, and the cases
        // here are malformed imports rather than assertions.
        zdc_runtime::Mode::Development,
    );
}

/// What an emitted program does when a handler throws (#139).
///
/// Its own context because `dom.test.js` already sits at the allocation
/// threshold `boa` panics at, and its own suite because it is about one
/// decision. Run against both builds: the containment is not an assertion,
/// so stripping must not remove it.
#[test]
fn the_handler_failure_suite_passes() {
    handler_suite(zdc_runtime::Mode::Development);
}

#[test]
fn the_release_handler_failure_suite_passes() {
    handler_suite(zdc_runtime::Mode::Release);
}

fn handler_suite(mode: zdc_runtime::Mode) {
    run_suite(
        "handler.test.js",
        include_str!("../runtime/handler.test.js"),
        &[
            (
                "signal.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::SIGNAL_JS, mode)),
            ),
            (
                "dom.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::DOM_JS, mode)),
            ),
        ],
        4,
        mode,
    );
}

/// Document key listeners: `keys.js` against the shim.
///
/// Its own context for the reason `handler.test.js` has one, and its own
/// suite because it is about one decision: what a listener on the whole
/// document may observe, and when it stops observing it.
///
/// Run against both builds. Neither the focus rule nor the removal is an
/// assertion — they are the behaviour — so stripping `// $dev` must not
/// change either, and running only the development build would not say so.
#[test]
fn the_document_key_suite_passes() {
    keys_suite(zdc_runtime::Mode::Development);
}

#[test]
fn the_release_document_key_suite_passes() {
    keys_suite(zdc_runtime::Mode::Release);
}

fn keys_suite(mode: zdc_runtime::Mode) {
    run_suite(
        "keys.test.js",
        include_str!("../runtime/keys.test.js"),
        &[
            (
                "signal.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::SIGNAL_JS, mode)),
            ),
            (
                "keys.js",
                flatten(&zdc_runtime::for_mode(zdc_runtime::KEYS_JS, mode)),
            ),
        ],
        7,
        mode,
    );
}

/// Keyed list reconciliation: `list.js` against the shim.
///
/// Its own suite for the reason `foreign.test.js` and `elements.test.js`
/// are: the `BorrowMutError` above is a per-context allocation threshold,
/// and `dom.test.js` with a reconciler suite added to it sat on that
/// threshold deterministically. How many moves a minimal move set costs is
/// gated in `crates/zdc-bench`; what is here is that every shape those
/// counts are taken over still ends with the right list, which a count
/// cannot say.
#[test]
fn the_list_reconciler_suite_passes() {
    run_suite(
        "list.test.js",
        include_str!("../runtime/list.test.js"),
        &[
            ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
            ("dom.js", flatten(zdc_runtime::DOM_JS)),
            ("list.js", flatten(zdc_runtime::LIST_JS)),
        ],
        7,
        // The unstripped source, which is the point rather than an
        // omission: `assertPlaced` lives in `list.js` and is stripped from
        // a release build, so it is this suite — every reconciliation
        // shape there is — that runs the reconciler with its own
        // invariant checked.
        zdc_runtime::Mode::Development,
    );
}

/// The clock: `clock.js` against a scheduler the suite controls.
///
/// **The suite for the module that had none.** `media.js` was the one entry
/// in the mutation harness's `UNREACHED` list with no answer to give:
/// `browser_state.rs` asserts a bundle *links* it, which is a claim about
/// the emitter, and nothing anywhere evaluated a line of it.
///
/// `matchMedia` is faked for the reason `clock.test.js` fakes a scheduler.
/// The question worth asking is the one a real browser cannot be asked on
/// demand — does a reader who turns Reduce Motion on *while the page is
/// open* see it — and that is the whole reason this is a signal rather
/// than a `foreign` reading `.matches` once.
#[test]
fn the_media_suite_passes() {
    run_suite(
        "media.test.js",
        include_str!("../runtime/media.test.js"),
        &[
            ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
            ("media.js", flatten(zdc_runtime::MEDIA_JS)),
        ],
        7,
        // `media.js` carries no `// $dev` block, so either mode runs the
        // same bytes.
        zdc_runtime::Mode::Development,
    );
}

/// **The one suite here whose subject is what happens after a dispose.** A
/// timer that outlives its view is a leak with no symptom — nothing
/// renders wrongly, a callback simply keeps running — so it cannot be
/// found by looking at output, only by asking the scheduler what it is
/// still holding. `clock.test.js` therefore replaces `setInterval`,
/// `setTimeout` and `requestAnimationFrame` with a queue it can count,
/// which also makes every case instant rather than a real wait.
///
/// The DOM shim is loaded because `run_suite` loads it, and goes unused:
/// `clock.js` imports `signal.js` and touches no node.
#[test]
fn the_clock_suite_passes() {
    run_suite(
        "clock.test.js",
        include_str!("../runtime/clock.test.js"),
        &[
            ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
            ("clock.js", flatten(zdc_runtime::CLOCK_JS)),
        ],
        12,
        // Either mode runs the same bytes: `clock.js` carries no `// $dev`
        // block for a release build to strip, so the suite is testing one
        // source rather than two.
        zdc_runtime::Mode::Development,
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
        include_str!("../runtime/elements.test.js"),
        &[
            ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
            ("dom.js", flatten(zdc_runtime::DOM_JS)),
            ("markup.js", flatten(zdc_runtime::MARKUP_JS)),
            ("list.js", flatten(zdc_runtime::LIST_JS)),
            ("elements.js", flatten(zdc_runtime::ELEMENTS_JS)),
        ],
        8,
        zdc_runtime::Mode::Development,
    );
}
