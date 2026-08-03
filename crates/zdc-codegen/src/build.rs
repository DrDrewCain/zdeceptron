//! The `BUILD` root — spec §17.4.8.
//!
//! A `static` signal is computed once, on the build host, and its value is
//! inlined into the bundle as a literal. §17.4.8 rejected a Rust
//! tree-walking interpreter for it: that would need a third implementation
//! of every primitive, checked by nothing, and it would have no rule at all
//! for a `foreign` declaration, which has no body to walk.
//!
//! **What replaces it.** The `BUILD` root is printed as an ordinary
//! JavaScript module, exactly like a server root, and executed on the build
//! host — which §14G.1.5 already established *is* a server environment. One
//! implementation of each primitive, and `foreign` works at build time
//! because the build host can import the module.
//!
//! This module is the printer. Running the result is [`crate::evaluate`],
//! and inlining what it printed is [`crate::expr::Emitter::reference`].

use zdc_graph::{MemberForm, BUILD};
use zdc_hir::{DefId, DefKind};

use crate::expr::Emitter;
use crate::js;
use crate::names::Names;
use crate::server::function_text;

/// The `BUILD` root, printed, together with the names it exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildModule {
    pub source: String,
    /// Every `static` signal the module computes, by its **source** name.
    ///
    /// The source name, not the emitted one: it is what the diagnostics
    /// print, and it is the key the inlining step looks values up by, so
    /// the two sides cannot drift apart the way a mangled name would.
    pub statics: Vec<String>,
    /// The files this build writes, as `(path in the bundle, source name)`
    /// — §14C.3b's sub-requirement.
    ///
    /// Empty for a program that only reads at build time. `rss.xml` and
    /// `llms.txt` are the case this exists for: build-time *outputs*
    /// derived from build-time *inputs*, which is what stops them drifting
    /// from the pages built from the same state.
    pub emits: Vec<(String, String)>,
}

/// Print the `BUILD` root, or `None` if the program has no `static` state.
///
/// `None` is not an error and not an empty module: §17.4.8's named cost is
/// that `zdc build` needs a JavaScript runtime on the build host **for any
/// program using `static`**, and a program that uses none must not pay it.
/// `hello.zd` through `todo.zd` still build on a host with no `node`.
pub fn module(emitter: &mut Emitter<'_>, names: &Names, source_path: &str) -> Option<BuildModule> {
    let hir = emitter.hir;
    let split = emitter.split;

    let members: Vec<(DefId, MemberForm)> = split.members_of(BUILD).collect();
    let statics: Vec<DefId> = members
        .iter()
        .filter(|(_, form)| *form == MemberForm::Inlined)
        .map(|(def, _)| *def)
        .collect();
    if statics.is_empty() {
        return None;
    }

    emitter.root = BUILD;
    emitter.ctx = split.root(BUILD).ctx;

    let mut out = String::new();
    out.push_str(&format!(
        "// zdc {} · {source_path} · the build root, generated, do not edit\n",
        env!("CARGO_PKG_VERSION")
    ));

    for (def, form) in &members {
        if *form != MemberForm::Function {
            continue;
        }
        out.push_str(&function_text(hir, names, emitter, *def, 0));
    }

    // Dependencies first. A `const` referenced above its declaration is a
    // temporal-dead-zone `ReferenceError`, not a hoisted `undefined`, so
    // this order is a correctness requirement and not a formatting choice.
    let mut bindings: Vec<DefId> = members
        .iter()
        .filter(|(_, form)| matches!(form, MemberForm::Binding | MemberForm::Inlined))
        .map(|(def, _)| *def)
        .collect();
    bindings.sort_by_key(|def| {
        split
            .static_order
            .iter()
            .position(|id| id == def)
            .unwrap_or(usize::MAX)
    });

    if !bindings.is_empty() {
        out.push('\n');
    }
    for def in bindings {
        let DefKind::Signal(signal) = &hir.defs[def].kind else {
            continue;
        };
        let init = signal.init;
        let value = emitter.value(init).into_text();
        out.push_str(&format!("const {} = {value};\n", names.def(def)));
    }

    // Keyed by source name, in declaration order, so the printed module and
    // the diagnostics agree about what a value is called.
    let mut exported: Vec<String> = Vec::new();
    let mut entries: Vec<String> = Vec::new();
    let mut files: Vec<(String, String)> = Vec::new();
    let mut file_entries: Vec<String> = Vec::new();
    for (def, _) in hir.defs.iter() {
        if !statics.contains(&def) {
            continue;
        }
        let source_name = hir.defs[def].name.clone();
        entries.push(format!(
            "  {}: {}",
            js::string(&source_name),
            names.def(def)
        ));
        exported.push(source_name.clone());

        let DefKind::Signal(signal) = &hir.defs[def].kind else {
            continue;
        };
        if let Some(emitted) = &signal.emits {
            file_entries.push(format!(
                "  {}: {}",
                js::string(&emitted.path),
                names.def(def)
            ));
            files.push((emitted.path.clone(), source_name));
        }
    }

    out.push_str(&format!(
        "\nexport const $values = {{\n{},\n}};\n",
        entries.join(",\n")
    ));
    // Always exported, empty or not. A conditional export would make "this
    // program emits no files" and "this module predates file emission" the
    // same observation for the driver that reads it.
    out.push_str(&format!(
        "\nexport const $files = {{{}}};\n",
        if file_entries.is_empty() {
            String::new()
        } else {
            format!("\n{},\n", file_entries.join(",\n"))
        }
    ));

    Some(BuildModule {
        source: out,
        statics: exported,
        emits: files,
    })
}
