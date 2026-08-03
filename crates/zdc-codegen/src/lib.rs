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
//! megamorphic across the nine built-ins.
//!
//! **What this milestone covers.** `client` placement only — M5a in §16.5.
//! `hello.zd` and `counter.zd` need `zdc-hir` and `zdc-resolve` and nothing
//! else: with only client signals there is no placement to compute and no
//! boundary to cross. Everything that would need `zdc-graph` or `zdc-types`
//! emits a diagnostic naming what is missing, because a program that
//! compiles to something broken is worse than one that refuses.

mod analysis;
mod elements;
mod expr;
mod js;
mod names;
mod pages;
mod stmt;
mod styles;
mod view;

use zdc_hir::{DefKind, Hir, HirNode};
use zdc_lexer::Span;
use zdc_types::{Page, TypeTable};

use crate::analysis::{Analysis, Shared};
use crate::expr::Emitter;
use crate::names::Names;
use crate::pages::Bindings;
use crate::stmt::Statements;
use crate::styles::Styles;
use crate::view::{Emission, Lowering, RuntimeImports};

pub use crate::elements::BUILT_INS;

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
    /// The page title, normally the source file's stem.
    pub name: String,
}

impl Options {
    pub fn new(source_path: impl Into<String>, name: impl Into<String>) -> Options {
        Options {
            source_path: source_path.into(),
            name: name.into(),
        }
    }
}

/// Everything a build writes out.
pub struct Bundle {
    pub client_js: String,
    pub styles_css: String,
    pub index_html: String,
    pub manifest_json: String,
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
    pub document_html: String,
}

/// Every document a program emits, and the map from URL to module.
pub struct SiteBundle {
    pub pages: Vec<PageBundle>,
    /// URL to module, for a host that has to answer a request without
    /// running the compiler.
    pub routes_json: String,
}

/// Compile a resolved, typechecked program into a client bundle.
///
/// `types` is not optional and never reconstructed here. §16.7 lists what
/// code generation is silently wrong without — the operand types of `+` and
/// `is`, the container behind `empty`, the choice a `when` eliminates — and
/// a compiler that answered those itself would be checking a program twice
/// and could disagree with itself about the result.
pub fn compile(
    hir: &Hir,
    types: &TypeTable,
    options: &Options,
) -> Result<Bundle, Vec<CodegenError>> {
    let nodes = view_nodes(hir)?;
    let emitted = emit(
        hir,
        types,
        options,
        &nodes,
        &Bindings::default(),
        Layout::Single,
        None,
    )?;
    Ok(Bundle {
        client_js: emitted.client_js,
        styles_css: emitted.styles_css,
        index_html: index_html(&options.name, "./client.js", "./styles.css"),
        manifest_json: manifest_json(hir, &emitted.names),
    })
}

/// Compile every document a program emits.
///
/// An unrouted program is one document at `/`, which is what it has
/// always been; a routed one is one document per enumerated URL plus the
/// not-found page. The two go through the same emitter, so there is no
/// second code path for a routed program to be wrong in.
pub fn compile_site(
    hir: &Hir,
    types: &TypeTable,
    options: &Options,
) -> Result<SiteBundle, Vec<CodegenError>> {
    let nodes = view_nodes(hir)?;
    let site = zdc_types::site(hir);
    if site.pages.is_empty() {
        let bundle = compile(hir, types, options)?;
        return Ok(SiteBundle {
            pages: vec![PageBundle {
                url: "/".to_string(),
                slug: "index".to_string(),
                client_js: bundle.client_js,
                styles_css: bundle.styles_css,
                document_html: bundle.index_html,
            }],
            routes_json: routes_json(&[("/".to_string(), "index".to_string())], None),
        });
    }

    // Computed once for the whole program, not once per page. §17.2's
    // split is already quadratic in definitions × roots and routing puts
    // one root per page on that axis; re-running the reactive-function
    // fixpoint per page would make it cubic, which is the one thing that
    // would bite at a realistic page count.
    let shared = Shared::new(hir);
    let mut pages = Vec::with_capacity(site.pages.len());
    let mut errors = Vec::new();
    let mut index = Vec::new();
    let mut not_found = None;
    for page in &site.pages {
        let specialised = pages::specialise(hir, &nodes, page);
        let module = format!("/pages/{}.js", page.slug);
        let styles = format!("/pages/{}.css", page.slug);
        match emit(
            hir,
            types,
            options,
            &specialised.nodes,
            &specialised.bindings,
            Layout::Page,
            Some(&shared),
        ) {
            Ok(emitted) => {
                if page.variant.is_none() {
                    not_found = Some(page.url.clone());
                }
                index.push((page.url.clone(), page.slug.clone()));
                pages.push(PageBundle {
                    url: page.url.clone(),
                    slug: page.slug.clone(),
                    client_js: emitted.client_js,
                    styles_css: emitted.styles_css,
                    document_html: index_html(&page_title(options, page), &module, &styles),
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
    Ok(SiteBundle {
        routes_json: routes_json(&index, not_found.as_deref()),
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
}

fn view_nodes(hir: &Hir) -> Result<Vec<HirNode>, Vec<CodegenError>> {
    let Some(view) = hir.view else {
        return Err(vec![CodegenError {
            message: "This program has no `view`, so there is nothing to render. Add one, or use \
                      `zdc check` to verify the file without building it."
                .to_string(),
            span: Span::new(0, 0),
        }]);
    };
    let DefKind::View(view) = &hir.defs[view].kind else {
        unreachable!("`Hir::view` names a view");
    };
    Ok(view.nodes.clone())
}

fn page_title(options: &Options, page: &Page) -> String {
    if page.url == "/" {
        return options.name.clone();
    }
    format!("{} · {}", options.name, page.url)
}

#[allow(clippy::too_many_arguments)]
fn emit(
    hir: &Hir,
    types: &TypeTable,
    options: &Options,
    nodes: &[HirNode],
    bindings: &Bindings,
    layout: Layout,
    shared: Option<&Shared>,
) -> Result<Emitted, Vec<CodegenError>> {
    let owned;
    let analysis = match (layout, shared) {
        (Layout::Single, _) => Analysis::new(hir),
        (Layout::Page, Some(shared)) => Analysis::page(hir, nodes, bindings, shared),
        (Layout::Page, None) => {
            owned = Shared::new(hir);
            Analysis::page(hir, nodes, bindings, &owned)
        }
    };
    let names = Names::new(hir, &analysis);
    let mut emitter = Emitter {
        hir,
        types,
        names: &names,
        analysis: &analysis,
        bindings,
        used: RuntimeImports::default(),
        errors: Vec::new(),
    };

    refuse_unsupported_placements(&mut emitter);

    let mut styles = Styles::default();
    let region = Lowering::new(&mut emitter, &mut styles).region(nodes);

    let functions = emit_functions(&mut emitter);
    let declarations = emit_declarations(&mut emitter);

    let errors = std::mem::take(&mut emitter.errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut used = std::mem::take(&mut emitter.used);

    let mut emission = Emission::new(&mut used);
    let mut body = emission.instance(&region, "$r", 2);
    let templates: Vec<String> = emission.templates().to_vec();
    let by_position = emission.needs_by_position();
    used.dom.insert("mount");
    body.push_str("  return mount($r, container);\n");

    let mut client_js = String::new();
    client_js.push_str(&format!(
        "// zdc {} · {} · generated, do not edit\n",
        env!("CARGO_PKG_VERSION"),
        options.source_path
    ));
    let runtime = layout.runtime();
    if !used.signal.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from '{runtime}/signal.js';\n",
            used.signal.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    if !used.dom.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from '{runtime}/dom.js';\n",
            used.dom.iter().copied().collect::<Vec<_>>().join(", ")
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
    if !functions.is_empty() {
        client_js.push('\n');
        client_js.push_str(&functions);
    }
    if !declarations.is_empty() {
        client_js.push('\n');
        client_js.push_str(&declarations);
    }
    client_js.push_str("\nexport function main(container) {\n");
    client_js.push_str(&body);
    client_js.push_str("}\n");

    Ok(Emitted {
        client_js,
        styles_css: styles.stylesheet(),
        names,
    })
}

/// This milestone emits a client bundle and nothing else, so every other
/// placement is refused by name rather than silently mis-emitted.
fn refuse_unsupported_placements(emitter: &mut Emitter) {
    let mut refusals: Vec<CodegenError> = Vec::new();
    for (_, def) in emitter.hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        if signal.secret {
            refusals.push(CodegenError {
                message: format!(
                    "`{}` is `secret`, and keeping a secret out of the client bundle is the \
                     information-flow pass's verdict to give. `zdc-graph` does not exist, and \
                     codegen refuses to run without a verdict rather than emit an unenforced \
                     guarantee (spec §16.3.12).",
                    def.name
                ),
                span: def.span,
            });
            continue;
        }
        // `static` is emitted: it is evaluated on the build host and
        // inlined, so no boundary is crossed at runtime and nothing that
        // does not exist is needed to cross one (§14C.3b).
        if !matches!(
            signal.placement,
            zdc_ast::Placement::Client | zdc_ast::Placement::Static
        ) {
            refusals.push(CodegenError {
                message: format!(
                    "`{}` is `{}`-placed, and this compiler emits a client bundle only. Crossing a \
                     placement boundary needs the placement closure from `zdc-graph`, the RPC \
                     client `runtime/rpc.js`, and — for `durable` — `runtime/store.js`. None of \
                     the three exists yet (spec §16.5, M6).",
                    def.name,
                    signal.placement.word()
                ),
                span: def.span,
            });
        }
    }
    emitter.errors.extend(refusals);
}

/// Signal declarations, per §16.3.4.
fn emit_declarations(emitter: &mut Emitter) -> String {
    let mut out = String::new();
    let ids: Vec<_> = emitter
        .hir
        .defs
        .iter()
        .map(|(id, _)| id)
        .filter(|id| emitter.analysis.client_closure().contains(id))
        .collect();

    for id in ids {
        let DefKind::Signal(signal) = &emitter.hir.defs[id].kind else {
            continue;
        };
        let is_source = signal.is_source;
        let placement = signal.placement;
        let init = signal.init;
        let name = emitter.names.def(id).to_string();
        let setter = emitter.names.setter(id).map(str::to_string);
        let value = emitter.value(init).into_text();

        // A `static` signal is a constant of the build, so it is a
        // binding rather than a cell: no `signal()`, no setter, no
        // subscription, and every read of it is a bare name.
        if placement == zdc_ast::Placement::Static {
            out.push_str(&format!("const {name} = {value};\n"));
            continue;
        }

        if is_source {
            emitter.used.signal.insert("signal");
            match setter {
                // `HirPlace.base` is a `Res`, so whether a signal is ever
                // written is exactly decidable — a never-written one needs
                // no setter binding at all.
                Some(setter) => {
                    out.push_str(&format!("const [{name}, {setter}] = signal({value});\n"))
                }
                None => out.push_str(&format!("const [{name}] = signal({value});\n")),
            }
        } else {
            // No dependency array and no topological sort: `derived` is
            // lazy, so source-order declaration is sound.
            emitter.used.signal.insert("derived");
            out.push_str(&format!("const {name} = derived(() => {value});\n"));
        }
    }
    out
}

/// Every function in the client closure. A function is colorless, so it is
/// emitted wherever it is reachable from (§16.3.12).
fn emit_functions(emitter: &mut Emitter) -> String {
    let mut out = String::new();
    let ids: Vec<_> = emitter
        .hir
        .defs
        .iter()
        .map(|(id, _)| id)
        .filter(|id| emitter.analysis.client_closure().contains(id))
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
        }
        .block(body, 2, &mut statements);

        out.push_str(&format!("function {name}({}) {{\n", params.join(", ")));
        out.push_str(&statements);
        out.push_str("}\n");
    }
    out
}

fn index_html(name: &str, module: &str, styles: &str) -> String {
    format!(
        "<!doctype html>\n\
         <meta charset=\"utf-8\">\n\
         <title>{}</title>\n\
         <link rel=\"stylesheet\" href=\"{styles}\">\n\
         <div id=\"app\"></div>\n\
         <script type=\"module\">\n\
         \x20 import {{ main }} from '{module}';\n\
         \x20 main(document.getElementById('app'));\n\
         </script>\n",
        js::html_text(name)
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
fn manifest_json(hir: &Hir, names: &Names) -> String {
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
    format!(
        "{{\"entry\":\"client.js\",\"functions\":[],\"durable\":[],\"signals\":{{{}}}}}\n",
        signals.join(",")
    )
}

/// The runtime files a bundle links against, as `(relative path, source)`.
///
/// `elements.js` is deliberately absent: generated code never imports it.
/// It remains the reference implementation the parity test checks the
/// compiler's shape table against.
pub fn runtime_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("runtime/signal.js", zdc_runtime::SIGNAL_JS),
        ("runtime/dom.js", zdc_runtime::DOM_JS),
    ]
}
