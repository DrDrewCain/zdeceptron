//! Function bundles — one file per emitted server root.
//!
//! A function bundle emits **zero import statements**. Its only external
//! references are `$env` and `$store`, injected by the platform adapter
//! (§8.2), which is what makes §16.3.12's invariant 4 a syntactic property
//! of the output rather than a claim about it.
//!
//! What goes in the file is not a decision made here. `members(r)` is the
//! split's answer, `hoisted[(d, r)]` says whether a member can live at
//! module scope or must be nested where the lifted parameters are in
//! lexical scope, and `params(r)` is the wire signature. This module is a
//! printer.

use zdc_graph::{EndpointKind, MemberForm, RootId, TierSplit};
use zdc_hir::{DefId, DefKind, Hir};

use crate::expr::Emitter;
use crate::js;
use crate::names::Names;
use crate::stmt::Statements;

/// One emitted server file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFunction {
    /// Path relative to the output directory, such as
    /// `functions/greeting.js`.
    pub path: String,
    /// The endpoint name the client calls, such as `visits.incr`.
    pub name: String,
    /// The wire order of the endpoint's inputs.
    pub inputs: Vec<String>,
    pub source: String,
}

/// The file name an endpoint is emitted to. `.` is legal in a POSIX file
/// name and is what makes `visits.incr` and `visits.decr` distinct files
/// without an escaping scheme (§17.2.5 fatal 3).
pub fn file_name(endpoint: &str) -> String {
    format!("functions/{endpoint}.js")
}

pub fn emit_one(
    hir: &Hir,
    split: &TierSplit,
    names: &Names,
    emitter: &mut Emitter<'_>,
    endpoint: &zdc_graph::Endpoint,
    source_path: &str,
) -> Option<ServerFunction> {
    let root = endpoint.root;
    let inputs: Vec<String> = endpoint
        .params
        .iter()
        .map(|param| names.def(*param).to_string())
        .collect();

    let body = match &endpoint.kind {
        EndpointKind::Value(def) => value_body(hir, split, names, emitter, root, *def, &inputs),
        EndpointKind::Command(key) => command_body(hir, names, key),
    };

    let mut source = String::new();
    source.push_str(&format!(
        "// zdc {} · {source_path} · generated, do not edit\n",
        env!("CARGO_PKG_VERSION")
    ));
    source.push_str(
        "// No imports. `$env` and `$store` are injected by the platform adapter (§8.2).\n",
    );
    source.push_str(&body);

    Some(ServerFunction {
        path: file_name(&endpoint.name),
        name: endpoint.name.clone(),
        inputs,
        source,
    })
}

/// A value endpoint: the browser reads a `server` or `durable` signal, and
/// this recomputes it from the inputs the browser supplied.
fn value_body(
    hir: &Hir,
    split: &TierSplit,
    names: &Names,
    emitter: &mut Emitter<'_>,
    root: RootId,
    result: DefId,
    inputs: &[String],
) -> String {
    let members: Vec<(DefId, MemberForm)> = split.members_of(root).collect();
    let hoisted = |def: DefId| split.hoisted.get(&(def, root)).copied().unwrap_or(true);

    let mut module = String::new();
    let mut nested = String::new();

    // Functions first, at whichever scope they belong to. A function that
    // needs no lifted value is emitted at module scope in every root, so
    // no instantiation ever changes arity (§17.2.8).
    for (def, form) in &members {
        if *form != MemberForm::Function {
            continue;
        }
        let text = function_text(hir, names, emitter, *def, if hoisted(*def) { 0 } else { 2 });
        if hoisted(*def) {
            module.push_str(&text);
        } else {
            nested.push_str(&text);
        }
    }

    // Then the signal bindings, dependencies first. `static_order` is the
    // topological order E0320 has just established exists.
    let mut signals: Vec<(DefId, MemberForm)> = members
        .iter()
        .filter(|(_, form)| matches!(form, MemberForm::Binding | MemberForm::StoreRead))
        .copied()
        .collect();
    // `static_order` is dependencies-first, which is the order a run of
    // `const` bindings has to be written in: a `const` referenced before
    // its declaration is a temporal-dead-zone `ReferenceError`, not a
    // hoisted `undefined`.
    signals.sort_by_key(|(def, _)| {
        split
            .static_order
            .iter()
            .position(|id| id == def)
            .unwrap_or(usize::MAX)
    });

    for (def, form) in &signals {
        let name = names.def(*def).to_string();
        match form {
            MemberForm::StoreRead => {
                nested.push_str(&format!(
                    "  const {name} = await $store.get({});\n",
                    js::string(&hir.defs[*def].name)
                ));
            }
            _ => {
                let DefKind::Signal(signal) = &hir.defs[*def].kind else {
                    continue;
                };
                let init = signal.init;
                let value = emitter.value(init).into_text();
                nested.push_str(&format!("  const {name} = {value};\n"));
            }
        }
    }

    let mut out = module;
    out.push_str(&format!(
        "\nexport async function handler({{ {} }}) {{\n",
        inputs.join(", ")
    ));
    out.push_str(&nested);
    out.push_str(&format!("  return {};\n}}\n", names.def(result)));
    out
}

/// A command endpoint: the browser asked for a write it cannot perform.
///
/// Only the place resolution and the store operator run here; the
/// right-hand side and every index arrived as arguments, evaluated in the
/// region that asked (§17.2.7).
fn command_body(hir: &Hir, names: &Names, key: &zdc_graph::CommandKey) -> String {
    let _ = names;
    let store_key = js::string(&hir.defs[key.signal].name);
    // The same word the command's own name was rendered from, so the
    // endpoint and the store operation cannot disagree.
    let operator = key.op.word();
    let mut arguments = vec!["$args[0]".to_string()];
    for (index, _) in key
        .path
        .iter()
        .filter(|segment| matches!(segment, zdc_graph::PathKeySeg::Index))
        .enumerate()
    {
        arguments.push(format!("$args[{}]", index + 1));
    }
    let path: Vec<String> = key
        .path
        .iter()
        .filter_map(|segment| match segment {
            zdc_graph::PathKeySeg::Field(field) => Some(js::string(field)),
            zdc_graph::PathKeySeg::Index => None,
        })
        .collect();
    let path = if path.is_empty() {
        String::new()
    } else {
        format!(", [{}]", path.join(", "))
    };

    format!(
        "\nexport async function handler($args) {{\n  return await $store.{operator}({store_key}, \
         {}{path});\n}}\n",
        arguments.join(", ")
    )
}

pub(crate) fn function_text(
    hir: &Hir,
    names: &Names,
    emitter: &mut Emitter<'_>,
    def: DefId,
    indent: usize,
) -> String {
    let DefKind::Function(function) = &hir.defs[def].kind else {
        return String::new();
    };
    let body = function.body;
    let params: Vec<String> = function
        .params
        .iter()
        .map(|param| names.local(*param).to_string())
        .collect();
    let name = names.def(def).to_string();

    let mut statements = String::new();
    Statements {
        emitter,
        temporaries: 0,
    }
    .block(body, indent + 2, &mut statements);

    let pad = " ".repeat(indent);
    format!(
        "{pad}function {name}({}) {{\n{statements}{pad}}}\n",
        params.join(", ")
    )
}
