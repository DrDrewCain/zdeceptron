// Each integration test binary compiles this module separately, so a
// helper only one of them uses is not dead code.
#![allow(dead_code)]

//! Compile an example the way `zdc deploy` does, and generate a deployment
//! from it.

use zdc_codegen::Bundle;
use zdc_deploy::{Deployment, File, Options, Program, Target};

pub fn repository_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// The same pipeline `zdc deploy` runs, up to the bundle.
pub fn compile_example(relative: &str) -> Bundle {
    let path = repository_path(relative);
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {relative}: {e}"));
    let program =
        zdc_parser::parse(&source).unwrap_or_else(|e| panic!("{relative}: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{relative}: {}", errors[0].message));
    let split = zdc_graph::split(&hir);
    let verdict = zdc_graph::ifc(&hir, &split);
    let table = zdc_types::check(&hir, &split).unwrap_or_default();
    let options = zdc_codegen::Options::new(relative, "test");
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
    };
    zdc_codegen::compile(&inputs, &options)
        .unwrap_or_else(|errors| panic!("{relative}: {}", errors[0].message))
}

pub fn program(bundle: &Bundle) -> Program<'_> {
    Program {
        functions: &bundle.functions,
        durable: &bundle.durable,
        environment: &bundle.environment,
    }
}

/// A deployment of `relative` for `target`, with default options.
pub fn deploy(relative: &str, target: Target) -> (Bundle, Deployment) {
    let bundle = compile_example(relative);
    let deployment = {
        let program = program(&bundle);
        zdc_deploy::generate(&program, &Options::new(target, "test-app"))
            .unwrap_or_else(|refusal| panic!("{target:?} refused: {}", refusal.message))
    };
    (bundle, deployment)
}

/// One generated file, by path.
pub fn file<'a>(deployment: &'a Deployment, path: &str) -> &'a File {
    deployment
        .files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| {
            let paths: Vec<&str> = deployment.files.iter().map(|f| f.path.as_str()).collect();
            panic!("no generated file at {path}; there are {paths:?}")
        })
}

/// Whether the deployment has a file at `path`.
pub fn has(deployment: &Deployment, path: &str) -> bool {
    deployment.files.iter().any(|file| file.path == path)
}
