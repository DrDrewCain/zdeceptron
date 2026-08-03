// Each integration test binary compiles this module separately, so a
// helper only one of them uses is not dead code.
#![allow(dead_code)]

//! Shared plumbing: compile a `.zd` file, and run JavaScript against the
//! DOM shim inside the embedded engine.
//!
//! Everything here runs under `cargo test` with no browser and no
//! JavaScript toolchain installed, which is the point: the runtime and the
//! demo pages were verified in a real browser once, and this is how that
//! verification is inherited by generated output.

use boa_engine::{Context, Source};

use zdc_codegen::{Bundle, Options};

/// The minimal DOM `dom.js` and `elements.js` are exercised against. It
/// lives next to the runtime's own tests because that is what it was
/// written for; the emitter reuses it rather than growing a second one that
/// could disagree.
pub const DOM_SHIM: &str = include_str!("../../../zdc-runtime/tests/dom-shim.js");

pub fn repository_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Parse, resolve and emit one example.
pub fn compile_example(relative: &str) -> Bundle {
    compile_source_named(
        &std::fs::read_to_string(repository_path(relative))
            .unwrap_or_else(|e| panic!("reading {relative}: {e}")),
        relative,
    )
}

pub fn compile_source(source: &str) -> Bundle {
    compile_source_named(source, "test.zd")
}

pub fn compile_source_named(source: &str, path: &str) -> Bundle {
    match try_compile(source, path) {
        Ok(bundle) => bundle,
        Err(errors) => panic!(
            "{path} failed to compile:\n{}",
            errors
                .iter()
                .map(|e| format!("  {}", e.message))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

/// The same pipeline `zdc build` runs: parse, resolve, split, typecheck,
/// check information flow, emit.
///
/// None of the five is optional here for the same reason none is optional
/// there — §16.7's and §17.1.3's lists are what codegen reads, and a test
/// that skipped one would be exercising a compiler nobody can run.
pub fn try_compile(source: &str, path: &str) -> Result<Bundle, Vec<zdc_codegen::CodegenError>> {
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("{path}: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{path}: {}", errors[0].message));
    let options = Options::new(path, "test");
    // Emission reads all four (§17.1.3). The split and the flow pass are
    // run here rather than stubbed, so a test that emits is testing what
    // `zdc build` emits.
    let split = zdc_graph::split(&hir);
    let verdict = zdc_graph::ifc(&hir, &split);
    let table = zdc_types::check(&hir, &split).unwrap_or_default();
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
    };
    zdc_codegen::compile(&inputs, &options)
}

/// The compile diagnostics for a source that is expected to be refused.
pub fn refusals(source: &str) -> Vec<String> {
    match try_compile(source, "test.zd") {
        Ok(_) => panic!("expected this program to be refused:\n{source}"),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

/// Remove ES module syntax so modules can be evaluated as one script.
///
/// The runtime's modules import from each other and generated code imports
/// from both; flattening them into one scope is what lets the exact shipped
/// source run here without a module loader or a bundler in the test path.
pub fn flatten(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A JavaScript context with the DOM shim and the runtime already in it.
///
/// `elements` decides whether `elements.js` is present. Generated code never
/// imports it (§16.3.1); the hand-written demo pages do, and the parity test
/// needs both sides in the same context.
pub fn context(elements: bool) -> Context {
    let mut context = Context::default();
    let mut sources = vec![
        ("dom shim", DOM_SHIM.to_string()),
        ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
        ("dom.js", flatten(zdc_runtime::DOM_JS)),
    ];
    if elements {
        sources.push(("elements.js", flatten(zdc_runtime::ELEMENTS_JS)));
    }
    for (what, source) in sources {
        context
            .eval(Source::from_bytes(source.as_bytes()))
            .unwrap_or_else(|e| panic!("{what} failed to evaluate: {e}"));
    }
    context
}

/// Evaluate `module` and then `driver`, returning the driver's value.
pub fn run(context: &mut Context, module: &str, driver: &str) -> String {
    context
        .eval(Source::from_bytes(flatten(module).as_bytes()))
        .unwrap_or_else(|e| panic!("the module failed to evaluate: {e}\n\n{module}"));
    context
        .eval(Source::from_bytes(driver.as_bytes()))
        .unwrap_or_else(|e| panic!("the driver failed: {e}"))
        .to_string(context)
        .expect("the driver returns a string")
        .to_std_string_escaped()
}

/// A context with the runtime *and* `rpc.js` in it.
///
/// Separate from [`context`] because a client-only program never imports
/// `rpc.js` (§16.3.1), and a shared context would hide a bundle that
/// referenced `$call` without importing it.
pub fn rpc_context() -> Context {
    let mut context = context(false);
    context
        .eval(Source::from_bytes(flatten(zdc_runtime::RPC_JS).as_bytes()))
        .unwrap_or_else(|e| panic!("rpc.js failed to evaluate: {e}"));
    // Generated code renames on import — `call as $call` — and `flatten`
    // deletes the import line along with the rename. Binding the aliases
    // here is what the module loader would have done, and it keeps the
    // emitted bundle running unmodified.
    context
        .eval(Source::from_bytes(
            b"const $call = call, $remote = remote, $failed = reportFailure;",
        ))
        .expect("the rpc aliases bind");
    context
}

/// Evaluate a setup, a module and a driver, let every pending promise
/// settle, and then read the report.
///
/// `setup` runs **before** the module, and that ordering is load-bearing
/// rather than tidy. A `$remote` binding is emitted at module scope
/// (§16.3.4) and its effect runs on evaluation, so a transport installed
/// after the module has already missed the first call — the test would
/// then be asserting about whatever the default transport did, which in
/// this engine is "`fetch` is not defined".
///
/// The job queue is drained explicitly. A cross-region write is a promise,
/// so a test that read its result before draining would be asserting about
/// a handler that had only started — which is the exact failure mode
/// "the compiler emitted it" already hides once.
pub fn run_settled(
    context: &mut Context,
    setup: &str,
    module: &str,
    driver: &str,
    report: &str,
) -> String {
    context
        .eval(Source::from_bytes(setup.as_bytes()))
        .unwrap_or_else(|e| panic!("the setup failed: {e}"));
    context
        .eval(Source::from_bytes(flatten(module).as_bytes()))
        .unwrap_or_else(|e| panic!("the module failed to evaluate: {e}\n\n{module}"));
    context
        .eval(Source::from_bytes(driver.as_bytes()))
        .unwrap_or_else(|e| panic!("the driver failed: {e}"));
    context
        .run_jobs()
        .unwrap_or_else(|e| panic!("a pending job failed: {e}"));
    context
        .eval(Source::from_bytes(report.as_bytes()))
        .unwrap_or_else(|e| panic!("the report failed: {e}"))
        .to_string(context)
        .expect("the report returns a string")
        .to_std_string_escaped()
}

/// A context with the runtime, `rpc.js` and `store.js` in it.
///
/// `URLSearchParams` is part of ECMA-429 and of every browser, and is not
/// in this engine — so a shim stands in for it. Deliberately minimal and
/// deliberately here rather than in `store.js`: shipping a polyfill for a
/// standard API would be shipping bytes to every browser that already has
/// it, to make one test engine happy.
pub fn live_context() -> Context {
    let mut context = rpc_context();
    context
        .eval(Source::from_bytes(
            br#"
class URLSearchParams {
  constructor() { this._pairs = []; }
  set(key, value) { this._pairs.push([key, value]); }
  toString() {
    return this._pairs
      .map(([k, v]) => encodeURIComponent(k) + '=' + encodeURIComponent(v))
      .join('&');
  }
}
"#,
        ))
        .expect("the URLSearchParams shim evaluates");
    context
        .eval(Source::from_bytes(
            flatten(zdc_runtime::STORE_JS).as_bytes(),
        ))
        .unwrap_or_else(|e| panic!("store.js failed to evaluate: {e}"));
    context
        .eval(Source::from_bytes(
            b"const $durable = durable, $subscribe = subscribe;",
        ))
        .expect("the store aliases bind");
    context
}
