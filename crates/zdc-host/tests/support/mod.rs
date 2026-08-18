// Each integration test binary compiles this module separately, so a
// helper only one of them uses is not dead code.
#![allow(dead_code)]

//! Compile a `.zd` file the way `zdc build` does, and hand the emitted
//! server files to a host that runs them.
//!
//! Nothing here shortcuts the compiler. The bytes the host executes are
//! the bytes `zdc build` writes into `dist/functions/`, because a test
//! that ran a hand-written equivalent would leave the emitted file exactly
//! as unexecuted as it was before this crate existed.

use std::sync::Arc;

use zdc_host::{Endpoint, Endpoints, Environment, Host, Shape};
use zdc_store::{DurableStore, EmbeddedStore};

pub fn repository_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// The full front end plus emission, exactly as `zdc build` runs it.
pub fn emit(source: &str, path: &str) -> Vec<zdc_codegen::ServerFunction> {
    bundle(source, path).functions
}

/// The whole bundle rather than only its server half.
///
/// One test needs both ends of the same compilation: the browser computes
/// a value and encodes it, and the endpoint stores what arrived. Emitting
/// twice would let the two halves be of different programs.
pub fn bundle(source: &str, path: &str) -> zdc_codegen::Bundle {
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("{path}: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{path}: {}", errors[0].message));
    let split = zdc_graph::split(&hir);
    assert!(
        !split.has_errors(),
        "{path} did not survive the split: {:?}",
        split
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );
    let verdict = zdc_graph::ifc(&hir, &split);
    let table = zdc_types::check(&hir, &split).unwrap_or_else(|errors| {
        panic!("{path} did not typecheck: {}", errors[0].message);
    });
    let cleared = verdict
        .clearance()
        .unwrap_or_else(|| panic!("{path} was refused by the information-flow pass"));
    zdc_codegen::compile(
        &zdc_codegen::Inputs {
            hir: &hir,
            split: &split,
            verdict: &verdict,
            table: &table,
            cleared,
        },
        &zdc_codegen::Options::new(path, "test"),
    )
    .unwrap_or_else(|errors| panic!("{path} did not emit: {}", errors[0].message))
}

pub fn emit_example(relative: &str) -> Vec<zdc_codegen::ServerFunction> {
    let source = std::fs::read_to_string(repository_path(relative))
        .unwrap_or_else(|e| panic!("reading {relative}: {e}"));
    emit(&source, relative)
}

/// Turn what the emitter produced into what the host runs.
///
/// The `kind` the emitter recorded becomes the `Shape` the host dispatches
/// on, with no guess in between — that translation being wrong is the bug
/// this mapping exists to make impossible.
pub fn endpoints(functions: Vec<zdc_codegen::ServerFunction>) -> Endpoints {
    functions
        .into_iter()
        // A scheduled job is not an endpoint and must not become one here
        // either: the host dispatches by name over whatever this returns,
        // so admitting a trigger would give a test a way to start a job
        // that no deployment exposes (§14G.4).
        .filter(|function| !matches!(function.kind, zdc_codegen::FunctionKind::Trigger(_)))
        .map(|function| Endpoint {
            name: function.name,
            shape: match function.kind {
                zdc_codegen::FunctionKind::Value => Shape::Value,
                zdc_codegen::FunctionKind::Command => Shape::Command,
                zdc_codegen::FunctionKind::Trigger(_) => {
                    unreachable!("the filter above removed the triggers")
                }
            },
            inputs: function.inputs,
            source: function.source,
        })
        .collect()
}

/// A host over a fresh in-memory store.
pub fn host(source: &str, env: Environment) -> Host {
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    Host::new(endpoints(emit(source, "test.zd")), store, env)
}

/// A host over a store the caller already has — the two-window case, where
/// two sessions must share one store or the test proves nothing.
pub fn host_on(source: &str, store: Arc<dyn DurableStore>, env: Environment) -> Host {
    Host::new(endpoints(emit(source, "test.zd")), store, env)
}
