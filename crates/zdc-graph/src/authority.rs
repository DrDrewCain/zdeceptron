//! The interprocedural argument-authority fixpoint — §18.1, §19.10, §21.7.3.
//!
//! [`crate::integrity`] answers *what is this expression worth* for one
//! expression at a time. That question is not answerable one expression at
//! a time, because two of its cases cross a definition boundary:
//!
//! * a **signal**'s read label is G-SIG, whose second clause reads the
//!   *initialiser*, and an initialiser may call a function;
//! * a **function or release** result is whatever its body computes, and a
//!   body may read a signal, call another function, or call itself.
//!
//! So signal labels and result labels are mutually recursive, and the
//! answer is a fixpoint over the call graph rather than a walk down one
//! expression tree. This module computes it, and then answers the question
//! §19.10.1 asks at every release call site: *is this argument Trusted, and
//! if not, did the declaration endorse the parameter it lands on?*
//!
//! # Two fixpoints, and why they do not interleave
//!
//! **Fixpoint 1** ([`Solution`]) solves signal read-labels and result
//! [`Flow`]s **together**, in one worklist, because each needs the other.
//!
//! **Fixpoint 2** ([`Analysis::params`]) merges, for every parameter, the
//! authority of every argument any call site passes to it. It is what the
//! obligation sites inside a body need: `orders at k` can only be ruled on
//! once `k`'s authority is known, and `k` is a parameter.
//!
//! The second is **stratified above** the first rather than interleaved
//! with it, and the reason is that [`Flow`] is *relational*: a result is
//! recorded as a join over the **parameter positions** it depends on, not
//! over the values any caller supplied. Nothing in fixpoint 1 needs to know
//! what a call site passed, so fixpoint 1 never has to be re-run when
//! fixpoint 2 learns something. Had summaries instead been merged —
//! one authority per parameter, joined over all callers, fed back into the
//! result — the two would have been one interleaved fixpoint *and* less
//! precise: a single Untrusted call site would poison every other call site
//! of the same function, and E-REL-08 would then fire at sites where no
//! attacker-chosen value flows.
//!
//! Fixpoint 2 is still a fixpoint, and not a single pass, because a
//! recursive function passes its own parameters back to itself.
//!
//! # Termination
//!
//! Both fixpoints terminate, and the argument is the same one twice.
//!
//! [`Authority`] is a two-point lattice. A [`Flow`] over a definition of
//! `n` parameters is a pair `(base, params ⊆ 0..n)`, normalised so that
//! `base = Untrusted` forces `params = ∅` — a finite lattice of height
//! `n + 1`. Every transfer function here is built from `⊔` and
//! [`Flow::substitute`] alone, and both are monotone, so each value moves
//! only upward and can move at most `n + 1` times. The worklist re-enqueues
//! a definition only when a value it reads has *changed*, so the total
//! number of recomputations is bounded by the sum of those heights times
//! the in-degree of the call graph. A recursive or mutually recursive group
//! is not a special case: its members simply stop changing, which is what
//! the tests `a_recursive_function_terminates` and
//! `mutually_recursive_functions_terminate` pin.
//!
//! # What is **not** claimed
//!
//! No robustness property, for the reasons [`crate::integrity`] states at
//! length. E-REL-08 says a release argument is a value the compiler cannot
//! trace to a grant. It does not say the program leaks, and a program with
//! no E-REL-08 anywhere is not thereby free of laundering — §21.8.1's
//! `launder3.zd` has none, and the test `launder3_compiles_clean_and_that_is_r1`
//! holds that behaviour in place deliberately.

use std::collections::{BTreeMap, BTreeSet};

use zdc_hir::{
    BlockId, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirExprKind, HirMutation, HirNode,
    HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, LocalId, Res,
};
use zdc_lexer::Span;

use crate::diag::GraphError;
use crate::integrity::{Authority, Grant, Integrity, Writers};
use crate::sites::{arg_expr, sites_of, Site};
use crate::split::TierSplit;

// ---------------------------------------------------------------------
// The relational domain.
// ---------------------------------------------------------------------

/// What a value is worth, as a function of the enclosing definition's
/// parameters.
///
/// `base` is everything the parameters did not contribute — literals,
/// signal reads, foreign results. `params` is the set of parameter
/// positions whose authority reaches this value. Concretely,
/// `apply(args) = base ⊔ ⨆ { args[i] | i ∈ params }`.
///
/// **Normalised**: `Untrusted` absorbs, so a `Flow` with
/// `base = Untrusted` always has an empty `params` set. That makes
/// `(Untrusted, ∅)` the unique top and keeps the lattice's height at
/// `params + 1`, which is the termination argument.
///
/// [`Flow::default`] is that top, deliberately: an absent entry — a
/// definition the solver never reached, a parameter index no argument
/// filled — reads Untrusted, so a missed case fails closed exactly as
/// [`Authority::default`] does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Flow {
    base: Authority,
    params: BTreeSet<u32>,
}

impl Flow {
    /// The lattice's bottom: Trusted, and independent of every parameter.
    pub fn trusted() -> Flow {
        Flow {
            base: Authority::Trusted,
            params: BTreeSet::new(),
        }
    }

    /// The lattice's top: Untrusted whatever the caller passed.
    pub fn untrusted() -> Flow {
        Flow::default()
    }

    /// Exactly parameter `index`, and nothing else.
    pub fn param(index: u32) -> Flow {
        Flow {
            base: Authority::Trusted,
            params: BTreeSet::from([index]),
        }
    }

    /// A value already known concretely.
    pub fn exact(authority: Authority) -> Flow {
        match authority {
            Authority::Trusted => Flow::trusted(),
            Authority::Untrusted => Flow::untrusted(),
        }
    }

    fn normalise(mut self) -> Flow {
        if self.base == Authority::Untrusted {
            self.params.clear();
        }
        self
    }

    /// `⊔`, the only way authority ever moves.
    pub fn join(mut self, other: &Flow) -> Flow {
        self.base = self.base.join(other.base);
        self.params.extend(other.params.iter().copied());
        self.normalise()
    }

    /// `⊔=`, reporting whether anything moved. The worklists use the
    /// answer, and it is also what bounds the number of iterations.
    pub fn join_assign(&mut self, other: &Flow) -> bool {
        let before = self.clone();
        *self = before.clone().join(other);
        *self != before
    }

    /// Instantiate this summary at a call site whose arguments have the
    /// given flows — function composition in the relational domain.
    ///
    /// An argument position with no argument is [`Flow::untrusted`]: a call
    /// missing an argument is a program the checker rejects, and until it
    /// does the analysis must not read the absence as a grant.
    pub fn substitute(&self, args: &[Flow]) -> Flow {
        let mut out = Flow {
            base: self.base,
            params: BTreeSet::new(),
        };
        for index in &self.params {
            match args.get(*index as usize) {
                Some(arg) => {
                    out.base = out.base.join(arg.base);
                    out.params.extend(arg.params.iter().copied());
                }
                None => out.base = out.base.join(Authority::Untrusted),
            }
        }
        out.normalise()
    }

    /// Concretise against known parameter authorities.
    pub fn apply(&self, args: &[Authority]) -> Authority {
        let mut out = self.base;
        for index in &self.params {
            out = out.join(args.get(*index as usize).copied().unwrap_or_default());
        }
        out
    }

    /// Concretise where there are no parameters to concretise against.
    ///
    /// A residual parameter here means the walk that built this `Flow`
    /// never bound one, so the answer is Untrusted — closed, not open.
    pub fn authority(&self) -> Authority {
        self.apply(&[])
    }

    /// The parameter positions this value depends on, in order.
    pub fn depends_on(&self) -> impl Iterator<Item = u32> + '_ {
        self.params.iter().copied()
    }
}

// ---------------------------------------------------------------------
// Fixpoint 1 — signal labels and result summaries, together.
// ---------------------------------------------------------------------

/// The solved labels: what a read of each signal is worth, and what each
/// callable returns as a function of its arguments.
///
/// Built once by [`Solution::solve`] and then read-only. Before it existed,
/// a read of a signal was answered by walking that signal's initialiser at
/// every read — which re-derived the same answer once per reference and
/// recursed without bound through a chain of derived signals. One table,
/// filled once.
#[derive(Debug, Clone, Default)]
pub struct Solution {
    signals: BTreeMap<DefId, (Authority, Option<Grant>)>,
    results: BTreeMap<DefId, Flow>,
    /// How many times the worklist popped a definition. Not a correctness
    /// property; it is what the scaling measurement reads.
    steps: u32,
}

impl Solution {
    /// Solve both mutually recursive families in one worklist.
    pub fn solve(hir: &Hir, writers: &Writers) -> Solution {
        let mut solution = Solution::default();

        // Seed at the lattice's bottom. This is a least fixpoint: the
        // answer to "what can this value be" is itself the least solution
        // of these equations, so starting optimistic and rising is the
        // precise answer rather than merely a safe one. A recursive
        // function whose only base case is a literal is Trusted, which is
        // true of it.
        for (id, def) in hir.defs.iter() {
            match &def.kind {
                DefKind::Signal(_) => {
                    solution.signals.insert(id, (Authority::Trusted, None));
                }
                DefKind::Function(_) | DefKind::Release(_) => {
                    solution.results.insert(id, Flow::trusted());
                }
                // A `foreign` has no body to summarise: its result is
                // settled at the call site by G-FGN-T / G-FGN-A. A
                // `record`, `choice` or `view` is not a value. A
                // `component` is inlined before this pass runs, so the
                // declaration is dead and walking it would classify each
                // reference a second time in a context no instance has.
                DefKind::Foreign(_)
                | DefKind::View(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_) => {}
            }
        }

        // Who must be recomputed when a definition's value moves.
        let mut readers: BTreeMap<DefId, BTreeSet<DefId>> = BTreeMap::new();
        let mut worklist: Vec<DefId> = Vec::new();
        for (id, def) in hir.defs.iter() {
            if !matches!(
                def.kind,
                DefKind::Signal(_) | DefKind::Function(_) | DefKind::Release(_)
            ) {
                continue;
            }
            for read in reads_of(hir, id) {
                readers.entry(read).or_default().insert(id);
            }
            worklist.push(id);
        }

        while let Some(def) = worklist.pop() {
            solution.steps = solution.steps.saturating_add(1);
            let moved = match &hir.defs[def].kind {
                DefKind::Signal(_) => {
                    let next = signal_label(hir, writers, &solution, def);
                    let previous = solution.signals.insert(def, next);
                    previous != Some(next)
                }
                DefKind::Function(_) | DefKind::Release(_) => {
                    let next = result_flow(hir, &solution, def);
                    let entry = solution.results.entry(def).or_default();
                    entry.join_assign(&next)
                }
                DefKind::Foreign(_)
                | DefKind::View(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_) => false,
            };
            if moved {
                if let Some(dependents) = readers.get(&def) {
                    worklist.extend(dependents.iter().copied());
                }
            }
        }

        solution
    }

    /// What a **read** of a signal is worth — G-SIG, both clauses.
    pub fn signal(&self, signal: DefId) -> (Authority, Option<Grant>) {
        self.signals
            .get(&signal)
            .copied()
            .unwrap_or((Authority::Untrusted, None))
    }

    /// What a callable returns, as a function of its arguments.
    pub fn result(&self, callable: DefId) -> Flow {
        self.results.get(&callable).cloned().unwrap_or_default()
    }

    /// How many definitions the worklist popped. The scaling measurement
    /// reads this; nothing else should.
    pub fn steps(&self) -> u32 {
        self.steps
    }
}

/// G-SIG, both clauses, for one signal.
fn signal_label(
    hir: &Hir,
    writers: &Writers,
    solution: &Solution,
    signal: DefId,
) -> (Authority, Option<Grant>) {
    let DefKind::Signal(declared) = &hir.defs[signal].kind else {
        return (Authority::Untrusted, None);
    };
    // Clause 1: the declaration is the grant.
    if declared.trusted {
        return (Authority::Trusted, Some(Grant::Signal));
    }
    // Clause 2: no write site anywhere, and a Trusted initialiser.
    // `Writers` counts a two-way `Input` binding as a write, which is the
    // §21.8.4 repair — the browser writes it on every keystroke and there
    // is no `set` statement for a query over statement forms to find.
    if writers.is_written(signal) {
        return (Authority::Untrusted, None);
    }
    let integrity = Integrity::new(hir, solution);
    match integrity.of(declared.init).0 {
        Authority::Trusted => (Authority::Trusted, Some(Grant::Signal)),
        Authority::Untrusted => (Authority::Untrusted, None),
    }
}

/// The summary of one callable's body.
fn result_flow(hir: &Hir, solution: &Solution, callable: DefId) -> Flow {
    let mut integrity = Integrity::new(hir, solution);
    let (params, body) = match &hir.defs[callable].kind {
        DefKind::Function(function) => (&function.params, function.body),
        DefKind::Release(release) => (&release.params, release.body),
        DefKind::Signal(_)
        | DefKind::Foreign(_)
        | DefKind::View(_)
        | DefKind::Record(_)
        | DefKind::Choice(_)
        | DefKind::Component(_) => return Flow::untrusted(),
    };
    for (index, local) in params.iter().enumerate() {
        integrity.bind(*local, Flow::param(index as u32));
    }
    let body = Body {
        hir,
        integrity: &integrity,
    }
    .block(body);

    match &hir.defs[callable].kind {
        // §19.2 rule 3, unchanged by §19.10.3(a): a release's result is the
        // join of its arguments' integrity, taken **as written at the call
        // site, before endorsement**. So every parameter is in the summary
        // whether or not the body reads it — an endorsement discharges
        // REL-ARG at this release's sites and raises nothing anywhere,
        // because raising it inside would make four lines a universal
        // integrity launderer.
        //
        // The body's own contribution is joined on top rather than
        // replaced: a body that reaches an ungranted `foreign` returns
        // something its arguments did not determine, and rule 3 is a floor.
        DefKind::Release(release) => {
            let mut flow = body;
            for index in 0..release.params.len() {
                flow = flow.join(&Flow::param(index as u32));
            }
            flow
        }
        DefKind::Function(_)
        | DefKind::Signal(_)
        | DefKind::Foreign(_)
        | DefKind::View(_)
        | DefKind::Record(_)
        | DefKind::Choice(_)
        | DefKind::Component(_) => body,
    }
}

/// Every definition whose solved value this one's own value reads.
///
/// This is the reverse-dependency edge set of fixpoint 1, and it is read
/// off [`sites_of`] rather than re-derived, so a new edge kind has to be
/// ruled on in one place.
fn reads_of(hir: &Hir, def: DefId) -> BTreeSet<DefId> {
    let mut out = BTreeSet::new();
    for site in sites_of(hir, def) {
        match site {
            Site::Read { signal, .. } => {
                out.insert(signal);
            }
            Site::Call { callee, .. } => {
                out.insert(callee);
            }
            // A `foreign`'s result is a constant of its declaration, so it
            // never moves and nothing has to be recomputed when it does
            // not. A write, a binding, a non-place and an `environment`
            // read contribute no value to *this* definition's result.
            Site::ForeignCall { .. }
            | Site::Write { .. }
            | Site::Bind { .. }
            | Site::NotAPlace { .. }
            | Site::Environment { .. }
            // A capability's answer is inlined by the build, so it depends
            // on no other definition's solved value.
            | Site::Build { .. } => {}
        }
    }
    out
}

/// The value a body produces, as a [`Flow`] over the enclosing parameters.
struct Body<'a, 'b> {
    hir: &'a Hir,
    integrity: &'b Integrity<'a>,
}

impl Body<'_, '_> {
    fn block(&self, id: BlockId) -> Flow {
        let mut value = Flow::trusted();
        let mut produced = false;
        for stmt in &self.hir.blocks[id].stmts {
            match stmt {
                // A pipeline threads one value through its clauses. Every
                // clause expression is joined in, including a `keep`
                // predicate: §18.1 semantics 12 keeps `shape` and `value`
                // apart for secrecy, and this lattice carries one point
                // rather than the triple, so the join is over both.
                HirStmt::Pipeline(clause) => {
                    produced = true;
                    match clause {
                        HirPipeline::From(expr) | HirPipeline::TakeFirst(expr) => {
                            value = value.join(&self.flow(*expr));
                        }
                        HirPipeline::Keep { cond: expr, .. }
                        | HirPipeline::Sort { key: expr, .. }
                        | HirPipeline::MapEach { to: expr, .. } => {
                            value = value.join(&self.flow(*expr));
                        }
                    }
                }
                HirStmt::Give(expr) => {
                    produced = true;
                    value = value.join(&self.flow(*expr));
                }
                // The scrutinee and the condition are joined in because the
                // result depends on which arm ran — an implicit flow in the
                // integrity direction is still a flow.
                HirStmt::When(when) => {
                    produced = true;
                    value = value.join(&self.flow(when.scrutinee));
                    for arm in &when.arms {
                        value = match &arm.body {
                            HirArmBody::Show(expr) => value.join(&self.flow(*expr)),
                            HirArmBody::Block(block) => value.join(&self.block(*block)),
                        };
                    }
                }
                HirStmt::If(conditional) => {
                    produced = true;
                    value = value.join(&self.flow(conditional.cond));
                    value = value.join(&self.block(conditional.then));
                    if let Some(otherwise) = conditional.otherwise {
                        value = value.join(&self.block(otherwise));
                    }
                }
                HirStmt::Each(each) => {
                    produced = true;
                    value = value.join(&self.flow(each.iter));
                    value = value.join(&self.block(each.body));
                }
                // A mutation writes; it does not produce the body's value.
                HirStmt::Mutation(_) => {}
            }
        }
        if produced {
            value
        } else {
            // A body that produces nothing is not a value, and the caller
            // must not read the absence as a grant.
            Flow::untrusted()
        }
    }

    fn flow(&self, expr: ExprId) -> Flow {
        self.integrity.flow(expr).0
    }
}

// ---------------------------------------------------------------------
// The obligation sites.
// ---------------------------------------------------------------------

/// The closed set of authority obligation sites — §18.1 semantics 8, as
/// amended a third time by §21.7.6.
///
/// The spec calls this enum `Authority`; that name is spent here on the
/// two-point lattice itself, which is the load-bearing use, so the sites
/// carry the longer name and the spec's own labels as their codes.
///
/// **Four, and A4 must never return.** A4 was *a selector expression
/// inside a `release` body*, added to discharge REL-SELECT, which §19.9
/// refuted by counterexample and §19.10.1 deleted. Its replacement A5 is a
/// site at the release's **parameter list** instead, which is the whole of
/// §19.10.1's argument: the quantifier belongs on the boundary, because the
/// boundary is finite and named in the source while the body's spellings
/// are not. Anyone reaching for a fifth variant should read §21.7.6 first.
///
/// Deliberately not `#[non_exhaustive]`, for the reason [`Grant`] is not:
/// a new site must break every downstream `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObligationSite {
    /// **A1** — an index expression in a read or write place over a
    /// `trusted` signal. This is IDOR/BOLA: a browser that chooses the key
    /// chooses whose row comes back.
    A1,
    /// **A2** — an argument to a `foreign` parameter declared `trusted`.
    /// Path traversal and injection.
    A2,
    /// **A3** — the value written to a place declared `trusted`.
    A3,
    /// **A5** — an endorsed `release` parameter.
    ///
    /// Discharged trivially, by the declaration, exactly as A2 is
    /// discharged by `gives trusted T`. It exists so the site is **counted
    /// and printed** rather than being an absence: an endorsement is a
    /// human's signature and the point of the audit trail is that every
    /// signature is enumerable.
    A5,
}

impl ObligationSite {
    /// Every site there is. §18.1 semantics 8's list is closed, and this
    /// is the list a test can count.
    pub const CLOSED_LIST: [ObligationSite; 4] = [
        ObligationSite::A1,
        ObligationSite::A2,
        ObligationSite::A3,
        ObligationSite::A5,
    ];

    /// The spec's own label.
    pub fn code(self) -> &'static str {
        match self {
            ObligationSite::A1 => "A1",
            ObligationSite::A2 => "A2",
            ObligationSite::A3 => "A3",
            ObligationSite::A5 => "A5",
        }
    }

    /// The diagnostic an undischarged obligation at this site raises, if
    /// any. A5 has none: it is discharged by the declaration that creates
    /// it, and the *unendorsed* case is REL-ARG's E-REL-08 rather than a
    /// failure of A5.
    pub fn error_code(self) -> Option<&'static str> {
        match self {
            ObligationSite::A1 => Some("E-INT-02"),
            ObligationSite::A2 => Some("E-INT-05"),
            ObligationSite::A3 => Some("E-INT-03"),
            ObligationSite::A5 => None,
        }
    }
}

/// One obligation, at one site, with what was required and what was found.
///
/// # Why the key is an ordinal and not a span
///
/// Components are inlined and monomorphised before this pass runs, and
/// instantiation **reuses the caller's argument expression** rather than
/// copying it — so one `ExprId`, with one `Span`, can occupy two positions
/// in one owner's body. Keyed on a span, two obligations would collapse
/// into one and whichever was recorded last would answer for both; that is
/// the same defect `TierSplit::mutations_at` carries a `DefId` in its key
/// to avoid. `(owner, ordinal)` is unique by construction, because the
/// ordinal is handed out by the walk in source order.
#[derive(Debug, Clone, PartialEq)]
pub struct Obligation {
    pub site: ObligationSite,
    pub owner: DefId,
    pub ordinal: u32,
    /// Always [`Authority::Trusted`]. Carried rather than assumed so the
    /// report prints the rule instead of restating it.
    pub required: Authority,
    pub found: Authority,
    /// Which grant answered for it, when one did.
    pub discharged_by: Option<Grant>,
    pub span: Span,
}

impl Obligation {
    /// Whether something answered for this site.
    ///
    /// A Trusted value answers for itself. A grant answers for an
    /// Untrusted one, and A5 is the site where that always happens: the
    /// `trusted` clause discharges it **because** the value is a value the
    /// browser chose, which is the only reason to write the clause.
    pub fn is_discharged(&self) -> bool {
        self.found.is_trusted() || self.discharged_by.is_some()
    }
}

// ---------------------------------------------------------------------
// Fixpoint 2, and the check.
// ---------------------------------------------------------------------

/// The whole analysis: the solved labels, the merged parameter
/// authorities, every obligation, and every diagnostic.
pub struct Analysis {
    pub writers: Writers,
    pub solution: Solution,
    params: BTreeMap<(DefId, u32), Authority>,
    obligations: Vec<Obligation>,
    diagnostics: Vec<GraphError>,
    walks: u32,
}

impl Analysis {
    /// What every call site, joined, passes to one parameter.
    pub fn param(&self, callable: DefId, index: u32) -> Authority {
        self.params
            .get(&(callable, index))
            .copied()
            .unwrap_or_default()
    }

    pub fn obligations(&self) -> &[Obligation] {
        &self.obligations
    }

    /// Every obligation raised at one site kind, in source order.
    pub fn at(&self, site: ObligationSite) -> impl Iterator<Item = &Obligation> {
        self.obligations.iter().filter(move |o| o.site == site)
    }

    pub fn diagnostics(&self) -> &[GraphError] {
        &self.diagnostics
    }

    /// Hand the diagnostics to the one [`crate::Verdict`] §17.1.2's table
    /// gives the `ifc` stage.
    ///
    /// The obligations and the solved labels stay behind, deliberately.
    /// They are §19.5's audit trail, and §21.6 item 18 forbids shipping
    /// `report.json`'s framing before the claim question is settled — so
    /// the rules reach the user as diagnostics and the report does not
    /// reach them at all.
    pub fn into_diagnostics(self) -> Vec<GraphError> {
        self.diagnostics
    }

    pub fn errors(&self) -> impl Iterator<Item = &GraphError> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }

    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }

    /// Definition-body walks performed, across both fixpoints and the
    /// check. The scaling measurement reads this.
    pub fn walks(&self) -> u32 {
        self.walks
    }
}

/// Run the whole thing.
///
/// **Wired, on §21.8.8 option 2's terms.** [`crate::ifc`] calls this and
/// merges its diagnostics into the one [`crate::Verdict`] §17.1.2's table
/// gives the `ifc` pass, which is where §21.6 item 2 schedules the second
/// lattice — *"Plan 4 tail, on the pass that exists"*. §21.6 item 18's
/// third amendment is the instruction: **ship the rules, do not ship the
/// claim.** A pass that is built and never called ships neither: it rejects
/// none of the attacks §21.8.6 enumerates, and *"costs 0 on programs that
/// opt into nothing"* is vacuous when it costs 0 on every program.
///
/// What must not ship is any statement that these rules deliver robustness
/// of any kind. They are review aids. `launder3.zd` satisfies every one of
/// them and launders a credit-card number (R1).
pub fn authority(hir: &Hir, split: &TierSplit) -> Analysis {
    let writers = Writers::of(hir, split);
    let solution = Solution::solve(hir, &writers);

    let mut params: BTreeMap<(DefId, u32), Authority> = BTreeMap::new();
    // Seed at the lattice's bottom, and note what that means: a callable
    // no call site reaches keeps Trusted parameters. That is the least
    // fixpoint and it is right — a parameter with no argument has no value
    // and raises no obligation instance — but it is only sound while the
    // enumeration of call sites is complete, which is why the walk below
    // is a total function over `HirExprKind` with no wildcard.
    for (id, def) in hir.defs.iter() {
        let count = match &def.kind {
            DefKind::Function(function) => function.params.len(),
            DefKind::Release(release) => release.params.len(),
            DefKind::Signal(_)
            | DefKind::Foreign(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => 0,
        };
        for index in 0..count {
            params.insert((id, index as u32), Authority::Trusted);
        }
    }

    let mut walks = 0u32;
    let mut worklist: Vec<DefId> = hir.defs.iter().map(|(id, _)| id).collect();
    let mut queued: BTreeSet<DefId> = worklist.iter().copied().collect();
    // Fixpoint 2. Every parameter can move at most once — Trusted to
    // Untrusted — so the number of re-enqueues is bounded by the number of
    // parameters in the program, and each pop costs one walk of one body.
    //
    // Only the *callee* is re-enqueued when one of its parameters moves,
    // and that is the whole of what stratification buys. A caller's
    // argument authorities are computed from its own parameters and from
    // fixpoint 1's tables, neither of which this fixpoint can disturb — so
    // nothing propagates backwards and there is no second interleaving.
    while let Some(def) = worklist.pop() {
        queued.remove(&def);
        walks += 1;
        let mut walk = Walk::new(hir, split, &solution, &params, def, Mode::Propagate);
        walk.def(def);
        for ((callee, index), authority) in walk.raised {
            let entry = params.entry((callee, index)).or_default();
            let joined = entry.join(authority);
            if joined != *entry {
                *entry = joined;
                if queued.insert(callee) {
                    worklist.push(callee);
                }
            }
        }
    }

    // One final pass, with the parameter authorities settled, to raise the
    // obligations and the diagnostics.
    let mut obligations = Vec::new();
    let mut diagnostics = Vec::new();
    for (id, _) in hir.defs.iter() {
        walks += 1;
        let mut walk = Walk::new(hir, split, &solution, &params, id, Mode::Check);
        walk.def(id);
        obligations.append(&mut walk.obligations);
        diagnostics.append(&mut walk.diagnostics);
    }

    // §18.1's one declaration rule on state, checked over the whole
    // program for the same reason the three release rules below are.
    diagnostics.extend(crate::integrity::int_01(hir));

    // The declaration-level release rules. REL-ARG is raised by the walk
    // above, at call sites; these three are properties of the declaration
    // and are checked once each, here, so that there is one entry point
    // rather than three a driver could forget one of.
    for (id, def) in hir.defs.iter() {
        match &def.kind {
            DefKind::Release(_) => {
                diagnostics.extend(crate::integrity::rel_closed(hir, id));
                diagnostics.extend(crate::integrity::rel_pure(hir, id));
                diagnostics.extend(crate::integrity::w_rel_01(hir, id));
            }
            DefKind::Signal(_)
            | DefKind::Function(_)
            | DefKind::Foreign(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => {}
        }
    }

    Analysis {
        writers,
        solution,
        params,
        obligations,
        diagnostics,
        walks,
    }
}

/// Which expression lands on which parameter.
///
/// Named arguments are matched by name and positional ones fill the
/// remaining slots in order, because the HIR keeps arguments as written
/// rather than reordered. A slot no argument fills stays `None`, and every
/// caller reads that as Untrusted: a call missing an argument is a program
/// the checker rejects, and until it does the absence must not read as a
/// grant.
pub(crate) fn match_args(hir: &Hir, params: &[LocalId], args: &[HirArg]) -> Vec<Option<ExprId>> {
    let mut out = vec![None; params.len()];
    let mut positional = args.iter().filter_map(|arg| match arg {
        HirArg::Positional(expr) => Some(*expr),
        HirArg::Named { .. } => None,
    });
    for (index, param) in params.iter().enumerate() {
        let name = &hir.locals[*param].name;
        let named = args.iter().find_map(|arg| match arg {
            HirArg::Named { name: given, value } if given == name => Some(*value),
            HirArg::Named { .. } | HirArg::Positional(_) => None,
        });
        out[index] = match named {
            Some(expr) => Some(expr),
            None => positional.next(),
        };
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Fixpoint 2: push argument authorities onto parameters, emit nothing.
    Propagate,
    /// The final pass: raise obligations and diagnostics.
    Check,
}

/// One walk of one definition's body, with the enclosing parameters bound
/// to what every call site passes them.
struct Walk<'a> {
    hir: &'a Hir,
    split: &'a TierSplit,
    owner: DefId,
    mode: Mode,
    integrity: Integrity<'a>,
    /// The authority of the decisions that led here — §18.1 semantics 11's
    /// program counter, over this lattice rather than the secrecy one.
    ///
    /// *Whether* a write happens is the same decision as what it holds, so
    /// a write to a `trusted` place under a browser-chosen condition is
    /// E-INT-04 even when the value written is a literal.
    pc: Flow,
    ordinal: u32,
    raised: Vec<((DefId, u32), Authority)>,
    obligations: Vec<Obligation>,
    diagnostics: Vec<GraphError>,
}

impl<'a> Walk<'a> {
    fn new(
        hir: &'a Hir,
        split: &'a TierSplit,
        solution: &'a Solution,
        params: &BTreeMap<(DefId, u32), Authority>,
        owner: DefId,
        mode: Mode,
    ) -> Walk<'a> {
        let mut integrity = Integrity::new(hir, solution);
        let own = match &hir.defs[owner].kind {
            DefKind::Function(function) => Some(&function.params),
            DefKind::Release(release) => Some(&release.params),
            DefKind::Signal(_)
            | DefKind::Foreign(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => None,
        };
        if let Some(own) = own {
            for (index, local) in own.iter().enumerate() {
                let authority = params
                    .get(&(owner, index as u32))
                    .copied()
                    .unwrap_or_default();
                integrity.bind(*local, Flow::exact(authority));
            }
        }
        Walk {
            hir,
            split,
            owner,
            mode,
            integrity,
            // Nothing has been decided yet, so the empty join: Trusted.
            pc: Flow::trusted(),
            ordinal: 0,
            raised: Vec::new(),
            obligations: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn authority(&self, expr: ExprId) -> Authority {
        self.integrity.of(expr).0
    }

    fn next_ordinal(&mut self) -> u32 {
        let ordinal = self.ordinal;
        self.ordinal += 1;
        ordinal
    }

    fn def(&mut self, def: DefId) {
        match &self.hir.defs[def].kind {
            DefKind::Signal(signal) => self.expr(signal.init),
            DefKind::Function(function) => self.block(function.body),
            DefKind::Release(release) => self.block(release.body),
            DefKind::View(view) => {
                let nodes = view.nodes.clone();
                self.nodes(&nodes);
            }
            // A `component` is inlined into the view before this pass
            // runs; walking the declaration would raise every obligation
            // inside it a second time, in a context no instance has.
            // A `foreign`, `record` and `choice` have no body.
            DefKind::Component(_)
            | DefKind::Foreign(_)
            | DefKind::Record(_)
            | DefKind::Choice(_) => {}
        }
    }

    fn block(&mut self, id: BlockId) {
        let stmts = self.hir.blocks[id].stmts.clone();
        for stmt in &stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Pipeline(clause) => match clause {
                HirPipeline::From(expr) | HirPipeline::TakeFirst(expr) => self.expr(*expr),
                HirPipeline::Keep { var, cond } => {
                    self.bind_opaque(*var);
                    self.expr(*cond);
                }
                HirPipeline::Sort { var, key } => {
                    self.bind_opaque(*var);
                    self.expr(*key);
                }
                HirPipeline::MapEach { var, to } => {
                    self.bind_opaque(*var);
                    self.expr(*to);
                }
            },
            HirStmt::Give(expr) => self.expr(*expr),
            HirStmt::Mutation(mutation) => self.mutation(mutation),
            HirStmt::When(when) => {
                self.expr(when.scrutinee);
                let scrutinee = self.integrity.flow(when.scrutinee).0;
                let outer = self.pc.clone();
                self.pc = outer.clone().join(&scrutinee);
                let arms = when.arms.clone();
                for arm in &arms {
                    for binding in &arm.bindings {
                        self.integrity.bind(*binding, scrutinee.clone());
                    }
                    match &arm.body {
                        HirArmBody::Show(expr) => self.expr(*expr),
                        HirArmBody::Block(block) => self.block(*block),
                    }
                }
                self.pc = outer;
            }
            HirStmt::Each(each) => {
                self.expr(each.iter);
                let iter = self.integrity.flow(each.iter).0;
                self.integrity.bind(each.var, iter);
                self.block(each.body);
            }
            HirStmt::If(conditional) => {
                self.expr(conditional.cond);
                let outer = self.pc.clone();
                self.pc = outer.clone().join(&self.integrity.flow(conditional.cond).0);
                self.block(conditional.then);
                if let Some(otherwise) = conditional.otherwise {
                    self.block(otherwise);
                }
                self.pc = outer;
            }
        }
    }

    /// A binder whose value is one element of whatever is being walked.
    ///
    /// The pipeline's running value is not reconstructed here — this walk
    /// exists to find sites, not to compute the body's result — so the
    /// binder is bound Untrusted, which over-reports rather than
    /// under-reports. [`Body`] is where the value is computed, and it is
    /// the one that has to be precise.
    fn bind_opaque(&mut self, var: LocalId) {
        self.integrity.bind(var, Flow::untrusted());
    }

    fn mutation(&mut self, mutation: &HirMutation) {
        let place = mutation.place();
        let value = mutation.value();
        self.expr(value);

        let trusted_signal = match place.base {
            Res::Def(def) => matches!(
                &self.hir.defs[def].kind,
                DefKind::Signal(signal) if signal.trusted
            )
            .then_some(def),
            Res::Local(_) | Res::Builtin(_) | Res::BuiltinVariant(_) | Res::Variant { .. } => None,
        };
        let trusted_place = trusted_signal.is_some();

        // **A3** — the value written to a place declared `trusted`.
        if let Some(signal) = trusted_signal {
            // §18.1 semantics 4, which survives §21.7.6's deletion of
            // semantics 5 and is not derivable from the grant set. A
            // client-rooted write to `durable` state is a *command*: the
            // expression in the source is what this program posts, and the
            // endpoint accepts whatever any browser posts to it. So the
            // crossing decides, not the expression — otherwise a literal
            // would grant Trusted to a value the compiler never sees.
            let commanded = self.split.is_commanded(place.span, signal);
            let (found, grant) = if commanded {
                (Authority::Untrusted, None)
            } else {
                (self.authority(value), self.integrity.of(value).1)
            };
            self.raise_because(ObligationSite::A3, found, grant, place.span, commanded);
            self.implicit_flow(place.span);
        }

        for segment in &place.path {
            let HirPathSeg::Index(index) = segment else {
                continue;
            };
            self.expr(*index);
            // **A1**, on the write side.
            if trusted_place {
                let found = self.authority(*index);
                let grant = self.integrity.of(*index).1;
                self.raise(
                    ObligationSite::A1,
                    found,
                    grant,
                    self.hir.exprs[*index].span,
                );
            }
        }
    }

    fn nodes(&mut self, nodes: &[HirNode]) {
        for node in nodes {
            match node {
                HirNode::Element(element) => {
                    let args: Vec<ExprId> = element.args.iter().map(arg_expr).collect();
                    for arg in args {
                        self.expr(arg);
                    }
                    let children = element.children.clone();
                    self.nodes(&children);
                }
                HirNode::Each(each) => {
                    self.expr(each.iter);
                    let iter = self.integrity.flow(each.iter).0;
                    self.integrity.bind(each.var, iter);
                    let body = each.body.clone();
                    self.nodes(&body);
                }
                HirNode::When(when) => {
                    self.expr(when.scrutinee);
                    let scrutinee = self.integrity.flow(when.scrutinee).0;
                    let arms = when.arms.clone();
                    for arm in &arms {
                        for binding in &arm.bindings {
                            self.integrity.bind(*binding, scrutinee.clone());
                        }
                        match &arm.body {
                            HirNodeArmBody::Show(element) => {
                                self.nodes(&[HirNode::Element((**element).clone())]);
                            }
                            HirNodeArmBody::Nodes(nodes) => self.nodes(nodes),
                        }
                    }
                }
                HirNode::If(conditional) => {
                    self.expr(conditional.cond);
                    let then = conditional.then.clone();
                    self.nodes(&then);
                    if let Some(otherwise) = &conditional.otherwise {
                        let otherwise = otherwise.clone();
                        self.nodes(&otherwise);
                    }
                }
                HirNode::Handler(handler) => self.block(handler.body),
                // A component instance's own state. It is `client`-placed
                // storage the browser owns, so a read of one is Untrusted
                // by the absence of a grant rather than by a rule — the
                // binder is deliberately left unbound.
                HirNode::Scope(scope) => {
                    let locals = scope.locals.clone();
                    for local in &locals {
                        self.expr(local.init);
                    }
                    let body = scope.body.clone();
                    self.nodes(&body);
                }
                // Instantiation replaced every one of these with the nodes
                // nested under the call site, so none survives into a view.
                HirNode::Children(_) => {}
            }
        }
    }

    /// The obligation walk over expressions.
    ///
    /// **Total over `HirExprKind`, with no wildcard**, for the reason
    /// [`Integrity::flow`] is: a new expression form is a new place a value
    /// can reach a `trusted` index, a `trusted` foreign parameter or a
    /// release argument, and the compiler is what makes somebody rule on it.
    fn expr(&mut self, id: ExprId) {
        match self.hir.exprs[id].kind.clone() {
            HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            | HirExprKind::Environment(_)
            | HirExprKind::Address
            | HirExprKind::Ref(_) => {}
            HirExprKind::List(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            HirExprKind::Map(entries) => {
                for (key, value) in entries {
                    self.expr(key);
                    self.expr(value);
                }
            }
            HirExprKind::Unary { operand, .. } | HirExprKind::Operator { operand, .. } => {
                self.expr(operand)
            }
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            // The argument is walked: `build read (orders at i)` still puts
            // `i` in an index place, and the obligation it raises is the
            // same one it would raise anywhere else.
            HirExprKind::Build { argument, .. } => self.expr(argument),
            HirExprKind::Field { base, .. } => self.expr(base),
            HirExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
                // **A1** — an index in a *read* place over a `trusted`
                // signal. `orders at visitor` passes; `orders at candidate`
                // off the wire is E-INT-02, which is IDOR caught.
                if self.reads_trusted_signal(base) {
                    let found = self.authority(index);
                    let grant = self.integrity.of(index).1;
                    self.raise(ObligationSite::A1, found, grant, self.hir.exprs[index].span);
                }
            }
            HirExprKind::Call { callee, args } => {
                for arg in &args {
                    self.expr(arg_expr(arg));
                }
                self.call(callee, &args, self.hir.exprs[id].span);
            }
            HirExprKind::OfCall { callee, operand } => {
                self.expr(operand);
                self.call(
                    callee,
                    &[HirArg::Positional(operand)],
                    self.hir.exprs[id].span,
                );
            }
        }
    }

    /// Whether a place expression bottoms out at a signal declared
    /// `trusted`. `orders at k`, `orders.rows at k` and `orders at a at b`
    /// are all places over `orders`.
    fn reads_trusted_signal(&self, expr: ExprId) -> bool {
        let mut current = expr;
        loop {
            match &self.hir.exprs[current].kind {
                HirExprKind::Ref(Res::Def(def)) => {
                    return matches!(
                        &self.hir.defs[*def].kind,
                        DefKind::Signal(signal) if signal.trusted
                    )
                }
                HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => {
                    current = *base
                }
                HirExprKind::Number(_)
                | HirExprKind::Text(_)
                | HirExprKind::Truth(_)
                | HirExprKind::Empty
                | HirExprKind::Environment(_)
                | HirExprKind::Address
                | HirExprKind::Ref(_)
                | HirExprKind::List(_)
                | HirExprKind::Map(_)
                | HirExprKind::Unary { .. }
                | HirExprKind::Operator { .. }
                | HirExprKind::Binary { .. }
                | HirExprKind::Call { .. }
                | HirExprKind::OfCall { .. }
                // A capability's result is a fresh value the compiler
                // produced, not a place over a declared signal.
                | HirExprKind::Build { .. } => return false,
            }
        }
    }

    /// One call site: A2, A5, REL-ARG, and fixpoint 2's propagation.
    fn call(&mut self, callee: Res, args: &[HirArg], span: Span) {
        let Res::Def(def) = callee else {
            return;
        };
        match &self.hir.defs[def].kind {
            DefKind::Foreign(foreign) => {
                let matched = self.match_args(&foreign.params, args);
                for (index, trusted) in foreign.trusted_params.iter().enumerate() {
                    if !trusted {
                        continue;
                    }
                    // §18.1 semantics 7: integrity obligations exist only
                    // where something can be protected. There is no such
                    // thing as protecting a browser from itself, so a
                    // `foreign … is client` raises no A2 — the whole client
                    // walk is exempt and it falls out of the declaration
                    // rather than needing a rule.
                    if foreign.site == zdc_ast::ForeignSite::Client {
                        continue;
                    }
                    let (found, grant, at) = match matched.get(index).copied().flatten() {
                        Some(arg) => (
                            self.authority(arg),
                            self.integrity.of(arg).1,
                            self.hir.exprs[arg].span,
                        ),
                        // A missing argument is a program the checker
                        // rejects; until it does, the absence is not a
                        // grant.
                        None => (Authority::Untrusted, None, span),
                    };
                    // **A2** — an argument to a `foreign` parameter
                    // declared `trusted`.
                    self.raise(ObligationSite::A2, found, grant, at);
                }
            }
            DefKind::Function(function) => {
                let params = function.params.clone();
                self.propagate(def, &params, args);
            }
            DefKind::Release(release) => {
                let params = release.params.clone();
                let endorsed = release.endorsed.clone();
                self.propagate(def, &params, args);
                self.rel_arg(def, &params, &endorsed, args, span);
            }
            DefKind::Signal(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => {}
        }
    }

    /// Fixpoint 2's transfer: the argument's authority lands on the
    /// parameter, joined with every other call site's.
    fn propagate(&mut self, callee: DefId, params: &[LocalId], args: &[HirArg]) {
        if self.mode != Mode::Propagate {
            return;
        }
        let matched = self.match_args(params, args);
        for (index, arg) in matched.iter().enumerate() {
            let authority = match arg {
                Some(expr) => self.authority(*expr),
                None => Authority::Untrusted,
            };
            self.raised.push(((callee, index as u32), authority));
        }
    }

    /// **REL-ARG** — §19.10.1, error **E-REL-08** — and obligation site
    /// **A5**.
    ///
    /// ```text
    ///   ∀ i . p_i ∉ endorsed(f) ⟹ integrity(e_i).value = Trusted
    ///  ─────────────────────────────────────────────────────────
    ///   f with e⃗  may occur at site
    /// ```
    ///
    /// It never inspects `body(f)`. That is the whole of §19.10.1's
    /// argument against REL-SELECT: every rewrite of the body — a
    /// `keep each … where`, an index-recursive fold, an `at`, a helper, a
    /// recursive descent nobody has invented — receives the identical
    /// verdict, because none of them changes which values crossed the
    /// parameter list.
    fn rel_arg(
        &mut self,
        release: DefId,
        params: &[LocalId],
        endorsed: &[bool],
        args: &[HirArg],
        span: Span,
    ) {
        if self.mode != Mode::Check {
            return;
        }
        let matched = self.match_args(params, args);
        for (index, param) in params.iter().enumerate() {
            let (found, at) = match matched.get(index).copied().flatten() {
                Some(arg) => (self.authority(arg), self.hir.exprs[arg].span),
                None => (Authority::Untrusted, span),
            };
            let is_endorsed = endorsed.get(index).copied().unwrap_or(false);
            if is_endorsed {
                // **A5.** Discharged by the declaration, and raised so the
                // signature is counted rather than being an absence.
                self.raise(ObligationSite::A5, found, Some(Grant::Release), at);
                continue;
            }
            if found.is_trusted() {
                continue;
            }
            let name = self.hir.locals[*param].name.clone();
            let release_name = self.hir.defs[release].name.clone();
            self.diagnostics.push(
                GraphError::new(
                    "E-REL-08",
                    // Inside §7.3's inline budget, which the corpus test
                    // measures on a provoked message rather than on this
                    // literal. What the rule *requires* of the declaration
                    // is one `zdc explain E-REL-08` away.
                    format!(
                        "`{release_name}` is given a value for `{name}` that no grant accounts \
                         for, and `{name}` is not endorsed. Rule REL-ARG (§19.10.1)."
                    ),
                    at,
                )
                .with_notes(vec![(
                    self.hir.defs[release].span,
                    format!("`{name}` is declared here, with no `trusted {name}`"),
                )])
                .with_help(format!(
                    "Writing `trusted {name}` in `{release_name}`'s declaration accepts the \
                     argument. It is a signature, not a check: it records that this makes the \
                     release a function of a value the browser chose, and it will appear in \
                     `zdc build --report`."
                )),
            );
        }
    }

    fn match_args(&self, params: &[LocalId], args: &[HirArg]) -> Vec<Option<ExprId>> {
        match_args(self.hir, params, args)
    }

    fn raise(&mut self, site: ObligationSite, found: Authority, grant: Option<Grant>, span: Span) {
        self.raise_because(site, found, grant, span, false);
    }

    /// [`Walk::raise`], with the A3 message told which of its two reasons
    /// applies. A value a browser *chose* and a value a browser *sends*
    /// are different findings, and a repair aimed at the wrong one is
    /// wasted work.
    fn raise_because(
        &mut self,
        site: ObligationSite,
        found: Authority,
        grant: Option<Grant>,
        span: Span,
        commanded: bool,
    ) {
        if self.mode != Mode::Check {
            return;
        }
        let ordinal = self.next_ordinal();
        let discharged_by = match site {
            // G-REL, awarded at the call site rather than inside the body
            // (§19.10.3(a)). It discharges A5 whatever the argument was,
            // because discharging an Untrusted argument is the entire
            // content of an endorsement.
            ObligationSite::A5 => Some(Grant::Release),
            ObligationSite::A1 | ObligationSite::A2 | ObligationSite::A3 => match found {
                Authority::Trusted => grant,
                Authority::Untrusted => None,
            },
        };
        self.obligations.push(Obligation {
            site,
            owner: self.owner,
            ordinal,
            required: Authority::Trusted,
            found,
            discharged_by,
            span,
        });
        if found.is_trusted() {
            return;
        }
        let Some(code) = site.error_code() else {
            return;
        };
        let (message, help) = match site {
            ObligationSite::A1 => (
                "this key was chosen by the browser, and the collection it indexes is `trusted`. \
                 Site A1 (§18.1 semantics 8): an index into a `trusted` place must itself be \
                 Trusted."
                    .to_string(),
                "Index by a value the program owns, or declare the signal without `trusted` and \
                 accept that no index into it is checked."
                    .to_string(),
            ),
            ObligationSite::A2 => (
                "this argument was chosen by the browser, and the parameter is declared \
                 `trusted`. Site A2 (§18.1 semantics 8): a `foreign` that asks for a Trusted \
                 argument must be given one."
                    .to_string(),
                "The declaration is what asks for this. Either pass a value that derives from a \
                 grant, or drop `trusted` from the parameter and record why."
                    .to_string(),
            ),
            ObligationSite::A3 if commanded => (
                "a browser sends this write, and the place written is declared `trusted`. Site \
                 A3 (§18.1 semantics 8, semantics 4): what arrives at the endpoint is whatever a \
                 browser posted, not what is written here."
                    .to_string(),
                "The value in the source is not the value on the wire. Write from a server-rooted \
                 body, or drop `trusted` from the declaration."
                    .to_string(),
            ),
            ObligationSite::A3 => (
                "this value was chosen by the browser, and the place written is declared \
                 `trusted`. Site A3 (§18.1 semantics 8): a write to a `trusted` place must carry \
                 a Trusted value."
                    .to_string(),
                "A browser must not choose who is a moderator. Write a value the program owns, \
                 or drop `trusted` from the declaration."
                    .to_string(),
            ),
            // A5 has no error code, so this arm is unreachable through the
            // `let … else` above and is written out rather than wildcarded.
            ObligationSite::A5 => (String::new(), String::new()),
        };
        self.diagnostics
            .push(GraphError::new(code, message, span).with_help(help));
    }

    /// **E-INT-04** — §18.1 semantics 11. A write to a `trusted` place
    /// under a pc a browser chose.
    ///
    /// Not an [`ObligationSite`]: §21.7.6 closes that list at four, and
    /// this is a rule about *whether* the write runs rather than about a
    /// value at a site. It is raised beside A3 because it is the same
    /// write, and because a program where only the condition is
    /// attacker-chosen would otherwise pass with the value a literal.
    fn implicit_flow(&mut self, span: Span) {
        if self.mode != Mode::Check {
            return;
        }
        if self.pc.authority().is_trusted() {
            return;
        }
        self.diagnostics.push(
            GraphError::new(
                "E-INT-04",
                "whether this write happens was decided by a browser, and the place written is \
                 declared `trusted`. §18.1 semantics 11: an implicit flow is a flow."
                    .to_string(),
                span,
            )
            .with_help(
                "The condition is as much a part of the write as the value is. Decide it from a \
                 value the program owns, or drop `trusted` from the declaration."
                    .to_string(),
            ),
        );
    }
}
