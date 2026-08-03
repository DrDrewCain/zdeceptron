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
    let source = TempSource::new("syntax", "view Text\n");
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
fn a_refusal_is_the_one_zdc_build_would_have_given() {
    // Scope: `zdc dev` must refuse exactly what `zdc build` refuses and
    // say exactly what it says — so this compares against the pipeline
    // `zdc build` runs, not against a copy of its wording that could
    // drift. The program is `guestbook.zd` with the secret rendered,
    // because that is the refusal it matters most that both agree about.
    let file = leaking_guestbook("parity");
    let site = build_once(&file, &Settings::default());
    let report = site.report().expect("expected a refusal");

    assert_eq!(
        report,
        build_report(&file),
        "the dev server invented its own diagnostic"
    );
    assert!(
        report.contains("E-IFC-05"),
        "the refusal must be the leak:\n{report}"
    );
    let _ = std::fs::remove_file(&file);
}

/// `guestbook.zd` with the one line its own comment says is a compile
/// error, written to a scratch file.
fn leaking_guestbook(name: &str) -> std::path::PathBuf {
    let text = std::fs::read_to_string(example("guestbook.zd")).expect("guestbook is readable");
    let leaked = text.replace(
        "        Input name, hint is \"your name\"",
        "        Input name, hint is \"your name\"\n        Text apiKey",
    );
    let temp = std::env::temp_dir().join(format!("zdc-{}-{name}-leak.zd", std::process::id()));
    std::fs::write(&temp, leaked).expect("writing the fixture");
    temp
}

/// A rendered secret is refused before anything is emitted, and the
/// refusal names the path along which it would have escaped (§7.3).
#[test]
fn a_rendered_secret_is_refused_before_anything_is_emitted() {
    let temp = leaking_guestbook("sink");

    let site = build_once(&temp, &Settings::default());
    let report = site.report().expect("expected a refusal");
    let _ = std::fs::remove_file(&temp);

    assert!(
        !site.is_ready(),
        "a secret must never reach a served bundle"
    );
    assert!(
        report.contains("apiKey") && report.contains("secret"),
        "the secret must be named:\n{report}"
    );
    assert!(
        report.contains("E-IFC-05"),
        "the view sink must be the one that rejected it:\n{report}"
    );
}

/// And the untouched program is not refused at all: the flow pass clears
/// it, because its secret is used correctly.
#[test]
fn guestbook_itself_is_not_refused_for_its_secret() {
    let file = example("guestbook.zd");
    let site = build_once(&file, &Settings::default());
    assert!(
        site.is_ready(),
        "guestbook's secret is used correctly and must not be reported:\n{}",
        site.report().unwrap_or_default()
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

    let name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("app");
    let options = zdc_codegen::Options::new(&path, name);
    let split = zdc_graph::split(&hir);
    if split.has_errors() {
        let errors: Vec<zdc_graph::GraphError> = split
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        return render_all(&src, &path, errors);
    }
    // Both report, in `zdc_dev::compile`'s order: a program that renders a
    // secret *and* has a type error is told about both.
    let verdict = zdc_graph::ifc(&hir, &split);
    let leaks: Vec<zdc_graph::GraphError> = verdict
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .cloned()
        .collect();
    let table = match zdc_types::check(&hir, &split) {
        Ok(table) if leaks.is_empty() => table,
        Ok(_) => return render_all(&src, &path, leaks),
        Err(errors) => {
            let mut report = render_all(&src, &path, errors);
            report.push_str(&render_all(&src, &path, leaks));
            return report;
        }
    };
    let Some(cleared) = verdict.clearance() else {
        return render_all(&src, &path, leaks);
    };
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
        cleared,
    };
    match zdc_codegen::compile(&inputs, &options) {
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
