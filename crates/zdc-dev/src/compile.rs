//! One build, start to finish, producing either a servable bundle or the
//! diagnostics that explain why there is none.
//!
//! The pipeline is the same one `zdc build` runs, and the diagnostics are
//! rendered by the same `zdc-diagnostics` call, so the browser and the
//! terminal cannot disagree about what is wrong (spec §7.3). In particular
//! the refusals for `server` and `durable` placements come from
//! `zdc-codegen` itself rather than being restated here: `zdc dev` is
//! client-only because the compiler is, and it says so in the compiler's
//! own words.

use std::path::Path;

use zdc_diagnostics::{render, Diagnostic};

use crate::assets::Assets;
use crate::page;

/// What the server has to serve right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    /// The program compiled. These are the files.
    Ready(Assets),
    /// It did not. This is the report, verbatim from `zdc-diagnostics`,
    /// escape sequences included.
    Broken { source_path: String, report: String },
}

impl Site {
    pub fn is_ready(&self) -> bool {
        matches!(self, Site::Ready(_))
    }

    /// The diagnostics, if the build failed. Terminal-formatted: this is
    /// what `zdc dev` prints to stderr, byte for byte what `zdc build`
    /// would have printed.
    pub fn report(&self) -> Option<&str> {
        match self {
            Site::Ready(_) => None,
            Site::Broken { report, .. } => Some(report),
        }
    }
}

/// How to build.
///
/// Empty today, and kept because it is the one place a future build option
/// belongs: `zdc dev` and `zdc build` must run the same pipeline, and a
/// flag that existed on one and not the other is exactly how the two come
/// to disagree about whether a program compiles.
#[derive(Debug, Clone, Default)]
pub struct Settings {}

/// Compile `file` into an in-memory bundle.
///
/// Never returns an error: a build that fails is a `Site::Broken`, because
/// the dev server has to keep serving *something* and a diagnostic on the
/// page is more use than a dead port.
pub fn compile(file: &Path, _settings: &Settings) -> Site {
    let source_path = file.display().to_string();

    // The entry file plus everything it imports (§14D.2). The watcher
    // watches the directory, so editing an imported file rebuilds too.
    let linked = match zdc_resolve::load(file) {
        Ok(linked) => linked,
        Err(errors) => {
            let src = std::fs::read_to_string(file).unwrap_or_default();
            return broken(&source_path, report_all(&src, &source_path, errors));
        }
    };

    let hir = match zdc_resolve::Resolver::linked(&linked).resolve() {
        Ok(hir) => hir,
        Err(errors) => return broken(&source_path, report_in(&linked, errors)),
    };

    // The same pipeline `zdc build` runs, typechecking included. Without
    // it the dev server would serve a bundle the CLI refuses to produce.
    let types = match zdc_types::check(&hir) {
        Ok(types) => types,
        Err(errors) => return broken(&source_path, report_in(&linked, errors)),
    };

    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("app");
    let discovered = zdc_codegen::assets::discover(file);
    let options = zdc_codegen::Options::new(&source_path, name)
        .with_stylesheets(discovered.stylesheets.clone());

    let bundle = match zdc_codegen::compile(&hir, &types, &options) {
        Ok(bundle) => bundle,
        Err(errors) => return broken(&source_path, report_in(&linked, errors)),
    };

    let mut assets = Assets::default();
    // The same files `zdc build` copies, served from memory. An asset the
    // server could not read is simply not served, which shows up as the
    // 404 it is rather than as a stale copy of an earlier build.
    for asset in &discovered.files {
        if let Ok(body) = std::fs::read(&asset.source) {
            assets.insert(format!("/{}", asset.relative), body);
        }
    }
    assets.insert("/index.html", page::with_live_reload(&bundle.index_html));
    assets.insert("/client.js", bundle.client_js);
    assets.insert("/styles.css", bundle.styles_css);
    assets.insert("/manifest.json", bundle.manifest_json);
    for (relative, source) in zdc_codegen::runtime_files() {
        assets.insert(format!("/{relative}"), source);
    }
    Site::Ready(assets)
}

fn broken(source_path: &str, report: String) -> Site {
    Site::Broken {
        source_path: source_path.to_string(),
        report,
    }
}

/// Render **every** diagnostic, not just the first.
///
/// A developer with three undefined names should see three of them from
/// one save, exactly as `zdc check` and `zdc build` already promise.
fn report_all<E>(src: &str, path: &str, errors: Vec<E>) -> String
where
    Diagnostic: From<E>,
{
    let mut report = String::new();
    for error in errors {
        report.push_str(&render(src, path, &Diagnostic::from(error)));
    }
    report
}

/// The same, against the file each span actually belongs to.
///
/// A span carries no file, so a diagnostic about an imported module would
/// otherwise be rendered against whatever text sat at that offset in the
/// entry file.
fn report_in<E>(linked: &zdc_resolve::Linked, errors: Vec<E>) -> String
where
    Diagnostic: From<E>,
{
    let mut report = String::new();
    for error in errors {
        let mut diagnostic = Diagnostic::from(error);
        match diagnostic.span {
            Some(span) => {
                let (path, source, local) = linked.locate(span);
                diagnostic.span = Some(local);
                report.push_str(&render(source, &path.display().to_string(), &diagnostic));
            }
            None => report.push_str(&render("", "", &diagnostic)),
        }
    }
    report
}
