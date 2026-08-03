#![forbid(unsafe_code)]

//! JavaScript and CSS emission, per spec §16.
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

mod analysis;
pub mod assets;
mod build;
mod elements;
mod evaluate;
mod events;
mod expr;
mod intrinsics;
mod js;
mod names;
mod pages;
mod server;
mod stmt;
mod styles;
mod view;

use std::collections::{BTreeMap, BTreeSet};

use zdc_graph::{Cleared, EndpointKind, RootId, TierSplit, Verdict, CLIENT};
use zdc_hir::{DefId, DefKind, Hir, HirNode, Metadata, View};
use zdc_lexer::Span;
use zdc_types::TypeTable;

use crate::analysis::{Analysis, Shared};
use crate::expr::Emitter;
use crate::names::Names;
use crate::pages::Bindings;
use crate::stmt::Statements;
use crate::styles::Styles;
use crate::view::{Emission, Lowering, RuntimeImports};

pub use crate::build::BuildModule;
pub use crate::elements::{BUILT_INS, HEADING_TAGS};
pub use crate::evaluate::{evaluate, Evaluated, EvaluationError};
pub use crate::server::{file_name, FunctionKind, ServerFunction};

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
}

impl Options {
    pub fn new(source_path: impl Into<String>, name: impl Into<String>) -> Options {
        Options {
            source_path: source_path.into(),
            name: name.into(),
            statics: BTreeMap::new(),
            stylesheets: Vec::new(),
        }
    }

    pub fn with_statics(mut self, statics: BTreeMap<String, String>) -> Options {
        self.statics = statics;
        self
    }

    /// The stylesheets the asset directory contributed, in cascade order.
    pub fn with_stylesheets(mut self, stylesheets: Vec<String>) -> Options {
        self.stylesheets = stylesheets;
        self
    }
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
    pub manifest_json: String,
    /// One file per emitted server root — §17.2.3's `Endpoint` and
    /// `Command` origins. Empty for a program with no crossing, which is
    /// how `hello.zd` still ships nothing it does not use.
    pub functions: Vec<ServerFunction>,
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

/// One document of a routed program.
///
/// Per-page rather than per-program because §14A's bundle-size argument
/// is the whole reason the design exists: a site that ships every page's
/// code to every visitor has forfeited it. The split is not a bundler
/// pass bolted on afterwards — it falls out of the address fold, because
/// a document whose route is known reaches only the arm it renders.
pub struct PageBundle {
    /// The URL this document is served at.
    pub url: String,
    /// A file-name-safe name for its module and stylesheet.
    pub slug: String,
    pub client_js: String,
    pub styles_css: String,
    /// The document, or `None` for a module with no `view` — the same
    /// artifact, and absent for the same reason, as [`Bundle::index_html`].
    pub document_html: Option<String>,
}

/// Every document a program emits, and the map from URL to module.
pub struct SiteBundle {
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
        &shared,
    )?;

    let durable = durable_keys(inputs.hir, inputs.split);
    Ok(Bundle {
        runtime: emitted.runtime,
        client_js: emitted.client_js,
        styles_css: emitted.styles_css,
        index_html: nodes.is_some().then(|| {
            index_html(
                &metadata,
                options,
                &page_title(options, &metadata, "/"),
                "./client.js",
                "./styles.css",
            )
        }),
        manifest_json: manifest_json(inputs.hir, &emitted.names, &emitted.functions, &durable),
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
                url: "/".to_string(),
                slug: "index".to_string(),
                client_js: bundle.client_js,
                styles_css: bundle.styles_css,
                document_html: bundle.index_html,
            }],
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
    let mut names: Option<Names> = None;
    for page in &site.pages {
        let specialised = pages::specialise(hir, &nodes, page);
        let module = format!("/pages/{}.js", page.slug);
        let styles = format!("/pages/{}.css", page.slug);
        match emit(
            inputs,
            options,
            Some(&specialised.nodes),
            &specialised.bindings,
            Layout::Page,
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
                names.get_or_insert(emitted.names);
                pages.push(PageBundle {
                    url: page.url.clone(),
                    slug: page.slug.clone(),
                    client_js: emitted.client_js,
                    styles_css: emitted.styles_css,
                    document_html: Some(index_html(
                        &metadata,
                        options,
                        &page_title(options, &metadata, &page.url),
                        &module,
                        &styles,
                    )),
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
        routes_json: routes_json(&index, not_found.as_deref()),
        manifest_json: manifest_json(hir, &names, &functions, &durable),
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
    styles_css: String,
    names: Names,
    runtime: BTreeSet<&'static str>,
    functions: Vec<ServerFunction>,
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
    // not run yet, so that part of the closure is added here.
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
        errors: Vec::new(),
    };

    let mut styles = Styles::default();
    let region = nodes.map(|nodes| Lowering::new(&mut emitter, &mut styles).region(nodes));

    let is_module = region.is_none();
    let functions = emit_functions(&mut emitter, &client_members, is_module);
    let declarations = emit_declarations(&mut emitter, &client_members, is_module);
    let remotes = emit_remotes(&mut emitter);

    // The server roots, emitted last so every diagnostic from the client
    // walk is already collected and the two lists come out together.
    let server = {
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
            errors: Vec::new(),
        };
        let served = emit_server(
            hir,
            split,
            &names,
            &mut server_emitter,
            &options.source_path,
        );
        emitter.errors.extend(server_emitter.errors);
        served
    };

    let errors = std::mem::take(&mut emitter.errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut used = std::mem::take(&mut emitter.used);

    let mut templates: Vec<String> = Vec::new();
    let mut by_position = false;
    let mut main = None;
    if let Some(region) = region {
        let mut emission = Emission::new(&mut used);
        let mut body = emission.instance(&region, "$r", 2);
        templates = emission.templates().to_vec();
        by_position = emission.needs_by_position();
        used.dom.insert("mount");
        body.push_str("  return mount($r, container);\n");
        main = Some(body);
    }

    let mut client_js = String::new();
    client_js.push_str(&format!(
        "// zdc {} · {} · generated, do not edit\n",
        env!("CARGO_PKG_VERSION"),
        options.source_path
    ));
    let runtime_root = layout.runtime();
    if !used.signal.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from '{runtime_root}/signal.js';\n",
            used.signal.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    if !used.dom.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from '{runtime_root}/dom.js';\n",
            used.dom.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    if !used.rpc.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from '{runtime_root}/rpc.js';\n",
            used.rpc.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    if !used.store.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from '{runtime_root}/store.js';\n",
            used.store.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    if !templates.is_empty() {
        client_js.push('\n');
        for (index, html) in templates.iter().enumerate() {
            client_js.push_str(&format!(
                "const $t{index} = template({});\n",
                js::string(html)
            ));
        }
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
        styles_css: styles.stylesheet(),
        runtime: linked_runtime(&used),
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
            message: "The placement pass rejected this program, so there is nothing to emit."
                .to_string(),
            span: Span::new(0, 0),
        }]);
    }
    if verdict.has_errors() {
        return Err(vec![CodegenError {
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
        errors: Vec::new(),
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
            | zdc_hir::HirExprKind::List(_)
            | zdc_hir::HirExprKind::Map(_)
            | zdc_hir::HirExprKind::Ref(_)
            | zdc_hir::HirExprKind::Call { .. }
            | zdc_hir::HirExprKind::OfCall { .. }
            | zdc_hir::HirExprKind::Operator { .. }
            | zdc_hir::HirExprKind::Unary { .. }
            | zdc_hir::HirExprKind::Binary { .. }
            | zdc_hir::HirExprKind::Field { .. }
            | zdc_hir::HirExprKind::Index { .. } => None,
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
        emitter.used.rpc.insert("call as $call");
    }
    out
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
        let name = emitter.names.def(id).to_string();
        let setter = emitter.names.setter(id).map(str::to_string);
        let value = emitter.value(init).into_text();

        if is_source {
            emitter.used.signal.insert("signal");
            match setter {
                // `HirPlace.base` is a `Res`, so whether a signal is ever
                // written is exactly decidable — a never-written one needs
                // no setter binding at all.
                Some(setter) => out.push_str(&format!(
                    "{export}const [{name}, {setter}] = signal({value});\n"
                )),
                None => out.push_str(&format!("{export}const [{name}] = signal({value});\n")),
            }
        } else {
            // No dependency array and no topological sort: `derived` is
            // lazy, so source-order declaration is sound.
            emitter.used.signal.insert("derived");
            out.push_str(&format!("{export}const {name} = derived(() => {value});\n"));
        }
    }
    out
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

    for id in ids {
        let DefKind::Function(function) = &emitter.hir.defs[id].kind else {
            continue;
        };
        let body = function.body;
        let params: Vec<String> = function
            .params
            .iter()
            .map(|param| emitter.names.local(*param).to_string())
            .collect();
        let name = emitter.names.def(id).to_string();

        let mut statements = String::new();
        Statements {
            emitter,
            temporaries: 0,
            awaited: false,
        }
        .block(body, 2, &mut statements);

        out.push_str(&format!(
            "{export}function {name}({}) {{\n",
            params.join(", ")
        ));
        out.push_str(&statements);
        out.push_str("}\n");
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
/// `module` and `styles` are passed rather than fixed, because a routed
/// program's documents sit at their own URLs and reach a module one
/// directory below the site root. One function writes every document, so
/// a routed page and an unrouted one cannot drift in what their head says.
///
/// Every interpolation is escaped, and metadata is a string literal from
/// the source by construction (`zdc-resolve` refuses anything else), so
/// nothing computed reaches this file.
fn index_html(
    metadata: &Metadata,
    options: &Options,
    title: &str,
    module: &str,
    styles: &str,
) -> String {
    let language = metadata.language.as_deref().unwrap_or("en");

    let mut head = format!(
        "  <meta charset=\"utf-8\">\n\
         \x20 <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         \x20 <title>{}</title>\n",
        js::html_text(title)
    );
    if let Some(description) = &metadata.description {
        head.push_str(&format!(
            "  <meta name=\"description\" content=\"{}\">\n",
            js::html_attribute(description)
        ));
    }
    head.push_str(&format!(
        "  <link rel=\"stylesheet\" href=\"{}\">\n",
        js::html_attribute(styles)
    ));
    for stylesheet in &options.stylesheets {
        head.push_str(&format!(
            "  <link rel=\"stylesheet\" href=\"{}\">\n",
            js::html_attribute(stylesheet)
        ));
    }

    format!(
        "<!doctype html>\n\
         <html lang=\"{}\">\n\
         <head>\n\
         {head}\
         </head>\n\
         <body>\n\
         \x20 <div id=\"app\"></div>\n\
         \x20 <script type=\"module\">\n\
         \x20   import {{ main }} from '{}';\n\
         \x20   main(document.getElementById('app'));\n\
         \x20 </script>\n\
         </body>\n\
         </html>\n",
        js::html_attribute(language),
        js::html_attribute(module)
    )
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
            Some(url) => js::json_string(url),
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

/// The manifest is client-readable, so it may carry endpoint names, input
/// orders, durable keys and cadence rules — never an initializer and never
/// an `environment` key name (spec §16.3.12, assertion C).
fn manifest_json(
    hir: &Hir,
    names: &Names,
    functions: &[ServerFunction],
    durable: &[String],
) -> String {
    let mut signals: Vec<String> = Vec::new();
    for (id, def) in hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        signals.push(format!(
            "\"{}\":\"{}\"",
            names.def(id),
            signal.placement.word()
        ));
    }

    let emitted: Vec<String> = functions
        .iter()
        .map(|function| {
            let inputs: Vec<String> = function
                .inputs
                .iter()
                .map(|input| js::json_string(input))
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

    let durable: Vec<String> = durable.iter().map(|key| js::json_string(key)).collect();

    format!(
        "{{\"entry\":\"client.js\",\"functions\":[{}],\"durable\":[{}],\"signals\":{{{}}}}}\n",
        emitted.join(","),
        durable.join(","),
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
pub fn runtime_files(runtime: &BTreeSet<&'static str>) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    for module in runtime {
        out.push(match *module {
            "runtime/signal.js" => ("runtime/signal.js", zdc_runtime::SIGNAL_JS),
            "runtime/dom.js" => ("runtime/dom.js", zdc_runtime::DOM_JS),
            "runtime/wire.js" => ("runtime/wire.js", zdc_runtime::WIRE_JS),
            "runtime/rpc.js" => ("runtime/rpc.js", zdc_runtime::RPC_JS),
            "runtime/store.js" => ("runtime/store.js", zdc_runtime::STORE_JS),
            other => unreachable!("`linked_runtime` named `{other}`, which is not a runtime file"),
        });
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
    if !used.store.is_empty() {
        // `store.js` imports `remoteCell` from `rpc.js`, so a live-sync
        // program links the RPC half whether or not it named it.
        out.insert("runtime/store.js");
        out.insert("runtime/rpc.js");
    }
    if !used.rpc.is_empty() {
        out.insert("runtime/rpc.js");
    }
    if out.contains("runtime/rpc.js") || out.contains("runtime/store.js") {
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
