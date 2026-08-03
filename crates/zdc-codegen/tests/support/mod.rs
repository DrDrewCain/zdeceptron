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
    // Asked here rather than left to `compile`'s own check, because
    // there is no longer a way to build an `Inputs` without asking.
    let Some(cleared) = verdict.clearance() else {
        return Err(vec![zdc_codegen::CodegenError {
            message:
                "The information-flow pass rejected this program, so there is nothing to emit."
                    .to_string(),
            span: zdc_lexer::Span::new(0, 0),
        }]);
    };
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
        cleared,
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
