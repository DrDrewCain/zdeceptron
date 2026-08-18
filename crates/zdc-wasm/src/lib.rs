#![forbid(unsafe_code)]

//! The ZDeceptron front end, compiled to WebAssembly — issue #171.
//!
//! # What this crate is for
//!
//! The language's claim is that a program is one file and there is nothing
//! to install. Until this existed, trying it meant cloning a repository
//! and building a twenty-crate Rust workspace, which is a strange thing to
//! ask of someone evaluating a claim about *not* installing things.
//!
//! This compiler is unusually well placed to answer that, because it emits
//! JavaScript and ships its own runtime. A program compiled in a browser
//! tab can run in that same tab. There is no server anywhere in the
//! picture — not for the compile, and not for the run.
//!
//! # The feasibility question, and its answer
//!
//! The whole front end — `zdc-lexer`, `zdc-ast`, `zdc-hir`, `zdc-parser`,
//! `zdc-resolve`, `zdc-graph`, `zdc-types`, `zdc-lib`, `zdc-codegen`,
//! `zdc-doc` and `zdc-diagnostics` — builds for WebAssembly. That is not
//! luck: those passes are pure computation over data handed in, and the
//! two places that are not (`zdc-codegen`'s `assets.rs`, and the module
//! loader's file reads) were already isolated for their own reasons.
//! `ariadne`, which draws every diagnostic, is pure Rust and needs no
//! substitute — so the playground shows the real reports rather than bare
//! strings.
//!
//! **One dependency stood in the way, and it is worth naming precisely.**
//! `zdc-codegen` depended on `zdc-runtime` for the `.js` and `.css` a
//! bundle ships, and `zdc-runtime` also holds `Sandbox`, which is
//! `boa_engine`, a JavaScript interpreter written in Rust. The chain that
//! broke the build was:
//!
//! ```text
//! zdc-codegen -> zdc-runtime -> boa_engine -> rand -> rand_core -> getrandom
//! ```
//!
//! `getrandom` 0.3 refuses `wasm32-unknown-unknown` outright unless the
//! embedder picks an entropy backend with a `--cfg` flag. So the entire
//! front end was unbuildable for the browser on account of a transitive
//! dependency of a JavaScript interpreter — inside a browser, which has
//! one. The fix was a feature seam through `zdc-runtime` and `zdc-codegen`
//! (`evaluate`, on by default), not a `--cfg` flag: shipping a JavaScript
//! engine to a JavaScript engine is not a thing to make work, it is a
//! thing to stop doing.
//!
//! `redb`, the other candidate, was never reachable: it is `zdc-store`'s,
//! and `zdc-store` is reached only from `zdc-host` and `zdc-dev`, neither
//! of which the front end depends on.
//!
//! # What the playground cannot do, stated rather than hidden
//!
//! Four things, and every one of them is reported by name rather than
//! failing somewhere the reader cannot see.
//!
//! * **`static` state does not compute.** Build-time evaluation *is* the
//!   feature that was cut. [`compile_to_json`] detects a build root and
//!   refuses, naming each `static` it found, rather than emitting a bundle
//!   with `undefined` inlined where a value should be.
//! * **`use` of another module cannot resolve.** There is one file, so
//!   there is nothing to import from; the loader's read fails and says so.
//! * **`build read`, `build list` and `build markdown` cannot answer.**
//!   They are the same refusal as `static`: all three are build-root
//!   capabilities, and the build root is what does not run here.
//! * **A `server` or `durable` program compiles but does not run.** It
//!   emits — endpoints included, which is the interesting part — and
//!   `Run` carries the reason the page does not execute it. Running it
//!   would mean a host and a store, and a playground that quietly ran a
//!   crippled version would teach the wrong thing about the language whose
//!   whole subject is where code runs.
//!
//! A playground that quietly compiled a different program than `zdc build`
//! would is worse than one that says what it cannot do.

mod json;

use std::collections::BTreeSet;
use std::path::Path;

use zdc_ast::Placement;
use zdc_diagnostics::Diagnostic;
use zdc_graph::{EndpointKind, TierSplit};
use zdc_hir::{DefKind, Hir};

/// The file name the playground compiles under.
///
/// It shows up in diagnostics and in the emitted header comment, so it is
/// a real-looking name rather than `<stdin>`: a reader should see the same
/// shape of message they would get from `zdc check`.
const ENTRY: &str = "playground.zd";

/// One file of a bundle, as `(path relative to the bundle root, source)`.
///
/// The sources travel with the answer rather than being fetched, because
/// there is no server to fetch them from. That is the whole difference
/// between this and `zdc build`, which writes the same set to a directory.
type File = (String, String);

/// Whether the page may execute what it just compiled, and why not.
///
/// A refusal is a first-class result here rather than an error, because
/// the compile *succeeded*: a `durable` counter is a correct program, and
/// the playground has its bundle, its endpoints and its split in hand. The
/// only thing missing is a machine to run half of it on.
struct Run {
    can: bool,
    /// Empty when `can`. One sentence otherwise, naming the specific thing
    /// this page does not have.
    why: String,
}

/// Where every declaration ended up, and what the compiler derived from
/// that.
///
/// **This is the panel no other playground can show.** Every field is read
/// back from a pass that has already run — the placements from the HIR,
/// the browser-side type from `zdc-doc`'s `prose`, which asks §14G.1.4's
/// read table, and the endpoints from the tier split, which is the pass
/// that invented them. Nothing here is a second description that could go
/// out of date, which is the same rule `zdc doc` follows and the reason
/// this borrows its sentences instead of writing new ones.
struct Placements {
    signals: Vec<Signal>,
    /// One sentence per placement the program actually uses. A reader of a
    /// client-only program should not have to skip four of them.
    legend: Vec<(&'static str, &'static str)>,
    endpoints: Vec<Endpoint>,
    /// Every durable key and every environment key the program touches.
    /// Both are what a deployment would have to provision, and both are
    /// empty for a program that runs entirely in the browser.
    durable: Vec<String>,
    environment: Vec<String>,
}

struct Signal {
    name: String,
    ty: String,
    placement: &'static str,
    secret: bool,
    /// What the browser gets when it reads this, from
    /// [`zdc_doc::prose::from_the_browser`].
    read: String,
    line: usize,
}

/// An endpoint nobody wrote.
struct Endpoint {
    name: String,
    /// Where the emitter puts it, from the emitter's own naming function,
    /// so this cannot name a file the bundle does not contain.
    file: String,
    what: String,
    takes: Vec<String>,
}

/// Everything one compilation produced: the bundle, exactly as
/// `zdc build` would write it — `index.html`, `boot.js`, `client.js`,
/// `styles.css`, the runtime modules it reaches, `manifest.json`, and one
/// file per derived endpoint.
struct Compiled {
    files: Vec<File>,
}

/// Compile source text and describe the outcome as JSON.
///
/// Always returns a document, never an error: a failed compile is a
/// result, and its diagnostics are the most useful thing this compiler
/// produces. The shape is
///
/// ```text
/// { ok, diagnostics, placement, bundle: [{path, source}], run: {can, why} }
/// ```
///
/// with `diagnostics` carrying the rendered reports — every one of them,
/// not the first. A programmer with three undefined names should see three
/// diagnostics from one run, which is the rule `zdc check` already follows
/// and the reason the passes below accumulate rather than return early.
///
/// `placement` is present whenever the program got as far as a settled
/// split, **including when no bundle was emitted**. A program with `static`
/// state is refused here and still has a placement table worth reading, and
/// hiding it because the last pass declined would throw away the answer the
/// reader came for.
pub fn compile_to_json(source: &str) -> String {
    let mut reports: Vec<String> = Vec::new();
    let outcome = compile_source(source, &mut reports);

    let (placement, compiled, run) = match outcome {
        Some((placement, compiled, run)) => (Some(placement), compiled, run),
        None => (None, None, refused("this program does not compile")),
    };

    let files: Vec<String> = compiled
        .as_ref()
        .map(|bundle| {
            bundle
                .files
                .iter()
                .map(|(path, text)| {
                    json::object(&[("path", json::string(path)), ("source", json::string(text))])
                })
                .collect()
        })
        .unwrap_or_default();

    json::object(&[
        (
            "ok",
            if compiled.is_some() { "true" } else { "false" }.to_string(),
        ),
        ("diagnostics", json::string(&reports.join(""))),
        (
            "placement",
            match &placement {
                Some(placement) => encode_placement(placement),
                // `null`, not an empty table: "the compiler never settled
                // where anything lives" and "this program declares no
                // state" are different facts, and the page shows a
                // different thing for each.
                None => "null".to_string(),
            },
        ),
        ("bundle", json::array(&files)),
        (
            "run",
            json::object(&[
                ("can", if run.can { "true" } else { "false" }.to_string()),
                ("why", json::string(&run.why)),
            ]),
        ),
    ])
}

fn encode_placement(placement: &Placements) -> String {
    let signals: Vec<String> = placement
        .signals
        .iter()
        .map(|signal| {
            json::object(&[
                ("name", json::string(&signal.name)),
                ("type", json::string(&signal.ty)),
                ("placement", json::string(signal.placement)),
                (
                    "secret",
                    if signal.secret { "true" } else { "false" }.to_string(),
                ),
                ("read", json::string(&signal.read)),
                ("line", signal.line.to_string()),
            ])
        })
        .collect();
    let legend: Vec<String> = placement
        .legend
        .iter()
        .map(|(word, sentence)| {
            json::object(&[
                ("placement", json::string(word)),
                ("sentence", json::string(sentence)),
            ])
        })
        .collect();
    let endpoints: Vec<String> = placement
        .endpoints
        .iter()
        .map(|endpoint| {
            let takes: Vec<String> = endpoint
                .takes
                .iter()
                .map(|name| json::string(name))
                .collect();
            json::object(&[
                ("name", json::string(&endpoint.name)),
                ("file", json::string(&endpoint.file)),
                ("what", json::string(&endpoint.what)),
                ("takes", json::array(&takes)),
            ])
        })
        .collect();
    let durable: Vec<String> = placement
        .durable
        .iter()
        .map(|key| json::string(key))
        .collect();
    let environment: Vec<String> = placement
        .environment
        .iter()
        .map(|key| json::string(key))
        .collect();

    json::object(&[
        ("signals", json::array(&signals)),
        ("legend", json::array(&legend)),
        ("endpoints", json::array(&endpoints)),
        ("durable", json::array(&durable)),
        ("environment", json::array(&environment)),
    ])
}

/// Parse, resolve, split, typecheck, check flow, and emit.
///
/// The order is `zdc-cli`'s, and it is spec §17.1.2's: the split runs
/// before the type checker, because the type of a cross-placement read
/// depends on the crossing. Rather than duplicate the reasoning, the short
/// version is that this function is `zdc-cli`'s `front_end` followed by
/// its `build`, with three differences and no fourth:
///
/// 1. diagnostics are pushed onto `reports` instead of printed to stderr;
/// 2. the bundle is returned as strings instead of written to a directory;
/// 3. the placement table is read back out, because showing it is half of
///    why this exists.
///
/// It is deliberately *not* factored out of `zdc-cli` into something both
/// call. The CLI's version reads files, writes files, copies assets and
/// evaluates a build root, and the useful common part after removing all
/// of that is the pass order — which is four lines and a spec reference.
/// A shared abstraction here would be larger than what it shared.
#[allow(clippy::type_complexity)]
fn compile_source(
    source: &str,
    reports: &mut Vec<String>,
) -> Option<(Placements, Option<Compiled>, Run)> {
    let entry = Path::new(ENTRY);

    // The entry's text is supplied rather than read, which is what makes
    // this possible at all: `load` would open a file, and there is no
    // filesystem. Imports are still read from disk and so still fail —
    // see this crate's module doc.
    let linked = match zdc_resolve::load_with_entry(entry, source.to_string()) {
        Ok(linked) => linked,
        Err(failure) => {
            for error in &failure.errors {
                let message = error.message.clone();
                let mut diagnostic = Diagnostic::from(error.clone());
                // Against the file each span belongs to, not the entry's
                // text. In the playground there is only ever one file, but
                // the branch stays because `locate` is also what reports
                // "nothing was read at all", which is the shape a failed
                // `use` arrives in.
                //
                // Every error is rendered, so a span-less one falls
                // through to the file-level form rather than ending the
                // loop: a reader with three problems gets three messages.
                let located = diagnostic.span.and_then(|span| failure.locate(span)).map(
                    |(path, text, local)| (path.display().to_string(), text.to_string(), local),
                );
                match located {
                    Some((path, text, local)) => {
                        diagnostic.span = Some(local);
                        reports.push(render(&text, &path, &diagnostic));
                    }
                    None => reports.push(render("", ENTRY, &Diagnostic::file_error(message))),
                }
            }
            return None;
        }
    };

    // Every entry point compiles against the prelude (§17.4.1), and the
    // linked program on top of it.
    let prelude = zdc_lib::load();
    let hir = match zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked).resolve()
    {
        Ok(hir) => hir,
        Err(errors) => {
            report(reports, &linked, errors);
            return None;
        }
    };

    // The type checker refuses to run if the split found an error: a
    // program whose placements do not resolve has no settled read table,
    // so every cross-placement type after the first would be invented
    // (§17.1.3).
    let split = zdc_graph::split(&hir);
    if split.has_errors() {
        report(reports, &linked, errors_in(&split.diagnostics));
        return None;
    }

    let verdict = zdc_graph::ifc(&hir, &split);
    let checked = zdc_types::check(&hir, &split);

    // Both report. A program that renders a secret and has a type error
    // should be told about the leak as well as the type — the leak is the
    // more interesting of the two.
    let mut failed = false;
    if let Err(errors) = &checked {
        report(reports, &linked, errors.clone());
        failed = true;
    }
    let leaks = errors_in(&verdict.diagnostics);
    if !leaks.is_empty() {
        report(reports, &linked, leaks);
        failed = true;
    }
    if failed {
        return None;
    }
    let table = checked.ok()?;

    // Everything below this line has a settled split, so the placement
    // table is answerable whatever the emitter goes on to say.
    let mut placements = placements(&hir, &split, &table, &linked);

    // The flow pass's own permission to emit. `Inputs` cannot be built
    // without asking, which is what makes §16.3.12's invariant 3 a
    // property of the type system rather than a convention.
    let cleared = verdict.clearance()?;
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
        cleared,
    };
    // No first paint. This crate takes `zdc-codegen` with
    // `default-features = false` to keep a JavaScript engine out of a WASM
    // build, and that alone does not hold: Cargo unifies features across a
    // workspace build, so `cargo test --workspace` compiles this against a
    // codegen with `evaluate` on and the playground quietly began shipping
    // a painted container. Saying so is a decision this crate states, and
    // survives whatever the build graph does to the feature.
    let options = zdc_codegen::Options::new(ENTRY, "playground").without_first_paint();

    // §17.4.8's build root, which this build cannot run. Checked *before*
    // emitting rather than after, because the emitter would otherwise
    // inline `undefined` for every `static` read and produce a bundle that
    // loads and is wrong — the failure mode a blank page three layers from
    // its cause. Refusing here costs the playground one placement and
    // keeps every program it does compile honest.
    match zdc_codegen::build_module(&inputs, &options) {
        Ok(None) => {}
        Ok(Some(module)) => {
            reports.push(render(
                "",
                ENTRY,
                &Diagnostic::file_error(unevaluated(&module.statics)),
            ));
            return Some((
                placements,
                None,
                refused("`static` state is computed at build time, and this build of the compiler has no engine to compute it with"),
            ));
        }
        Err(errors) => {
            report(reports, &linked, errors);
            return Some((placements, None, refused("this program does not compile")));
        }
    }

    match zdc_codegen::compile(&inputs, &options) {
        Ok(bundle) => {
            // Both lists are what a deployment would have to provision,
            // and the emitter is where they are settled: §16.3.12
            // assertion C keeps environment key names out of the manifest,
            // so the bundle reports them separately and this is the same
            // read a deploy adapter does.
            placements.durable.clone_from(&bundle.durable);
            placements.environment.clone_from(&bundle.environment);
            let run = runnable(&bundle, &placements);
            Some((placements, Some(files(bundle)), run))
        }
        Err(errors) => {
            report(reports, &linked, errors);
            Some((placements, None, refused("this program does not compile")))
        }
    }
}

/// The bundle as a flat path-to-source list, in the order a reader wants
/// to meet it: the page, then what the page loads, then the machinery.
///
/// It is the same set `zdc build` writes and it is computed the same way —
/// `runtime_files` decides which runtime modules travel, so a program that
/// renders no `Prose` carries no `markup.js` here either (§16.3.1).
fn files(bundle: zdc_codegen::Bundle) -> Compiled {
    let mut files: Vec<File> = Vec::new();

    if let Some(html) = bundle.index_html {
        files.push(("index.html".to_string(), html));
    }
    if let Some(boot) = bundle.boot_js {
        files.push(("boot.js".to_string(), boot));
    }
    files.push(("client.js".to_string(), bundle.client_js));
    // The name the document links, which carries a content hash (#137).
    // A playground that showed the stylesheet under a name the page does
    // not name would be showing a bundle that does not work.
    files.push((bundle.styles_path, bundle.styles_css));
    // `Development`, so the runtime keeps its `// $dev` assertions (#140).
    // `zdc build` strips them and `zdc dev` does not, and a playground is
    // the second of those: someone is here to find out what the language
    // does, and an assertion that names the mistake beats a silent wrong
    // render. It costs bytes nobody is shipping.
    for (path, source) in
        zdc_codegen::runtime_files(&bundle.runtime, zdc_codegen::Mode::Development)
    {
        files.push((path.to_string(), source));
    }
    // The endpoints, which are the point of showing the bundle at all: a
    // reader who declared `state visits is durable Whole` gets to read the
    // file the compiler wrote for a call they never made.
    for function in bundle.functions {
        files.push((function.path, function.source));
    }
    files.push(("manifest.json".to_string(), bundle.manifest_json));

    Compiled { files }
}

/// Whether this page may execute the bundle it just built.
///
/// Three refusals, in the order they matter. Each names the specific thing
/// that is missing, because "the playground cannot run this" would send a
/// reader looking for a bug in their program.
fn runnable(bundle: &zdc_codegen::Bundle, placements: &Placements) -> Run {
    if bundle.index_html.is_none() {
        return refused("this program has no `view`, so there is no page to run — it compiled, and the bundle below is what it produced");
    }
    if !bundle.linked_modules.is_empty() {
        return refused("this program imports a `foreign` module by relative path, and there is no file to import here");
    }
    // A derived endpoint is exactly the evidence that half of this program
    // belongs somewhere else. The browser can hold the other half, and
    // running it alone would show a page of failed reads — which is a true
    // picture of a broken deployment and a false one of the language.
    if !placements.endpoints.is_empty() {
        return refused(&format!(
            "this program's state does not all live in the browser, so running it needs a host this page does not have. The compiler derived {} endpoint{} from the placements — read them below; the split is the interesting half.",
            placements.endpoints.len(),
            if placements.endpoints.len() == 1 { "" } else { "s" },
        ));
    }
    Run {
        can: true,
        why: String::new(),
    }
}

fn refused(why: &str) -> Run {
    Run {
        can: false,
        why: why.to_string(),
    }
}

/// Read the placement table back out of the passes that decided it.
fn placements(
    hir: &Hir,
    split: &TierSplit,
    table: &zdc_types::TypeTable,
    linked: &zdc_resolve::Linked,
) -> Placements {
    let mut signals: Vec<Signal> = Vec::new();
    let mut used: BTreeSet<&'static str> = BTreeSet::new();

    for (id, def) in hir.user_defs() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        // The checked type if there is one, and the written type if the
        // checker had nothing to say — `zdc-doc`'s rule, for the same
        // reason: a table that printed the surface syntax would disagree
        // with the checker about an inferred type.
        let ty = match table.def(id) {
            Some(ty) => ty.to_string(),
            None => zdc_doc::prose::render_type(&signal.ty),
        };
        let (_, text, local) = linked.locate(def.span);
        used.insert(signal.placement.word());
        signals.push(Signal {
            name: def.name.clone(),
            read: zdc_doc::prose::from_the_browser(signal.placement, signal.secret, &ty),
            ty,
            placement: signal.placement.word(),
            secret: signal.secret,
            line: line_of(text, local.start),
        });
    }

    // `Placement::ALL`'s order, so the legend reads client-to-durable
    // rather than alphabetically, and a sixth placement appears here
    // without anyone editing this line.
    let legend: Vec<(&'static str, &'static str)> = Placement::ALL
        .iter()
        .filter(|placement| used.contains(placement.word()))
        .map(|placement| {
            (
                placement.word(),
                zdc_doc::prose::placement_sentence(*placement),
            )
        })
        .collect();

    let endpoints = split
        .endpoints
        .iter()
        .map(|endpoint| Endpoint {
            what: match &endpoint.kind {
                EndpointKind::Value(def) => {
                    format!("a value the browser reads — `{}`", hir.defs[*def].name)
                }
                EndpointKind::Command(key) => format!(
                    "a command the browser performs — `{}` on `{}`",
                    key.op.word(),
                    hir.defs[key.signal].name
                ),
            },
            takes: endpoint
                .params
                .iter()
                .map(|param| hir.defs[*param].name.clone())
                .collect(),
            // The emitter's own table, so a reader looking for this file
            // in the bundle finds it (§17.2.5 fatal 3).
            file: zdc_codegen::file_name(&endpoint.name),
            name: endpoint.name.clone(),
        })
        .collect();

    Placements {
        signals,
        legend,
        endpoints,
        durable: Vec::new(),
        environment: Vec::new(),
    }
}

/// The 1-based line a byte offset falls on.
fn line_of(source: &str, offset: u32) -> usize {
    let offset = (offset as usize).min(source.len());
    source[..offset].matches('\n').count() + 1
}

/// The refusal a program with `static` state gets here.
///
/// Names every `static` it found, because the fix is to remove or rewrite
/// those specific declarations, and a message that said only "build-time
/// evaluation is unavailable" would leave the reader to find them.
fn unevaluated(statics: &[String]) -> String {
    let named = statics.join("`, `");
    format!("the playground cannot compute `{named}`: a `static` signal runs at build time, in a JavaScript engine this build does not carry (§17.4.8). `zdc build` computes it.")
}

/// Only the diagnostics that are errors.
///
/// A pass's `diagnostics` list carries warnings too, and a warning must
/// not stop a build.
fn errors_in(diagnostics: &[zdc_graph::GraphError]) -> Vec<zdc_graph::GraphError> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .cloned()
        .collect()
}

/// Render every diagnostic against the file its span belongs to.
fn report<E>(reports: &mut Vec<String>, linked: &zdc_resolve::Linked, errors: Vec<E>)
where
    Diagnostic: From<E>,
{
    for error in errors {
        let mut diagnostic = Diagnostic::from(error);
        let Some(span) = diagnostic.span else {
            reports.push(render("", ENTRY, &diagnostic));
            continue;
        };
        let (path, source, local) = linked.locate(span);
        let path = path.display().to_string();
        let source = source.to_string();
        diagnostic.span = Some(local);
        reports.push(render(&source, &path, &diagnostic));
    }
}

/// One diagnostic, as text.
///
/// `ariadne` draws it, exactly as it does for `zdc check` — the reports
/// are this project's best asset and a playground showing bare strings
/// would waste them. It is pure Rust with no filesystem and no terminal
/// probing, so it needed no substitute for this target.
///
/// Colour is off, and asked for explicitly rather than left to
/// `zdc_diagnostics::render`. That function consults `NO_COLOR`, and
/// `std::env::var_os` under WebAssembly answers for an environment that
/// does not exist — so the choice would be made by an accident of the
/// target rather than by this crate. The host wraps the text in HTML;
/// ANSI escapes would arrive as mojibake.
fn render(source: &str, path: &str, diagnostic: &Diagnostic) -> String {
    zdc_diagnostics::render_in_colour(source, path, diagnostic, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host build and the WebAssembly build run the same library; the
    /// binary is a stdin/stdout wrapper around this one function. So these
    /// tests cover the browser's path without a browser, and what they do
    /// not cover is exactly the WASI shim.
    fn compiled(source: &str) -> String {
        compile_to_json(source)
    }

    /// **The playground ships an empty container, whatever the build graph
    /// decided about features.**
    ///
    /// This crate takes `zdc-codegen` with `default-features = false` so a
    /// WASM build links no JavaScript engine. That is not enough on its
    /// own: Cargo unifies features across a workspace, so under
    /// `cargo test --workspace` this compiles against a codegen with
    /// `evaluate` on, and the first paint — which runs the emitted program
    /// in an engine — started happening here. It went unnoticed because a
    /// single-crate `cargo test -p zdc-wasm` does not unify anything and
    /// passes either way, which is the shape of bug a per-crate run cannot
    /// see.
    ///
    /// So the guarantee is asked for by name (`without_first_paint`) and
    /// asserted here rather than inferred from a dependency declaration.
    #[test]
    fn the_playground_never_paints_on_the_build_host() {
        let json =
            compiled("state name is client Text starting \"world\"\n\nview\n    Text name\n");
        assert!(json.contains(r#"<div id=\"app\"></div>"#), "{json}");
        // Not merely "no markup": the painted form would have the view's
        // own text inside the container, so name the thing that must not
        // be there.
        assert!(
            !json.contains(r#"<div id=\"app\">world"#),
            "the playground shipped a painted container: {json}"
        );
    }

    /// The smallest program that renders, end to end.
    #[test]
    fn a_client_program_produces_a_runnable_bundle() {
        let json =
            compiled("state name is client Text starting \"world\"\n\nview\n    Text name\n");
        assert!(json.contains(r#""ok":true"#), "{json}");
        assert!(json.contains(r#""can":true"#), "{json}");
        // The page mounts the module, and the module exports what the page
        // imports. Either half missing is a bundle that loads and throws.
        assert!(json.contains("export function main"), "{json}");
        assert!(json.contains(r#"<div id=\"app\"></div>"#), "{json}");
        // `boot.js` is what the page's one `<script>` names. Without it in
        // the bundle the document loads and 404s, which is exactly the
        // failure a browser test would have found and a unit test would
        // not (#146).
        assert!(json.contains(r#""path":"boot.js""#), "{json}");
        // The reactivity core is always reached, and its source travels
        // with the answer because there is no server to fetch it from.
        assert!(json.contains(r#""path":"runtime/signal.js""#), "{json}");
        assert!(json.contains(r#""diagnostics":"""#), "{json}");
    }

    /// The panel this playground exists for: where each declaration went,
    /// and what a read of it costs from the browser.
    #[test]
    fn the_placement_table_names_each_signal_and_what_a_browser_read_costs() {
        // `visits` is *read* by the view on purpose. A durable signal
        // nothing reads yields no endpoint at all — §16.3.1's dead-code
        // rule reaching the network — and a fixture that did not read it
        // would have asserted against an empty table.
        let json = compiled(
            "state count is client Whole starting 0\nstate visits is durable Whole starting 0\n\nview\n    Column\n        Text count\n        when visits\n            Loading show Spinner\n            Failed with error show Text \"no\"\n            Ready with total show Text total\n",
        );
        assert!(json.contains(r#""name":"count""#), "{json}");
        assert!(json.contains(r#""placement":"client""#), "{json}");
        assert!(json.contains(r#""placement":"durable""#), "{json}");
        // The claim the column exists to make, in `zdc-doc`'s own words:
        // reading durable state from the browser is a round trip and the
        // type says so.
        assert!(
            json.contains(r#"Remote of Whole` — the network is here"#),
            "{json}"
        );
        // Nobody wrote this endpoint. The split derived it, and the file
        // it names is in the bundle above it.
        assert!(
            json.contains(r#"a value the browser reads — `visits`"#),
            "{json}"
        );
        assert!(json.contains(r#""file":"functions/visits.js""#), "{json}");
        // The durable key a deployment would have to provision, read off
        // the bundle rather than off the source.
        assert!(json.contains(r#""durable":["visits"]"#), "{json}");
    }

    /// A `durable` program compiles, emits its endpoints, and is refused a
    /// run — with the reason, rather than a page of failed reads.
    #[test]
    fn a_durable_program_compiles_and_is_refused_a_run() {
        let json = compiled(
            "state visits is durable Whole starting 0\n\nview\n    when visits\n        Loading show Spinner\n        Failed with error show Text \"no\"\n        Ready with total show Text total\n",
        );
        assert!(json.contains(r#""ok":true"#), "{json}");
        assert!(json.contains(r#""can":false"#), "{json}");
        assert!(
            json.contains("needs a host this page does not have"),
            "{json}"
        );
        // The endpoint file is in the bundle, because reading it is what
        // the refusal sends the reader to do.
        assert!(json.contains(r#""path":"functions/visits.js""#), "{json}");
    }

    /// A program that does not compile returns its diagnostics rather than
    /// nothing — the case the playground exists to show off.
    #[test]
    fn a_broken_program_returns_a_rendered_diagnostic() {
        let json = compiled("view\n    Text nowhere\n");
        assert!(json.contains(r#""ok":false"#), "{json}");
        assert!(json.contains("nowhere"), "{json}");
        // Rendered, not just listed: the caret row is what makes these
        // worth showing, and it has to survive JSON encoding.
        assert!(json.contains("playground.zd"), "{json}");
        assert!(json.contains("─"), "{json}");
        // Colour off. An ANSI escape would reach the page as mojibake,
        // and `render_in_colour(.., false)` is the only thing stopping
        // it — `render` consults an environment WebAssembly does not
        // have, so the answer would be the target's accident.
        assert!(!json.contains('\u{1b}'), "{json}");
    }

    /// A `secret` reaching a client output is refused, and the refusal is
    /// the demo that makes the language's point in one screen.
    #[test]
    fn a_secret_read_from_the_view_is_refused() {
        let json = compiled(
            "secret state apiKey is server Text from environment \"KEY\"\n\nview\n    Text apiKey\n",
        );
        assert!(json.contains(r#""ok":false"#), "{json}");
        assert!(json.contains("apiKey"), "{json}");
        // The information-flow code, so the page can say which rule this
        // was rather than only that something was refused.
        assert!(json.contains("IFC"), "{json}");
    }

    /// `static` is refused by name rather than emitted as `undefined`, and
    /// the placement table survives the refusal.
    #[test]
    fn a_static_signal_is_refused_by_name_and_still_reports_its_placement() {
        let json = compiled("state slugs is static List of Text starting [\"a\"]\n\nview\n    each slug in slugs\n        Text slug\n");
        assert!(json.contains(r#""ok":false"#), "{json}");
        assert!(json.contains("slugs"), "{json}");
        assert!(json.contains("17.4.8"), "{json}");
        // The point of separating the two: the emitter declined and the
        // split still answered.
        assert!(json.contains(r#""placement":"static""#), "{json}");
    }

    /// Whatever the source, the answer is always parseable JSON with the
    /// keys the host reads. A compiler that returned a truncated document
    /// would fail in the host's `JSON.parse`, far from its cause.
    #[test]
    fn every_answer_carries_every_key() {
        for source in [
            "",
            "view",
            "not a program at all",
            "view\n    Text \"\\\"\"\n",
        ] {
            let json = compile_to_json(source);
            for key in [
                "\"ok\"",
                "\"diagnostics\"",
                "\"placement\"",
                "\"bundle\"",
                "\"run\"",
            ] {
                assert!(json.contains(key), "{key} missing for {source:?}: {json}");
            }
        }
    }

    /// The one refusal this crate turns into a `Diagnostic`, held to the
    /// inline budget `check-message-budget.py` enforces everywhere else.
    ///
    /// That script scans the shapes the compiler builds diagnostics with —
    /// `GraphError::new`, a `message:` field — and this one is a `format!`
    /// handed to `Diagnostic::file_error`, so it is outside the scan and
    /// nothing but this test holds the line. The long version of what a
    /// `static` costs here belongs in the playground's own prose, where it
    /// is free to the reader who does not want it.
    #[test]
    fn the_static_refusal_is_within_the_inline_budget() {
        let message = unevaluated(&["slugs".to_string(), "posts".to_string()]);
        assert!(message.contains("slugs") && message.contains("posts"));
        assert!(
            message.chars().count() <= 200,
            "{} characters: {message}",
            message.chars().count()
        );
    }
}
