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

use std::collections::BTreeMap;

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

/// The same pipeline `zdc build` runs: parse, resolve against the prelude,
/// split, typecheck, check information flow, emit.
///
/// None of the six is optional here for the same reason none is optional
/// there — §16.7's and §17.1.3's lists are what codegen reads, and §17.4.1
/// makes the library part of the compilation unit, so a test that skipped
/// one would be exercising a compiler nobody can run.
pub fn try_compile(source: &str, path: &str) -> Result<Bundle, Vec<zdc_codegen::CodegenError>> {
    try_compile_with_statics(source, path, BTreeMap::new())
}

/// The same pipeline, with the build host's answers supplied by hand.
///
/// §17.4.8 runs the build root under a JavaScript runtime, and these tests
/// deliberately install none — so the values it would have printed are
/// passed in, and [`build_module_of`] checks separately that the module
/// which produces them says what it should.
pub fn try_compile_with_statics(
    source: &str,
    path: &str,
    statics: BTreeMap<String, String>,
) -> Result<Bundle, Vec<zdc_codegen::CodegenError>> {
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("{path}: {}", e.message));
    let hir = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{path}: {}", errors[0].message));
    let options = Options::new(path, "test").with_statics(statics);
    // Emission reads all four (§17.1.3). The split and the flow pass are
    // run here rather than stubbed, so a test that emits is testing what
    // `zdc build` emits.
    let split = zdc_graph::split(&hir);
    // The split reports first, for the same reason `zdc build` lets it: a
    // program whose placements do not resolve has no settled read table,
    // so every answer after the first would be invented (§17.1.3).
    let rejected: Vec<zdc_codegen::CodegenError> = split
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| zdc_codegen::CodegenError {
            message: d.message.clone(),
            span: d.span,
        })
        .collect();
    if !rejected.is_empty() {
        return Err(rejected);
    }
    // Both report, as `zdc build` does. A program that renders a secret
    // *and* has a type error should say so about the leak too: the leak is
    // the more interesting of the two, and the type error would otherwise
    // hide it.
    let verdict = zdc_graph::ifc(&hir, &split);
    // The split's and the flow pass's own diagnostics, rather than
    // codegen's one-line "there is nothing to emit". Both are refusals a
    // programmer sees from `zdc check`, and a test that read only the
    // summary could not tell a rejected `secret` from a rejected
    // placement.
    let refused: Vec<zdc_codegen::CodegenError> = split
        .diagnostics
        .iter()
        .chain(verdict.diagnostics.iter())
        .filter(|d| d.is_error())
        .map(|d| zdc_codegen::CodegenError {
            message: d.message.clone(),
            span: d.span,
        })
        .collect();
    if !refused.is_empty() {
        return Err(refused);
    }
    // A type, routing or integrity error is a refusal, not a broken
    // harness: all three are things `zdc build` reports and stops on, so
    // they reach the caller in the shape codegen's own refusals do.
    // Swallowing them here would emit a bundle from an empty type table
    // and assert about whatever fell out.
    let table = match zdc_types::check(&hir, &split) {
        Ok(table) => table,
        Err(errors) => {
            return Err(errors
                .into_iter()
                .map(|error| zdc_codegen::CodegenError {
                    message: error.message,
                    span: error.span,
                })
                .collect())
        }
    };
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
    };
    zdc_codegen::compile(&inputs, &options)
}

/// One example on disk, compiled the way `zdc build` compiles it: the
/// files it imports linked in, and its build root actually run.
///
/// [`compile_example`] cannot do either. It resolves a single source, so a
/// program with a `use` clause loses the module it names, and it takes the
/// build host's answers by hand rather than computing them. `blog.zd` needs
/// both — it imports `layout.zd`, and its posts are three markdown files in
/// `examples/content/blog/` that only a real build reads.
///
/// The build runs against the example's own directory, which is what makes
/// the sandbox's rule (`examples/` and nothing above it) the same rule the
/// compiler applies to a developer's project.
pub fn build_example(relative: &str) -> Bundle {
    let path = repository_path(relative);
    let linked = zdc_resolve::load(&path)
        .unwrap_or_else(|errors| panic!("{relative} does not link: {}", errors[0].message));
    let prelude = zdc_lib::load();
    let hir = zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked)
        .resolve()
        .unwrap_or_else(|errors| panic!("{relative} does not resolve: {}", errors[0].message));

    let split = zdc_graph::split(&hir);
    if let Some(error) = split.diagnostics.iter().find(|d| d.is_error()) {
        panic!("{relative} was rejected by the split: {}", error.message);
    }
    let verdict = zdc_graph::ifc(&hir, &split);
    if let Some(error) = verdict.diagnostics.iter().find(|d| d.is_error()) {
        panic!(
            "{relative} was rejected by the flow pass: {}",
            error.message
        );
    }
    let table = zdc_types::check(&hir, &split)
        .unwrap_or_else(|errors| panic!("{relative} does not typecheck: {}", errors[0].message));
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
    };

    let options = Options::new(relative, "test");
    let module = zdc_codegen::build_module(&inputs, &options)
        .unwrap_or_else(|errors| panic!("{relative}'s build root: {}", errors[0].message));
    let statics = match module {
        None => BTreeMap::new(),
        Some(module) => {
            let directory = path.parent().expect("an example has a directory");
            zdc_codegen::evaluate(&module, directory)
                .unwrap_or_else(|error| {
                    panic!("{relative}'s build did not run: {}", error.report())
                })
                .values
        }
    };
    zdc_codegen::compile(&inputs, &options.with_statics(statics))
        .unwrap_or_else(|errors| panic!("{relative} does not emit: {}", errors[0].message))
}

/// The `BUILD` root for a source, or `None` if it declares no `static`
/// state (§17.4.8).
pub fn build_module_of(source: &str, path: &str) -> Option<zdc_codegen::BuildModule> {
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("{path}: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{path}: {}", errors[0].message));
    let options = Options::new(path, "test");
    let split = zdc_graph::split(&hir);
    let verdict = zdc_graph::ifc(&hir, &split);
    let table = zdc_types::check(&hir, &split).unwrap_or_default();
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
    };
    zdc_codegen::build_module(&inputs, &options).expect("the build root must print")
}

/// The name-resolution diagnostics for a source expected to be refused
/// before codegen ever sees it.
///
/// `refusals` cannot be used for these: it panics on a resolve error,
/// because for every other case reaching codegen is the point.
pub fn resolve_refusals(source: &str) -> Vec<String> {
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("test.zd: {}", e.message));
    match zdc_resolve::Resolver::new(&program).resolve() {
        Ok(_) => panic!("expected this program to be refused:\n{source}"),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

/// The type-checking diagnostics for a source expected to be refused
/// before codegen sees it.
pub fn check_refusals(source: &str) -> Vec<String> {
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("test.zd: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| panic!("test.zd: {}", errors[0].message));
    let split = zdc_graph::split(&hir);
    match zdc_types::check(&hir, &split) {
        Ok(_) => panic!("expected this program to be refused:\n{source}"),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

/// Codegen's **own** refusals, with the checker's verdict set aside.
///
/// A handful of guarantees belong to emission rather than to inference —
/// `is` on a shape the runtime cannot compare by value is the example —
/// and the checker refuses those programs too. Running the strict path
/// would report the type error and stop, so the emission guarantee would
/// have no test at all. This reaches codegen with an empty table, which is
/// the state the guarantee is *about*: a verdict codegen did not get.
pub fn codegen_refusals(source: &str) -> Vec<String> {
    let path = "test.zd";
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("{path}: {}", e.message));
    let hir = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{path}: {}", errors[0].message));
    let options = Options::new(path, "test");
    let split = zdc_graph::split(&hir);
    let verdict = zdc_graph::ifc(&hir, &split);
    let table = zdc_types::check(&hir, &split).unwrap_or_default();
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
    };
    match zdc_codegen::compile(&inputs, &options) {
        Ok(_) => panic!("expected codegen to refuse this program:\n{source}"),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
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
        .eval(Source::from_bytes(flatten(zdc_runtime::WIRE_JS).as_bytes()))
        .unwrap_or_else(|e| panic!("wire.js failed to evaluate: {e}"));
    context
        .eval(Source::from_bytes(flatten(zdc_runtime::RPC_JS).as_bytes()))
        .unwrap_or_else(|e| panic!("rpc.js failed to evaluate: {e}"));
    // Generated code renames on import — `call as $call` — and `flatten`
    // deletes the import line along with the rename. Binding the aliases
    // here is what the module loader would have done, and it keeps the
    // emitted bundle running unmodified.
    context
        .eval(Source::from_bytes(
            b"const $call = call, $atomic = atomic, $remote = remote, $failed = reportFailure;",
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
    // `store.js` renames on import — `decode as decodeValue` — and
    // `flatten` deletes the import line along with the rename. The alias
    // has to be bound *before* the module that closes over it: a `const`
    // in a later script is not in scope for a function declared in an
    // earlier one, which is an ordering a real module loader gets right
    // for free.
    context
        .eval(Source::from_bytes(b"globalThis.decodeValue = decode;"))
        .expect("the wire alias binds");
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

/// The page a bundle contains, for tests that are about the page.
///
/// `Bundle::index_html` is `None` for a module with no `view` (§16.3.1),
/// and every caller here compiles a program that has one — so a `None`
/// means the test's own program was wrong, not that the page is optional.
pub fn page(bundle: &Bundle) -> &str {
    bundle
        .index_html
        .as_deref()
        .expect("this program has a `view`, so it has a page")
}
