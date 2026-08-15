//! The pages themselves: one overview, and one page per source module.
//!
//! # Why the overview leads with placement
//!
//! A documentation generator for an ordinary language prints the things a
//! programmer wrote down: names, parameters, types, and whatever prose was
//! attached. Everything about *deployment* — which of these runs in the
//! browser, which is a network call, which is persisted — is somewhere
//! else entirely, in a router file, an ORM, a deployment manifest and a
//! bundler config, and no generator reads all four.
//!
//! Here it is on the left-hand side of every `state` declaration, and the
//! compiler has already turned it into a set of endpoints and a set of
//! store keys. So the overview is organised around placement rather than
//! around alphabetical order, and its first table's last column is the one
//! that could not exist elsewhere: what the browser gets when it reads
//! this signal, answered by the same read table (§14G.1.4) the type
//! checker used.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use zdc_ast as ast;
use zdc_graph::EndpointKind;
use zdc_hir::{DefId, DefKind, HirExprKind};

use crate::comments::Comments;
use crate::prose;
use crate::{DocFile, Inputs, Subject};

/// One source file and everything declared in it.
struct Module<'a> {
    path: &'a Path,
    source: &'a str,
    /// The page this module is written to, relative to the output
    /// directory.
    page: String,
    /// Its declarations, in source order.
    defs: Vec<DefId>,
}

pub fn render(inputs: &Inputs<'_>) -> Vec<DocFile> {
    let modules = modules(inputs);
    let mut files = vec![DocFile {
        path: PathBuf::from("index.md"),
        text: index(inputs, &modules),
    }];
    for module in &modules {
        files.push(DocFile {
            path: PathBuf::from(&module.page),
            text: page(inputs, module),
        });
    }
    files
}

/// Bucket every declaration these pages document by the file it was
/// written in.
///
/// The subject decides which declarations those are, and it cannot be read
/// off `Hir::is_prelude_def` in either direction:
///
/// * Documenting a **program**, `user_defs` rather than `defs`. The prelude
///   is resolved into the same arena and `zdc_lib::load` parses each of its
///   files from offset zero, so their spans overlap and documenting them
///   here would attribute eight files of standard library to whichever
///   module happened to sit at that offset.
/// * Documenting the **prelude**, every definition. `library::resolve`
///   compiles the library *as* the prelude — which is what makes its own
///   `contains` find `textContains` — so `user_defs` is empty and the
///   spans are the ones `library::linked` shifted into place.
fn modules<'a>(inputs: &Inputs<'a>) -> Vec<Module<'a>> {
    let mut out: Vec<Module<'a>> = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();

    let documented: Vec<(DefId, &zdc_hir::Def)> = match inputs.subject {
        Subject::Program(_) => inputs.hir.user_defs().collect(),
        Subject::Prelude => inputs.hir.defs.iter().collect(),
    };
    for (id, def) in documented {
        let (path, source, _) = inputs.linked.locate(def.span);
        match out.iter().position(|module| module.path == path) {
            Some(index) => out[index].defs.push(id),
            None => {
                let page = page_name(path, &mut taken);
                out.push(Module {
                    path,
                    source,
                    page,
                    defs: vec![id],
                });
            }
        }
    }
    out
}

/// The page file name for a source path, made unique.
///
/// Two modules in different directories can share a stem — `blog/post.zd`
/// and `docs/post.zd` — and a generator that wrote both to `post.md` would
/// silently document one of them twice. Mangling the whole path into the
/// name would be uglier for every program that has no collision, so the
/// suffix is added only to the module that would collide.
fn page_name(path: &Path, taken: &mut BTreeSet<String>) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("module");
    let mut candidate = format!("{stem}.md");
    let mut next = 2;
    while taken.contains(&candidate) {
        candidate = format!("{stem}-{next}.md");
        next += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

// --- the overview ---

fn index(inputs: &Inputs<'_>, modules: &[Module<'_>]) -> String {
    let name = match inputs.subject {
        Subject::Program(entry) => entry
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("program"),
        Subject::Prelude => "The ZDeceptron prelude",
    };

    let mut out = format!("# {name}\n\n");

    // The `view`'s own metadata is the closest thing a program has to a
    // one-line description of itself, and it is already a literal (§16.3.1
    // writes it into `index.html`), so it needs no interpretation here.
    if let Some(view) = inputs.hir.view {
        if let DefKind::View(view) = &inputs.hir.defs[view].kind {
            if let Some(title) = &view.metadata.title {
                let _ = writeln!(out, "**{title}**\n");
            }
            if let Some(description) = &view.metadata.description {
                let _ = writeln!(out, "{description}\n");
            }
        }
    }

    match inputs.subject {
        Subject::Program(entry) => {
            let _ = writeln!(
                out,
                "Generated by `zdc doc` from `{}`. Every line below is read from the program's \
                 own declarations — the placements, the types and the derived endpoints are the \
                 compiler's own answers, not a second description of them that could go out of \
                 date.\n",
                entry.display()
            );
        }
        // The library is a compilation unit rather than a set of compiler
        // built-ins (§17.4.1), and that is exactly why it can appear here:
        // these pages come out of the same resolver, split and type checker
        // that compile a program, run over the library's own sources.
        Subject::Prelude => {
            out.push_str(
                "Generated by `zdc doc --prelude` from the standard library's own sources, which \
                 ship inside the compiler and are written in ZDeceptron. Every line below is read \
                 from those declarations by the passes that compile them.\n\n",
            );
        }
    }

    state_section(inputs, modules, &mut out);
    network_section(inputs, &mut out);
    environment_section(inputs, &mut out);
    route_section(inputs, &mut out);
    module_section(inputs, modules, &mut out);
    out
}

/// The table this command exists for.
fn state_section(inputs: &Inputs<'_>, modules: &[Module<'_>], out: &mut String) {
    let mut signals: Vec<(DefId, &zdc_hir::Signal, &str)> = Vec::new();
    for module in modules {
        for id in &module.defs {
            if let DefKind::Signal(signal) = &inputs.hir.defs[*id].kind {
                signals.push((*id, signal, module.page.as_str()));
            }
        }
    }

    out.push_str("## Where the state lives\n\n");
    if signals.is_empty() {
        match inputs.subject {
            Subject::Program(_) => out.push_str("This program declares no state.\n\n"),
            // Not an assertion written here: this heading is filled by
            // the same walk over the same declarations that fills it for a
            // program, so an empty table *is* the colourlessness §17.4.1
            // step 6 requires. Put a `state` in the library and this
            // sentence becomes a table.
            Subject::Prelude => out.push_str(
                "The library declares no state, which is what stops a library call from adding \
                 an edge to the signal graph and therefore from changing any placement fact \
                 (spec §17.4.1). The absence is read off the declarations below, not asserted \
                 here.\n\n",
            ),
        }
        return;
    }

    // Counted per placement rather than listed, because the count is the
    // thing a reader wants at a glance: "most of this runs in the browser
    // and one thing is persisted" is the shape of a program.
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (_, signal, _) in &signals {
        *counts.entry(signal.placement.word()).or_default() += 1;
    }
    // `Placement::ALL`'s order, so the sentence reads client-to-durable
    // rather than alphabetically, and a fifth placement appears here
    // without anyone editing this line.
    let split: Vec<String> = ast::Placement::ALL
        .iter()
        .filter_map(|placement| {
            counts
                .get(placement.word())
                .map(|count| format!("{count} `{}`", placement.word()))
        })
        .collect();
    let _ = writeln!(
        out,
        "{} signal{} — {}.\n",
        signals.len(),
        if signals.len() == 1 { "" } else { "s" },
        split.join(", ")
    );

    out.push_str("| Signal | Type | Lives | Read from the browser as |\n|---|---|---|---|\n");
    for (id, signal, page) in &signals {
        let name = &inputs.hir.defs[*id].name;
        let ty = declared_type(inputs, *id, signal);
        let secret = if signal.secret { ", `secret`" } else { "" };
        let _ = writeln!(
            out,
            "| [`{name}`]({page}#{anchor}) | `{ty}` | `{placement}`{secret} | {read} |",
            anchor = anchor(name),
            placement = signal.placement.word(),
            read = cell(&prose::from_the_browser(
                signal.placement,
                signal.secret,
                &ty
            )),
        );
    }
    out.push('\n');

    // The legend, for the placements this program actually uses. A reader
    // who has never met `durable` needs the sentence; a reader of a
    // client-only program should not have to skip three of them.
    for placement in ast::Placement::ALL {
        if counts.contains_key(placement.word()) {
            let _ = writeln!(
                out,
                "- A `{}` signal {}",
                placement.word(),
                prose::placement_sentence(placement)
            );
        }
    }
    out.push('\n');
}

/// The endpoints, which nobody wrote.
fn network_section(inputs: &Inputs<'_>, out: &mut String) {
    out.push_str("## Where the network is\n\n");
    if inputs.split.endpoints.is_empty() {
        let _ = writeln!(
            out,
            "Nothing here crosses a placement boundary, so the compiler derived no endpoints: {} \
             is one bundle and makes no calls of its own.\n",
            match inputs.subject {
                Subject::Program(_) => "this program",
                Subject::Prelude => "the library",
            }
        );
        return;
    }

    let _ = writeln!(
        out,
        "The compiler derived {} endpoint{} from the placements above. None of them is written by \
         hand: an edge from one placement to another *is* the transport, so this table changes \
         when a `state` line changes and at no other time.\n",
        inputs.split.endpoints.len(),
        if inputs.split.endpoints.len() == 1 {
            ""
        } else {
            "s"
        }
    );
    out.push_str("| Endpoint | Emitted to | What it is | Takes |\n|---|---|---|---|\n");
    for endpoint in &inputs.split.endpoints {
        let what = match &endpoint.kind {
            EndpointKind::Value(def) => format!(
                "a value the browser reads — `{}`",
                inputs.hir.defs[*def].name
            ),
            EndpointKind::Command(key) => format!(
                "a command the browser performs — `{}` on `{}`",
                key.op.word(),
                inputs.hir.defs[key.signal].name
            ),
        };
        let takes: Vec<String> = endpoint
            .params
            .iter()
            .map(|param| format!("`{}`", inputs.hir.defs[*param].name))
            .collect();
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {what} | {} |",
            endpoint.name,
            // The emitter's own table, so a reader looking for this file
            // in `dist/` finds it (§17.2.5 fatal 3).
            zdc_codegen::file_name(&endpoint.name),
            if takes.is_empty() {
                "—".to_string()
            } else {
                takes.join(", ")
            }
        );
    }
    out.push('\n');
}

/// What has to exist in the environment before the program runs.
fn environment_section(inputs: &Inputs<'_>, out: &mut String) {
    // Every `environment "KEY"` the program's own expressions hold,
    // wherever it sits: a key read inside a `from` clause several calls
    // deep is as much a deployment requirement as one read directly.
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    for (id, expr) in inputs.hir.exprs.iter() {
        // The same split `modules` makes above, for the same reason:
        // documenting the prelude, every expression is a prelude one.
        let borrowed = match inputs.subject {
            Subject::Program(_) => inputs.hir.is_prelude_expr(id),
            Subject::Prelude => false,
        };
        if borrowed {
            continue;
        }
        if let HirExprKind::Environment(key) = &expr.kind {
            keys.insert(key.as_str());
        }
    }
    if keys.is_empty() {
        return;
    }

    out.push_str("## What it needs from the environment\n\n");
    out.push_str(
        "These keys are read with `environment`. A deployment that does not set them runs a \
         program that cannot compute the state below.\n\n",
    );
    for key in keys {
        let _ = writeln!(out, "- `{key}`");
    }
    out.push('\n');
}

/// The URLs, which are a bijection onto a `choice` (§14G.2).
fn route_section(inputs: &Inputs<'_>, out: &mut String) {
    let Some((choice_id, table)) = &inputs.hir.routes else {
        return;
    };
    let DefKind::Choice(choice) = &inputs.hir.defs[*choice_id].kind else {
        return;
    };

    out.push_str("## The URLs it serves\n\n");
    let _ = writeln!(
        out,
        "`{}` is a `route`, so these are the documents `zdc build` writes. A URL is a variant of \
         a `choice` and nothing else: `when` takes one apart exactly as it takes any choice \
         apart.\n",
        inputs.hir.defs[*choice_id].name
    );
    out.push_str("| URL | Variant | Parameter |\n|---|---|---|\n");
    for (index, variant) in table.variants.iter().enumerate() {
        // The URL as a pattern: the parameter's *name* stands in for the
        // value, because the value is chosen per document and the shape is
        // what a reader is looking for.
        let placeholders: Vec<String> = variant
            .params
            .iter()
            .map(|param| format!("{{{}}}", param.name))
            .collect();
        let url = table.url(index, &placeholders);
        let name = choice
            .variants
            .get(index)
            .map(|variant| variant.name.as_str())
            .unwrap_or("—");
        let params: Vec<String> = variant
            .params
            .iter()
            .map(|param| match param.enumerated_in {
                // §18.1 semantics 5 as amended by §21.7.6: the enumeration
                // decides how many documents are built and never that the
                // value is trustworthy, so the phrasing says the first and
                // not the second.
                Some(source) => format!(
                    "`{}`, one document per value of `{}`",
                    param.name, inputs.hir.defs[source].name
                ),
                None => format!("`{}`, from the URL bar", param.name),
            })
            .collect();
        let _ = writeln!(
            out,
            "| `{url}` | `{name}` | {} |",
            if params.is_empty() {
                "—".to_string()
            } else {
                cell(&params.join("; "))
            }
        );
    }
    out.push('\n');
}

fn module_section(inputs: &Inputs<'_>, modules: &[Module<'_>], out: &mut String) {
    out.push_str("## The files\n\n");
    for module in modules {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for id in &module.defs {
            *counts
                .entry(kind_word(&inputs.hir.defs[*id].kind))
                .or_default() += 1;
        }
        let summary: Vec<String> = counts
            .iter()
            .map(|(what, count)| format!("{count} {what}{}", if *count == 1 { "" } else { "s" }))
            .collect();
        let _ = writeln!(
            out,
            "- [`{}`]({}) — {}",
            module
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("module"),
            module.page,
            summary.join(", ")
        );
    }
    out.push('\n');
}

// --- one module's page ---

fn page(inputs: &Inputs<'_>, module: &Module<'_>) -> String {
    let comments = Comments::of(module.source);
    let mut out = format!(
        "# {}\n\n",
        module
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("module")
    );
    if let Some(header) = comments.header() {
        let _ = writeln!(out, "{header}\n");
    }

    // Grouped by the word [`kind_word`] gives each declaration rather
    // than by a `matches!` per section. Two reasons, and the second is the
    // load-bearing one: the order within a section stays source order, and
    // the *only* place this file decides what kind a declaration is is one
    // total match, so a ninth `DefKind` is a compile error here rather
    // than a section that silently documents nothing.
    let mut grouped: BTreeMap<&'static str, Vec<DefId>> = BTreeMap::new();
    for id in &module.defs {
        grouped
            .entry(kind_word(&inputs.hir.defs[*id].kind))
            .or_default()
            .push(*id);
    }

    for (word, heading) in SECTIONS {
        let Some(members) = grouped.get(word) else {
            continue;
        };
        let _ = writeln!(out, "## {heading}\n");
        for id in members {
            entry(inputs, &comments, &mut out, *id);
        }
    }
    out
}

/// The sections a page has, in the order it has them, keyed by the word
/// [`kind_word`] returns.
///
/// State first because placement is what a reader came for. The view last
/// because it is the one declaration whose *body* is the interesting part,
/// and no page here documents a body.
///
/// The length is written out, so a ninth kind of declaration cannot be
/// added without deciding where on a page it goes.
const SECTIONS: [(&str, &str); 8] = [
    ("signal", "State"),
    ("function", "Functions"),
    ("component", "Components"),
    ("record", "Records"),
    ("choice", "Choices"),
    ("foreign", "Foreign"),
    ("release", "Releases"),
    ("view", "The view"),
];

/// One declaration, in full.
fn entry(inputs: &Inputs<'_>, comments: &Comments<'_>, out: &mut String, id: DefId) {
    let def = &inputs.hir.defs[id];
    let name = def.name.as_str();
    let _ = writeln!(out, "### `{name}`\n");

    // The declaration line first and the prose after it, which is the
    // order a hover uses, so a reader moving between the editor and these
    // pages meets the same shape twice.
    let _ = writeln!(out, "{}\n", prose::fenced(&declaration(inputs, id)));

    // The programmer's own words, where there are any. This is the only
    // part of a page that is not derived, and it is the part worth reading.
    let (_, _, local) = inputs.linked.locate(def.span);
    if let Some(doc) = comments.above(local.start) {
        let _ = writeln!(out, "{doc}\n");
    }

    notes(inputs, out, id);
}

/// The declaration line for any definition.
fn declaration(inputs: &Inputs<'_>, id: DefId) -> String {
    let hir = inputs.hir;
    let def = &hir.defs[id];
    let name = def.name.as_str();
    let params = |ids: &[zdc_hir::LocalId]| -> Vec<String> {
        ids.iter().map(|id| hir.locals[*id].name.clone()).collect()
    };

    match &def.kind {
        DefKind::Signal(signal) => prose::signal_line(
            name,
            signal.placement,
            &declared_type(inputs, id, signal),
            signal.secret,
            signal.is_source,
            signal.clock,
            signal.schedule.as_ref().map(|schedule| schedule.cadence),
        ),
        DefKind::Function(function) => {
            prose::function_line(name, &params(&function.params), function.form)
        }
        DefKind::Component(component) => prose::component_line(
            name,
            &params(&component.params),
            component.children.is_some(),
        ),
        DefKind::Record(record) => {
            let mut out = format!("record {name}");
            for field in &record.fields {
                let _ = write!(
                    out,
                    "\n    {} is {}",
                    field.name,
                    prose::render_type(&field.ty)
                );
            }
            out
        }
        DefKind::Choice(choice) => {
            // A `route` is a `choice` plus a bijection onto URLs, and the
            // keyword the program wrote is the one to print back.
            let routed = hir.routes.as_ref().is_some_and(|(routed, _)| *routed == id);
            let keyword = if routed { "route" } else { "choice" };
            let mut out = format!("{keyword} {name}");
            for variant in &choice.variants {
                let _ = write!(out, "\n    {}", variant.name);
                let fields: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|field| format!("{} is {}", field.name, prose::render_type(&field.ty)))
                    .collect();
                if !fields.is_empty() {
                    let _ = write!(out, " with {}", fields.join(", "));
                }
            }
            out
        }
        DefKind::Foreign(foreign) => prose::foreign_line(name, foreign, &params(&foreign.params)),
        DefKind::Release(release) => {
            let mut out = format!("release {name}");
            let names = params(&release.params);
            if !names.is_empty() {
                let _ = write!(out, " with {}", names.join(", "));
            }
            let _ = write!(out, "\n    gives {}", prose::render_type(&release.gives));
            if let Some(limit) = release.limit {
                let _ = write!(out, "\n    limit {} per visitor", limit.count);
            }
            out
        }
        DefKind::View(view) => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(title) = &view.metadata.title {
                parts.push(format!("title is \"{title}\""));
            }
            if let Some(description) = &view.metadata.description {
                parts.push(format!("description is \"{description}\""));
            }
            if let Some(language) = &view.metadata.language {
                parts.push(format!("language is \"{language}\""));
            }
            if parts.is_empty() {
                "view".to_string()
            } else {
                format!("view {}", parts.join(", "))
            }
        }
    }
}

/// What is true of this declaration because of the kind of thing it is.
fn notes(inputs: &Inputs<'_>, out: &mut String, id: DefId) {
    match &inputs.hir.defs[id].kind {
        DefKind::Signal(signal) => {
            let name = &inputs.hir.defs[id].name;
            let _ = writeln!(out, "{}\n", prose::placement_note(name, signal.placement));
            let _ = writeln!(
                out,
                "Read from the browser it is {}.\n",
                prose::from_the_browser(
                    signal.placement,
                    signal.secret,
                    &declared_type(inputs, id, signal)
                )
            );
            if signal.secret {
                let _ = writeln!(out, "{}\n", prose::SECRET_NOTE);
            }
            if let Some(clock) = signal.clock {
                let _ = writeln!(out, "{}\n", prose::CLOCK_NOTE);
                let _ = writeln!(out, "It holds {}.\n", clock.describe());
            } else if signal.schedule.is_some() {
                let _ = writeln!(out, "{}\n", prose::SCHEDULE_NOTE);
            } else if !signal.is_source {
                let _ = writeln!(out, "{}\n", prose::DERIVED_NOTE);
            }
            if let Some(emitted) = &signal.emits {
                let _ = writeln!(
                    out,
                    "Its value is written to `{}` at build time (spec §14C.3b).\n",
                    emitted.path
                );
            }
        }
        DefKind::Function(_) => {
            let _ = writeln!(out, "{}\n", prose::FUNCTION_NOTE);
        }
        DefKind::Component(_) => {
            let _ = writeln!(out, "{}\n", prose::COMPONENT_NOTE);
        }
        DefKind::Record(record) => {
            let _ = writeln!(
                out,
                "{} field{}.\n",
                record.fields.len(),
                if record.fields.len() == 1 { "" } else { "s" }
            );
        }
        DefKind::Choice(choice) => {
            let _ = writeln!(
                out,
                "{} variant{}. Take it apart with `when`, which must eliminate every one of \
                 them.\n",
                choice.variants.len(),
                if choice.variants.len() == 1 { "" } else { "s" }
            );
        }
        DefKind::Foreign(foreign) => {
            let _ = writeln!(
                out,
                "Its types are asserted rather than inferred, because it has no ZDeceptron body \
                 (spec §14E.4). {}\n",
                prose::foreign_site_note(foreign.site)
            );
        }
        DefKind::Release(release) => {
            let _ = writeln!(
                out,
                "Declassifies: the result is Public however Secret the inputs were (spec \
                 §19.1).\n"
            );
            if let Some(limit) = release.limit {
                let _ = writeln!(
                    out,
                    "**`limit {} per visitor` is not a cumulative disclosure bound.** It counts \
                     evaluations of this one declaration against one anonymous session: a second \
                     release declaration carries its own budget, and clearing a cookie mints a \
                     fresh one (§21.8.7).\n",
                    limit.count
                );
            }
        }
        DefKind::View(_) => {
            let _ = writeln!(
                out,
                "The program's view. Everything under it runs in client context (spec §5.6).\n"
            );
        }
    }
}

/// A signal's type, preferring the checker's answer to the written one.
///
/// They agree wherever both exist; the written form is the fallback for a
/// signal the table has no entry for, which is better than printing
/// nothing.
fn declared_type(inputs: &Inputs<'_>, id: DefId, signal: &zdc_hir::Signal) -> String {
    match inputs.table.def(id) {
        Some(ty) => ty.to_string(),
        None => prose::render_type(&signal.ty),
    }
}

/// The word for a kind of declaration, for the per-file summary.
fn kind_word(kind: &DefKind) -> &'static str {
    match kind {
        DefKind::Signal(_) => "signal",
        DefKind::Function(_) => "function",
        DefKind::Component(_) => "component",
        DefKind::Record(_) => "record",
        DefKind::Choice(_) => "choice",
        DefKind::Foreign(_) => "foreign",
        DefKind::Release(_) => "release",
        DefKind::View(_) => "view",
    }
}

/// A heading's anchor, as the Markdown renderers generate it: lowercased,
/// with everything that is not a word character or a dash removed.
fn anchor(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Text that is about to sit inside a table cell.
///
/// A `|` in a cell ends the cell, so a type or a message holding one would
/// silently shift every column after it. Nothing in the vocabulary
/// produces one today; this is here so that the day something does, the
/// table survives it.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}
