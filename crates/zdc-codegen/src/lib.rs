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
    let names = Names::new(hir, analysis.written());
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
        index_html: index_html(&options.name),
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

fn index_html(name: &str) -> String {
    format!(
        "<!doctype html>\n\
         <meta charset=\"utf-8\">\n\
         <title>{}</title>\n\
         <link rel=\"stylesheet\" href=\"./styles.css\">\n\
         <div id=\"app\"></div>\n\
         <script type=\"module\">\n\
         \x20 import {{ main }} from './client.js';\n\
         \x20 main(document.getElementById('app'));\n\
         </script>\n",
        js::html_text(name)
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
