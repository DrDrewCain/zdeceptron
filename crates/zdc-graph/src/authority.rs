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
//! **Fixpoint 2**, which lands next, merges for every parameter the
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
//! Both fixpoints terminate; only the first is here. Fixpoint 2 and the
//! obligation sites land on top of it.
//!
//! # What is **not** claimed
//!
//! No robustness property, for the reasons [`crate::integrity`] states at
//! length. A solved label says which grant, if any, accounts for a value.
//! It does not say the program leaks, and a program in which every label
//! is Trusted is not thereby free of laundering — §21.8.1's `launder3.zd`
//! is such a program.

use std::collections::{BTreeMap, BTreeSet};

use zdc_hir::{
    BlockId, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirPipeline, HirStmt, LocalId,
};

use crate::integrity::{Authority, Grant, Integrity, Writers};
use crate::sites::{sites_of, Site};

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
            | Site::Environment { .. } => {}
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
