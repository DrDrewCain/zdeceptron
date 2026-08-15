//! §19.5's audit trail, as data — and residual risk **R6**, narrowed.
//!
//! [`crate::integrity`]'s grant set is closed by grammar, which is the
//! whole of §19.5's completeness argument. Two of the eight grants are
//! **asserted**: [`Grant::ForeignPure`] and [`Grant::ForeignTrusted`] are a
//! human's word about JavaScript nobody checks (R5). §21.7's soundness
//! argument leans on them, so a reviewer has to read them — and until this
//! module existed there was no artifact that listed them. `zdc build
//! --report` writes one.
//!
//! # What R6 said, and which half of it this closes
//!
//! §21.8.3's R6 is *"a purity grant has no argument chain for an
//! attacker-reachability walk to follow"*. It is two claims wearing one
//! sentence:
//!
//! 1. **The grants soundness rests on are invisible to review.** True, and
//!    closed here. Every `foreign` whose `gives` line carries `pure` or
//!    `trusted` is enumerated, with where it is declared, what it imports,
//!    every call site that reaches it, and every `release` whose body
//!    reaches it transitively. That last list is the one that answers the
//!    question R6 says review is now responsible for — *is this unchecked
//!    assertion load-bearing for a declassification in this program?* — and
//!    it is computed by REL-PURE's own walk, which already existed.
//!
//! 2. **`attacker_reachable` cannot be computed for them.** Also true, and
//!    **not** closed, because it cannot be: see [`NOT_CLAIMED`]. Giving the
//!    grant an argument chain would not help. A `gives pure` foreign's
//!    channel is *inside the JavaScript*, not in its argument list —
//!    §21.8.1's `queryParam` takes a string literal and reads
//!    `location.search` — so an argument walk over it terminates at a
//!    literal and reports `false`. That is the false verdict §21.8.7
//!    withdrew the field for, and it stays withdrawn. The field is absent
//!    from the emitted report and [`NOT_CLAIMED`] says so in the report
//!    itself rather than only here.
//!
//! # What this is not
//!
//! Not a verdict. It reports what declarations **say**, exactly as
//! [`crate::integrity::rel_pure`] does, and nothing about what the
//! JavaScript does. An empty `asserted` list means the program declares no
//! foreign grant; it does not mean the program is free of laundering, and
//! §21.8.1's `launder3.zd` — which has one entry here and no diagnostic
//! anywhere — is the standing counterexample.

use zdc_hir::{DefId, DefKind, Hir};
use zdc_lexer::Span;

use crate::integrity::Grant;
use crate::sites::{sites_of, Site};

/// The sentences the emitted report carries about what it does not answer.
///
/// **This slice is the only copy of them.** A renderer indexes it rather
/// than restating it, so the scan in `zdc-diagnostics`'s
/// `no_robustness_claim.rs` — which §21.8.8 names as a surface the
/// withdrawn claim must not reach — sees exactly what a user sees.
///
/// Each is a negation, deliberately. §21.8.8 option 2 keeps the report as a
/// review aid and withdraws the claim laid over it, and a report that says
/// nothing about its own limits is a report a reader supplies limits for.
pub const NOT_CLAIMED: &[&str] = &[
    "This is an enumeration, not a verdict. It records what declarations claim, not what \
     the JavaScript behind them does.",
    "There is no `attackerReachable` field. It was specified (§19.5, §21.7.7) and withdrawn \
     (§21.8.3, §21.8.7): a `gives pure` foreign's channel is inside its JavaScript rather \
     than in its argument list, so a walk over the arguments of a foreign that reads the \
     request URL terminates at a string literal and answers `false`.",
    "An empty `asserted` list means the program declares no foreign grant. It does not mean \
     that no visitor steers a declassification, and it is not evidence that nothing was \
     laundered: §21.8.1's `launder3.zd`, with the purity marker written on its query-string \
     reader, has one entry here and raises no diagnostic anywhere.",
    "A grant marked `asserted` is checked by nobody, at build or ever (residual risk R5). \
     Reading the module it names is the check.",
    LIBRARY_NOTE,
];

/// Why the prelude's purity grants are named in `library` and not listed
/// beside the program's own, with spans.
///
/// **A prelude span cannot be located.** Each prelude file is parsed on its
/// own, so its spans index that file from zero and collide with the
/// program's; the linked program does not contain the library's text, and
/// `Linked::locate` would resolve one to whatever byte range sat at that
/// offset in a file the reader wrote. A wrong line number in an audit trail
/// is worse than none.
///
/// So they are named rather than located. The names are the whole of what
/// there is to review here anyway: the primitive layer is identical in
/// every program, this compiler emits its modules, and the split between
/// pure and impure is pinned by `zdc-graph`'s own
/// `all_but_one_of_the_primitives_are_pure_and_the_clock_is_not`.
pub const LIBRARY_NOTE: &str =
    "`library` names the prelude's own purity grants instead of locating them. Each prelude \
     file is parsed on its own, so its spans index that file rather than anything the linked \
     program contains, and a line number resolved against the wrong file is worse than none. \
     They are still assertions; they are the language's rather than this program's.";

/// One `foreign` whose `gives` line awards a grant the compiler does not
/// check — G-FGN-P or G-FGN-T.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertedGrant {
    pub grant: Grant,
    pub def: DefId,
    pub name: String,
    /// The modifier as written: `pure` or `trusted`.
    pub marker: &'static str,
    /// The `is` line's word. Reported because a reviewer reads it, and
    /// **not** consulted by anything here: R1 is the record of what
    /// happens when it is (§21.9).
    pub site: &'static str,
    /// The module specifier, or `None` for a method or a property — both
    /// come with the receiver and import nothing.
    pub module: Option<String>,
    /// The export, method or property name.
    pub export: String,
    /// Whether the module names the language's own primitive layer
    /// (`zd:`) rather than a package on the platform.
    ///
    /// A different trust anchor, so a reader can tell the two apart: a
    /// `zd:` module is emitted by this compiler and §17.4.10 lists why
    /// each one is a primitive, while `./gauge.js` is somebody's file.
    /// Both are still assertions, and both are still listed.
    pub primitive: bool,
    pub declared_at: Span,
    /// Every call to it, in source order.
    pub calls: Vec<Span>,
    /// Every `release` whose body reaches it, transitively over calls.
    ///
    /// This is REL-PURE's own walk, read for its other answer. A release
    /// is the program's declassification boundary, so an entry here is the
    /// report saying *this assertion is what lets that release compile*.
    pub releases: Vec<ReleaseReach>,
}

/// One `trusted p` clause on a `release` — obligation site **A5**.
///
/// The other human signature, and it is here because E-REL-08's help text
/// already promises it: *"it records that this makes the release a function
/// of a value the browser chose, and it will appear in
/// `zdc build --report`"*. That sentence shipped before the flag did.
///
/// [`crate::authority::ObligationSite::A5`] says why the site exists at
/// all — *"an endorsement is a human's signature and the point of the audit
/// trail is that every signature is enumerable"* — and an audit trail that
/// enumerated one kind of signature and not the other would be the same
/// gap R6 names, one grant over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endorsement {
    pub release: String,
    pub parameter: String,
    /// The parameter's own span, so a reader lands on the name rather than
    /// on the declaration that holds four of them.
    pub declared_at: Span,
}

/// A `release` that reaches an asserted grant, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseReach {
    pub name: String,
    pub declared_at: Span,
    /// The call site inside the release's reachable body.
    pub reached_at: Span,
}

/// The prelude's asserted grants, by name.
///
/// Names and not spans: see [`LIBRARY_NOTE`]. Sorted, because the order of
/// the prelude's own files is not a fact about the program being reported
/// on and a reviewer diffing two reports should see only what changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryGrants {
    pub pure: Vec<String>,
    pub trusted: Vec<String>,
}

/// Every asserted grant in a program, in declaration order, with the
/// library's held apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The program's own declarations — the ones that vary between
    /// programs and that nobody but this program's reviewer will read.
    pub asserted: Vec<AssertedGrant>,
    /// Every `trusted p` clause, in declaration order.
    pub endorsed: Vec<Endorsement>,
    /// The prelude's, which are the same in every program.
    pub library: LibraryGrants,
}

/// Collect the audit trail.
///
/// Reads declarations and call sites only, and no fixpoint: what a
/// `foreign` claims about its result is a constant of its declaration, so
/// the report says the same thing whether or not the program typechecks
/// past this point.
pub fn report(hir: &Hir) -> Report {
    let calls = foreign_calls(hir);
    let mut asserted = Vec::new();
    let mut library = LibraryGrants::default();
    for (id, def) in hir.defs.iter() {
        let DefKind::Foreign(foreign) = &def.kind else {
            continue;
        };
        if hir.is_prelude_def(id) {
            match foreign.result_grant {
                zdc_ast::ForeignGrant::Pure => library.pure.push(def.name.clone()),
                zdc_ast::ForeignGrant::Trusted => library.trusted.push(def.name.clone()),
                zdc_ast::ForeignGrant::Opaque => {}
            }
            continue;
        }
        // Exhaustive over `ForeignGrant`, with no wildcard, so a fourth
        // claim about a result has to be ruled on here as well as at the
        // two enforcement sites. A report that silently omitted one would
        // be worse than no report: §19.5's completeness is the only thing
        // it has that a configured taint tool does not.
        let grant = match foreign.result_grant {
            zdc_ast::ForeignGrant::Pure => Grant::ForeignPure,
            zdc_ast::ForeignGrant::Trusted => Grant::ForeignTrusted,
            zdc_ast::ForeignGrant::Opaque => continue,
        };
        let marker = foreign
            .result_grant
            .describe()
            .expect("`Opaque` is the only grant with no modifier and it is skipped above");
        asserted.push(AssertedGrant {
            grant,
            def: id,
            name: def.name.clone(),
            marker,
            site: foreign.site.describe(),
            module: foreign.module().map(str::to_string),
            export: foreign.export.as_str().to_string(),
            primitive: foreign.is_primitive(),
            declared_at: def.span,
            calls: calls
                .iter()
                .filter(|(callee, _)| *callee == id)
                .map(|(_, span)| *span)
                .collect(),
            releases: releases_reaching(hir, id),
        });
    }
    library.pure.sort();
    library.trusted.sort();
    Report {
        asserted,
        endorsed: endorsements(hir),
        library,
    }
}

/// Every `trusted p` clause in the program.
fn endorsements(hir: &Hir) -> Vec<Endorsement> {
    let mut out = Vec::new();
    for (id, def) in hir.defs.iter() {
        let DefKind::Release(release) = &def.kind else {
            continue;
        };
        if hir.is_prelude_def(id) {
            continue;
        }
        // `endorsed` matches `params` positionally, which is the invariant
        // `Release::endorsed` states; `zip` is what reads it rather than an
        // index that could outrun one of the two.
        for (param, _) in release
            .params
            .iter()
            .zip(release.endorsed.iter())
            .filter(|(_, endorsed)| **endorsed)
        {
            out.push(Endorsement {
                release: def.name.clone(),
                parameter: hir.locals[*param].name.clone(),
                declared_at: hir.locals[*param].span,
            });
        }
    }
    out
}

/// Every `(foreign, call site)` in the program, in source order.
///
/// One walk over every definition rather than one per foreign: a program
/// with `n` foreigns and `m` bodies would otherwise be `n × m` walks, and
/// the prelude alone declares two dozen.
fn foreign_calls(hir: &Hir) -> Vec<(DefId, Span)> {
    let mut out = Vec::new();
    for (id, _) in hir.defs.iter() {
        for site in sites_of(hir, id) {
            match site {
                Site::ForeignCall { callee, span } => out.push((callee, span)),
                // None of these calls a `foreign`. Written out rather than
                // matched with a wildcard for the reason every other walk
                // in this crate is: a new site kind has to be ruled on
                // here, and an audit trail that quietly skipped one would
                // still print as complete.
                Site::Call { .. }
                | Site::Read { .. }
                | Site::Write { .. }
                | Site::Bind { .. }
                | Site::NotAPlace { .. }
                | Site::Environment { .. }
                | Site::Media { .. }
                | Site::Build { .. }
                | Site::Outbound { .. }
                | Site::DocumentKey { .. } => {}
            }
        }
    }
    out
}

/// Every `release` whose body reaches `foreign`, transitively over calls.
fn releases_reaching(hir: &Hir, foreign: DefId) -> Vec<ReleaseReach> {
    let mut out = Vec::new();
    for (id, def) in hir.defs.iter() {
        if !matches!(def.kind, DefKind::Release(_)) {
            continue;
        }
        // The first reach, and only the first. One release body may call
        // the same foreign several times; the answer a reader needs is
        // *whether* this release depends on the assertion and one place to
        // look, and every call site is already in `calls` above.
        let reached = crate::integrity::reachable_foreigns(hir, id)
            .into_iter()
            .find(|(callee, _)| *callee == foreign);
        if let Some((_, span)) = reached {
            out.push(ReleaseReach {
                name: def.name.clone(),
                declared_at: def.span,
                reached_at: span,
            });
        }
    }
    out
}
