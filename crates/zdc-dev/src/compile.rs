//! One build, start to finish, producing either a servable bundle or the
//! diagnostics that explain why there is none.
//!
//! The pipeline is the same one `zdc build` runs — front end, build root,
//! emission — and the diagnostics are rendered by the same
//! `zdc-diagnostics` call, so the browser and the terminal cannot disagree
//! about what is wrong (spec §7.3). Nothing here restates a rule the
//! compiler already enforces: every refusal a developer sees from `zdc
//! dev` came out of `zdc-codegen` in its own words.
//!
//! What this module adds beyond the files is the runnable half. A `Ready`
//! carries the emitted endpoints and the durable keys as well as the
//! assets, because a rebuild has to replace both at once: serving the new
//! `client.js` against the old endpoints would be a boundary where the two
//! sides of one compilation disagree.

use std::path::Path;

use zdc_diagnostics::{render, Diagnostic};
use zdc_host::{Endpoint, Endpoints, Shape};
use zdc_store::Keys;

use crate::assets::Assets;
use crate::page;

/// A compiled program, ready to serve.
///
/// The static files **and** the runnable half. They travel together
/// because a rebuild replaces both at once: serving the new `client.js`
/// against the old endpoints would be a boundary where the two sides of
/// one compilation disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub assets: Assets,
    /// The emitted server functions, runnable rather than readable.
    pub endpoints: Endpoints,
    /// The `durable` keys this program declares — what a live-sync
    /// subscription is allowed to ask for.
    pub keys: Keys,
}

/// What the server has to serve right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    /// The program compiled.
    Ready(Box<Ready>),
    /// It did not. This is the report, verbatim from `zdc-diagnostics`,
    /// escape sequences included.
    Broken { source_path: String, report: String },
}

impl Site {
    pub fn is_ready(&self) -> bool {
        matches!(self, Site::Ready(_))
    }

    /// The files, if it compiled.
    pub fn assets(&self) -> Option<&Assets> {
        match self {
            Site::Ready(ready) => Some(&ready.assets),
            Site::Broken { .. } => None,
        }
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

    // Every entry point compiles against the prelude (§17.4.1), and the
    // linked program on top of it.
    let prelude = zdc_lib::load();
    let hir = match zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked).resolve()
    {
        Ok(hir) => hir,
        Err(errors) => return broken(&source_path, report_in(&linked, errors)),
    };

    // The same gates `zdc build` applies, in spec §17.1.2's order: the
    // split, then the type checker and the flow pass together. Code
    // generation refuses without all three (§17.1.3). Every diagnostic
    // from here on is rendered against the file its span belongs to,
    // because the split sees every imported module too.
    let split = zdc_graph::split(&hir);
    if split.has_errors() {
        let errors: Vec<zdc_graph::GraphError> = split
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        return broken(&source_path, report_in(&linked, errors));
    }

    // Both report. A program that renders a secret *and* has a type error
    // should be told about the leak, not only about the type — the leak is
    // the more interesting of the two, and it is the one the type error
    // would otherwise hide.
    let verdict = zdc_graph::ifc(&hir, &split);
    let checked = zdc_types::check(&hir, &split);
    let leaks: Vec<zdc_graph::GraphError> = verdict
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .cloned()
        .collect();
    let table = match checked {
        Ok(table) if leaks.is_empty() => table,
        Ok(_) => return broken(&source_path, report_in(&linked, leaks)),
        Err(errors) => {
            let mut report = report_in(&linked, errors);
            report.push_str(&report_in(&linked, leaks));
            return broken(&source_path, report);
        }
    };

    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("app");
    let options = zdc_codegen::Options::new(&source_path, name);

    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
    };

    // §17.4.8's build root, run on the build host before the bundle that
    // inlines what it computed. `zdc dev` runs the same two steps `zdc
    // build` runs, because a dev server that skipped one would disagree
    // with the compiler about whether a program works.
    let evaluated = match zdc_codegen::build_module(&inputs, &options) {
        Ok(None) => zdc_codegen::Evaluated::default(),
        Ok(Some(module)) => {
            let directory = file.parent().unwrap_or(Path::new("."));
            match zdc_codegen::evaluate(&module, directory) {
                Ok(evaluated) => evaluated,
                Err(error) => {
                    let diagnostic = Diagnostic::file_error(error.report());
                    return broken(&source_path, render("", "", &diagnostic));
                }
            }
        }
        Err(errors) => return broken(&source_path, report_in(&linked, errors)),
    };
    let options = options.with_statics(evaluated.values);

    let bundle = match zdc_codegen::compile(&inputs, &options) {
        Ok(bundle) => bundle,
        Err(errors) => return broken(&source_path, report_in(&linked, errors)),
    };

    let mut assets = Assets::default();
    assets.insert("/index.html", page::with_live_reload(&bundle.index_html));
    // Read before the strings are moved out: the set is a property of the
    // bundle, and only the modules this program reaches are served (§16.3.1).
    for (relative, source) in zdc_codegen::runtime_files(&bundle) {
        assets.insert(format!("/{relative}"), source);
    }
    assets.insert("/client.js", bundle.client_js.clone());
    assets.insert("/styles.css", bundle.styles_css.clone());
    assets.insert("/manifest.json", bundle.manifest_json.clone());
    // The generated server halves are served as text too, so a developer
    // can read what the split produced (§9). Serving them was once the
    // *only* thing done with them, which is how `POST /_zd/greeting`
    // came to answer "not part of this bundle" — they are now also
    // registered as endpoints below.
    for function in &bundle.functions {
        assets.insert(format!("/{}", function.path), function.source.clone());
    }
    // §14C.3b's generated files, served from memory. `rss.xml` is part of
    // the site being developed, so `zdc dev` has to serve it or the thing
    // under development is not the thing that ships.
    for (path, contents) in evaluated.files {
        assets.insert(format!("/{path}"), contents);
    }

    let endpoints: Endpoints = bundle
        .functions
        .iter()
        .map(|function| Endpoint {
            name: function.name.clone(),
            shape: match function.kind {
                zdc_codegen::FunctionKind::Value => Shape::Value,
                zdc_codegen::FunctionKind::Command => Shape::Command,
            },
            inputs: function.inputs.clone(),
            source: function.source.clone(),
        })
        .collect();

    let keys = Keys::new(
        split
            .reads_keys
            .values()
            .chain(split.writes_keys.values())
            .flat_map(|keys| keys.iter().map(|key| hir.defs[*key].name.clone())),
    );

    Site::Ready(Box::new(Ready {
        assets,
        endpoints,
        keys,
    }))
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
