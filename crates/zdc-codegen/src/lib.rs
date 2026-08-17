#![forbid(unsafe_code)]

//! JavaScript and CSS emission, per spec §16.
//!
//! **What this crate emits**, which is more than the view it started as:
//!
//! | | |
//! |---|---|
//! | `view`, `pages`, `elements`, `style` | the document, its bindings and its stylesheet |
//! | `server` | one file per emitted server root — §17.2.3's endpoints and commands |
//! | `build`, `evaluate`, `capability` | the build root: `static` state evaluated at compile time, and the closed capability set (`build read`, `build list`, `build markdown`) that lets it reach the project directory |
//! | `assets` | the one part of this crate that touches the filesystem, kept separate for that reason — `compile` reads no file and takes its result as data |
//! | `intrinsics`, `expr`, `stmt`, `names`, `js` | the shared machinery underneath all of it |
//!
//! Three of those postdate the sentence this doc used to be. It described
//! template cloning and document emission and nothing else, which was the
//! whole crate once and had not been for some time (#14).
//!
//! **Template cloning.** A view region compiles to one static HTML string,
//! parsed once into a `<template>`, cloned per instantiation, walked to
//! compile-time-computed `firstChild`/`nextSibling` offsets, with reactive
//! bindings attached only at the holes. Generated code never imports
//! `elements.js`: emitting calls into it would forfeit §14A.1's
//! monomorphic-shape claim by construction, because `props()` builds its
//! object by dynamic key insertion and `el`'s `Object.entries` then goes
//! megamorphic across the built-ins.
//!
//! **One document or many.** [`compile`] prints the one document an
//! unrouted program is. [`compile_site`] prints one per URL a `route`
//! declares, through the same emitter: a routed page and an unrouted one
//! cannot drift in what they say, because there is no second code path for
//! one of them to be wrong in.

//! **One optional half.** `evaluate` and `capability` are behind the
//! crate's `evaluate` feature, on by default. Everything else — the whole
//! path from HIR to `client.js`, `styles.css` and `index.html` — is pure
//! computation over data the caller hands in, and turning the feature off
//! is what lets that path be compiled to WebAssembly and run in a browser
//! (#171). The feature's comment in `Cargo.toml` says what the browser
//! gives up by dropping it.

mod analysis;
pub mod assets;
mod build;
#[cfg(feature = "evaluate")]
mod capability;
mod claim;
mod elements;
#[cfg(feature = "evaluate")]
mod evaluate;
mod events;
mod expr;
mod intrinsics;
mod js;
mod names;
mod pages;
#[cfg(feature = "evaluate")]
mod prerender;
mod server;
mod stmt;
mod style;
mod styles;
mod tailgroup;
mod view;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use zdc_graph::{Cleared, EndpointKind, RootId, TierSplit, Verdict, CLIENT};
use zdc_hir::{DefId, DefKind, Hir, HirNode, Metadata, ModuleTarget, View};
use zdc_lexer::Span;
use zdc_types::{SignalPlacement, TypeTable};

use crate::analysis::{Analysis, Shared};
use crate::expr::Emitter;
use crate::names::Names;
use crate::pages::Bindings;
use crate::stmt::Statements;
use crate::styles::Styles;
use crate::view::{Emission, Lowering, RuntimeImports};

pub use crate::build::{BuildModule, Claim};
pub use crate::elements::{BUILT_INS, HEADING_TAGS};
// Outside the feature on purpose: a `Broken` is what a *report* is made
// of, and `zdc-diagnostics` renders one without an engine. See
// `claim.rs`.
pub use crate::claim::Broken;
#[cfg(feature = "evaluate")]
pub use crate::evaluate::{evaluate, run_tests, ClaimVerdict, Evaluated, EvaluationError, Outcome};
pub use crate::server::{file_name, FunctionKind, ServerFunction};
// The one URL scheme set lives in `zdc-hir`: `zdc-graph`'s
// information-flow pass and this crate both rule on the same URLs and
// neither crate depends on the other. Re-exported rather than restated
// so a caller here reads the same list the flow pass does.
pub use zdc_hir::{url_is_safe, url_scheme, URL_SCHEMES};
// Which build the runtime is emitted for (#140). Re-exported so a caller
// that already depends on this crate does not have to add a dependency on
// `zdc-runtime` to name the mode it wants.
pub use zdc_runtime::Mode;
// The minifier (#135), re-exported for the same reason `Mode` is. It lives
// in `zdc-runtime` because that crate owns the JavaScript text this
// workspace ships — the runtime modules are minified by `for_mode`, and
// emission is minified by `Bundle::minified` below, and both have to be
// the same rules or the two halves of one bundle disagree about what
// JavaScript is.
pub use zdc_runtime::minify;

/// The tag a built-in becomes, at the top of a document.
///
/// A heading's tag is its nesting depth, so this reports the level it
/// takes when nothing encloses it; `HEADING_TAGS` is the rest.
pub fn tag_of(name: &str) -> Option<&'static str> {
    crate::elements::shape(name).map(|shape| shape.tag)
}

/// A reason a program could not be compiled, pointing at the source that
/// caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenError {
    pub message: String,
    pub span: Span,
}

/// What to compile.
pub struct Options {
    /// The path shown in the generated file's header comment.
    pub source_path: String,
    /// The fallback page title: the source file's stem, used when the
    /// `view` does not give one.
    pub name: String,
    /// What the build host computed for each `static` signal, by source
    /// name, as JSON — §17.4.8.
    ///
    /// Empty for a program with no `static` state, and empty when printing
    /// the build root itself, which is what computes them. A `static` read
    /// with no answer here is a codegen error rather than a guess: an
    /// inlined `undefined` is a blank page three layers from its cause.
    pub statics: BTreeMap<String, String>,
    /// Stylesheets to link after `styles.css`, as hrefs relative to the
    /// bundle root — the `.css` files under the program's asset directory,
    /// which `assets::discover` finds. `compile` reads no file itself, so
    /// the list arrives as data and its order is the cascade order.
    pub stylesheets: Vec<String>,
    /// The site's icon, as a root-absolute href, if it has one.
    pub icon: Option<String>,
    /// Whether to paint the document on the build host.
    ///
    /// **A build wants this and a caller that throws the page away does
    /// not.** Painting means *running the emitted program* in a JavaScript
    /// engine, which is real work and is a step of `zdc build` rather than
    /// of every caller that happens to link this crate.
    ///
    /// It has to be an option and cannot be left to the `evaluate` feature.
    /// `zdc-wasm` depends on this crate with `default-features = false`
    /// precisely to keep an engine out of a WASM build, and that does not
    /// hold: Cargo unifies features across a workspace build, so a
    /// `cargo test --workspace` compiles `zdc-wasm` against a codegen with
    /// `evaluate` on and the playground silently starts shipping a first
    /// paint. Asking is a decision the caller states; a feature is one the
    /// build graph can flip underneath it.
    ///
    /// Nothing downstream depends on the answer. The client builds the same
    /// tree whether or not a document arrived painted.
    pub first_paint: bool,
}

impl Options {
    pub fn new(source_path: impl Into<String>, name: impl Into<String>) -> Options {
        Options {
            source_path: source_path.into(),
            name: name.into(),
            statics: BTreeMap::new(),
            stylesheets: Vec::new(),
            icon: None,
            first_paint: true,
        }
    }

    /// Skip the first paint: what a caller that throws the page away wants.
    pub fn without_first_paint(mut self) -> Options {
        self.first_paint = false;
        self
    }

    pub fn with_statics(mut self, statics: BTreeMap<String, String>) -> Options {
        self.statics = statics;
        self
    }

    /// The stylesheets the asset directory contributed, in cascade order.
    pub fn with_icon(mut self, icon: Option<String>) -> Options {
        self.icon = icon;
        self
    }

    pub fn with_stylesheets(mut self, stylesheets: Vec<String>) -> Options {
        self.stylesheets = stylesheets;
        self
    }
}

/// One `foreign` module a bundle imports by relative path, and where it
/// has to land for that import to resolve.
///
/// The emitter writes the author's specifier verbatim, so the destination
/// is that specifier resolved against the directory of the file doing the
/// importing: `client.js` sits at the bundle root, an endpoint sits in
/// `functions/`, and the same module imported from both is shipped twice.
/// Duplicating it is the same trade §16.3.12 already accepts for colourless
/// functions — the alternative is a shared module, which is the import edge
/// invariant 4 exists to keep out.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LinkedModule {
    /// Exactly as written in the `from` clause, such as `./io.js`.
    /// Resolved against the entry file's directory by the caller, under
    /// the same sandbox rules a `build read` path gets — it comes from the
    /// source text, so it is a path a *program* named.
    pub specifier: String,
    /// Where it goes, relative to the bundle root, such as
    /// `functions/io.js`.
    pub destination: String,
}

/// The module a relative specifier names, and where it lands in a bundle
/// whose importing file sits in `dir` (empty for the bundle root).
///
/// `None` for a bare specifier such as `marked`, and for a URL: neither
/// names a file this build owns, so there is nothing to copy and refusing
/// to guess is the whole of the handling.
///
/// That rule is unchanged by #238 and is the reason the package mapping
/// resolves *before* this is reached. A mapping to `./vendor/marked.js`
/// arrives here as that path, so a vendored copy is shipped by exactly the
/// machinery a directly-written relative specifier already used — the
/// mapping added a way for the project to answer the question, not a
/// second way for the build to copy files.
pub fn linked_module(specifier: &str, dir: &str) -> Option<LinkedModule> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return None;
    }
    // The destination keeps the specifier's own shape so the emitted
    // import resolves without rewriting it. `./io.js` beside an endpoint
    // is `functions/io.js`; the same string beside `client.js` is `io.js`.
    let tail = specifier.trim_start_matches("./");
    let destination = if dir.is_empty() {
        tail.to_string()
    } else {
        format!("{dir}/{tail}")
    };
    Some(LinkedModule {
        specifier: specifier.to_string(),
        destination,
    })
}

/// Everything a build writes out.
pub struct Bundle {
    /// The runtime modules `client_js` reaches, transitively, as paths
    /// relative to the bundle root. [`runtime_files`] turns these into the
    /// sources to write; nothing outside that set is shipped (§16.3.1).
    pub runtime: BTreeSet<&'static str>,
    pub client_js: String,
    pub styles_css: String,
    /// The page, or `None` for a module with no `view`.
    ///
    /// §16.3.1's page is a `<div id=app>` and a script calling `main()`.
    /// A program that renders nothing has no `main`, so writing that page
    /// anyway would ship a document whose only script throws on load —
    /// which is the one artifact here that would be actively *wrong*
    /// rather than merely unused. `styles.css` and the runtime files are
    /// inert, so they are written either way.
    pub index_html: Option<String>,
    /// `boot.js`, the module the page loads, or `None` alongside no page.
    ///
    /// It is the whole of what used to be an inline `<script>`, moved into
    /// a file so the page can carry a policy with no inline-script
    /// exception (#146). It is `Some` exactly when `index_html` is: a
    /// document and the module it names are one artifact.
    pub boot_js: Option<String>,
    pub manifest_json: String,
    /// One file per emitted server root — §17.2.3's `Endpoint` and
    /// `Command` origins. Empty for a program with no crossing, which is
    /// how `hello.zd` still ships nothing it does not use.
    pub functions: Vec<ServerFunction>,
    /// The `foreign` modules this bundle imports by relative path, and
    /// where each has to land for that import to resolve (#223).
    ///
    /// The emitter cannot copy them: `assets.rs` is the one part of this
    /// crate that touches the filesystem, and `compile` takes its result as
    /// data. So the bundle *reports* what must be shipped and the caller
    /// ships it — the same division that already exists for stylesheets.
    ///
    /// A bare specifier such as `marked` is absent, because it names a
    /// package the target resolves rather than a file this build owns.
    pub linked_modules: BTreeSet<LinkedModule>,
    /// Every durable key the program touches, sorted.
    ///
    /// Also in the manifest, because the browser is allowed to know it. It
    /// is repeated here so that a deploy adapter, which has to provision a
    /// store for exactly these keys, does not have to parse its own output
    /// back.
    pub durable: Vec<String>,
    /// Every environment key the program reads, sorted.
    ///
    /// Deliberately **not** in the manifest: §16.3.12 assertion C forbids
    /// an environment key name from reaching the browser. A deploy target
    /// still needs the list, to emit a reference to each one in the
    /// platform's secret store — never a value.
    pub environment: Vec<String>,
}

impl Bundle {
    /// The same bundle with the comments and the formatting taken out —
    /// issue #135.
    ///
    /// # Why this is a step and not something `compile` does
    ///
    /// Minifying is a decision about *shipping*, and the two commands
    /// that ship are `zdc build` and `zdc deploy`. `zdc dev` serves what
    /// `compile` returned, and a developer stepping through `client.js`
    /// in a browser's debugger is looking at the compiler's own emission
    /// — which is also what every emission test in this workspace reads,
    /// and what `zdc-bench` measures the emitter by. Folding minification
    /// into `compile` would have made "what the emitter printed" and
    /// "what a reader downloads" the same string, and they are two
    /// different things worth being able to look at separately.
    ///
    /// It is the same division [`runtime_files`] already makes for the
    /// runtime, and for the same stated reason: the caller names the
    /// build, because the caller is the command.
    ///
    /// # What it leaves alone
    ///
    /// `index.html`, `manifest.json` and the server functions, each for a
    /// reason given in [`minify`]'s module documentation. The runtime is
    /// not here either — it is not part of this struct, and
    /// [`runtime_files`] minifies it under the same `Mode::Release` that
    /// strips its assertions.
    ///
    /// # What it takes that is not whitespace
    ///
    /// The `// zdc … generated, do not edit` header, because it is a
    /// comment and there is no exception for it. That is the right answer
    /// rather than a regrettable one: the line exists to stop someone
    /// editing a file the next build overwrites, and a minified bundle is
    /// not a file anyone edits by hand. `zdc dev` still serves the header,
    /// and so does every emission test, which is where a person actually
    /// reads generated code.
    pub fn minified(self) -> Bundle {
        Bundle {
            client_js: minify::javascript(&self.client_js),
            styles_css: minify::css(&self.styles_css),
            boot_js: self.boot_js.as_deref().map(minify::javascript),
            ..self
        }
    }
}

/// Every diagnostic a build of this program would report, and nothing
/// written out.
///
/// This is what `zdc check` and the language server run, and it is
/// `compile` itself rather than a second implementation of its rules.
/// **That identity is the whole point.** Codegen owns fifty-odd refusals —
/// the injection refusals of §16.3.5, the `only_children`/`only_inside`
/// shape checks of §16.3.6, the placement refusals of §16.5 — and every one
/// of them is raised by the same `match` arm that decides what to emit
/// instead. A validator that re-derived them from `Hir` and `TypeTable`
/// would be the compiler "checking a program twice", which `compile`'s own
/// contract below names as the thing that lets a compiler disagree with
/// itself; here that disagreement has a name, because it is exactly the
/// editor-versus-command-line disagreement §14's language-server section
/// forbids. One traversal, two callers, no second opinion to drift.
///
/// The cost is the emission, paid on every keystroke in the editor. It is
/// paid deliberately: see `crates/zdc-lsp/src/latency.rs`, which measures it.
///
/// # There is no refusal this does not repeat
///
/// There used to be one: a file with no `view` was refused by `compile`
/// and accepted here, on the reading that "this program has no `view`" is
/// an answer to `zdc build`'s question rather than a statement about the
/// program. That is still the right reading, and `compile` no longer needs
/// the exception to hold it — §14D.2 makes every `.zd` file a module, and
/// a module with no `view` is emitted as one, with no page. So the two
/// commands report the same set with nothing subtracted, which is a
/// stronger claim than the one this function was written to make.
pub fn check(inputs: &Inputs<'_>) -> Vec<CodegenError> {
    if inputs.hir.view.is_none() {
        return Vec::new();
    }
    // The options only name the source path and the fallback page title, and
    // both are read after the last refusal is collected, so no diagnostic
    // can depend on them.
    //
    // The `static` values are the exception, and they are stubbed rather
    // than computed. §17.4.8 evaluates the build root in a JavaScript
    // engine, which is a step of `zdc build` and not of a keystroke in an
    // editor — but a `static` read with no answer is a refusal, so leaving
    // the map empty would make `check` reject every program with `static`
    // state that `build` accepts. That is the diagnostic split this
    // function exists to close, pointing the other way. The value is never
    // read: `check` throws the bundle away, and a build-host failure is
    // `evaluate`'s own diagnostic, raised before the emitter runs.
    let mut statics = BTreeMap::new();
    for (_, def) in inputs.hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        if signal.placement == zdc_ast::Placement::Static {
            statics.insert(def.name.clone(), "null".to_string());
        }
    }
    // No first paint: painting runs the emitted program in a JavaScript
    // engine, which is the same per-keystroke cost the stubbed `statics`
    // above exist to avoid. `check` throws the page away regardless, and
    // the language server calls this on every edit.
    let options = Options::new("<check>", "check")
        .with_statics(statics)
        .without_first_paint();
    // Every document, not the first one. A routed program's refusals are
    // per page after specialisation, and a page nobody emitted is a page
    // nobody checked.
    match compile_site(inputs, &options) {
        Ok(_) => Vec::new(),
        Err(errors) => errors,
    }
}

/// One document of a routed program.
///
/// Per-page rather than per-program because §14A's bundle-size argument
/// is the whole reason the design exists: a site that ships every page's
/// code to every visitor has forfeited it. The split is not a bundler
/// pass bolted on afterwards — it falls out of the address fold, because
/// a document whose route is known reaches only the arm it renders.
pub struct PageBundle {
    /// The `foreign` modules this page's `client_js` imports by relative
    /// path (#223). Same contract as [`Bundle::linked_modules`].
    pub linked_modules: BTreeSet<LinkedModule>,
    /// The URL this document is served at.
    pub url: String,
    /// A file-name-safe name for its module and stylesheet.
    pub slug: String,
    pub client_js: String,
    pub styles_css: String,
    /// The document, or `None` for a module with no `view` — the same
    /// artifact, and absent for the same reason, as [`Bundle::index_html`].
    pub document_html: Option<String>,
    /// The module the document loads, or `None` alongside no document
    /// (#146). Written to `pages/<slug>.boot.js`.
    pub boot_js: Option<String>,
}

impl PageBundle {
    /// One document's artifacts, as a release build ships them — #135.
    /// Same rules and same exclusions as [`Bundle::minified`].
    pub fn minified(self) -> PageBundle {
        PageBundle {
            client_js: minify::javascript(&self.client_js),
            styles_css: minify::css(&self.styles_css),
            boot_js: self.boot_js.as_deref().map(minify::javascript),
            ..self
        }
    }
}

/// Every document a program emits, and the map from URL to module.
pub struct SiteBundle {
    /// The `foreign` modules the emitted imports point at, and where each
    /// has to land (#223). Same contract as [`Bundle::linked_modules`].
    pub linked_modules: BTreeSet<LinkedModule>,
    pub pages: Vec<PageBundle>,
    /// URL to module, for a host that has to answer a request without
    /// running the compiler.
    pub routes_json: String,
    /// The union over every page of the runtime modules it reaches. The
    /// runtime is shared by the documents, so the *set* is a union while
    /// each page's import list stays its own (§16.3.1).
    pub runtime: BTreeSet<&'static str>,
    pub manifest_json: String,
    pub functions: Vec<ServerFunction>,
    pub durable: Vec<String>,
    pub environment: Vec<String>,
}

impl SiteBundle {
    /// Every document, as a release build ships it — issue #135.
    ///
    /// `routes.json` and `manifest.json` are left alone: both are emitted
    /// without a space in them already, so there is nothing to take out.
    pub fn minified(self) -> SiteBundle {
        SiteBundle {
            pages: self.pages.into_iter().map(PageBundle::minified).collect(),
            ..self
        }
    }
}

/// Everything emission reads. All four, or it refuses (§17.1.3) — plus
/// the permission to emit at all.
///
/// `Cleared` has no public constructor, so this struct cannot be built
/// without calling [`zdc_graph::Verdict::clearance`] and being given one.
/// That is what makes §16.3.12's invariant 3 a property of the type
/// system: a driver that forgets to look at the verdict does not compile.
pub struct Inputs<'a> {
    pub hir: &'a Hir,
    pub split: &'a TierSplit,
    pub verdict: &'a Verdict,
    pub table: &'a TypeTable,
    /// Proof that *a* flow verdict was clean. Not proof that it was
    /// `verdict`, and it says nothing about `split`, so both are checked
    /// again below.
    pub cleared: Cleared,
}

/// Compile a resolved, split, typed and cleared program.
///
/// Emission is defined as printing `members(r)` for every root with
/// `emitted: true`. That is what makes §14A.1's dead-code claim provable:
/// the client bundle excludes server logic because the walk **stopped** at
/// the crossing, not because a bundler guessed from an import graph.
///
/// The type table is not optional and never reconstructed here. §16.7 lists
/// what code generation is silently wrong without — the operand types of
/// `+` and `is`, the container behind `empty`, the choice a `when`
/// eliminates — and a compiler that answered those itself would be checking
/// a program twice and could disagree with itself about the result.
pub fn compile(inputs: &Inputs<'_>, options: &Options) -> Result<Bundle, Vec<CodegenError>> {
    // `Inputs::cleared` is not read here. The token's whole job is done by
    // the time `Inputs` exists, because it is what made building one
    // possible. What it does *not* prove — that this is the verdict it came
    // from, and that the split agrees — the two checks below prove, for the
    // same reason E-IFC-01 exists.
    refuse_without_a_verdict(inputs.split, inputs.verdict)?;

    let shared = Shared::new(inputs.hir, inputs.table);
    let view = view_of(inputs.hir);
    let metadata = view.map(|view| view.metadata.clone()).unwrap_or_default();
    let nodes = view.map(|view| view.nodes.clone());

    let emitted = emit(
        inputs,
        options,
        nodes.as_deref(),
        &Bindings::default(),
        Layout::Single,
        // `None`, and the reason is not that an unrouted program has no
        // links. It is that this document's URL is not known: a bundle
        // with no `route` is `index.html` and a `client.js` beside it,
        // and a static host may serve that pair from `/`, from `/app/`,
        // or from a preview URL nobody chose. A routed program's URLs are
        // the `route` declaration's own, which is what makes the
        // comparison in `mark_current_page` a fact rather than a guess.
        None,
        &shared,
    )?;

    let durable = durable_keys(inputs.hir, inputs.split);
    // Before the fields move: the prerender reads both.
    let painted = (options.first_paint && nodes.is_some())
        .then(|| painted_markup(&emitted.client_js, &emitted.runtime))
        .flatten();
    Ok(Bundle {
        runtime: emitted.runtime,
        client_js: emitted.client_js,
        styles_css: emitted.styles_css,
        index_html: nodes.is_some().then(|| {
            index_html(
                &metadata,
                options,
                &page_title(options, &metadata, "/"),
                Shell {
                    boot: "./boot.js",
                    styles: "./styles.css",
                    import_map: &emitted.import_map,
                    connect: &emitted.connect_origins,
                    painted: painted.as_deref(),
                },
            )
        }),
        boot_js: nodes.is_some().then(|| boot_js("./client.js")),
        manifest_json: manifest_json(
            inputs.hir,
            &emitted.names,
            &emitted.functions,
            &durable,
            &emitted.transactions,
            &emitted.remote_origins,
            &emitted.connect_origins,
        ),
        linked_modules: emitted
            .linked_modules
            .iter()
            .cloned()
            .chain(
                emitted
                    .functions
                    .iter()
                    .flat_map(|f| f.linked.iter().cloned()),
            )
            .collect(),
        functions: emitted.functions,
        environment: environment_keys(inputs.hir),
        durable,
    })
}

/// Compile every document a program emits.
///
/// An unrouted program is one document at `/`, which is what it has
/// always been; a routed one is one document per enumerated URL plus the
/// not-found page. The two go through the same emitter, so there is no
/// second code path for a routed program to be wrong in.
pub fn compile_site(
    inputs: &Inputs<'_>,
    options: &Options,
) -> Result<SiteBundle, Vec<CodegenError>> {
    refuse_without_a_verdict(inputs.split, inputs.verdict)?;

    let hir = inputs.hir;
    let site = zdc_types::site(hir);
    // One document: a program with no `route`, and a module with no
    // `view`, which has no document at all.
    let Some(view) = view_of(hir).filter(|_| !site.pages.is_empty()) else {
        let bundle = compile(inputs, options)?;
        return Ok(SiteBundle {
            pages: vec![PageBundle {
                linked_modules: bundle.linked_modules.clone(),
                url: "/".to_string(),
                slug: "index".to_string(),
                client_js: bundle.client_js,
                styles_css: bundle.styles_css,
                document_html: bundle.index_html,
                boot_js: bundle.boot_js,
            }],
            linked_modules: bundle.linked_modules,
            routes_json: routes_json(&[("/".to_string(), "index".to_string())], None),
            runtime: bundle.runtime,
            manifest_json: bundle.manifest_json,
            functions: bundle.functions,
            durable: bundle.durable,
            environment: bundle.environment,
        });
    };
    let metadata = view.metadata.clone();
    let nodes = view.nodes.clone();

    // Computed once for the whole program, not once per page. §17.2's
    // split is already quadratic in definitions × roots and routing puts
    // one root per page on that axis; re-running the reactive-function
    // fixpoint per page would make it cubic, which is the one thing that
    // would bite at a realistic page count.
    let shared = Shared::new(hir, inputs.table);
    let mut pages = Vec::with_capacity(site.pages.len());
    let mut errors = Vec::new();
    let mut index = Vec::new();
    let mut not_found = None;
    let mut runtime: BTreeSet<&'static str> = BTreeSet::new();
    let mut functions: Vec<ServerFunction> = Vec::new();
    // Unlike the endpoints, a handler is a property of the document it
    // sits in: routing specialises the view per URL, so a page carries
    // only the handlers its own nodes declare. The manifest describes the
    // program, so the write sets are unioned over the pages, and a handler
    // that survives specialisation onto several pages is recorded once.
    let mut transactions: Vec<HandlerWrites> = Vec::new();
    let mut names: Option<Names> = None;
    // The manifest describes the program, so a package one page imports is
    // a package this bundle fetches (#238). The *map* stays per document,
    // because a page that imports nothing should carry no map at all.
    let mut remote_origins: BTreeSet<String> = BTreeSet::new();
    // The same union for the origins a request reaches, and for the same
    // reason: the manifest describes the program. The *policy* stays per
    // document, because a page that declares no request must carry the
    // narrower one.
    let mut connect_origins: BTreeSet<String> = BTreeSet::new();
    for page in &site.pages {
        let specialised = pages::specialise(hir, &nodes, page);
        let module = format!("/pages/{}.js", page.slug);
        let boot = format!("/pages/{}.boot.js", page.slug);
        let styles = format!("/pages/{}.css", page.slug);
        match emit(
            inputs,
            options,
            Some(&specialised.nodes),
            &specialised.bindings,
            Layout::Page,
            Some(&page.url),
            &shared,
        ) {
            Ok(emitted) => {
                if page.variant.is_none() {
                    not_found = Some(page.url.clone());
                }
                index.push((page.url.clone(), page.slug.clone()));
                runtime.extend(emitted.runtime.iter().copied());
                // The endpoints are the program's, not the page's: the
                // split names them once, and every document that reaches
                // one reaches the same file.
                if functions.is_empty() {
                    functions = emitted.functions;
                }
                for handler in emitted.transactions {
                    if !transactions.contains(&handler) {
                        transactions.push(handler);
                    }
                }
                names.get_or_insert(emitted.names);
                remote_origins.extend(emitted.remote_origins);
                connect_origins.extend(emitted.connect_origins.iter().cloned());
                // Before the fields move: the prerender reads two of them.
                let painted = options
                    .first_paint
                    .then(|| painted_markup(&emitted.client_js, &emitted.runtime))
                    .flatten();
                pages.push(PageBundle {
                    linked_modules: emitted.linked_modules,
                    url: page.url.clone(),
                    slug: page.slug.clone(),
                    client_js: emitted.client_js,
                    styles_css: emitted.styles_css,
                    document_html: Some(index_html(
                        &metadata,
                        options,
                        &page_title(options, &metadata, &page.url),
                        Shell {
                            boot: &boot,
                            styles: &styles,
                            import_map: &emitted.import_map,
                            connect: &emitted.connect_origins,
                            painted: painted.as_deref(),
                        },
                    )),
                    boot_js: Some(boot_js(&module)),
                });
            }
            Err(found) => errors.extend(found),
        }
    }
    if !errors.is_empty() {
        // One document's mistake is every document's mistake — the
        // program is one program — so the whole list is reported once
        // rather than once per page.
        errors.dedup_by(|a, b| a.message == b.message && a.span == b.span);
        return Err(errors);
    }

    let durable = durable_keys(hir, inputs.split);
    let names = match names {
        Some(names) => names,
        None => {
            let analysis = Analysis::whole(hir, &shared);
            Names::new(hir, &analysis, &BTreeSet::new())
        }
    };
    Ok(SiteBundle {
        linked_modules: pages
            .iter()
            .flat_map(|p| p.linked_modules.iter().cloned())
            .chain(functions.iter().flat_map(|f| f.linked.iter().cloned()))
            .collect(),
        routes_json: routes_json(&index, not_found.as_deref()),
        manifest_json: manifest_json(
            hir,
            &names,
            &functions,
            &durable,
            &transactions,
            &remote_origins,
            &connect_origins,
        ),
        environment: environment_keys(hir),
        durable,
        functions,
        runtime,
        pages,
    })
}

/// Where a document's module and stylesheet sit relative to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// `dist/client.js` beside `dist/index.html`.
    Single,
    /// `dist/pages/<slug>.js`, one directory below the site root, with
    /// the runtime shared by every page.
    Page,
}

impl Layout {
    fn runtime(self) -> &'static str {
        match self {
            Layout::Single => "./runtime",
            Layout::Page => "../runtime",
        }
    }
}

struct Emitted {
    client_js: String,
    /// The `foreign` modules `client_js` imports by relative path (#223).
    linked_modules: BTreeSet<LinkedModule>,
    /// Bare specifier to the target the project mapped it to, for the
    /// packages this document actually imports (#238).
    ///
    /// Only the ones imported, not the whole of `[packages]`: a map naming
    /// a module the page never loads tells the browser to reserve a name
    /// for nothing, and tells a reader the page fetches something it does
    /// not.
    import_map: BTreeMap<String, String>,
    /// Every remote origin this document's imports reach, client and
    /// server together (#238).
    remote_origins: BTreeSet<String>,
    /// Every origin this document's `request` declarations fetch from,
    /// which is what widens `connect-src` (#19).
    ///
    /// A different set from `remote_origins`, and deliberately not folded
    /// into it: that one is where the page loads *modules* from and
    /// governs `script-src` and the import map, this one is where it sends
    /// *requests* to. Merging them would let a `foreign` module's origin
    /// widen `connect-src`, which no program asked for.
    connect_origins: BTreeSet<String>,
    styles_css: String,
    names: Names,
    runtime: BTreeSet<&'static str>,
    functions: Vec<ServerFunction>,
    transactions: Vec<HandlerWrites>,
}

/// Where a called `foreign`'s specifier resolves, as resolution decided
/// (#238).
///
/// Read off the definition rather than recomputed here. The project's
/// package mapping is a file on disk, and an emitter that consulted it
/// would be a second reader of it — so a caller who built without one
/// would emit a bundle whose first import failed, which is precisely the
/// "compiles and cannot load" outcome the mapping exists to end. One
/// answer, decided once, carried on the definition.
/// `None` for a method, which imports nothing and so resolves nothing.
fn foreign_target(hir: &Hir, def: DefId) -> Option<ModuleTarget> {
    let DefKind::Foreign(foreign) = &hir.defs[def].kind else {
        unreachable!("only a foreign is imported");
    };
    foreign.target.clone()
}

/// The origin a specifier fetches from, for the ones that fetch.
///
/// The origin rather than the whole URL, because that is the unit the
/// question is asked in: a deploy target writing a Content-Security-Policy
/// needs the host, a reader auditing what a page talks to needs the host,
/// and neither is served by a list of paths under it. Two imports from one
/// CDN are one entry.
///
/// `None` for everything else — a path, `zd:`, a bare specifier — because
/// none of them is fetched from anywhere the page did not already ship.
fn remote_origin(specifier: &str) -> Option<String> {
    let rest = ["https://", "http://"]
        .iter()
        .find_map(|scheme| specifier.strip_prefix(scheme).map(|rest| (scheme, rest)));
    let (scheme, rest) = rest?;
    // Up to the first `/`, which per RFC 3986 ends the authority. A URL
    // with no path at all — `https://esm.sh` — is its own authority.
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}{authority}"))
}

/// The `view` a program renders, or `None` for a module.
///
/// A module with no `view` is a legitimate program shape, not an error:
/// §14D.2 makes every `.zd` file a module and every top-level declaration
/// importable, so a file that declares types and functions and renders
/// nothing is exactly what the importing file names after `for`. Building
/// it emits the module and stops there — no page, no `main`, and no `view`
/// walk to run.
fn view_of(hir: &Hir) -> Option<&View> {
    let view = hir.view?;
    let DefKind::View(view) = &hir.defs[view].kind else {
        unreachable!("`Hir::view` names a view");
    };
    Some(view)
}

/// The `<title>` of one document.
///
/// A routed program has one `view` and therefore one declared title, so
/// the URL is what distinguishes the documents from one another. The root
/// document is the site itself and carries the title unqualified.
fn page_title(options: &Options, metadata: &Metadata, url: &str) -> String {
    let base = metadata
        .title
        .clone()
        .unwrap_or_else(|| options.name.clone());
    if url == "/" {
        return base;
    }
    format!("{base} · {url}")
}

/// One document: its module, its stylesheet, and the server halves the
/// split derived from the program it belongs to.
fn emit(
    inputs: &Inputs<'_>,
    options: &Options,
    nodes: Option<&[HirNode]>,
    bindings: &Bindings,
    layout: Layout,
    // The URL this document is served at. A `Link` whose destination
    // renders to it is the link to the page a reader is already on, which
    // is the one fact `aria-current` exists to state (#142). Passed rather
    // than derived: only the caller emitting a routed program's documents
    // knows which one this is.
    page_url: Option<&str>,
    shared: &Shared,
) -> Result<Emitted, Vec<CodegenError>> {
    let hir = inputs.hir;
    let split = inputs.split;
    let table = inputs.table;

    let roots = nodes.unwrap_or(&[]);
    let statics = statics_by_def(hir, &options.statics);
    let analysis = match layout {
        Layout::Single => Analysis::whole(hir, shared),
        Layout::Page => Analysis::page(hir, roots, bindings, shared),
    };

    // The split proved what the program's own code reaches. It could not
    // prove what a type-directed operator reaches, because the checker had
    // not run yet, so that part of the closure is added here — seeded from
    // this root's own members, so the bundle grows only by what it can
    // actually reach (§17.4.5).
    let mut client_members = split.client_members();
    client_members.extend(analysis.operator_closure(hir, &client_members));
    // A document reaches only the arm it renders, so the page walk
    // *narrows* the split's answer. It never widens it: the intersection
    // cannot readmit a definition the split stopped at, which is what
    // keeps §14A.1's exclusion an exclusion.
    if layout == Layout::Page {
        client_members.retain(|id| analysis.client_closure().contains(id));
    }
    // A module with no `view` has no root for the split to walk from, so
    // the split names nothing and the module would come out empty. Its
    // use is the importing file's `for` list, which is outside this
    // compilation unit — so §14D.2's rule applies instead: every
    // top-level declaration is importable, and the walk seeds from all of
    // them. Nothing crosses a boundary here, because a module with no
    // view has no client for anything to cross to.
    if hir.view.is_none() {
        client_members.extend(analysis.client_closure().iter().copied());
    }

    let names = Names::new(hir, &analysis, &client_members);
    let mut emitter = Emitter {
        hir,
        types: table,
        names: &names,
        analysis: &analysis,
        bindings,
        used: RuntimeImports::default(),
        split,
        ctx: split.root(CLIENT).ctx,
        root: CLIENT,
        statics: &statics,
        read_statics: BTreeSet::new(),
        errors: Vec::new(),
        transactions: Vec::new(),
        media: BTreeMap::new(),
        scroll: false,
    };

    let mut styles = Styles::default();
    let region =
        nodes.map(|nodes| Lowering::new(&mut emitter, &mut styles, page_url).region(nodes));

    let is_module = region.is_none();
    let functions = emit_functions(&mut emitter, &client_members, is_module);
    let declarations = emit_declarations(&mut emitter, &client_members, is_module);
    let remotes = emit_remotes(&mut emitter);

    // The server roots, emitted last so every diagnostic from the client
    // walk is already collected and the two lists come out together.
    //
    // The endpoints' own foreign imports come back with them. They are not
    // folded into the client emitter's set — an endpoint is a separate
    // bundle and its imports are not the browser's — but what a page's
    // *server half* fetches is still something this bundle imports, so the
    // manifest's origin list is the union of the two (#238).
    let (server, server_foreign) = {
        let mut server_emitter = Emitter {
            hir,
            types: table,
            names: &names,
            analysis: &analysis,
            bindings,
            used: RuntimeImports::default(),
            split,
            ctx: split.root(CLIENT).ctx,
            root: CLIENT,
            statics: &statics,
            read_statics: BTreeSet::new(),
            errors: Vec::new(),
            transactions: Vec::new(),
            media: BTreeMap::new(),
            scroll: false,
        };
        let served = emit_server(
            hir,
            split,
            &names,
            &mut server_emitter,
            &options.source_path,
        );
        emitter.errors.extend(server_emitter.errors);
        (served, server_emitter.used.foreign)
    };

    let errors = std::mem::take(&mut emitter.errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut used = std::mem::take(&mut emitter.used);

    let mut templates: Vec<(String, bool)> = Vec::new();
    let mut by_position = false;
    let mut main = None;
    if let Some(region) = region {
        let mut emission = Emission::new(&mut used);
        // The *root*: it binds against a prerendered container when there
        // is one, clones when there is not, and emits its own return
        // because the two paths mount at different moments.
        let body = emission.root_instance(&region, "$r", 2);
        templates = emission.templates().to_vec();
        by_position = emission.needs_by_position();
        main = Some(body);
    }

    let mut client_js = String::new();
    let mut linked_modules: BTreeSet<LinkedModule> = BTreeSet::new();
    client_js.push_str(&format!(
        "// zdc {} · {} · generated, do not edit\n",
        env!("CARGO_PKG_VERSION"),
        options.source_path
    ));
    let runtime_root = layout.runtime();
    if !used.signal.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.signal.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/signal.js"))
        ));
    }
    if !used.dom.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.dom.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/dom.js"))
        ));
    }
    if !used.lifecycle.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.lifecycle
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(", "),
            js::string(&format!("{runtime_root}/foreign.js"))
        ));
    }
    if !used.rendered.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.rendered.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/markup.js"))
        ));
    }
    if !used.reconcile.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.reconcile
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(", "),
            js::string(&format!("{runtime_root}/list.js"))
        ));
    }
    if !used.clock.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.clock.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/clock.js"))
        ));
    }
    if !used.keys.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.keys.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/keys.js"))
        ));
    }
    if !used.request.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.request.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/request.js"))
        ));
    }
    if !used.rpc.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.rpc.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/rpc.js"))
        ));
    }
    if !used.store.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.store.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/store.js"))
        ));
    }
    if !used.remembered.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.remembered
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(", "),
            js::string(&format!("{runtime_root}/remembered.js"))
        ));
    }
    if !used.media.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.media.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/media.js"))
        ));
    }
    if !used.viewport.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.viewport.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/viewport.js"))
        ));
    }
    if !used.scene.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.scene.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/scene.js"))
        ));
    }
    if !used.vector.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from {};\n",
            used.vector.iter().copied().collect::<Vec<_>>().join(", "),
            js::string(&format!("{runtime_root}/vector.js"))
        ));
    }
    // §14E: a foreign the emission actually called. The export is a
    // validated `js::ident` — an `import` clause takes it as syntax, so
    // nothing here can escape it — while the module specifier is a string
    // literal and `js::string` owns its quotes.
    let mut import_map: BTreeMap<String, String> = BTreeMap::new();
    let mut remote_origins: BTreeSet<String> = BTreeSet::new();
    for (def, (module, export)) in &used.foreign {
        let local = names.def(*def);
        let export = js::ident(export)
            .expect("the export was validated at parse time and again at emission");
        // The specifier the program wrote, always — including for a bare
        // one, which the import map resolves. Substituting the target here
        // instead would work and would be worse: two declarations importing
        // one package would name two URLs, and the browser would fetch and
        // instantiate the module twice, with two copies of its state.
        // A relative specifier is written as the program wrote it only when
        // the document *is* the bundle root. A routed document sits at its
        // own URL — `/writing/<slug>/` — and its page module lives in
        // `/pages/`, so `./rings.js` there resolves to `/pages/rings.js`
        // and 404s while the file sits at the root. The module fails to
        // load, the whole page bundle fails with it, and what renders is a
        // blank body with a console error naming a path nothing wrote.
        //
        // This is the same class of bug as the asset stylesheet's, and the
        // same repair: root-absolute, matching what the page's own module
        // and stylesheet already use. The vendored branch below already did
        // this; a directly-written path had been left behind.
        //
        // A bare specifier is untouched, because it is resolved by the
        // import map rather than by the URL.
        let written = match (layout, linked_module(module, "")) {
            (Layout::Page, Some(linked)) => format!("/{}", linked.destination),
            _ => module.clone(),
        };
        client_js.push_str(&format!(
            "import {{ {export} as {local} }} from {};\n",
            js::string(&written)
        ));
        // `client.js` sits at the bundle root, so a relative specifier
        // lands beside it (#223).
        linked_modules.extend(linked_module(module, ""));
        remote_origins.extend(remote_origin(module));

        let Some(ModuleTarget::Mapped(target)) = &foreign_target(hir, *def) else {
            continue;
        };
        // A vendored target is a file this build ships, so it goes through
        // #223's machinery exactly as a directly-written path does — and
        // the map has to point the browser at where it *landed*, which is
        // not where the mapping named it from when the document sits
        // below the bundle root.
        let mapped = match linked_module(target, "") {
            Some(module) => {
                let destination = module.destination.clone();
                linked_modules.insert(module);
                match layout {
                    // The document is the bundle root, so the specifier
                    // resolves against it unchanged — and stays relative,
                    // which keeps the bundle openable over `file:`.
                    Layout::Single => target.clone(),
                    // A routed document sits at its own URL, one or more
                    // directories down, so a relative target would resolve
                    // against that URL instead. Root-absolute is what the
                    // page's own module and stylesheet already use.
                    Layout::Page => format!("/{destination}"),
                }
            }
            None => target.clone(),
        };
        remote_origins.extend(remote_origin(target));
        import_map.insert(module.clone(), mapped);
    }
    // What the endpoints fetch is fetched by this bundle too, so it is
    // reported. It is deliberately not added to `import_map`: that map is
    // the browser's, and an endpoint resolves its own imports by having the
    // target substituted into them (see `server.rs`).
    for (def, (module, _)) in &server_foreign {
        remote_origins.extend(remote_origin(module));
        if let Some(ModuleTarget::Mapped(target)) = &foreign_target(hir, *def) {
            remote_origins.extend(remote_origin(target));
        }
    }
    // Hoisted `static` values, before anything that could read one.
    //
    // A `static` is a constant and inlining a constant is right for a
    // number or a short string — no indirection, no name, nothing to look
    // up. It is catastrophic for a list: a blog's fourteen posts read
    // nine times on one page put the same ninety-eight kilobytes into the
    // bundle nine times, and that page came to a megabyte of which seven
    // eighths was one value repeated.
    //
    // Declared from what the emission *read* rather than from the split's
    // member list, because the split makes a `static` an inlined member
    // and inlined members are in no bundle's member list — there was
    // nothing to declare until this existed.
    if !emitter.read_statics.is_empty() {
        client_js.push('\n');
        for def in &emitter.read_statics {
            let name = emitter.names.def(*def);
            // unreached: a name is recorded only where its value was
            // looked up, so the map has it.
            if let Some(json) = emitter.statics.get(def) {
                client_js.push_str(&format!("const {name} = {};\n", js::literal(json)));
            }
        }
    }
    if !templates.is_empty() {
        client_js.push('\n');
        for (index, (html, svg)) in templates.iter().enumerate() {
            // The second argument is passed only when it is needed, so a
            // program that draws nothing reads the same as it always did.
            // Two names rather than a flag: the SVG one is its own
            // module, so a program that draws nothing must not so much as
            // mention it.
            let builder = if *svg { "templateSvg" } else { "template" };
            client_js.push_str(&format!(
                "const $t{index} = {builder}({});\n",
                js::string(html)
            ));
        }
    }
    // One cell per distinct media query, hoisted here rather than emitted
    // at each read: `matchMedia` returns a live object and subscribing
    // twice to one query installs two listeners that always agree. They
    // are ordered by the index the emitter handed out, so the file reads
    // in the order the queries were first written.
    if !emitter.media.is_empty() {
        let mut queries: Vec<(&String, &usize)> = emitter.media.iter().collect();
        queries.sort_by_key(|(_, index)| **index);
        client_js.push('\n');
        for (query, index) in queries {
            client_js.push_str(&format!(
                "const $q{index} = mediaMatch({});\n",
                js::string(query)
            ));
        }
    }
    // One cell for the whole program: there is one document, so a second
    // subscription would be a second listener that always agreed.
    if emitter.scroll {
        client_js.push_str("\nconst $scroll = scrollFraction();\n");
    }
    // §16.6: one key function per module, and identity is the slot until a
    // `record` declares `unique`.
    if by_position {
        client_js.push_str("\nconst $byPosition = (item, index) => index;\n");
    }
    // §17.4.7: the prelude's primitive layer, inlined rather than
    // imported, and only the parts this program reached.
    if !used.helpers.is_empty() {
        client_js.push('\n');
        for name in &used.helpers {
            let (source, _) = intrinsics::helper(name)
                .unwrap_or_else(|| unreachable!("`{name}` was used, so it has a source"));
            client_js.push_str(source);
        }
    }
    if !functions.is_empty() {
        client_js.push('\n');
        client_js.push_str(&functions);
    }
    if !declarations.is_empty() {
        client_js.push('\n');
        client_js.push_str(&declarations);
    }
    if !remotes.is_empty() {
        client_js.push_str(&remotes);
    }
    if let Some(body) = &main {
        client_js.push_str("\nexport function main(container) {\n");
        client_js.push_str(body);
        client_js.push_str("}\n");
    }

    Ok(Emitted {
        client_js,
        linked_modules,
        import_map,
        remote_origins,
        connect_origins: used.connect.clone(),
        styles_css: styles.stylesheet(),
        runtime: linked_runtime(&used),
        transactions: emitter.transactions,
        functions: server,
        names,
    })
}

/// §16.3.12: code generation refuses to run without a verdict, and refuses
/// to run on a rejected one. An unenforced invariant 3 is worse than no
/// build. Shared by [`compile`] and [`build_module`] so the build root
/// cannot be printed from a program the client bundle would be refused for.
fn refuse_without_a_verdict(split: &TierSplit, verdict: &Verdict) -> Result<(), Vec<CodegenError>> {
    if split.has_errors() {
        return Err(vec![CodegenError {
            // unreached: `zdc-graph` reports this first, in its own words. Every
            // caller renders the split's diagnostics and stops; this is the
            // guard that makes emitting without a verdict impossible, not a
            // message anyone is meant to read.
            message: "The placement pass rejected this program, so there is nothing to emit."
                .to_string(),
            span: Span::new(0, 0),
        }]);
    }
    if verdict.has_errors() {
        return Err(vec![CodegenError {
            // unreached: `zdc-graph` reports this first, in its own words, for
            // the same reason as the line above.
            message:
                "The information-flow pass rejected this program, so there is nothing to emit."
                    .to_string(),
            span: Span::new(0, 0),
        }]);
    }
    Ok(())
}

/// The build host's answers, re-keyed from source names onto definitions.
///
/// A name that matches no `static` signal is dropped rather than reported:
/// the caller supplies whatever the previous build printed, and a stale
/// entry is not a reason to refuse a program.
/// How long a `static` value has to be before it is declared once and
/// named rather than inlined at each read.
///
/// Two hundred bytes. Below it the inline form is smaller for any
/// realistic number of reads and needs no name; above it, a second read
/// already pays for the declaration. The exact number matters little —
/// what matters is that there is one, because without it the cost of a
/// value grows with how often a program mentions it.
const HOIST_ABOVE: usize = 200;

/// Whether this `static` value is declared once rather than inlined.
///
/// Read by both halves that have to agree: the declaration in
/// `signal_declarations` and the read in `expr.rs`.
pub(crate) fn hoisted(json: &str) -> bool {
    json.len() > HOIST_ABOVE
}

fn statics_by_def(hir: &Hir, values: &BTreeMap<String, String>) -> BTreeMap<DefId, String> {
    let mut out = BTreeMap::new();
    for (id, def) in hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        if signal.placement != zdc_ast::Placement::Static {
            continue;
        }
        if let Some(json) = values.get(&def.name) {
            out.insert(id, json.clone());
        }
    }
    out
}

/// Print the `BUILD` root, for the build host to run — §17.4.8.
///
/// Returns `None` when the program declares no `static` state, which is what
/// keeps §17.4.8's named cost — a JavaScript runtime on the build host —
/// paid only by the programs that incur it.
///
/// This runs **before** [`compile`], and its output is what fills
/// [`Options::statics`]. The two share `Inputs`, an `Analysis` and a
/// `Names`, so the build root and the client bundle cannot disagree about
/// what anything is called.
pub fn build_module(
    inputs: &Inputs<'_>,
    options: &Options,
) -> Result<Option<BuildModule>, Vec<CodegenError>> {
    let Inputs {
        hir,
        split,
        verdict,
        table,
        // Not bound, for the reason `compile` gives: the token's job was
        // done when this `Inputs` was built, and the two checks below
        // prove what it does not.
        cleared: _,
    } = *inputs;

    refuse_without_a_verdict(split, verdict)?;

    let analysis = Analysis::new(hir, table);
    let names = Names::new(hir, &analysis, &BTreeSet::new());
    // Empty by construction: this is the pass that computes them, and a
    // `static` read inside the build root is an ordinary `const`.
    let statics = BTreeMap::new();
    let bindings = Bindings::default();
    let mut emitter = Emitter {
        hir,
        types: table,
        names: &names,
        analysis: &analysis,
        bindings: &bindings,
        used: RuntimeImports::default(),
        split,
        ctx: split.root(CLIENT).ctx,
        root: CLIENT,
        statics: &statics,
        read_statics: BTreeSet::new(),
        errors: Vec::new(),
        // A build root has no view, so it declares no handler and records
        // no write set. The field is here because the emitter is one type.
        transactions: Vec::new(),
        // A build root has no browser either, and E0362 has already
        // refused any `media` that could have reached one.
        media: BTreeMap::new(),
        scroll: false,
    };

    let module = build::module(&mut emitter, &names, &options.source_path);
    let errors = std::mem::take(&mut emitter.errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(module)
}

/// Every durable key the program reads or writes, sorted and deduplicated.
fn durable_keys(hir: &Hir, split: &TierSplit) -> Vec<String> {
    let mut keys: Vec<String> = split
        .reads_keys
        .values()
        .chain(split.writes_keys.values())
        .flat_map(|keys| keys.iter().map(|key| hir.defs[*key].name.clone()))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Every environment key the program reads, sorted and deduplicated.
///
/// Read off the HIR rather than off the emitted text: `$env("KEY")` is a
/// spelling decision made in one place, and a scan of generated JavaScript
/// would be a second place that has to agree with it. §5.6 confines
/// `environment` to server context and the split has already rejected every
/// other placement, so the whole arena is the right thing to walk.
fn environment_keys(hir: &Hir) -> Vec<String> {
    let mut keys: Vec<String> = hir
        .exprs
        .iter()
        .filter_map(|(_, expr)| match &expr.kind {
            zdc_hir::HirExprKind::Environment(key) => Some(key.clone()),
            zdc_hir::HirExprKind::Number(_)
            | zdc_hir::HirExprKind::Text(_)
            | zdc_hir::HirExprKind::Truth(_)
            | zdc_hir::HirExprKind::Empty
            | zdc_hir::HirExprKind::Address
            | zdc_hir::HirExprKind::Media(_)
            | zdc_hir::HirExprKind::Scroll
            | zdc_hir::HirExprKind::Build { .. }
            // Every expression in the arena is visited, so a conditional
            // needs no descent here: its three children are entries of
            // their own.
            | zdc_hir::HirExprKind::Conditional { .. }
            | zdc_hir::HirExprKind::List(_)
            | zdc_hir::HirExprKind::Map(_)
            | zdc_hir::HirExprKind::Ref(_)
            | zdc_hir::HirExprKind::Call { .. }
            | zdc_hir::HirExprKind::OfCall { .. }
            | zdc_hir::HirExprKind::Operator { .. }
            | zdc_hir::HirExprKind::Unary { .. }
            | zdc_hir::HirExprKind::Binary { .. }
            | zdc_hir::HirExprKind::Field { .. }
            | zdc_hir::HirExprKind::Index { .. }
            | zdc_hir::HirExprKind::Append { .. }
            // A request names a destination, never an environment key:
            // §5.6 confines `environment` to server context and a request
            // is `client`, so the two can never meet.
            | zdc_hir::HirExprKind::Outbound { .. }
            | zdc_hir::HirExprKind::Insert { .. }
            | zdc_hir::HirExprKind::MapInside { .. } => None,
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn emit_server(
    hir: &Hir,
    split: &TierSplit,
    names: &Names,
    emitter: &mut Emitter<'_>,
    source_path: &str,
) -> Vec<ServerFunction> {
    let mut out = Vec::new();
    for endpoint in &split.endpoints {
        emitter.root = endpoint.root;
        emitter.ctx = split.root(endpoint.root).ctx;
        out.extend(server::emit_one(
            hir,
            split,
            names,
            emitter,
            endpoint,
            source_path,
        ));
    }
    out
}

/// One `$remote` or `$durable` binding per endpoint the client bundle
/// depends on, and the subscription that keeps the durable ones live.
///
/// The parameter getters are emitted lexically, in the wire order the
/// manifest records, so the endpoint and the browser agree without the
/// runtime ever reading a name out of the manifest (§16.3.12 rule 2).
///
/// **Why `durable` gets a different binding.** A `server` signal is
/// recomputed when its inputs change and at no other time — nothing else
/// can move it. A `durable` signal is shared across visitors (§5.7), so it
/// can change because *somebody else* wrote it, and the only way this
/// window learns that is a push. `$durable` registers the cell against its
/// store key so an announced write updates it in place; `$remote` has
/// nothing to register and would be one more thing to keep in sync.
fn emit_remotes(emitter: &mut Emitter<'_>) -> String {
    let split = emitter.split;
    let names = emitter.names;
    let mut out = String::new();
    let mut depended: Vec<RootId> = split
        .depends
        .get(&CLIENT)
        .map(|set| set.iter().copied().collect())
        .unwrap_or_default();
    depended.sort();

    for root in depended {
        let Some(endpoint) = split.endpoint_of(root) else {
            continue;
        };
        let EndpointKind::Value(def) = endpoint.kind else {
            continue;
        };
        let inputs: Vec<String> = endpoint
            .params
            .iter()
            .map(|param| names.def(*param).to_string())
            .collect();
        let durable = matches!(
            &emitter.hir.defs[def].kind,
            DefKind::Signal(signal) if signal.placement == zdc_ast::Placement::Durable
        );
        if durable {
            emitter.used.store.insert("durable as $durable");
            // Three names, and they are genuinely three: the JavaScript
            // binding, the endpoint the browser posts to, and the store key
            // the announcement is addressed to. They coincide today and
            // deriving one from another would make a rename silently wrong.
            out.push_str(&format!(
                "const {} = $durable({}, {}, [{}]);\n",
                names.def(def),
                js::string(&endpoint.name),
                js::string(&emitter.hir.defs[def].name),
                inputs.join(", ")
            ));
        } else {
            emitter.used.rpc.insert("remote as $remote");
            out.push_str(&format!(
                "const {} = $remote({}, [{}]);\n",
                names.def(def),
                js::string(&endpoint.name),
                inputs.join(", ")
            ));
        }
    }

    // One subscription for the whole page, after every cell is registered.
    // Not one per key: a stream per durable signal would be N connections
    // where the platform ceiling is measured in connections, and on a
    // Durable Object it would be N objects instead of one topic.
    if emitter.used.store.contains("durable as $durable") {
        emitter.used.store.insert("subscribe as $subscribe");
        out.push_str("$subscribe();\n");
    }

    // A command is called from a handler rather than bound at module
    // scope, so its import is decided by the split rather than by an
    // emission site.
    if split.depends.get(&CLIENT).is_some_and(|roots| {
        roots.iter().any(|root| {
            matches!(
                split.endpoint_of(*root).map(|e| &e.kind),
                Some(EndpointKind::Command(_))
            )
        })
    }) {
        // `atomic`, not `call`: a handler's writes leave together as one
        // transaction, so the client half a command needs is the batch
        // sender rather than the single-call one.
        emitter.used.rpc.insert("atomic as $atomic");
    }
    out
}

/// The `runtime/clock.js` call one clock clause becomes, recording the
/// import it needs.
///
/// A whole millisecond count is written without a decimal point so the
/// emission reads as the duration the program wrote — `everyMs(250)`
/// rather than `everyMs(250.0)`, which is not even valid JavaScript's
/// idea of tidy. The value came through `zdc_ast::parse_duration`, so it
/// is finite, positive and bounded by `LONGEST_CLOCK_MS`; there is no
/// literal here a reader has to check for injection, because there is no
/// path from source text to this number that is not a parsed duration.
fn clock_call(used: &mut crate::view::RuntimeImports, clock: zdc_ast::Clock) -> String {
    fn ms(value: f64) -> String {
        if value.fract() == 0.0 {
            format!("{value:.0}")
        } else {
            format!("{value}")
        }
    }
    match clock {
        zdc_ast::Clock::Interval(every) => {
            used.clock.insert("everyMs");
            format!("everyMs({})", ms(every))
        }
        zdc_ast::Clock::Frame => {
            used.clock.insert("everyFrame");
            "everyFrame()".to_string()
        }
        zdc_ast::Clock::Delay(delay) => {
            used.clock.insert("afterMs");
            format!("afterMs({})", ms(delay))
        }
    }
}

/// The `runtime/clock.js` call a *stepping* clock becomes.
///
/// The step is emitted as a thunk rather than as a value, because it
/// reads the cell it writes: evaluating it at declaration time would read
/// the cell before it exists. `everyFrame`'s `after` sibling is absent
/// for the reason the parser gives — a fold over one step is `starting`.
fn stepping_call(
    used: &mut crate::view::RuntimeImports,
    clock: zdc_ast::Clock,
    start: &str,
    step: &str,
) -> String {
    match clock {
        zdc_ast::Clock::Interval(every) => {
            used.clock.insert("steppingMs");
            let ms = if every.fract() == 0.0 {
                format!("{every:.0}")
            } else {
                format!("{every}")
            };
            format!("steppingMs({ms}, {start}, () => ({step}))")
        }
        zdc_ast::Clock::Frame => {
            used.clock.insert("steppingFrame");
            format!("steppingFrame({start}, () => ({step}))")
        }
        // unreached: the parser accepts `starting … to …` only after
        // `every`, and says so where it refuses.
        zdc_ast::Clock::Delay(_) => {
            used.clock.insert("steppingFrame");
            format!("steppingFrame({start}, () => ({step}))")
        }
    }
}

/// Signal declarations, per §16.3.4.
///
/// `exported` is set for a program with no `view`, where the file is a
/// module rather than an application: §14D.2 makes every top-level
/// declaration importable, so the emitted module has to say so.
fn emit_declarations(
    emitter: &mut Emitter,
    client_members: &BTreeSet<DefId>,
    exported: bool,
) -> String {
    let export = if exported { "export " } else { "" };
    let mut out = String::new();
    // A request's cell is **eager**: `$request` allocates an effect that
    // reads its arguments the moment the binding is evaluated, so a
    // request declared above a signal it reads would hit that signal's
    // temporal dead zone and the page would fail to load. Every other
    // declaration is lazy — `signal` takes a value and `derived` takes a
    // closure — which is why source order was sound for all of them.
    //
    // So the eager ones go last, which is exactly where the other eager
    // binding in this file already goes: `emit_remote_bindings` writes
    // `$remote(…)` after this function's output for the same reason.
    let mut requests = String::new();
    let ids: Vec<_> = emitter
        .hir
        .defs
        .iter()
        .map(|(id, _)| id)
        .filter(|id| client_members.contains(id))
        .collect();

    for id in ids {
        let DefKind::Signal(signal) = &emitter.hir.defs[id].kind else {
            continue;
        };
        let is_source = signal.is_source;
        let init = signal.init;
        let clock = signal.clock;
        let name = emitter.names.def(id).to_string();
        let setter = emitter.names.setter(id).map(str::to_string);

        // One `const`, and no callback anywhere in the emission. The
        // resting value is not emitted at all: `clock.js` holds it, because
        // the cell and the scheduler that writes it have to agree about
        // what "not yet" means and there is no reason for two files to
        // carry that number.
        if let Some(clock) = clock {
            // A stepping clock carries its own value, so the resting
            // value *is* emitted here — unlike a plain clock, where
            // `clock.js` holds it because the cell and the scheduler have
            // to agree about what "not yet" means.
            if let Some(step) = signal.step {
                let start = emitter.value(init).into_text();
                let next = emitter.value(step).into_text();
                let call = stepping_call(&mut emitter.used, clock, &start, &next);
                // The same `[read, write]` destructuring a source gets,
                // for the same reason: this cell's value is the program's
                // and a handler may write it. Only the writer differs, and
                // the scheduler is one more of them.
                match setter {
                    Some(setter) => {
                        out.push_str(&format!("{export}const [{name}, {setter}] = {call};\n"))
                    }
                    None => out.push_str(&format!("{export}const [{name}] = {call};\n")),
                }
                continue;
            }
            let call = clock_call(&mut emitter.used, clock);
            out.push_str(&format!("{export}const {name} = {call};\n"));
            continue;
        }

        let value = emitter.value(init).into_text();

        if is_source {
            // Which constructor makes the cell is the *placement's*
            // question and the only thing it changes here. `remembered`
            // returns the same `[read, write]` pair `signal` does — see
            // `runtime/remembered.js` — so every reader downstream, every
            // `derived` and every binding, is emitted unchanged. The
            // storage key is the signal's **source** name and not the
            // emitted one: the emitted name is renamed to dodge JavaScript
            // reserved words and collisions, and a key that moved when an
            // unrelated declaration was added would lose the value on the
            // next deploy, which is the one thing this placement promises
            // not to do.
            let source_name = emitter.hir.defs[id].name.clone();
            let constructor = match placement_of_signal(emitter.hir, id) {
                SignalPlacement::Remembered => {
                    emitter.used.remembered.insert("remembered");
                    format!("remembered({}, {value})", js::string(&source_name))
                }
                SignalPlacement::Client
                | SignalPlacement::Static
                | SignalPlacement::Server
                | SignalPlacement::Durable
                | SignalPlacement::DurablePerVisitor => {
                    emitter.used.signal.insert("signal");
                    format!("signal({value})")
                }
            };
            match setter {
                // `HirPlace.base` is a `Res`, so whether a signal is ever
                // written is exactly decidable — a never-written one needs
                // no setter binding at all.
                Some(setter) => out.push_str(&format!(
                    "{export}const [{name}, {setter}] = {constructor};\n"
                )),
                None => out.push_str(&format!("{export}const [{name}] = {constructor};\n")),
            }
        } else if matches!(
            emitter.hir.exprs[init].kind,
            zdc_hir::HirExprKind::Outbound { .. }
        ) {
            // A request is **already** a cell. `$request` allocates the
            // signal and the effect that drives it and hands back the
            // getter, exactly as `$remote` does — so wrapping it in
            // `derived` would allocate a fresh request cell on every
            // recomputation, and the page would fetch for ever.
            requests.push_str(&format!("{export}const {name} = {value};\n"));
        } else {
            // No dependency array and no topological sort: `derived` is
            // lazy, so source-order declaration is sound.
            emitter.used.signal.insert("derived");
            let body = js::arrow_body(&value);
            out.push_str(&format!("{export}const {name} = derived(() => {body});\n"));
        }
    }
    out.push_str(&requests);
    out
}

/// A definition's placement, for the one emission decision that turns on
/// it.
///
/// `Client` for anything that is not a signal, which is every caller's
/// situation already: `emit_declarations` has matched `DefKind::Signal`
/// before it asks.
fn placement_of_signal(hir: &Hir, def: DefId) -> SignalPlacement {
    match &hir.defs[def].kind {
        DefKind::Signal(signal) => SignalPlacement::from_ast(signal.placement),
        DefKind::Function(_)
        | DefKind::Release(_)
        | DefKind::View(_)
        | DefKind::Record(_)
        | DefKind::Choice(_)
        | DefKind::Component(_)
        | DefKind::Foreign(_) => SignalPlacement::Client,
    }
}

/// Every function in the client closure. A function is colorless, so it is
/// emitted wherever it is reachable from (§16.3.12).
fn emit_functions(
    emitter: &mut Emitter,
    client_members: &BTreeSet<DefId>,
    exported: bool,
) -> String {
    let export = if exported { "export " } else { "" };
    let mut out = String::new();
    let ids: Vec<_> = emitter
        .hir
        .defs
        .iter()
        .map(|(id, _)| id)
        .filter(|id| client_members.contains(id))
        .collect();

    // The mutual tail-recursion cycles, computed once for the program
    // rather than once per function (#198).
    let groups = crate::tailgroup::TailGroups::find(emitter.hir);

    for id in ids {
        let DefKind::Function(function) = &emitter.hir.defs[id].kind else {
            continue;
        };
        let body = function.body;
        // The binders themselves, for the tail rewrite below. Taken here
        // from the `function` this loop already destructured rather than
        // matched for a second time: the second match had to answer for a
        // `DefKind` that cannot reach this point, and answered `no
        // parameters` — which would have rewritten a self-call into a loop
        // that never advanced its arguments.
        let param_locals = function.params.clone();
        let params: Vec<String> = param_locals
            .iter()
            .map(|param| emitter.names.local(*param).to_string())
            .collect();
        let name = emitter.names.def(id).to_string();

        // A function that gives the result of calling itself is emitted as
        // a loop rather than as recursion (§17.4.10). One that does not is
        // emitted exactly as before, which is what leaves §16.4's worked
        // output untouched.
        let tail = crate::stmt::gives_a_self_call(emitter.hir, id, body).then_some(
            crate::stmt::TailSelfCall {
                def: id,
                params: param_locals,
            },
        );
        let indent = if tail.is_some() { 4 } else { 2 };

        // A cycle of functions that tail-call one another is emitted as a
        // trampoline (#198), but only when *every* member of the cycle is
        // in this same closure: the body bounces to its siblings' `$step$`
        // functions by name, and a sibling emitted into a different bundle
        // would leave that name undefined. A cycle split across the tier
        // boundary keeps the emission it had, which is one frame per hop
        // and the behaviour this fix improves on rather than the one it
        // breaks.
        let bounce = groups
            .group_of(id)
            .filter(|group| group.iter().all(|member| client_members.contains(member)))
            .map(|group| crate::stmt::BounceGroup {
                members: group.clone(),
            });
        let stepped = bounce.is_some();

        let mut statements = String::new();
        Statements {
            emitter,
            temporaries: 0,
            awaited: false,
            commands: 0,
            writes: Vec::new(),
            loops: 0,
            unbounded: false,
            tail,
            bounce,
        }
        .block(body, indent, &mut statements);

        // The member keeps its name for the wrapper, so every call site
        // in the program goes on naming what it always named, and the
        // body moves into the step the trampoline drives.
        if stepped {
            let step = crate::tailgroup::step_name(&name);
            out.push_str(&format!(
                "{export}function {step}({}) {{\n",
                params.join(", ")
            ));
        } else {
            out.push_str(&format!(
                "{export}function {name}({}) {{\n",
                params.join(", ")
            ));
        }
        if indent == 4 {
            out.push_str("  $tail: while (true) {\n");
        }
        out.push_str(&statements);
        if indent == 4 {
            out.push_str("  }\n");
        }
        out.push_str("}\n");
        if stepped {
            // Registered here rather than left to `tail_bounce`: every
            // member of a cycle does have a crossing call, so the body
            // would have registered it, but the wrapper needs the helper
            // whether or not the body reached that path.
            emitter.use_helper("$bounce");
            let step = crate::tailgroup::step_name(&name);
            out.push_str(&format!(
                "{export}function {name}({0}) {{\n  return $bounce({step}({0}));\n}}\n",
                params.join(", ")
            ));
        }
    }
    out
}

/// The document, including the head the program asked for.
///
/// `<html>` and `<body>` are written out rather than left implicit,
/// because `lang` belongs on the first of them and a document with no
/// declared language is one a screen reader has to guess the pronunciation
/// of. The viewport line is not optional either: without it a phone
/// renders the page at 980 CSS pixels and scales it down.
///
/// `boot` and `styles` are passed rather than fixed, because a routed
/// program's documents sit at their own URLs and reach a module one
/// directory below the site root. One function writes every document, so
/// a routed page and an unrouted one cannot drift in what their head says.
///
/// Every interpolation is escaped, and metadata is a string literal from
/// the source by construction (`zdc-resolve` refuses anything else), so
/// nothing computed reaches this file.
/// What a document links and what it already holds.
///
/// Grouped rather than passed one by one because they *are* one thing —
/// the parts of the shell that vary per page — and because the first paint
/// was the eighth argument, which is where a reader stops being able to
/// tell at a call site which string is which.
struct Shell<'a> {
    /// The module the page loads, as an href relative to the document.
    boot: &'a str,
    /// The generated stylesheet, likewise.
    styles: &'a str,
    import_map: &'a BTreeMap<String, String>,
    connect: &'a BTreeSet<String>,
    /// What the build host painted, or `None` for a document that ships
    /// its container empty.
    painted: Option<&'a str>,
}

fn index_html(metadata: &Metadata, options: &Options, title: &str, shell: Shell<'_>) -> String {
    let Shell {
        boot,
        styles,
        import_map,
        connect,
        painted,
    } = shell;
    let language = metadata.language.as_deref().unwrap_or("en");

    let mut head = format!(
        "  <meta charset=\"utf-8\">\n\
         \x20 <meta http-equiv=\"Content-Security-Policy\" content={}>\n\
         \x20 <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         \x20 <title>{}</title>\n",
        // The quotes are the escaper's, not this literal's: a site that
        // writes its own around a placeholder is the shape three injection
        // holes in this compiler had, and `scripts/check-emitted-strings.sh`
        // refuses it mechanically. The policy is a constant with no `&`,
        // `\"` or `<` in it, so the bytes are the same either way — which is
        // the point, since the rule cannot tell a constant from a value the
        // program supplied and should not have to.
        js::html_attribute(&content_security_policy(connect)),
        js::html_text(title)
    );
    if let Some(description) = &metadata.description {
        head.push_str(&format!(
            "  <meta name=\"description\" content={}>\n",
            js::html_attribute(description)
        ));
    }
    // The import map, before anything that could load a module (#238).
    //
    // This is document order the browser *requires*, not a preference: a
    // map that arrives after the first module load is ignored, and the page
    // then fails on the bare specifier exactly as it did when no map was
    // written at all. It goes at the top of the head rather than the
    // bottom for the same reason — the stylesheet links below cannot load
    // a module, but nothing about that is guaranteed to stay true, and the
    // cheap place to be right is first.
    //
    // Absent entirely when the bundle imports no package, so a program with
    // no `foreign` is byte-for-byte the document it was before.
    if !import_map.is_empty() {
        let imports: Vec<String> = import_map
            .iter()
            .map(|(specifier, target)| {
                format!("{}:{}", js::script_json(specifier), js::script_json(target))
            })
            .collect();
        // `js::script_json` rather than `js::json_string`, because this
        // JSON sits inside a `<script>` element: its content is raw text
        // that must not be HTML-escaped, and it ends at the first
        // `</script`. That escaper owns the quotes, the JSON escaping, and
        // the `<` that would otherwise end the element early.
        head.push_str(&format!(
            "  <script type=\"importmap\">{{\"imports\":{{{}}}}}</script>\n",
            imports.join(",")
        ));
    }
    // Before the stylesheets, because a browser asks for the icon early and
    // an icon named in the head is the only way to use one that is not
    // `/favicon.ico` — which is most of them.
    if let Some(icon) = &options.icon {
        head.push_str(&format!(
            "  <link rel=\"icon\" href={}>\n",
            js::html_attribute(icon)
        ));
    }
    head.push_str(&format!(
        "  <link rel=\"stylesheet\" href={}>\n",
        js::html_attribute(styles)
    ));
    for stylesheet in &options.stylesheets {
        head.push_str(&format!(
            "  <link rel=\"stylesheet\" href={}>\n",
            js::html_attribute(stylesheet)
        ));
    }

    format!(
        "<!doctype html>\n\
         <html lang={}>\n\
         <head>\n\
         {head}\
         </head>\n\
         <body>\n\
         {}\
         \x20 <script type=\"module\" src={}></script>\n\
         </body>\n\
         </html>\n",
        js::html_attribute(language),
        // The first paint, when the build host could compute one. The
        // container is empty otherwise, exactly as it always was — this
        // pass adds markup and never removes any.
        app_container(painted),
        js::html_attribute(boot)
    )
}

/// The one module a document loads, which mounts the program (#146).
///
/// It exists so that the document has **no inline script**, which is what
/// lets `script-src` be `'self'` with no `'unsafe-inline'` and no hash. The
/// alternative was to keep the inline script and put its SHA-256 in the
/// policy; it was rejected because a hash has to be recomputed every time
/// the two lines change and is wrong silently when it is not — a page whose
/// only script is blocked renders nothing, and the compiler would have
/// emitted the mistake itself.
///
/// Two lines rather than folding them into `client.js`: `client.js` is a
/// module with a `main` export, imported by the browser here and evaluated
/// by the emitter's own tests, and a module that mounts itself on
/// evaluation is a different thing from a module that exports an entry
/// point.
fn boot_js(module: &str) -> String {
    format!(
        "// zdc · generated, do not edit. `index.html` loads this and no\n\
         // inline script, so its Content-Security-Policy needs no exception.\n\
         import {{ main }} from {};\n\
         main(document.getElementById('app'));\n",
        // The module path sits in a JavaScript string literal, not in an
        // attribute. `html_attribute` is the wrong escaper for that
        // position twice over: it does not escape the apostrophe that ends
        // the literal, and the entities it writes are never decoded inside
        // script text, so `&` in a path would come back as `&amp;`.
        // `js::string` owns the quotes and escapes what ends this literal.
        js::string(module)
    )
}

/// The policy every emitted document carries — spec §16.3.5, #146.
///
/// **A policy the compiler can prove the program satisfies, and nothing
/// wider.** Each directive below is a fact about what this compiler emits,
/// not a guess about what an application might need:
///
/// * `default-src 'none'` — the fallback is refusal, so a fetch class
///   nobody thought about is blocked rather than allowed. Every directive
///   after it exists because something in the emitted output needs it.
/// * `script-src 'self'` — a document loads exactly one module, by `src`,
///   from its own origin (see [`boot_js`]). There is no inline script, no
///   `eval`, and no `new Function` anywhere in the runtime or in generated
///   code, so neither `'unsafe-inline'` nor `'unsafe-eval'` appears.
/// * `style-src 'self'` — styling is `styles.css` and the asset directory's
///   stylesheets. The emitter refuses a `style` argument outright and folds
///   static declarations into a generated class, so no `style` attribute
///   and no `<style>` element is ever written. A *reactive* style is
///   `bindStyle`, which calls `CSSStyleDeclaration.setProperty` — CSSOM,
///   which CSP does not govern, and which is why this needs no
///   `'unsafe-inline'` either.
/// * `img-src`, `media-src`, `font-src`, `frame-src` — `'self' http:
///   https:`, which is exactly [`URL_SCHEMES`] minus the two schemes that
///   fetch nothing. A program names these URLs, so the compiler cannot
///   narrow them to an origin; what it *can* say is that no other scheme
///   reaches an attribute, because `safeUrl` and `zdc_hir::url_is_safe`
///   refuse them. Stating it here is the browser enforcing the same
///   allowlist a second time, at the point of use.
/// * `connect-src 'self'` — an endpoint is a path on this origin
///   (`functions/…`), and live sync is `/_zd/live` on it. A program cannot
///   name a host to talk to, so nothing else is needed.
/// * `object-src 'none'` — `object` and `embed` are not in the element
///   vocabulary and cannot become one without editing the shape table.
/// * `base-uri 'none'` — nothing emits a `<base>`, and an injected one
///   would repoint every relative URL in the document, including the
///   module above.
/// * `form-action 'none'` — a `Form` has no `action` and its submit is a
///   handler the emitter wraps in `preventDefault`, so no form in any
///   emitted program navigates.
///
/// # What is deliberately absent
///
/// `frame-ancestors`, `report-uri` and `sandbox` are all **ignored** in a
/// `<meta http-equiv>` and only work as a response header. Writing them
/// here would look like protection and be a console warning instead, so
/// they are left to the deploy target, which owns the headers.
///
/// The policy is one constant rather than derived per program. A policy
/// that varied would be a policy that had to be re-verified per program,
/// and `crates/zdc-codegen/tests/csp.rs` verifies this one against the
/// emitted bytes of every example instead.
///
/// # The one thing a program may widen, and how (#19)
///
/// `connect-src 'self'` was true because a program could not name a host
/// to talk to. `request` is exactly the construct that lets it, so the
/// sentence stopped being true and the policy had to stop saying it —
/// silently keeping the constant would have meant the compiler emitting a
/// program the browser refuses, which the file above calls the specific
/// kind of dishonesty a policy must not have.
///
/// [`content_security_policy`] is the widening, and every part of its
/// shape is load-bearing:
///
/// * **This constant is still the answer** for a program that declares no
///   cross-origin request, byte for byte. A widened policy is not the new
///   normal; it is what a program buys by writing a host down.
/// * **Only `connect-src` moves.** Nothing a `request` does touches
///   `script-src`, `img-src` or any other directive, so nothing else may
///   change — a policy that loosened in sympathy would be the blanket
///   loosening this design exists to avoid.
/// * **The sources are origins, not schemes.** `connect-src https:` would
///   be one character shorter and would permit every host on the web. What
///   is written is `https://api.example.org`, taken from the program's own
///   `from` line, so the policy names what the program named and nothing
///   else. That the destination has to be a literal is what makes it
///   possible to write at all.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; \
     script-src 'self'; \
     style-src 'self'; \
     img-src 'self' http: https:; \
     font-src 'self' http: https:; \
     media-src 'self' http: https:; \
     frame-src 'self' http: https:; \
     connect-src 'self'; \
     object-src 'none'; \
     base-uri 'none'; \
     form-action 'none'";

/// The `connect-src` directive of [`CONTENT_SECURITY_POLICY`], as written.
///
/// Named rather than spelled out at the two sites that need it, so the
/// widening below and the test that checks it cannot disagree about what
/// they are widening.
const CONNECT_SRC: &str = "connect-src 'self'";

/// The policy this document carries, given the origins it fetches from.
///
/// Empty origins give [`CONTENT_SECURITY_POLICY`] unchanged — the same
/// `&'static str`, so a program with no `request` emits the bytes it
/// emitted before this existed. See that constant for why only
/// `connect-src` moves and why the sources are origins.
///
/// Sorted and deduplicated by the `BTreeSet` the caller passes, so two
/// declarations naming one host widen the policy once and the emitted
/// document does not depend on declaration order.
pub fn content_security_policy(origins: &BTreeSet<String>) -> Cow<'static, str> {
    if origins.is_empty() {
        return Cow::Borrowed(CONTENT_SECURITY_POLICY);
    }
    let widened = format!(
        "{CONNECT_SRC} {}",
        origins.iter().cloned().collect::<Vec<_>>().join(" ")
    );
    Cow::Owned(CONTENT_SECURITY_POLICY.replace(CONNECT_SRC, &widened))
}

/// The URL-to-module map, so a static host can answer a request without
/// running the compiler.
///
/// It is a build artefact and not something a program writes, which is
/// invariant 5's line: the compiler derives it from the `route`
/// declaration, and no one edits it.
fn routes_json(pages: &[(String, String)], not_found: Option<&str>) -> String {
    let entries: Vec<String> = pages
        .iter()
        .map(|(url, slug)| {
            format!(
                "{{\"url\":{},\"module\":{},\"styles\":{},\"document\":{}}}",
                js::json_string(url),
                js::json_string(&format!("/pages/{slug}.js")),
                js::json_string(&format!("/pages/{slug}.css")),
                js::json_string(&format!("/{}", document_path(url)))
            )
        })
        .collect();
    format!(
        "{{\"routes\":[{}],\"notFound\":{}}}\n",
        entries.join(","),
        match not_found {
            Some(url) => js::json_string(url).to_string(),
            None => "null".to_string(),
        }
    )
}

/// Where a URL's document is written.
///
/// `/blog/rust` becomes `blog/rust/index.html`, which is what every
/// static host already serves for that URL with no configuration — the
/// point of §14G.2's prerendering being total.
pub fn document_path(url: &str) -> String {
    let trimmed = url.trim_matches('/');
    if trimmed.is_empty() {
        "index.html".to_string()
    } else {
        format!("{trimmed}/index.html")
    }
}

/// What the build host painted, or `None` when it could not.
///
/// **Best effort, and never fatal.** Every reason a program might not
/// prerender — a `foreign` reaching for a package the host has no copy
/// of, a `view` touching something the stubs do not model, a budget
/// exhausted by a deep fold — is a reason to ship the document that was
/// shipped before this existed, and none of them is a reason to refuse
/// the program. The client builds the same tree either way, which is
/// what makes skipping it safe.
#[cfg(feature = "evaluate")]
fn painted_markup(client_js: &str, runtime: &BTreeSet<&'static str>) -> Option<String> {
    // Development sources, assertions and all: a prerender that tripped
    // one is a prerender whose answer was wrong, and this is the one
    // place a build can find that out before a reader does.
    let sources = runtime_files(runtime, Mode::Development);
    let linked: Vec<(&str, &str)> = sources
        .iter()
        .map(|(name, source)| (*name, source.as_str()))
        .collect();
    // On a deep stack, because painting *is* running the program: the same
    // recursion an evaluated `static` does, several engine frames per call.
    // Windows gives the main thread one megabyte against Unix's eight.
    evaluate::on_a_deep_stack(|| {
        prerender::prerender(client_js, &linked).map(|painted| painted.html)
    })
}

#[cfg(not(feature = "evaluate"))]
fn painted_markup(_: &str, _: &BTreeSet<&'static str>) -> Option<String> {
    None
}

/// The shell's container, with whatever the build host painted inside it.
///
/// Written straight in and not escaped: it is markup this compiler
/// produced from templates this compiler wrote, and every program value
/// that reached it was escaped on the way in. Escaping it again would
/// show the reader their own page as source.
fn app_container(painted: Option<&str>) -> String {
    match painted {
        Some(markup) => format!("  <div id=\"app\">{markup}</div>\n"),
        None => "  <div id=\"app\"></div>\n".to_string(),
    }
}

/// One event handler's complete durable write set, known at compile time.
///
/// **This is what a general-purpose database client cannot have, and it is
/// the reason the transaction works on the stores it has to work on.** A
/// client that must open a transaction and discover its writes as it goes
/// needs an *interactive* transaction, and of the surveyed backends only
/// Durable Objects and a local database have one. Because §17.2.7's
/// Command rule already evaluates every right-hand side and index in the
/// caller's region, this list is complete before the first write lands, so
/// a *non-interactive* atomic batch is sufficient — and Deno KV,
/// DynamoDB and D1 all have one of those.
///
/// It reaches the manifest so the caps on those batches — DynamoDB's on
/// `TransactWriteItems`, Deno KV's 100 checks and 1000 mutations — can be
/// checked when the bundle is deployed rather than when a user clicks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerWrites {
    /// The event that runs it, such as `click`.
    pub event: String,
    /// The command endpoints it writes, in source order.
    pub writes: Vec<String>,
    /// Whether the source bounds how many writes there are.
    ///
    /// `false` when a write sits inside an `each`: the *keys* are still
    /// statically known — that is what `watch(keys)` stands on — but the
    /// count is a property of the list at run time, so no build-time check
    /// against a batch cap can be conclusive. Stating that is better than
    /// a compiler that green-lights a handler which fails at 101 items.
    pub bounded: bool,
}

/// The manifest is client-readable, so it may carry endpoint names, input
/// orders, durable keys and cadence rules — never an initializer and never
/// an `environment` key name (spec §16.3.12, assertion C).
fn manifest_json(
    hir: &Hir,
    names: &Names,
    functions: &[ServerFunction],
    durable: &[String],
    transactions: &[HandlerWrites],
    remote_origins: &BTreeSet<String>,
    connect_origins: &BTreeSet<String>,
) -> String {
    let mut signals: Vec<String> = Vec::new();
    for (id, def) in hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        // A signal's emitted name is a program's own identifier, so it is
        // the same kind of value every other site here escapes. JSON has
        // its own escapes — `\'` is not one of them — so the manifest gets
        // its own printer rather than borrowing the JavaScript one. The
        // placement word comes from `Placement::word` rather than from a
        // second table here, so the two cannot drift.
        signals.push(format!(
            "{}:{}",
            js::json_string(names.def(id)),
            js::json_string(signal.placement.word())
        ));
    }

    let emitted: Vec<String> = functions
        .iter()
        .map(|function| {
            let inputs: Vec<String> = function
                .inputs
                .iter()
                .map(|input| js::json_string(input).to_string())
                .collect();
            // `kind` is the argument shape, not decoration: a caller that
            // sends an array to a value endpoint destructures `undefined`
            // into every input and gets a plausible-looking wrong answer.
            format!(
                "{{\"name\":{},\"file\":{},\"kind\":{},\"inputs\":[{}]}}",
                js::json_string(&function.name),
                js::json_string(&function.path),
                js::json_string(function.kind.word()),
                inputs.join(",")
            )
        })
        .collect();

    let durable: Vec<String> = durable
        .iter()
        .map(|key| js::json_string(key).to_string())
        .collect();

    // The write set of every handler, so a deploy adapter can measure it
    // against its target's batch cap without re-running the compiler.
    let transactions: Vec<String> = transactions
        .iter()
        .map(|handler| {
            let writes: Vec<String> = handler
                .writes
                .iter()
                .map(|w| js::json_string(w).to_string())
                .collect();
            format!(
                "{{\"event\":{},\"writes\":[{}],\"bounded\":{}}}",
                js::json_string(&handler.event),
                writes.join(","),
                handler.bounded
            )
        })
        .collect();

    // Every origin the page fetches a module from, sorted, deduplicated
    // (#238). It is here because the two readers who need it — a person
    // auditing what the page talks to, and a deploy target writing a
    // Content-Security-Policy or an allow-list — both have the manifest
    // and neither runs the compiler. It is client-readable under §16.3.12
    // assertion C for the same reason `durable` is: the browser is about
    // to fetch these, so it is not being told anything it will not see.
    let origins: Vec<String> = remote_origins
        .iter()
        .map(|origin| js::json_string(origin).to_string())
        .collect();

    // Every origin the page sends a *request* to (#19), which is a
    // different question from the one above and is here for the same
    // reader. A deploy target that wrote a Content-Security-Policy header
    // from `origins` alone would emit one that blocks the program's own
    // requests — the compiler emitting the mistake itself, which is what
    // #146 says a policy must never do.
    let connect: Vec<String> = connect_origins
        .iter()
        .map(|origin| js::json_string(origin).to_string())
        .collect();

    format!(
        "{{\"entry\":\"client.js\",\"functions\":[{}],\"durable\":[{}],\"transactions\":[{}],\
         \"origins\":[{}],\"connect\":[{}],\"signals\":{{{}}}}}\n",
        emitted.join(","),
        durable.join(","),
        transactions.join(","),
        origins.join(","),
        connect.join(","),
        signals.join(",")
    )
}

/// The runtime files **this** bundle links against, as
/// `(relative path, source)`.
///
/// `elements.js` is deliberately absent: generated code never imports it.
/// It remains the reference implementation the parity test checks the
/// compiler's shape table against.
///
/// Every other file is here only if the bundle reaches it. §16.3.1 says a
/// bundle ships nothing it does not use, and for a long time that was true
/// of the *import list* and false of the files beside it: `hello.zd`
/// imported `signal.js` and `dom.js` and was still shipped `rpc.js`,
/// `store.js` and `wire.js`. The claim is about bytes shipped, so the set
/// is computed from the same [`RuntimeImports`] that decided the imports —
/// one decision, not two that have to agree. A routed program passes the
/// union over its documents, for the same reason: the runtime directory is
/// shared, so the set is a union and never everything there is.
/// `mode` decides whether the sources carry their `// $dev` assertions
/// (#140). It is a parameter rather than a property of the bundle because
/// the two callers that matter are two *commands*: `zdc dev` serves a
/// development build and `zdc build` writes a release one, and neither
/// should be able to get it by default.
pub fn runtime_files(runtime: &BTreeSet<&'static str>, mode: Mode) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for module in runtime {
        let source = match *module {
            "runtime/signal.js" => zdc_runtime::SIGNAL_JS,
            "runtime/dom.js" => zdc_runtime::DOM_JS,
            "runtime/foreign.js" => zdc_runtime::FOREIGN_JS,
            "runtime/markup.js" => zdc_runtime::MARKUP_JS,
            "runtime/list.js" => zdc_runtime::LIST_JS,
            "runtime/clock.js" => zdc_runtime::CLOCK_JS,
            "runtime/keys.js" => zdc_runtime::KEYS_JS,
            "runtime/wire.js" => zdc_runtime::WIRE_JS,
            "runtime/request.js" => zdc_runtime::REQUEST_JS,
            "runtime/rpc.js" => zdc_runtime::RPC_JS,
            "runtime/store.js" => zdc_runtime::STORE_JS,
            "runtime/remembered.js" => zdc_runtime::REMEMBERED_JS,
            "runtime/media.js" => zdc_runtime::MEDIA_JS,
            "runtime/viewport.js" => zdc_runtime::VIEWPORT_JS,
            "runtime/scene.js" => zdc_runtime::SCENE_JS,
            "runtime/vector.js" => zdc_runtime::VECTOR_JS,
            other => unreachable!("`linked_runtime` named `{other}`, which is not a runtime file"),
        };
        out.push((*module, zdc_runtime::for_mode(source, mode).into_owned()));
    }
    out
}

/// Which runtime modules the emitted `client.js` reaches, transitively.
///
/// The direct imports are exactly the four non-empty sets in `used`. The
/// closure is the part a reader would get wrong: `rpc.js` and `store.js`
/// both import `wire.js`, and `store.js` imports `rpc.js`, so a program
/// with a `durable` read pulls in three files having named one.
fn linked_runtime(used: &RuntimeImports) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    if !used.signal.is_empty() {
        out.insert("runtime/signal.js");
    }
    if !used.dom.is_empty() {
        out.insert("runtime/dom.js");
    }
    // `foreign.js` imports `signal.js` and nothing else — the node is
    // handed in rather than looked up — so it adds one file and never
    // `dom.js`. A program can in principle reach it without reaching
    // `dom.js` at all, which is why it is not folded into the branch
    // above.
    if !used.lifecycle.is_empty() {
        out.insert("runtime/foreign.js");
    }
    // `markup.js` imports `signal.js` and nothing else, for the same
    // reason `foreign.js` does: the node is handed in. `Prose` is the only
    // element with a rendered slot, so a program without one never names
    // this and never ships the one call in the runtime that parses HTML.
    if !used.rendered.is_empty() {
        out.insert("runtime/markup.js");
    }
    // `list.js` is split out for the same reason as those two, and unlike
    // them it does import `dom.js` — `each` builds its own anchor pair. So
    // a program with a list links `dom.js` whether or not it named a
    // binding of its own, which is stated here rather than assumed for the
    // reason the `store.js` → `rpc.js` edge below is.
    if !used.reconcile.is_empty() {
        out.insert("runtime/list.js");
        out.insert("runtime/dom.js");
    }
    // `clock.js` imports `signal.js` and nothing else — it writes a cell
    // and touches no DOM — so it adds exactly one file, which is the whole
    // argument for it being one.
    if !used.clock.is_empty() {
        out.insert("runtime/clock.js");
    }
    // `keys.js` imports `signal.js` and nothing else. It never touches
    // `dom.js`: it reads `document` and `event.target` directly, because
    // what it needs from the DOM is a listener and a focus question rather
    // than a node to render into. So a program whose only DOM work is a
    // key handler still does not link the renderer — which is the whole
    // reason this is a file and not four lines in `dom.js`.
    if !used.keys.is_empty() {
        out.insert("runtime/keys.js");
    }
    if !used.store.is_empty() {
        // `store.js` imports `remoteCell` from `rpc.js`, so a live-sync
        // program links the RPC half whether or not it named it.
        out.insert("runtime/store.js");
        out.insert("runtime/rpc.js");
    }
    // `request.js` imports `signal.js` and nothing else — no DOM, no
    // wire format, no `rpc.js`. It is the whole of what a program pays
    // for declaring a `request`.
    if !used.request.is_empty() {
        out.insert("runtime/request.js");
    }
    if !used.rpc.is_empty() {
        out.insert("runtime/rpc.js");
    }
    // `remembered.js` encodes with `wire.js`, so a program with a
    // `remembered` signal links the wire format whether or not it crosses
    // a boundary — stated here for the reason the `store.js` → `rpc.js`
    // edge above is, rather than left for a reader to infer from the
    // import at the top of the file.
    if !used.remembered.is_empty() {
        out.insert("runtime/remembered.js");
    }
    // `media.js` imports `signal.js` and nothing else: the query is a
    // string and the answer is a boolean, so no DOM and no wire format.
    if !used.media.is_empty() {
        out.insert("runtime/media.js");
    }
    // `viewport.js` imports `signal.js` and nothing else: the answer is a
    // number and the listener is the window's, so no DOM and no wire format.
    if !used.viewport.is_empty() {
        out.insert("runtime/viewport.js");
    }
    // `scene.js` imports `signal.js` and nothing else. It touches the DOM
    // — it owns a `<canvas>` — but through `getContext`, not through the
    // template machinery, so it needs no part of `dom.js`.
    if !used.scene.is_empty() {
        out.insert("runtime/scene.js");
    }
    // `vector.js` imports nothing at all: it is one parser trick around
    // `document.createElement`.
    if !used.vector.is_empty() {
        out.insert("runtime/vector.js");
    }
    if out.contains("runtime/rpc.js")
        || out.contains("runtime/store.js")
        || out.contains("runtime/remembered.js")
    {
        out.insert("runtime/wire.js");
    }
    // Both `dom.js` and `rpc.js` import `signal.js`; a bundle that reaches
    // either reaches it, even if nothing in the program named a signal
    // helper directly.
    if !out.is_empty() {
        out.insert("runtime/signal.js");
    }
    out
}
