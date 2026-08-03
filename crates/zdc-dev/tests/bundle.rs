//! What one build produces, and what it refuses to produce.

use std::path::{Path, PathBuf};

use zdc_dev::{build_once, Settings, Site};

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// A file under the system temporary directory, removed when the test ends
/// whether it passed or not.
struct TempSource {
    path: PathBuf,
}

impl TempSource {
    fn new(name: &str, contents: &str) -> TempSource {
        let path = std::env::temp_dir().join(format!("zdc-dev-{}-{name}.zd", std::process::id()));
        std::fs::write(&path, contents).expect("could not write the temporary source");
        TempSource { path }
    }
}

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn assets(site: &Site) -> &zdc_dev::Assets {
    match site {
        Site::Ready(assets) => assets,
        Site::Broken { report, .. } => panic!("expected a build, got diagnostics:\n{report}"),
    }
}

fn text(site: &Site, path: &str) -> String {
    let asset = assets(site)
        .get(path)
        .unwrap_or_else(|| panic!("{path} is not in the bundle"));
    String::from_utf8(asset.body.clone()).expect("the bundle is UTF-8")
}

#[test]
fn a_client_only_program_produces_every_file_the_page_links_to() {
    // A bundle missing one of these loads as a blank page with a console
    // error, which is the failure mode hardest to attribute.
    let site = build_once(&example("counter.zd"), &Settings::default());
    let served: Vec<&str> = assets(&site).paths().collect();

    for required in [
        "/index.html",
        "/client.js",
        "/styles.css",
        "/manifest.json",
        "/runtime/signal.js",
        "/runtime/dom.js",
    ] {
        assert!(
            served.contains(&required),
            "{required} missing from {served:?}"
        );
    }
}

#[test]
fn the_served_client_is_the_compiler_output_not_a_placeholder() {
    let site = build_once(&example("counter.zd"), &Settings::default());
    let client = text(&site, "/client.js");

    assert!(
        client.contains("export function main"),
        "no entry:\n{client}"
    );
    assert!(client.contains("signal("), "no reactivity:\n{client}");
    assert!(
        client.contains("./runtime/signal.js"),
        "the client must import the runtime the server also serves:\n{client}"
    );
}

#[test]
fn every_runtime_module_the_client_imports_is_served() {
    // The import specifiers in `client.js` and the keys of the bundle are
    // produced by different code; if they drift, the page 404s on load.
    let site = build_once(&example("counter.zd"), &Settings::default());
    let client = text(&site, "/client.js");

    for line in client.lines().filter(|l| l.starts_with("import ")) {
        let specifier = line
            .rsplit_once("from '")
            .and_then(|(_, rest)| rest.split_once('\''))
            .map(|(path, _)| path)
            .unwrap_or_else(|| panic!("could not read the specifier out of {line:?}"));
        let served = specifier.trim_start_matches('.');
        assert!(
            assets(&site).get(served).is_some(),
            "{line} imports {served}, which the server does not have"
        );
    }
}

#[test]
fn the_served_page_carries_the_live_reload_client() {
    let site = build_once(&example("counter.zd"), &Settings::default());
    let page = text(&site, "/index.html");

    assert!(page.contains("EventSource"), "no live reload:\n{page}");
    assert!(page.contains("/__zdc/live"), "no stream path:\n{page}");
    assert!(
        page.contains("import { main } from './client.js'"),
        "the generated page was damaged by the injection:\n{page}"
    );
}

#[test]
fn a_syntax_error_produces_diagnostics_rather_than_a_bundle() {
    let source = TempSource::new("syntax", "view\n    Text \"a\" Text \"b\"\n");
    let site = build_once(&source.path, &Settings::default());

    let report = site.report().expect("expected diagnostics");
    assert!(!site.is_ready(), "a broken program must not build");
    assert!(
        report.contains("line break"),
        "the parse error is missing:\n{report}"
    );
    assert!(
        report.contains(&source.path.display().to_string()),
        "the report must name the file:\n{report}"
    );
}

#[test]
fn every_unresolved_name_is_reported_from_one_build() {
    // A developer with three typos should see three of them per save, as
    // `zdc check` and `zdc build` already promise.
    let source = TempSource::new(
        "unresolved",
        "state a is client Whole from missingOne + missingTwo\n\nview\n    Text a\n",
    );
    let site = build_once(&source.path, &Settings::default());
    let report = site.report().expect("expected diagnostics");

    assert!(
        report.contains("missingOne"),
        "first name missing:\n{report}"
    );
    assert!(
        report.contains("missingTwo"),
        "second name missing:\n{report}"
    );
}

#[test]
fn a_missing_file_is_a_diagnostic_not_a_panic() {
    // The source can disappear while the server is running — a branch
    // switch, a rename. That must show up on the page, not take the
    // process down.
    let site = build_once(Path::new("/definitely/not/here.zd"), &Settings::default());
    let report = site.report().expect("expected diagnostics");
    assert!(
        report.contains("Could not read"),
        "wrong message:\n{report}"
    );
}

#[test]
fn durable_and_server_placements_are_refused_in_the_compilers_own_words() {
    // Scope: `zdc dev` serves client-only programs. It must refuse the
    // rest exactly as `zdc build` does rather than emit something broken —
    // so this compares against the pipeline `zdc build` runs, not against
    // a copy of its wording that could drift.
    let file = example("guestbook.zd");
    let site = build_once(&file, &Settings::default());
    let report = site.report().expect("expected a refusal");

    assert_eq!(
        report,
        build_report(&file),
        "the dev server invented its own diagnostic"
    );
    assert!(
        report.contains("client bundle only"),
        "the refusal must say what is not supported:\n{report}"
    );
}

#[test]
fn a_secret_signal_is_refused_before_anything_is_emitted() {
    let file = example("guestbook.zd");
    let site = build_once(&file, &Settings::default());
    let report = site.report().expect("expected a refusal");

    assert!(
        !site.is_ready(),
        "a secret must never reach a served bundle"
    );
    assert!(
        report.contains("apiKey") && report.contains("secret"),
        "the secret must be named:\n{report}"
    );
}

/// Re-run exactly what `zdc build` runs, and render exactly what it
/// renders. Any divergence between this and `zdc_dev::compile` is the bug
/// the parity test above is looking for.
fn build_report(file: &Path) -> String {
    use zdc_diagnostics::{render, Diagnostic};

    let path = file.display().to_string();
    let src = std::fs::read_to_string(file).expect("the example is readable");

    let program = match zdc_parser::parse(&src) {
        Ok(program) => program,
        Err(error) => return render(&src, &path, &Diagnostic::from(error)),
    };
    let hir = match zdc_resolve::Resolver::new(&program).resolve() {
        Ok(hir) => hir,
        Err(errors) => return render_all(&src, &path, errors),
    };
    let types = match zdc_types::check(&hir) {
        Ok(types) => types,
        Err(errors) => return render_all(&src, &path, errors),
    };

    let name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("app");
    let options = zdc_codegen::Options::new(&path, name);
    match zdc_codegen::compile(&hir, &types, &options) {
        Ok(_) => String::new(),
        Err(errors) => render_all(&src, &path, errors),
    }
}

fn render_all<E>(src: &str, path: &str, errors: Vec<E>) -> String
where
    zdc_diagnostics::Diagnostic: From<E>,
{
    let mut out = String::new();
    for error in errors {
        out.push_str(&zdc_diagnostics::render(
            src,
            path,
            &zdc_diagnostics::Diagnostic::from(error),
        ));
    }
    out
}

/// A module with no `view` builds; it simply has no page in it.
///
/// The dev server still has to serve *something* at `/index.html`, because
/// a bare 404 there is indistinguishable from a compiler that failed to
/// emit the page. It serves a page that says so, with the live-reload
/// client on it, so adding a `view` swaps in the real page without a
/// manual refresh.
#[test]
fn a_module_with_no_view_builds_and_says_it_has_no_page() {
    let site = build_once(&example("model.zd"), &Settings::default());

    let client = text(&site, "/client.js");
    assert!(
        client.contains("export function visible(all)"),
        "the module itself is served:\n{client}"
    );
    assert!(
        !client.contains("main("),
        "a module has no entry point:\n{client}"
    );

    let page = text(&site, "/index.html");
    assert!(
        page.contains("This file is a module. It renders nothing."),
        "the page must explain itself rather than 404:\n{page}"
    );
    assert!(
        !page.contains("import { main }"),
        "the page must not import a `main` the module does not export:\n{page}"
    );
    assert!(
        page.contains("EventSource"),
        "adding a `view` must reload into the real page:\n{page}"
    );
}
