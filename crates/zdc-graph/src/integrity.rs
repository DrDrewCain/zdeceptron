//! The integrity direction — spec §18.1, §19, and §21.7 as narrowed by §21.8.
//!
//! `secret` answers *who may learn this value*. `trusted` answers *who
//! chose it*. This module is the second question, and it is **default
//! closed**: a value is Untrusted unless it derives, by join alone, from
//! the grant set in [`Grant`]. There is no other way to become Trusted.
//!
//! # Why closed, and what that overturns
//!
//! §21.7 inverted the lattice, and the inversion is the one part of that
//! decision §21.8 leaves standing. A default-open lattice is sound only
//! if the enumeration of untrusted sources is complete, and three
//! consecutive designs shipped an enumeration that was not: §19.9 found a
//! channel through a selection predicate, §19.11 found one through a
//! `foreign`, and §21.8.4 found one through a two-way `Input` binding.
//! Under a closed set the question is not "did we list every way in?" but
//! "which of these eight grants applies?", which is answerable by a total
//! function — and [`Grant`] is written as one, deliberately, because
//! §21.8.4's own diagnosis is that *grant sets stated as prose tables need
//! to be stated as a total function before anything is built on them*.
//!
//! # What is **not** claimed
//!
//! **No robustness property.** Three independent adversarial passes broke
//! the soundness argument, the third (§21.8) after §21.7 had repaired the
//! second. These rules are built on §21.8.8 option 2's terms: keep the
//! declaration shape, the report, `limit`, REL-PLACE′, REL-CLOSED,
//! REL-PURE and REL-ARG **as review aids**, and withdraw the claim.
//!
//! Two breaks are live in the rules below and are marked at the point of
//! use rather than hidden here:
//!
//! * **R1 — [`Grant::ForeignAnywhere`] and [`rel_pure`] are unsound.**
//!   Both are stated over `is anywhere`, which §14E.2's own heading makes
//!   a claim about *which bundles a library may be linked into* — a
//!   linkability classification, not a purity one. The prelude's own
//!   `clock` is `is anywhere`, takes no arguments, and reads the wall
//!   clock: under G-FGN-A its result is `⨆ ∅ = Trusted` forever. No
//!   repair is proposed here; §21.8.8's option 1 would need a `pure`
//!   modifier that does not exist.
//! * **R2 — G-SIG once granted Trusted to any signal the browser writes
//!   without a `set`.** That one *is* repaired here, because the HIR
//!   already carries the mechanism: [`Site::Bind`] records a two-way
//!   binding as a write site, so `examples/blog.zd`'s `query` has a
//!   writer and is Untrusted. See [`Writers`].
//!
//! Callers must not turn any of this into a promise. `limit` is not a
//! cumulative disclosure bound (§21.8.7), and nothing here establishes
//! that a program is free of laundering.

use std::collections::{BTreeMap, BTreeSet};

use zdc_hir::{Builtin, DefId, DefKind, ExprId, Hir, HirArg, HirExprKind, LocalId, Res};
use zdc_lexer::Span;

use crate::authority::{match_args, Flow, Solution};
use crate::diag::GraphError;
use crate::sites::{sites_of, Site};

/// The two points of the integrity lattice, ordered `Trusted ⊑ Untrusted`.
///
/// `Default` is [`Authority::Untrusted`], and that is the whole design:
/// under §21.7.0 a value is Untrusted unless a grant says otherwise, so
/// the type's own default is the safe one and a missing case in a walk
/// fails closed rather than open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Authority {
    Trusted,
    #[default]
    Untrusted,
}

impl Authority {
    /// `⊔`. The only operation the pass performs, which is what makes it
    /// polymorphic in its lattice (§18.1).
    pub fn join(self, other: Authority) -> Authority {
        match (self, other) {
            (Authority::Trusted, Authority::Trusted) => Authority::Trusted,
            (Authority::Untrusted, _) | (_, Authority::Untrusted) => Authority::Untrusted,
        }
    }

    /// The join of a sequence. `⨆ ∅ = Trusted`, the lattice's bottom.
    ///
    /// This identity is exactly the hazard R1 names: a no-argument
    /// `is anywhere` foreign joins the empty set and comes out Trusted.
    /// It is correct as lattice algebra and wrong as a security claim, and
    /// the defect is in the premise that `is anywhere` means pure, not in
    /// this fold.
    pub fn join_all(labels: impl IntoIterator<Item = Authority>) -> Authority {
        labels.into_iter().fold(Authority::Trusted, Authority::join)
    }

    pub fn is_trusted(self) -> bool {
        self == Authority::Trusted
    }
}

/// The closed set of ways to become Trusted — §21.7.3, as narrowed by §21.8.
///
/// Deliberately **not** `#[non_exhaustive]`, for the same reason
/// [`crate::ifc::Sink`] is not: adding a grant must break every downstream
/// `match`, so a ninth way into the Trusted half cannot be added without
/// every consumer being made to rule on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Grant {
    /// **G-LIT** — a literal. Checked by the grammar.
    Literal,
    /// **G-ENV** — `environment "K"`. The operator set it (§17.3.4).
    Environment,
    /// **G-VIS** — `visitor`, in any `(Server, View)` expression (§20.2 rule 6).
    ///
    /// **In the set, but unawardable today.** §20's visitor principal is
    /// not built, so no expression form resolves to it and
    /// [`Integrity::of`] can never return this. It is listed rather than
    /// omitted because the closed set is the whole argument: a grant that
    /// exists in the design and not in this enum would be a grant nobody
    /// had to rule on when §20 lands.
    Visitor,
    /// **G-FGN-T** — a `foreign` declaring `gives trusted T`.
    ///
    /// Unconditionally Trusted whatever its arguments were. Asserted by a
    /// human and checked by nobody, at build or ever (§21.7.5 assumption
    /// 2, residual risk R5).
    ForeignTrusted,
    /// **G-FGN-A** — a `foreign` declared `is anywhere`; result is the
    /// join of its arguments.
    ///
    /// **Unsound as stated (R1, §21.8.0–3).** `is anywhere` answers
    /// "which bundles may this be linked into?", not "is this pure". Kept
    /// because it is load-bearing for the review aid and its removal
    /// needs the `pure` modifier §21.8.8 option 1 would have to add.
    ForeignAnywhere,
    /// **G-SIG** — a read of a signal declared `trusted`, or of one with
    /// no write site anywhere whose initialiser is Trusted.
    ///
    /// The second clause is where §21.8.4's R2 bit. See [`Writers`].
    Signal,
    /// **G-BLD** — a build-time read at a literal path inside the project
    /// tree; the file is in the operator's version control.
    ///
    /// **In the set, but unawardable today**, for the same reason as
    /// [`Grant::Visitor`]: the `static` placement's build-time read is not
    /// built. Note what §21.7.3 settled here — `static` gets **no blanket
    /// grant**; §18.1 semantics 9's `static` half is overturned, so a
    /// build that fetches a feed through an ungranted `foreign` yields
    /// Untrusted state, and only the literal-path case is a grant.
    Build,
    /// **G-REL** — a `release` parameter named by a `trusted` clause.
    ///
    /// Site-local and result-transparent: it discharges REL-ARG at that
    /// release's call sites and does nothing anywhere else (§19.10.3(a)).
    Release,
}

impl Grant {
    /// Every grant there is. §19.5's audit trail is complete only if this
    /// list is, so it is a constant a test can count rather than a set a
    /// reader must assemble from prose.
    pub const CLOSED_LIST: [Grant; 8] = [
        Grant::Literal,
        Grant::Environment,
        Grant::Visitor,
        Grant::ForeignTrusted,
        Grant::ForeignAnywhere,
        Grant::Signal,
        Grant::Build,
        Grant::Release,
    ];

    /// The spec's own name for this grant, for the report and for tests
    /// that should not assert on prose.
    pub fn code(self) -> &'static str {
        match self {
            Grant::Literal => "G-LIT",
            Grant::Environment => "G-ENV",
            Grant::Visitor => "G-VIS",
            Grant::ForeignTrusted => "G-FGN-T",
            Grant::ForeignAnywhere => "G-FGN-A",
            Grant::Signal => "G-SIG",
            Grant::Build => "G-BLD",
            Grant::Release => "G-REL",
        }
    }

    /// Whether the grant is asserted by a human rather than checked by the
    /// compiler.
    ///
    /// §19.5 needs this to mark the entries a reviewer must actually read.
    /// It is not `attacker_reachable` — that field is **not** emitted, and
    /// §21.8.3 shows why it could not be trusted if it were: a purity
    /// grant has no argument to trace, so the walk that would set the flag
    /// has nothing to walk.
    pub fn is_asserted(self) -> bool {
        match self {
            Grant::ForeignTrusted | Grant::ForeignAnywhere => true,
            Grant::Literal
            | Grant::Environment
            | Grant::Visitor
            | Grant::Signal
            | Grant::Build
            | Grant::Release => false,
        }
    }
}

/// Which signals are written, and therefore which fail G-SIG's second clause.
///
/// **This is the repair for §21.8.4 (residual risk R2).** G-SIG as written
/// asks whether a signal "has no write site anywhere in the program", and
/// §21.7.5 item 6 decides that by "a whole-program reachability query over
/// **statement forms**". A two-way `Input` binding is not a statement
/// form: the browser writes the signal on every keystroke and there is no
/// `set` for the query to find, so `examples/blog.zd`'s `query` — a text
/// box — came out Trusted.
///
/// The decision §21.8.4 left open is taken here: **a two-way binding is a
/// write site.** [`Site::Bind`] already records one, so the repair is to
/// ask the site walk rather than the statement forms.
pub struct Writers {
    written: BTreeSet<DefId>,
    /// Where each signal is first written, for the diagnostic.
    at: BTreeMap<DefId, Span>,
}

impl Writers {
    pub fn of(hir: &Hir) -> Writers {
        let mut written = BTreeSet::new();
        let mut at = BTreeMap::new();
        for (id, _) in hir.defs.iter() {
            for site in sites_of(hir, id) {
                let (signal, span) = match site {
                    Site::Write { signal, span, .. } => (signal, span),
                    // The keystroke write. This arm is the R2 repair.
                    Site::Bind { signal, span, .. } => (signal, span),
                    Site::Call { .. }
                    | Site::ForeignCall { .. }
                    | Site::Read { .. }
                    | Site::NotAPlace { .. }
                    | Site::Environment { .. } => continue,
                };
                written.insert(signal);
                at.entry(signal).or_insert(span);
            }
        }
        Writers { written, at }
    }

    pub fn is_written(&self, signal: DefId) -> bool {
        self.written.contains(&signal)
    }

    pub fn written_at(&self, signal: DefId) -> Option<Span> {
        self.at.get(&signal).copied()
    }
}

/// The integrity of an expression, as a total function over `HirExprKind`.
///
/// Every arm is written out. There is no wildcard anywhere in this
/// function and there must never be one: a new expression form has to be
/// ruled on here, and the compiler is what makes that happen. That
/// property is the entire argument that the grant set is closed — §19.5's
/// completeness claim is a claim about the grammar, and it survives only
/// while this match is exhaustive by construction.
pub struct Integrity<'a> {
    hir: &'a Hir,
    /// Fixpoint 1's answers, so that a signal read and a call are table
    /// lookups rather than a walk into another definition's body.
    ///
    /// Before the table existed, a read of a signal re-derived that
    /// signal's initialiser at every reference and descended a chain of
    /// derived signals once per read — the same answer computed `n` times
    /// down a chain of length `n`, with nothing stopping a cycle. That is
    /// the interprocedural half of this analysis, and it belongs in a
    /// fixpoint rather than in an expression walk.
    solution: &'a Solution,
    /// What each binding in the body being walked is worth, as a [`Flow`]
    /// over the enclosing definition's parameters. Absent means Untrusted,
    /// because absent means no grant applies.
    locals: BTreeMap<LocalId, Flow>,
}

impl<'a> Integrity<'a> {
    pub fn new(hir: &'a Hir, solution: &'a Solution) -> Integrity<'a> {
        Integrity {
            hir,
            solution,
            locals: BTreeMap::new(),
        }
    }

    /// Bind `local` for the duration of one body's walk.
    ///
    /// A parameter is bound to [`Flow::param`] while its own definition is
    /// being summarised, and to [`Flow::exact`] once every call site's
    /// argument has been merged onto it. Those are the two modes, and they
    /// are the same walk.
    pub fn bind(&mut self, local: LocalId, flow: Flow) {
        self.locals.insert(local, flow);
    }

    pub fn clear_bindings(&mut self) {
        self.locals.clear();
    }

    /// What a **read** of a signal is worth — G-SIG, both clauses.
    ///
    /// Exposed separately from [`Integrity::of`] because G-SIG is a
    /// question about a declaration rather than about an expression, and
    /// the obligation sites (A1, A3) ask it directly.
    pub fn of_signal_read(&self, signal: DefId) -> Authority {
        self.solution.signal(signal).0
    }

    /// The authority of an expression, and the grant that awarded it.
    ///
    /// `None` for the grant means no grant applied, which under a closed
    /// lattice is the ordinary case and yields [`Authority::Untrusted`].
    pub fn of(&self, expr: ExprId) -> (Authority, Option<Grant>) {
        let (flow, grant) = self.flow(expr);
        (flow.authority(), grant)
    }

    /// The authority of an expression as a function of the enclosing
    /// definition's parameters, and the grant that awarded it.
    ///
    /// This is the total function. Every arm is written out. There is no
    /// wildcard anywhere in it and there must never be one: a new
    /// expression form has to be ruled on here, and the compiler is what
    /// makes that happen.
    pub fn flow(&self, expr: ExprId) -> (Flow, Option<Grant>) {
        match &self.hir.exprs[expr].kind {
            // G-LIT. The grammar is the check.
            HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty => (Flow::trusted(), Some(Grant::Literal)),

            // G-ENV. The operator set the variable, not a visitor.
            HirExprKind::Environment(_) => (Flow::trusted(), Some(Grant::Environment)),

            // A composite is the join of its parts, and carries no grant of
            // its own: joining is the only way authority moves.
            HirExprKind::List(items) => (
                items
                    .iter()
                    .fold(Flow::trusted(), |acc, item| acc.join(&self.flow(*item).0)),
                None,
            ),
            HirExprKind::Map(pairs) => (
                pairs.iter().fold(Flow::trusted(), |acc, (key, value)| {
                    acc.join(&self.flow(*key).0).join(&self.flow(*value).0)
                }),
                None,
            ),
            HirExprKind::Unary { operand, .. } => (self.flow(*operand).0, None),
            HirExprKind::Operator { operand, .. } => (self.flow(*operand).0, None),
            HirExprKind::Binary { lhs, rhs, .. } => {
                (self.flow(*lhs).0.join(&self.flow(*rhs).0), None)
            }
            HirExprKind::Field { base, .. } => (self.flow(*base).0, None),
            // An index joins the indexed value with the index itself: a
            // browser that chooses `i` chooses which element comes out,
            // which is the whole of obligation site A1.
            HirExprKind::Index { base, index } => {
                (self.flow(*base).0.join(&self.flow(*index).0), None)
            }

            HirExprKind::Ref(res) => self.of_res(*res),

            HirExprKind::Call { callee, args } => self.of_call(*callee, args),
            HirExprKind::OfCall { callee, operand } => {
                self.of_call(*callee, &[HirArg::Positional(*operand)])
            }
        }
    }

    /// A resolved name.
    fn of_res(&self, res: Res) -> (Flow, Option<Grant>) {
        match res {
            // A binder carries whatever was bound to it and no grant of its
            // own. In particular an endorsed release parameter is **not**
            // Trusted inside the body: §19.10.3(a) makes the endorsement
            // site-local and result-transparent, and raising the label
            // inside would turn four lines into a universal integrity
            // launderer. G-REL is awarded at the call site, by A5.
            Res::Local(local) => (self.locals.get(&local).cloned().unwrap_or_default(), None),
            Res::Def(def) => self.of_def(def),
            // A variant name is a constructor written in the source, so it
            // is as trusted as a literal is. Its *arguments* are joined by
            // the `Call` arm above; this is the bare name.
            Res::Variant { .. } | Res::BuiltinVariant(_) => (Flow::trusted(), Some(Grant::Literal)),
            // An element or type name is not a value a browser can choose.
            Res::Builtin(Builtin::Element(_)) | Res::Builtin(Builtin::Type) => {
                (Flow::trusted(), Some(Grant::Literal))
            }
        }
    }

    /// A reference to a top-level definition.
    fn of_def(&self, def: DefId) -> (Flow, Option<Grant>) {
        match &self.hir.defs[def].kind {
            // G-SIG, both clauses — solved once, in fixpoint 1, because
            // clause 2 reads the initialiser and an initialiser may call a
            // function whose body reads another signal.
            DefKind::Signal(_) => {
                let (authority, grant) = self.solution.signal(def);
                (Flow::exact(authority), grant)
            }
            // A bare reference to a callable is not a value in this
            // language; its result is settled at the call site.
            DefKind::Function(_)
            | DefKind::Foreign(_)
            | DefKind::Release(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => (Flow::untrusted(), None),
        }
    }

    /// The result of calling `callee`.
    fn of_call(&self, callee: Res, args: &[HirArg]) -> (Flow, Option<Grant>) {
        let joined = args.iter().fold(Flow::trusted(), |acc, arg| {
            acc.join(&self.flow(arg_of(arg)).0)
        });
        let Res::Def(def) = callee else {
            // A builtin constructor such as `Some with value is v` is as
            // trusted as what it wraps.
            return (joined, None);
        };
        match &self.hir.defs[def].kind {
            DefKind::Foreign(foreign) => {
                if foreign.gives_trusted {
                    // G-FGN-T. Unconditional, and unconditionally a
                    // human's word (R5).
                    return (Flow::trusted(), Some(Grant::ForeignTrusted));
                }
                if foreign.site == zdc_ast::ForeignSite::Anywhere {
                    // G-FGN-A. **Unsound (R1).** `is anywhere` is a
                    // linkability answer, and `clock` is the standing
                    // counterexample: no arguments, so this returns
                    // Trusted forever.
                    return (joined, Some(Grant::ForeignAnywhere));
                }
                // An `is server` / `is client` foreign with no grant reads
                // the environment for all the compiler knows.
                (Flow::untrusted(), None)
            }
            // Interprocedural, and this is the whole point of the summary:
            // the result is whatever the body computes, instantiated at
            // *this* call site's arguments. A function whose body reads an
            // Untrusted signal is Untrusted however Trusted its arguments
            // were, which the old "a function is transparent" rule got
            // backwards.
            //
            // A release's summary additionally carries every parameter,
            // because §19.2 rule 3 makes its result the join of its
            // arguments as written at the call site, before endorsement.
            DefKind::Function(function) => {
                let params = function.params.clone();
                (self.instantiate(def, &params, args), None)
            }
            DefKind::Release(release) => {
                let params = release.params.clone();
                (self.instantiate(def, &params, args), None)
            }
            DefKind::Signal(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => (joined, None),
        }
    }

    /// A callable's summary, instantiated at one call site's arguments.
    fn instantiate(&self, callee: DefId, params: &[LocalId], args: &[HirArg]) -> Flow {
        let matched = match_args(self.hir, params, args);
        let flows: Vec<Flow> = matched
            .iter()
            .map(|arg| match arg {
                Some(expr) => self.flow(*expr).0,
                None => Flow::untrusted(),
            })
            .collect();
        self.solution.result(callee).substitute(&flows)
    }
}

/// The expression an argument carries, named or positional.
fn arg_of(arg: &HirArg) -> ExprId {
    crate::sites::arg_expr(arg)
}

/// **REL-PURE** — §21.7.3, error **E-REL-10**.
///
/// A release body may reach only a `foreign` that is `is anywhere` or that
/// declares `gives trusted T`. Checked at the declaration and transitive
/// over the call graph, exactly as REL-CLOSED is.
///
/// **This rule does not do what its name says (R1).** It is stated over
/// `is anywhere`, which classifies linkability rather than purity, and the
/// prelude's own `clock` passes it while reading the wall clock. It is
/// built because §21.8.8 option 2 keeps it as a review aid: it still
/// rejects §19.11.1's `queryParam`, which is a real attack, and a reviewer
/// reading its output learns something true. It must not be described to a
/// user as establishing that a release body is pure.
pub fn rel_pure(hir: &Hir, release: DefId) -> Vec<GraphError> {
    let mut out = Vec::new();
    for (foreign, span) in reachable_foreigns(hir, release) {
        let DefKind::Foreign(decl) = &hir.defs[foreign].kind else {
            continue;
        };
        if decl.gives_trusted || decl.site == zdc_ast::ForeignSite::Anywhere {
            continue;
        }
        let name = hir.defs[foreign].name.clone();
        let where_it_runs = match decl.site {
            zdc_ast::ForeignSite::Client => "is client",
            zdc_ast::ForeignSite::Server => "is server",
            zdc_ast::ForeignSite::Anywhere => "is anywhere",
        };
        out.push(
            GraphError::new(
                "E-REL-10",
                format!(
                    "`{}` reaches the foreign `{name}` (`{where_it_runs}`), which is neither \
                     `is anywhere` nor declares `gives trusted T`. A release body may observe \
                     nothing but its parameters; this call reads the environment.",
                    hir.defs[release].name
                ),
                hir.defs[release].span,
            )
            .with_notes(vec![(span, format!("`{name}` is reached here"))])
            .with_help(
                "Either declare the foreign `gives trusted T`, signing that its result is not \
                 attacker-chosen, or lift the value into the release's parameter list where an \
                 endorsement has to name it."
                    .to_string(),
            ),
        );
    }
    out
}

/// **REL-CLOSED** — §19.2 rule 8, error **E-REL-04**.
///
/// A release body, and everything it reaches, may read no signal. This is
/// what makes the parameter list the release's entire input, and it is the
/// premise REL-ARG never had in §19.10.
pub fn rel_closed(hir: &Hir, release: DefId) -> Vec<GraphError> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = vec![release];
    while let Some(def) = queue.pop() {
        if !seen.insert(def) {
            continue;
        }
        for site in sites_of(hir, def) {
            match site {
                Site::Read { signal, span, .. } => {
                    let name = hir.defs[signal].name.clone();
                    out.push(
                        GraphError::new(
                            "E-REL-04",
                            format!(
                                "`{}` reads the signal `{name}`. A release's inputs are its \
                                 parameters and nothing else.",
                                hir.defs[release].name
                            ),
                            span,
                        )
                        .with_help(
                            "Pass the signal's value as an argument, where the call site has to \
                             account for where it came from."
                                .to_string(),
                        ),
                    );
                }
                Site::Call { callee, .. } => queue.push(callee),
                // A foreign call is REL-PURE's business, not REL-CLOSED's.
                Site::ForeignCall { .. }
                | Site::Write { .. }
                | Site::Bind { .. }
                | Site::NotAPlace { .. }
                | Site::Environment { .. } => {}
            }
        }
    }
    out
}

/// Every `foreign` reachable from a definition's body, with the span that
/// reaches it. Transitive over calls, as REL-PURE requires.
fn reachable_foreigns(hir: &Hir, from: DefId) -> Vec<(DefId, Span)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = vec![from];
    while let Some(def) = queue.pop() {
        if !seen.insert(def) {
            continue;
        }
        for site in sites_of(hir, def) {
            match site {
                Site::ForeignCall { callee, span } => out.push((callee, span)),
                // Transitive: REL-PURE is checked over the call graph,
                // exactly as REL-CLOSED is.
                Site::Call { callee, .. } => queue.push(callee),
                Site::Read { .. }
                | Site::Write { .. }
                | Site::Bind { .. }
                | Site::NotAPlace { .. }
                | Site::Environment { .. } => {}
            }
        }
    }
    out
}

/// **W-REL-01** — §19.4. An unbounded release.
///
/// The `gives` type is the per-evaluation bandwidth, and with no `limit`
/// there is no per-session ceiling on evaluations either.
///
/// The warning deliberately does **not** say that adding a `limit` bounds
/// disclosure, because it does not: `limit` is per declaration and per
/// anonymous session, k declarations give kN, clearing a cookie mints a
/// fresh budget, and nothing enforces it until `DurableStore` exists
/// (§21.8.7, residual risk R3).
pub fn w_rel_01(hir: &Hir, release: DefId) -> Option<GraphError> {
    let DefKind::Release(decl) = &hir.defs[release].kind else {
        return None;
    };
    if decl.limit.is_some() {
        return None;
    }
    Some(
        GraphError::warning(
            "W-REL-01",
            format!(
                "`{}` has no `limit`, so one session may evaluate it any number of times.",
                hir.defs[release].name
            ),
            hir.defs[release].span,
        )
        .with_help(
            "Writing `limit N per visitor` caps evaluations of this one declaration against one \
             anonymous session. It is not a cumulative disclosure bound: a second declaration \
             carries its own budget, clearing a cookie mints a fresh one, and budgets are not \
             enforced until durable storage exists."
                .to_string(),
        ),
    )
}
