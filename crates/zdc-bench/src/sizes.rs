//! Bundle size, which §14A.4 also makes a deliverable.
//!
//! Bytes as shipped, uncompressed. **Both sides of minification are
//! reported** — issue #135 asked the size gate to record the before and
//! the after, and it does, per file: the emitted column is what the
//! compiler printed, and the minified column is what a browser downloads.
//! Keeping both is what makes the saving a measurement rather than a
//! claim, and it is also what keeps the emitter honest, since a change
//! that grows the emission is still visible after the minifier has been
//! over it.
//!
//! Every arm's runtime cost is listed alongside, because a bundle that is
//! small only by leaving the runtime out is not small.

use zdc_codegen::{minify, Bundle, Options};

/// One compiled example, in bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleSize {
    pub name: String,
    pub client_js: usize,
    pub boot_js: usize,
    pub styles_css: usize,
    pub index_html: usize,
    pub manifest_json: usize,
    /// The whole bundle after minification — what a reader downloads.
    ///
    /// `index.html` and `manifest.json` are counted at their emitted
    /// length here, because neither is minified and pretending otherwise
    /// would report a saving nobody made. See `minify`'s module
    /// documentation for why those two are left alone.
    pub minified: usize,
}

impl BundleSize {
    /// Everything a build writes, excluding the runtime, which is shared.
    pub fn total(&self) -> usize {
        self.client_js + self.boot_js + self.styles_css + self.index_html + self.manifest_json
    }

    /// What minification took off this bundle.
    pub fn saved(&self) -> usize {
        self.total().saturating_sub(self.minified)
    }
}

pub fn repository_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Run the whole pipeline over a source, keeping the diagnostics.
///
/// The five passes are §17.1.2's, in §17.1.2's order, because the emitted
/// sizes are only the compiler's sizes if the pipeline is the compiler's.
///
/// A refusal is a result here rather than a failure: which constructs the
/// compiler still refuses is exactly what the benchmark's documented gap is
/// made of, and a test pins it. Placement, type and flow errors join
/// emission refusals in that result for the same reason — a benchmark arm
/// the language cannot yet express should report why, not crash. Parse and
/// resolve errors join them too, so that a survey over every example in the
/// repository can report which ones do not build instead of aborting on the
/// first (§14A.4).
pub fn try_compile(source: &str, name: &str) -> Result<Bundle, Vec<String>> {
    let program = zdc_parser::parse(source).map_err(|e| vec![e.message])?;
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .map_err(|errors| errors.into_iter().map(|e| e.message).collect::<Vec<_>>())?;

    let split = zdc_graph::split(&hir);
    if split.has_errors() {
        return Err(split.errors().map(|error| error.message.clone()).collect());
    }
    let verdict = zdc_graph::ifc(&hir, &split);
    let table = zdc_types::check(&hir, &split)
        .map_err(|errors| errors.into_iter().map(|e| e.message).collect::<Vec<_>>())?;

    // The flow pass's own permission to emit. There is no other way to
    // build an `Inputs`, so forgetting to ask is a compile error.
    let Some(cleared) = verdict.clearance() else {
        return Err(verdict
            .errors()
            .map(|error| error.message.clone())
            .collect());
    };

    let options = Options::new(name, "bench");
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
        cleared,
    };
    zdc_codegen::compile(&inputs, &options)
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
        let emitted = BundleSize {
            name: relative.to_string(),
            client_js: bundle.client_js.len(),
            // The two lines that used to be an inline `<script>` (#146).
            // Counted, because "everything a build writes" is what the
            // total above claims to be.
            boot_js: bundle.boot_js.as_ref().map_or(0, String::len),
            styles_css: bundle.styles_css.len(),
            // Every program sized here has a `view`; a module with none
            // ships no page, and zero is the honest number for it.
            index_html: bundle.index_html.as_ref().map_or(0, String::len),
            manifest_json: bundle.manifest_json.len(),
            minified: 0,
        };
        // The bundle `zdc build` writes, from the same call that command
        // makes — measuring a re-implementation of it here is how a
        // benchmark comes to report a number no user can obtain (#135).
        let shipped = bundle.minified();
        BundleSize {
            minified: shipped.client_js.len()
                + shipped.boot_js.as_ref().map_or(0, String::len)
                + shipped.styles_css.len()
                + shipped.index_html.as_ref().map_or(0, String::len)
                + shipped.manifest_json.len(),
            ..emitted
        }
    })
    .collect()
}

/// The runtime files a bundle links against, in bytes.
///
/// `elements.js` is listed separately because generated code never imports
/// it (§16.3.1) — it is what the direct-emission arm would have shipped.
///
/// `foreign.js` is annotated for the same kind of reason in the opposite
/// direction: it *is* shipped, but only to a program that writes a
/// `foreign … gives view`, so adding its bytes to the two above would
/// overstate what an ordinary page downloads. Which files a given bundle
/// links is `Bundle::runtime`, and the per-program table above is what
/// reports it.
pub fn runtime_sizes() -> Vec<RuntimeSize> {
    // As a release build ships them, and as they are written. Two
    // numbers, because two separate things stand between the file and the
    // reader: since #140 the `// $dev` assertions are in the file and not
    // in the bundle, and since #135 neither are the comments or the
    // indentation. Reporting only the file would name a cost nobody pays;
    // reporting only the bundle would hide how much of the runtime is
    // prose, which is a fact about this codebase worth being able to see.
    let shipped = |source| zdc_runtime::for_mode(source, zdc_runtime::Mode::Release).len();
    let size = |name: &'static str, source| RuntimeSize {
        name,
        shipped: shipped(source),
        source: str::len(source),
    };
    vec![
        size("runtime/signal.js", zdc_runtime::SIGNAL_JS),
        size("runtime/dom.js", zdc_runtime::DOM_JS),
        // No backticks inside the label: the table wraps every name in a
        // code span, and a nested pair closes it early.
        size(
            "runtime/foreign.js (a gives-view foreign only)",
            zdc_runtime::FOREIGN_JS,
        ),
        size(
            "runtime/markup.js (a program with Prose only)",
            zdc_runtime::MARKUP_JS,
        ),
        size(
            "runtime/list.js (a program with an each only)",
            zdc_runtime::LIST_JS,
        ),
        size(
            "runtime/branch.js (a program with a when or an if only)",
            zdc_runtime::BRANCH_JS,
        ),
        size(
            "runtime/adopt.js (a view with a hole in it only)",
            zdc_runtime::ADOPT_JS,
        ),
        // CSS, so `for_mode` does not apply — there is no `// $dev` block
        // in a stylesheet — but the minifier does.
        RuntimeSize {
            name: "runtime/base.css",
            shipped: minify::css(zdc_runtime::BASE_CSS).len(),
            source: zdc_runtime::BASE_CSS.len(),
        },
        size(
            "runtime/elements.js (direct emission only)",
            zdc_runtime::ELEMENTS_JS,
        ),
    ]
}

/// One runtime file, as written and as shipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSize {
    pub name: &'static str,
    /// Bytes a reader downloads: assertions stripped, then minified.
    pub shipped: usize,
    /// Bytes in the file a contributor opens.
    pub source: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_example_compiles_and_is_not_empty() {
        let sizes = bundle_sizes();
        assert_eq!(sizes.len(), 3, "three examples are listed: {sizes:?}");
        for size in sizes {
            assert!(size.client_js > 0, "{} emitted nothing", size.name);
            assert!(size.total() > size.client_js);
            // Both sides of #135, on every arm. A `minified` equal to
            // `total` would mean the minifier had been dropped from the
            // path this table measures and the column had become a copy
            // of the one beside it.
            assert!(
                size.minified < size.total(),
                "{}: minification saved nothing",
                size.name
            );
            assert!(
                size.minified > size.index_html + size.manifest_json,
                "{}: minification took more than the two files it does not touch",
                size.name
            );
        }
    }
}
