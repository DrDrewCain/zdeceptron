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
pub mod assets;
mod elements;
mod events;
mod expr;
mod js;
mod names;
mod stmt;
mod styles;
mod view;

use zdc_hir::{DefKind, Hir};
use zdc_lexer::Span;
use zdc_types::TypeTable;

use crate::analysis::Analysis;
use crate::expr::Emitter;
use crate::names::Names;
use crate::stmt::Statements;
use crate::styles::Styles;
use crate::view::{Emission, Lowering, RuntimeImports};

pub use crate::elements::{BUILT_INS, HEADING_TAGS};

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
            stylesheets: Vec::new(),
        }
    }

    /// The stylesheets the asset directory contributed, in cascade order.
    pub fn with_stylesheets(mut self, stylesheets: Vec<String>) -> Options {
        self.stylesheets = stylesheets;
        self
    }
}

/// Everything a build writes out.
pub struct Bundle {
    pub client_js: String,
    pub styles_css: String,
    pub index_html: String,
    pub manifest_json: String,
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
/// # The one refusal this does not repeat
///
/// A file with no `view` is refused by `compile` and accepted here. That is
/// not a diagnostic split sneaking back in, because "this program has no
/// `view`" is not a statement about the program: §14D.2 makes every `.zd`
/// file a module, and `examples/model.zd` is a module that declares a
/// `record` and a `function` and is meant to be imported. Nothing is wrong
/// with it. What `compile` is reporting is that it was asked for a page and
/// there is none to build — an answer to `zdc build`'s question, in the
/// same category as an unwritable output directory, and the refusal's own
/// text says so: it ends *"or use `zdc check` to verify the file without
/// building it"*. Making `zdc check` repeat it would refuse every library
/// file in the language and contradict the message while doing it.
///
/// Every other refusal codegen owns is a statement about the program, and
/// every one of them is below.
pub fn check(hir: &Hir, types: &TypeTable) -> Vec<CodegenError> {
    if hir.view.is_none() {
        return Vec::new();
    }
    // The options only name the source path and the fallback page title, and
    // both are read after the last refusal is collected, so no diagnostic
    // can depend on them.
    let options = Options::new("<check>", "check");
    match compile(hir, types, &options) {
        Ok(_) => Vec::new(),
        Err(errors) => errors,
    }
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
    let analysis = Analysis::new(hir);
    let names = Names::new(hir, &analysis);
    let mut emitter = Emitter {
        hir,
        types,
        names: &names,
        analysis: &analysis,
        used: RuntimeImports::default(),
        errors: Vec::new(),
    };

    refuse_unsupported_placements(&mut emitter);

    let Some(view) = hir.view else {
        return Err(vec![CodegenError {
            // build-only: a file with no `view` is a module, not a broken
            // program (§14D.2), so `check` accepts it and only a request
            // for a page is answered with this. See `check` above for why
            // that is one command's answer rather than a split.
            message: "This program has no `view`, so there is nothing to render. Add one, or use \
                      `zdc check` to verify the file without building it."
                .to_string(),
            span: Span::new(0, 0),
        }]);
    };

    let mut styles = Styles::default();

    let DefKind::View(view) = &hir.defs[view].kind else {
        unreachable!("`Hir::view` names a view");
    };
    let view_metadata = view.metadata.clone();
    let region = Lowering::new(&mut emitter, &mut styles).region(&view.nodes);

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
    if !used.signal.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from './runtime/signal.js';\n",
            used.signal.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    if !used.dom.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from './runtime/dom.js';\n",
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

    Ok(Bundle {
        client_js,
        styles_css: styles.stylesheet(),
        index_html: index_html(&view_metadata, options),
        manifest_json: manifest_json(hir, &names),
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
        if signal.placement != zdc_ast::Placement::Client {
            refusals.push(CodegenError {
                message: format!(
                    "`{}` is `{}`-placed, and this compiler emits a client bundle only. Crossing a \
                     placement boundary needs the placement closure from `zdc-graph`, the RPC \
                     client `runtime/rpc.js`, and — for `durable` — `runtime/store.js`. None of \
                     the three exists yet (spec §16.5, M6).",
                    def.name,
                    match signal.placement {
                        zdc_ast::Placement::Server => "server",
                        zdc_ast::Placement::Durable => "durable",
                        zdc_ast::Placement::Client => "client",
                    }
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

/// The document, including the head the program asked for.
///
/// `<html>` and `<body>` are written out rather than left implicit,
/// because `lang` belongs on the first of them and a document with no
/// declared language is one a screen reader has to guess the pronunciation
/// of. The viewport line is not optional either: without it a phone
/// renders the page at 980 CSS pixels and scales it down.
///
/// Every interpolation is escaped, and metadata is a string literal from
/// the source by construction (`zdc-resolve` refuses anything else), so
/// nothing computed reaches this file.
fn index_html(metadata: &zdc_hir::Metadata, options: &Options) -> String {
    let title = metadata.title.as_deref().unwrap_or(&options.name);
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
    head.push_str("  <link rel=\"stylesheet\" href=\"./styles.css\">\n");
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
         \x20   import {{ main }} from './client.js';\n\
         \x20   main(document.getElementById('app'));\n\
         \x20 </script>\n\
         </body>\n\
         </html>\n",
        js::html_attribute(language)
    )
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
        let placement = match signal.placement {
            zdc_ast::Placement::Client => "client",
            zdc_ast::Placement::Server => "server",
            zdc_ast::Placement::Durable => "durable",
        };
        signals.push(format!("\"{}\":\"{placement}\"", names.def(id)));
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
