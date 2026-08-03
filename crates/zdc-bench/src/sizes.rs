//! Bundle size, which §14A.4 also makes a deliverable.
//!
//! Bytes as shipped, uncompressed and unminified — there is no minifier in
//! the pipeline, so a minified figure would be a claim about a tool that
//! does not exist. Every arm's runtime cost is listed alongside, because a
//! bundle that is small only by leaving the runtime out is not small.

use zdc_codegen::{Bundle, Options};

/// One compiled example, in bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleSize {
    pub name: String,
    pub client_js: usize,
    pub styles_css: usize,
    pub index_html: usize,
    pub manifest_json: usize,
}

impl BundleSize {
    /// Everything a build writes, excluding the runtime, which is shared.
    pub fn total(&self) -> usize {
        self.client_js + self.styles_css + self.index_html + self.manifest_json
    }
}

pub fn repository_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Parse, resolve, typecheck and emit a source, keeping the diagnostics.
///
/// A refusal is a result here rather than a failure: which constructs the
/// compiler still refuses is exactly what the benchmark's documented gap is
/// made of, and a test pins it. Type errors join emission refusals in that
/// result for the same reason — a benchmark arm the language cannot yet
/// express should report why, not crash.
pub fn try_compile(source: &str, name: &str) -> Result<Bundle, Vec<String>> {
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("{name}: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{name}: {}", errors[0].message));
    let types = zdc_types::check(&hir)
        .map_err(|errors| errors.into_iter().map(|e| e.message).collect::<Vec<_>>())?;
    let options = Options::new(name, "bench");
    zdc_codegen::compile(&hir, &types, &options)
        .map_err(|errors| errors.into_iter().map(|e| e.message).collect())
}

/// Compile a file in the repository, or fail with every diagnostic.
pub fn compile(relative: &str) -> Bundle {
    let source = std::fs::read_to_string(repository_path(relative))
        .unwrap_or_else(|e| panic!("reading {relative}: {e}"));
    try_compile(&source, relative).unwrap_or_else(|errors| {
        panic!(
            "{relative} failed to compile:\n{}",
            errors
                .iter()
                .map(|message| format!("  {message}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

/// Every example this compiler can build today, sized.
pub fn bundle_sizes() -> Vec<BundleSize> {
    [
        "examples/hello.zd",
        "examples/counter.zd",
        "crates/zdc-bench/bench/row.zd",
    ]
    .into_iter()
    .map(|relative| {
        let bundle = compile(relative);
        BundleSize {
            name: relative.to_string(),
            client_js: bundle.client_js.len(),
            styles_css: bundle.styles_css.len(),
            index_html: bundle.index_html.len(),
            manifest_json: bundle.manifest_json.len(),
        }
    })
    .collect()
}

/// The runtime files a bundle links against, in bytes.
///
/// `elements.js` is listed separately because generated code never imports
/// it (§16.3.1) — it is what the direct-emission arm would have shipped.
pub fn runtime_sizes() -> Vec<(&'static str, usize)> {
    vec![
        ("runtime/signal.js", zdc_runtime::SIGNAL_JS.len()),
        ("runtime/dom.js", zdc_runtime::DOM_JS.len()),
        ("runtime/base.css", zdc_runtime::BASE_CSS.len()),
        (
            "runtime/elements.js (direct emission only)",
            zdc_runtime::ELEMENTS_JS.len(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_example_compiles_and_is_not_empty() {
        for size in bundle_sizes() {
            assert!(size.client_js > 0, "{} emitted nothing", size.name);
            assert!(size.total() > size.client_js);
        }
    }
}
