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

use std::collections::BTreeSet;

use zdc_graph::{MemberForm, BUILD};
use zdc_hir::{DefId, DefKind, ExprId, HirExprKind};
use zdc_lexer::Span;

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
    /// Every `test` declaration's claim, in the order `$tests` holds them
    /// — issue #169.
    ///
    /// Parallel to `statics` and for the same reason: the runner asks the
    /// module one question per index, so this vector *is* the contract
    /// between the printer and the evaluator, and neither side has a
    /// separate list that could disagree.
    pub tests: Vec<Claim>,
}

/// One `test` declaration, as the printed module carries it — issue #169.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// The sentence the test asserts, which is the definition's name.
    pub claim: String,
    /// The `expect` clause, for the caret of a failure.
    pub span: Span,
    /// Whether the expectation's outermost operator is `is` or `is not`.
    ///
    /// When it is, the two operands are printed as thunks of their own and
    /// a broken claim can say *what each side came to* rather than only
    /// that the whole thing was `no`. When it is not — `a and b`,
    /// `contains`, a call returning a `Truth` — there are no two sides to
    /// show and the report says so instead of inventing a pair.
    pub comparison: bool,
}

/// Print the `BUILD` root, or `None` if the program has no `static` state
/// and makes no claims.
///
/// `None` is not an error and not an empty module: §17.4.8's named cost is
/// that `zdc build` needs a JavaScript runtime on the build host **for any
/// program using `static`**, and a program that uses none must not pay it.
/// `hello.zd` through `todo.zd` still build on a host with no `node`.
///
/// A `test` declaration brings the module into existence for the same
/// reason a `static` signal does — its expectation is build-time code and
/// there is nowhere else for it to go — but it is printed as a **thunk**,
/// never as a `const`. That is the whole of why `zdc build` is unaffected
/// by a file full of claims: loading the module defines the thunks and
/// runs none of them, so a claim that is false, slow, or throws costs a
/// build nothing. Only `zdc test` calls them (issue #169).
pub fn module(emitter: &mut Emitter<'_>, names: &Names, source_path: &str) -> Option<BuildModule> {
    let hir = emitter.hir;
    let split = emitter.split;

    let members: Vec<(DefId, MemberForm)> = split.members_of(BUILD).collect();
    let statics: Vec<DefId> = members
        .iter()
        .filter(|(_, form)| *form == MemberForm::Inlined)
        .map(|(def, _)| *def)
        .collect();
    let claims: Vec<DefId> = members
        .iter()
        .filter(|(_, form)| *form == MemberForm::Test)
        .map(|(def, _)| *def)
        .collect();
    if statics.is_empty() && claims.is_empty() {
        return None;
    }

    emitter.root = BUILD;
    emitter.ctx = split.root(BUILD).ctx;

    // The body is built first and the header and preamble prepended at the
    // end, because which helpers the preamble needs is only known once the
    // body has asked for them. `$force` is the case that made this
    // necessary: an ordinary pipeline in a `static` initialiser emits a
    // call to it, and a build root that printed the call without the
    // definition failed at evaluation with `ReferenceError` rather than at
    // compile time.
    let mut out = String::new();

    // Every function in a build root is at one scope, so a cycle either
    // is wholly here or is not here at all (#198).
    let groups = crate::tailgroup::TailGroups::find(hir);
    let present: BTreeSet<DefId> = members
        .iter()
        .filter(|(_, form)| *form == MemberForm::Function)
        .map(|(def, _)| *def)
        .collect();

    for (def, form) in &members {
        if *form != MemberForm::Function {
            continue;
        }
        out.push_str(&function_text(
            hir, names, emitter, *def, 0, &groups, &present,
        ));
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

    // The empty case is spelled out rather than folded into the `{}\n…\n{}`
    // form above, because before claims existed it could not arise: a build
    // root was printed only when there was at least one `static` to put in
    // it, so `entries` was never empty and `{{\n{},\n}}` never printed the
    // bare `{\n,\n}` that is a syntax error. A `test` brings the module into
    // existence with no statics at all, which is the first time this branch
    // is reachable (issue #169).
    out.push_str(&format!(
        "\nexport const $values = {{{}}};\n",
        if entries.is_empty() {
            String::new()
        } else {
            format!("\n{},\n", entries.join(",\n"))
        }
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

    // The claims, in declaration order — issue #169.
    //
    // **Thunks, not bindings.** Every other entry above is a `const` whose
    // value is computed when the module loads; these are arrow functions
    // that compute nothing until called. That is the difference between
    // `zdc build` being unaffected by a program's tests and `zdc build`
    // failing because one of them is false.
    //
    // `left` and `right` are printed only for a comparison, and they are
    // printed as **separate thunks over the same subexpressions** rather
    // than by taking the result apart afterwards: `is` in this language is
    // not always `===` — the emitter chooses the comparison from the
    // operand types — so there is no result to take apart. Two thunks
    // evaluate the two sides the reader wrote, which is what the report
    // needs to show.
    let mut claim_entries: Vec<String> = Vec::new();
    let mut claimed: Vec<Claim> = Vec::new();
    for (def, _) in hir.defs.iter() {
        if !claims.contains(&def) {
            continue;
        }
        let DefKind::Signal(signal) = &hir.defs[def].kind else {
            continue;
        };
        let Some(span) = signal.expectation else {
            continue;
        };
        let claim = hir.defs[def].name.clone();
        let sides = comparison_sides(hir, signal.init);
        let mut fields = vec![
            format!("  claim: {}", js::string(&claim)),
            format!("  run: () => {}", emitter.value(signal.init).into_text()),
        ];
        if let Some((lhs, rhs)) = sides {
            fields.push(format!("  left: () => {}", emitter.value(lhs).into_text()));
            fields.push(format!("  right: () => {}", emitter.value(rhs).into_text()));
        }
        claim_entries.push(format!("{{\n{},\n}}", fields.join(",\n")));
        claimed.push(Claim {
            claim,
            span,
            comparison: sides.is_some(),
        });
    }
    // Exported unconditionally, for the reason `$files` is: a program with
    // no claims and a module printed before claims existed must not look
    // the same to the runner reading it.
    out.push_str(&format!(
        "\nexport const $tests = [{}];\n",
        if claim_entries.is_empty() {
            String::new()
        } else {
            format!("\n{},\n", claim_entries.join(",\n"))
        }
    ));

    // §17.4.8 runs this module in a sandbox with no `dom.js` in it, so
    // every name it uses is a name it declares. Without this a `static`
    // holding a variant printed `variant('Busy')` against nothing and the
    // build stopped with E10, and so did any `static` reaching a prelude
    // primitive with a helper form — `length of` among them.
    let mut source = format!(
        "// zdc {} · {source_path} · the build root, generated, do not edit\n",
        env!("CARGO_PKG_VERSION")
    );
    let preamble = crate::intrinsics::preamble(&emitter.used);
    if !preamble.is_empty() {
        source.push('\n');
        source.push_str(&preamble);
    }
    source.push_str(&out);

    Some(BuildModule {
        source,
        statics: exported,
        emits: files,
        tests: claimed,
    })
}

/// The two operands of a top-level `is` or `is not`, if that is what the
/// expectation's outermost operator is — issue #169.
///
/// Deliberately shallow. `(a is b) and (c is d)` has an `and` at the top,
/// so it has no two sides to show, and digging for a comparison inside it
/// would report one pair as though it were the claim. A report that shows
/// the wrong two values is worse than one that shows none: the reader
/// trusts it and then looks in the wrong place.
///
/// `is not` is included because a broken `is not` is exactly the case
/// where the two values are *equal*, and the one thing the reader wants is
/// to see which value turned up on both sides.
fn comparison_sides(hir: &zdc_hir::Hir, expr: ExprId) -> Option<(ExprId, ExprId)> {
    match &hir.exprs[expr].kind {
        HirExprKind::Binary {
            op: zdc_ast::BinOp::Is | zdc_ast::BinOp::IsNot,
            lhs,
            rhs,
        } => Some((*lhs, *rhs)),
        // A conditional is not a comparison, so a `test` written over one
        // has no two sides to show — the report falls back to the claim
        // as written, which is what every non-comparison does.
        HirExprKind::Conditional { .. } => None,
        // Spelled out rather than wildcarded: a new expression form that
        // has two comparable halves should be ruled on here rather than
        // silently reported as having none.
        HirExprKind::Binary { .. }
        | HirExprKind::Number(_)
        | HirExprKind::Text(_)
        | HirExprKind::Truth(_)
        | HirExprKind::Empty
        | HirExprKind::List(_)
        | HirExprKind::Map(_)
        | HirExprKind::Ref(_)
        | HirExprKind::Call { .. }
        | HirExprKind::OfCall { .. }
        | HirExprKind::Operator { .. }
        | HirExprKind::Environment(_)
        | HirExprKind::Address
        | HirExprKind::Media(_)
        | HirExprKind::Scroll
        | HirExprKind::Outbound { .. }
        | HirExprKind::MapInside { .. }
        | HirExprKind::Build { .. }
        | HirExprKind::Unary { .. }
        | HirExprKind::Field { .. }
        | HirExprKind::Index { .. }
        | HirExprKind::Append { .. }
        | HirExprKind::Insert { .. } => None,
    }
}
