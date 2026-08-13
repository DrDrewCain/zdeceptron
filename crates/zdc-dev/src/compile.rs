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
    /// What the compiler said about a program it nevertheless accepted:
    /// its warnings, rendered exactly as `zdc build` renders them, and
    /// empty when it had nothing to say.
    ///
    /// A successful build is not the same as a silent one, and the dev
    /// server has to print what the build would have printed or the two
    /// are telling a reader different things about one program.
    pub notices: String,
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

    /// What the compiler said about a program it accepted — its warnings,
    /// in the same bytes `zdc build` would have used. Empty for a build
    /// that had nothing to remark on, and for one that failed, whose
    /// warnings are part of [`Site::report`] instead.
    pub fn notices(&self) -> &str {
        match self {
            Site::Ready(ready) => &ready.notices,
            Site::Broken { .. } => "",
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
        Err(failure) => {
            // Against the file each span belongs to, not the entry's text.
            // A parse error in an imported module used to render with no
            // file and no caret here too (#4).
            return broken(&source_path, report_failed_load(&failure));
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
    //
    // Every diagnostic, not only the errors: `W0330` and `W0331` are
    // raised here, and `zdc build` prints them. A dev server that dropped
    // them would be showing a different program from the one the build
    // shows, which is the disagreement this whole module is written to
    // rule out.
    let split = zdc_graph::split(&hir);
    let found = reported(&linked, split.diagnostics.clone());
    if found.fatal {
        return broken(&source_path, found.text);
    }
    let notices = found.text;

    // Both report. A program that renders a secret *and* has a type error
    // should be told about the leak, not only about the type — the leak is
    // the more interesting of the two, and it is the one the type error
    // would otherwise hide.
    let verdict = zdc_graph::ifc(&hir, &split);
    let checked = zdc_types::check(&hir, &split);
    let flow = reported(&linked, verdict.diagnostics.clone());
    let table = match checked {
        Ok(table) if !flow.fatal => table,
        Ok(_) => return broken(&source_path, notices + &flow.text),
        Err(errors) => {
            let mut report = notices;
            report.push_str(&report_in(&linked, errors));
            report.push_str(&flow.text);
            return broken(&source_path, report);
        }
    };
    // Nothing stopped the build, so what was said is what a reader should
    // still see: `zdc build` prints its warnings on a program it accepts.
    let notices = notices + &flow.text;

    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("app");
    let discovered = zdc_codegen::assets::discover(file);
    let options = zdc_codegen::Options::new(&source_path, name)
        .with_stylesheets(discovered.stylesheets.clone())
        .with_icon(discovered.icon.clone());

    // The flow pass's own permission to emit. Nothing fatal came out of
    // it, so this always succeeds — but there is no way to build an
    // `Inputs` without asking, which is the point.
    let Some(cleared) = verdict.clearance() else {
        return broken(&source_path, notices);
    };
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
        cleared,
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
                    // No span to locate, so no source text to point into: a
                    // refused capability is about the build host, not about
                    // a line of the program.
                    let diagnostic = Diagnostic::file_error(error.report());
                    return broken(&source_path, report_in(&linked, vec![diagnostic]));
                }
            }
        }
        // Rendered against the file each span belongs to, like every other
        // refusal here: the build root is printed from every module the
        // entry file imports, so a span in it need not be in the entry.
        Err(errors) => return broken(&source_path, report_in(&linked, errors)),
    };
    let options = options.with_statics(evaluated.values);

    // One document per URL (spec §14G.2). An unrouted program has one, at
    // `/`, which is what it has always had — so this is one code path
    // rather than a routed one and an unrouted one that could disagree.
    let site = match zdc_codegen::compile_site(&inputs, &options) {
        Ok(site) => site,
        Err(errors) => return broken(&source_path, report_in(&linked, errors)),
    };

    let routed = site.pages.len() > 1;
    let mut assets = Assets::default();
    // The same files `zdc build` copies, served from memory. An asset the
    // server could not read is simply not served, which shows up as the
    // 404 it is rather than as a stale copy of an earlier build.
    for asset in &discovered.files {
        if let Ok(body) = std::fs::read(&asset.source) {
            assets.insert(format!("/{}", asset.relative), body);
        }
    }
    for page in site.pages {
        if routed {
            // The same layout `zdc build` writes, so a link that works
            // here works from `dist/` and from any static host: the
            // document at the URL, the module one directory below the
            // root, the runtime shared.
            if let Some(document_html) = &page.document_html {
                assets.insert(
                    format!("/{}", zdc_codegen::document_path(&page.url)),
                    page::with_live_reload(document_html),
                );
            }
            if let Some(boot_js) = page.boot_js {
                assets.insert(format!("/pages/{}.boot.js", page.slug), boot_js);
            }
            assets.insert(format!("/pages/{}.js", page.slug), page.client_js);
            assets.insert(format!("/pages/{}.css", page.slug), page.styles_css);
        } else {
            // A module with no `view` has no page to serve, so the dev
            // server answers with what it *is*: a module, and the list of
            // what it exports.
            let document = match &page.document_html {
                Some(html) => page::with_live_reload(html),
                None => page::module_page(&source_path),
            };
            assets.insert("/index.html", document);
            if let Some(boot_js) = page.boot_js {
                assets.insert("/boot.js", boot_js);
            }
            assets.insert("/client.js", page.client_js);
            assets.insert("/styles.css", page.styles_css);
        }
    }
    if routed {
        assets.insert("/routes.json", site.routes_json.clone());
    }
    assets.insert("/manifest.json", site.manifest_json.clone());
    // Only the modules this program reaches are served: the set is the
    // union over the documents, and the runtime directory is shared by
    // them (§16.3.1).
    // A development build, so the runtime carries its own assertions
    // (#140). That is the whole difference between what `zdc dev` serves
    // and what `zdc build` writes.
    for (relative, source) in
        zdc_codegen::runtime_files(&site.runtime, zdc_codegen::Mode::Development)
    {
        assets.insert(format!("/{relative}"), source);
    }
    // The generated server halves are served as text too, so a developer
    // can read what the split produced (§9). Serving them was once the
    // *only* thing done with them, which is how `POST /_zd/greeting`
    // came to answer "not part of this bundle" — they are now also
    // registered as endpoints below.
    for function in &site.functions {
        assets.insert(format!("/{}", function.path), function.source.clone());
    }
    // `/favicon.ico` at the root, whatever the icon is called, for the
    // reason `zdc build` copies it there: a browser asks for that path on
    // its own and a 404 in the console is the only thing that says so.
    if let Some(icon) = &discovered.icon {
        let relative = icon.trim_start_matches('/');
        if let Some(asset) = discovered
            .files
            .iter()
            .find(|asset| asset.relative == relative)
        {
            if let Ok(body) = std::fs::read(&asset.source) {
                assets.insert("/favicon.ico", body);
            }
        }
    }
    // A `foreign`'s own module, read off disk and served from memory.
    //
    // `zdc build` copies these beside the bundle and `zdc dev` did not
    // serve them at all, so a program with a `foreign … from "./x.js"`
    // built fine and showed a blank page under the dev server: the import
    // 404s, the page module fails with it, and nothing renders. The whole
    // point of `zdc dev` is that it serves what `zdc build` writes.
    for module in &site.linked_modules {
        let path = file
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(&module.destination);
        match std::fs::read(&path) {
            Ok(body) => assets.insert(format!("/{}", module.destination), body),
            // Reported rather than skipped: a module that is named and
            // missing is a program that cannot run, and finding that out
            // from a blank page is the afternoon this arm exists to save.
            Err(error) => eprintln!(
                "error: `{}` is imported by a `foreign` and could not be read: {error}",
                module.specifier
            ),
        }
    }
    // §14C.3b's generated files, served from memory. `rss.xml` is part of
    // the site being developed, so `zdc dev` has to serve it or the thing
    // under development is not the thing that ships.
    for (path, contents) in evaluated.files {
        assets.insert(format!("/{path}"), contents);
    }

    let endpoints: Endpoints = site
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
        notices,
    }))
}

fn broken(source_path: &str, report: String) -> Site {
    Site::Broken {
        source_path: source_path.to_string(),
        report,
    }
}

/// The same, for a load that never produced a [`zdc_resolve::Linked`].
///
/// The module table is all that survives a failed load, which is enough:
/// a span still has to be resolved back to the file it indexes before it
/// means anything to a reader (#4).
fn report_failed_load(failure: &zdc_resolve::LoadFailure) -> String {
    let mut report = String::new();
    for error in &failure.errors {
        let mut diagnostic = Diagnostic::from(error.clone());
        let located = diagnostic.span.and_then(|span| {
            failure.locate(span).map(|(path, source, local)| {
                (path.display().to_string(), source.to_string(), local)
            })
        });
        match located {
            Some((path, source, local)) => {
                diagnostic.span = Some(local);
                report.push_str(&render(&source, &path, &diagnostic));
            }
            // Nothing was read — an unreadable entry file. There is no
            // text to point into, and inventing one would point at a line
            // the reader does not have.
            None => report.push_str(&render("", "", &diagnostic)),
        }
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
    reported(linked, errors).text
}

/// A rendered run of diagnostics, and whether any of them stopped the
/// build.
///
/// The two answers come from one walk because they are one decision: the
/// process's [`zdc_diagnostics::Policy`] can silence a warning, in which
/// case it is neither printed nor counted, and can promote one, in which
/// case it is both. Deciding "is this fatal" anywhere other than beside
/// the printing is how `zdc dev` and `zdc build` come to disagree about a
/// program, which is the one thing this module exists to prevent.
struct Reported {
    text: String,
    fatal: bool,
}

fn reported<E>(linked: &zdc_resolve::Linked, errors: Vec<E>) -> Reported
where
    Diagnostic: From<E>,
{
    let policy = zdc_diagnostics::policy();
    let mut text = String::new();
    let mut fatal = false;
    for error in errors {
        let mut diagnostic = Diagnostic::from(error);
        if !policy.apply(&mut diagnostic) {
            continue;
        }
        fatal |= diagnostic.level.is_error();
        match diagnostic.span {
            Some(span) => {
                let (path, source, local) = linked.locate(span);
                diagnostic.span = Some(local);
                text.push_str(&render(source, &path.display().to_string(), &diagnostic));
            }
            None => text.push_str(&render("", "", &diagnostic)),
        }
    }
    Reported { text, fatal }
}
