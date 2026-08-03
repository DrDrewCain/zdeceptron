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
mod server;
mod stmt;
mod styles;
mod view;

use std::collections::BTreeSet;

use zdc_graph::{EndpointKind, RootId, TierSplit, Verdict, CLIENT};
use zdc_hir::{DefId, DefKind, Hir};
use zdc_lexer::Span;
use zdc_types::TypeTable;

use crate::analysis::Analysis;
use crate::expr::Emitter;
use crate::names::Names;
use crate::stmt::Statements;
use crate::styles::Styles;
use crate::view::{Emission, Lowering, RuntimeImports};

pub use crate::elements::BUILT_INS;
pub use crate::server::{file_name, FunctionKind, ServerFunction};

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
    /// One file per emitted server root — §17.2.3's `Endpoint` and
    /// `Command` origins. Empty for a program with no crossing, which is
    /// how `hello.zd` still ships nothing it does not use.
    pub functions: Vec<ServerFunction>,
}

/// Everything emission reads. All four, or it refuses (§17.1.3).
pub struct Inputs<'a> {
    pub hir: &'a Hir,
    pub split: &'a TierSplit,
    pub verdict: &'a Verdict,
    pub table: &'a TypeTable,
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
    let Inputs {
        hir,
        split,
        verdict,
        table,
    } = *inputs;

    // §16.3.12: code generation refuses to run without a verdict, and
    // refuses to run on a rejected one. An unenforced invariant 3 is worse
    // than no build.
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

    let client_members = split.client_members();
    let analysis = Analysis::new(hir);
    // A signal written only through a generated command has no cell in the
    // browser and therefore needs no setter: the write is an RPC.
    let written: BTreeSet<DefId> = analysis
        .written()
        .iter()
        .copied()
        .filter(|def| client_members.contains(def))
        .collect();
    let names = Names::new(hir, &written);

    let mut emitter = Emitter {
        hir,
        types: table,
        names: &names,
        analysis: &analysis,
        used: RuntimeImports::default(),
        split,
        ctx: split.root(CLIENT).ctx,
        root: CLIENT,
        errors: Vec::new(),
    };

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

    let functions = emit_functions(&mut emitter, &client_members);
    let declarations = emit_declarations(&mut emitter, &client_members);
    let remotes = emit_remotes(&mut emitter);

    // The server roots, emitted last so every diagnostic from the client
    // walk is already collected and the two lists come out together.
    let server = {
        let mut server_emitter = Emitter {
            hir,
            types: table,
            names: &names,
            analysis: &analysis,
            used: RuntimeImports::default(),
            split,
            ctx: split.root(CLIENT).ctx,
            root: CLIENT,
            errors: Vec::new(),
        };
        let emitted = emit_server(
            hir,
            split,
            &names,
            &mut server_emitter,
            &options.source_path,
        );
        emitter.errors.extend(server_emitter.errors);
        emitted
    };

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
    if !used.rpc.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from './runtime/rpc.js';\n",
            used.rpc.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    if !used.store.is_empty() {
        client_js.push_str(&format!(
            "import {{ {} }} from './runtime/store.js';\n",
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
    client_js.push_str("\nexport function main(container) {\n");
    client_js.push_str(&body);
    client_js.push_str("}\n");

    Ok(Bundle {
        client_js,
        styles_css: styles.stylesheet(),
        index_html: index_html(&options.name),
        manifest_json: manifest_json(hir, split, &names, &server),
        functions: server,
    })
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
fn emit_declarations(emitter: &mut Emitter, client_members: &BTreeSet<DefId>) -> String {
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
fn emit_functions(emitter: &mut Emitter, client_members: &BTreeSet<DefId>) -> String {
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
fn manifest_json(
    hir: &Hir,
    split: &TierSplit,
    names: &Names,
    functions: &[ServerFunction],
) -> String {
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

    let mut durable: Vec<String> = split
        .reads_keys
        .values()
        .chain(split.writes_keys.values())
        .flat_map(|keys| keys.iter().map(|key| hir.defs[*key].name.clone()))
        .collect();
    durable.sort();
    durable.dedup();
    let durable: Vec<String> = durable.iter().map(|key| js::json_string(key)).collect();

    format!(
        "{{\"entry\":\"client.js\",\"functions\":[{}],\"durable\":[{}],\"signals\":{{{}}}}}\n",
        emitted.join(","),
        durable.join(","),
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
        ("runtime/rpc.js", zdc_runtime::RPC_JS),
        ("runtime/store.js", zdc_runtime::STORE_JS),
    ]
}
