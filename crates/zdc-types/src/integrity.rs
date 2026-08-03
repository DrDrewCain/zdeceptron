//! The integrity direction of the lattice — spec §18.1.
//!
//! `secret` answers *who may learn this value*. `trusted` answers *who
//! chose it*. This pass is the second question, over the lattice
//! `Trusted ⊑ Untrusted`, and it is opt-in exactly as secrecy is: a
//! program that never writes the word is checked no differently than
//! before, which is the whole of §18.1.4's answer to Ballerina.
//!
//! # Where the untrusted set comes from
//!
//! §18.1 semantics 3 names four source kinds and claims the list is
//! complete because it is one arm of a classifier the compiler already
//! runs. Event payloads are a **fifth**, and §18.1 does not name them —
//! see the crate report. They are not, however, a hole: a payload lives
//! only inside a handler, a handler is a client region, and a client value
//! reaches a server region by exactly two routes, both of which this pass
//! labels Untrusted unconditionally. The fifth row is a precision
//! improvement (the diagnostic can say *which* browser-chosen value), not
//! a soundness repair. It is written down here so it is not rediscovered.
//!
//! | Source | Kind | Here |
//! |---|---|---|
//! | A lifted client signal | `Crossing::Lift` | a `client` read from a server-rooted body |
//! | A command argument | `MutCrossing::Command` | a client-rooted write to `server`/`durable` |
//! | An event payload | **new** | the binder of `on click with press` |
//! | A read of untrusted stored state | semantics 2 | a `server`/`durable` signal not declared `trusted` |
//! | An `inbound` trigger payload | §14G.4 | not built |
//! | A route parameter with no `in` | §14G.2 | the binder of a route `when` arm |
//! | `address` itself | §14G.7.3 | an unmatched read of the whole address |
//!
//! # Where the obligations are
//!
//! §18.1 semantics 8 closes them at three, of which two are reachable in
//! a language with no `foreign` declaration:
//!
//! * **A1 / E-INT-02** — an index in a place over a `trusted` signal.
//! * **A3 / E-INT-03** — the value written to a `trusted` place.
//!
//! plus the two rules that are about the declaration rather than a value:
//! **E-INT-01** (`trusted` on a placement that cannot carry it) and
//! **E-INT-04** (a write under an untrusted `pc`).
//!
//! A2 — an argument to a `trusted` `foreign` parameter — has nothing to
//! attach to until §14E is implemented, and is deliberately absent rather
//! than stubbed.

use std::collections::HashMap;

use zdc_hir::{
    BlockId, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirExprKind, HirMutation, HirNode,
    HirNodeArmBody, HirPathSeg, HirPipeline, HirPlace, HirStmt, LocalId, Res, RouteParam,
};
use zdc_lexer::Span;

use crate::placement::{Placements, ReadContext};
use crate::TypeError;

/// The two points of the lattice, and — when the answer is `Untrusted` —
/// what made it so.
///
/// The reason travels with the label rather than being reconstructed at
/// the obligation site, because by the time a diagnostic is written the
/// expression that introduced the taint may be three calls away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    untrusted: bool,
    /// How the value got its label, phrased to complete the sentence
    /// "… because {because}".
    because: Option<String>,
    /// Where that happened.
    at: Option<Span>,
}

impl Label {
    fn trusted() -> Label {
        Label {
            untrusted: false,
            because: None,
            at: None,
        }
    }

    fn untrusted(because: impl Into<String>, at: Span) -> Label {
        Label {
            untrusted: true,
            because: Some(because.into()),
            at: Some(at),
        }
    }

    /// `Sym ⊔ Sym`. The only operation this pass performs on a label,
    /// which is what §18.1's "a pass that only joins is polymorphic in its
    /// lattice" means in practice.
    fn join(self, other: Label) -> Label {
        match (self.untrusted, other.untrusted) {
            (true, _) => self,
            (false, true) => other,
            (false, false) => Label::trusted(),
        }
    }

    fn is_untrusted(&self) -> bool {
        self.untrusted
    }

    fn reason(&self) -> &str {
        self.because
            .as_deref()
            .unwrap_or("a browser had a hand in choosing it")
    }
}

/// Run the integrity pass over a resolved program.
///
/// Nothing is returned but diagnostics: integrity has *authorities*, not
/// sinks (§18.1 semantics 8), so there is no label for a later pass to
/// consume — the obligation is discharged where it is raised or it is
/// reported.
pub(crate) fn check(hir: &Hir, placements: &dyn Placements) -> Vec<TypeError> {
    let mut pass = Pass {
        hir,
        placements,
        params: HashMap::new(),
        results: HashMap::new(),
        derived: HashMap::new(),
        locals: HashMap::new(),
        payloads: HashMap::new(),
        errors: Vec::new(),
        reporting: false,
        context: ReadContext::Client,
    };

    // Phase 1 — the fixpoint. Parameter and result labels only ever move
    // Trusted -> Untrusted over a two-point lattice, so the walk is
    // monotone and one extra round past the last change settles it. The
    // bound is the number of definitions plus one, and it is asserted
    // rather than trusted.
    let rounds = hir.defs.iter().count() + 1;
    for _ in 0..rounds {
        let before = (
            pass.params.clone(),
            pass.results.clone(),
            pass.derived.clone(),
        );
        pass.sweep();
        if before
            == (
                pass.params.clone(),
                pass.results.clone(),
                pass.derived.clone(),
            )
        {
            break;
        }
    }

    // Phase 2 — the obligations, once, on labels that no longer move.
    pass.reporting = true;
    pass.sweep();
    pass.errors.sort_by_key(|error| error.span.start);
    pass.errors
}

/// The one context a definition's body runs in, or `Client` when the
/// placement pass reports more than one.
///
/// A definition reached from two contexts has two provenances for the same
/// expression, which one label cannot hold. `Client` is the conservative
/// answer: it is the context in which every authority is obliged.
fn context_of(placements: &dyn Placements, id: DefId) -> ReadContext {
    let reached = placements.read_contexts(id);
    if reached.len() == 1 {
        reached[0]
    } else {
        ReadContext::Client
    }
}

/// The label a `when` arm's binders start on.
///
/// For every pattern but a route variant's the answer is the scrutinee's
/// own label: a binder is as trusted as the value it was taken out of.
///
/// A route variant's binders are **route parameters**, and §14G.7.3 names
/// them the language's first untrusted-input source: `address` is written
/// by the browser, so a parameter destructured off it is a value a visitor
/// chose. §18.1 semantics 5 exempts a parameter carrying an `in` clause,
/// because the compiler renders one document per enumerated value and
/// reaching that document proves the value is one the build host wrote.
///
/// Note for the next integrator: the spec's §21.7.6 (2026-08-03) *deletes*
/// semantics 5 and rules that **every** route parameter is untrusted, `in`
/// clause or not. `feature/routing2` implements semantics 5 and has tests
/// asserting it, so its behaviour is carried across this merge unchanged
/// rather than adjudicated here. Dropping the `enumerated_in` arm below is
/// the whole of the change when that lands.
fn route_parameters<'a>(hir: &'a Hir, pattern: &str) -> Option<&'a [RouteParam]> {
    let (def, table) = hir.routes.as_ref()?;
    let DefKind::Choice(choice) = &hir.defs[*def].kind else {
        return None;
    };
    let index = choice
        .variants
        .iter()
        .position(|variant| variant.name == pattern)?;
    table
        .variants
        .get(index)
        .map(|variant| variant.params.as_slice())
}

struct Pass<'a> {
    hir: &'a Hir,
    placements: &'a dyn Placements,
    /// The join of every argument every call site passes, per parameter.
    params: HashMap<LocalId, Label>,
    /// The join of every `give` in a function's body.
    results: HashMap<DefId, Label>,
    /// The label of every `from` signal's initialiser.
    ///
    /// §18.1 semantics 2 declares integrity on state and derives it on
    /// values, and §4.5 already makes that split lexical: `starting`
    /// declares a source a program writes, `from` declares a value the
    /// compiler recomputes. A derived signal has no writers, so there is
    /// nothing to oblige and nothing to declare — its label is whatever it
    /// was computed from.
    derived: HashMap<DefId, Label>,
    /// Loop variables, arm binders, and component-local state.
    locals: HashMap<LocalId, Label>,
    /// Handler payload binders, and the event that raised them.
    payloads: HashMap<LocalId, String>,
    errors: Vec<TypeError>,
    /// Whether this sweep writes diagnostics. The fixpoint rounds do not,
    /// or a program would collect one copy of every error per round.
    reporting: bool,
    context: ReadContext,
}

impl<'a> Pass<'a> {
    /// Label a `when` arm's binders, per [`route_parameters`].
    fn bind_arm(&mut self, pattern: &str, bindings: &[LocalId], scrutinee: &Label, span: Span) {
        let params: Vec<Option<Span>> = match route_parameters(self.hir, pattern) {
            Some(params) => params
                .iter()
                .map(|param| param.enumerated_in.is_none().then_some(param.span))
                .collect(),
            None => Vec::new(),
        };
        for (at, binding) in bindings.iter().enumerate() {
            let label = match params.get(at).copied().flatten() {
                Some(_) => Label::untrusted(
                    "a browser chose it: it is a route parameter with no `in`, so it is whatever \
                     the visitor typed into the address bar",
                    span,
                ),
                None => scrutinee.clone(),
            };
            self.locals.insert(*binding, label);
        }
    }

    /// One walk of the whole program.
    fn sweep(&mut self) {
        self.errors.clear();
        let ids: Vec<DefId> = self.hir.defs.iter().map(|(id, _)| id).collect();
        for id in ids {
            self.context = context_of(self.placements, id);
            self.locals.clear();
            match &self.hir.defs[id].kind {
                DefKind::Signal(signal) => {
                    self.declaration(id);
                    let (init, is_source) = (signal.init, signal.is_source);
                    let label = self.expr(init);
                    if !is_source {
                        let previous = self
                            .derived
                            .get(&id)
                            .cloned()
                            .unwrap_or_else(Label::trusted);
                        self.derived.insert(id, previous.join(label));
                    }
                }
                DefKind::Function(function) => {
                    let body = function.body;
                    let result = self.block(body, Label::trusted());
                    let previous = self
                        .results
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(Label::trusted);
                    self.results.insert(id, previous.join(result));
                }
                DefKind::View(view) => {
                    let nodes = view.nodes.clone();
                    self.nodes(&nodes);
                }
                // A component declaration is written out at each call site
                // before this pass runs, so nothing reaches its body here.
                // A `foreign` has no body at all: it names a module and a
                // symbol, and §17.4.1 keeps the library free of state, so
                // there is no provenance in it to derive.
                DefKind::Component(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Foreign(_) => {}
            }
        }
    }

    /// E-INT-01: `trusted` on a placement that cannot carry it.
    fn declaration(&mut self, id: DefId) {
        let DefKind::Signal(signal) = &self.hir.defs[id].kind else {
            return;
        };
        if !signal.trusted {
            return;
        }
        if !signal.is_source {
            let name = self.hir.defs[id].name.clone();
            let span = self.hir.defs[id].span;
            self.report(
                format!(
                    "`{name}` is declared `trusted` and `from`, so nothing writes it — the \
                     compiler recomputes it. `trusted` is an obligation on writes, and there are \
                     none here (E-INT-01)."
                ),
                span,
                Some(
                    "A derived signal is exactly as trusted as what it is derived from, which \
                     the compiler already knows."
                        .to_string(),
                ),
            );
            return;
        }
        // §18.1 semantics 9. `static` is evaluated on the build host with
        // no browser attached and `client` is owned by the browser
        // outright, so the word is redundant on one and meaningless on the
        // other. Neither needs a rule of its own; both fall out of what the
        // placements are.
        let name = self.hir.defs[id].name.clone();
        let span = self.hir.defs[id].span;
        match signal.placement {
            zdc_ast::Placement::Static => self.report(
                format!(
                    "`{name}` is `static`, and `static` state is already trusted: it is computed \
                     on the build host, where no browser has any part in it. Remove `trusted` \
                     (E-INT-01)."
                ),
                span,
                None,
            ),
            zdc_ast::Placement::Client => self.report(
                format!(
                    "`{name}` is `trusted client`, and a browser owns its own memory. There is \
                     no such thing as protecting a browser from itself, so `client` state cannot \
                     be trusted (E-INT-01)."
                ),
                span,
                Some(
                    "Declare it `server` or `durable` if the point is that no browser may choose \
                     what goes in it."
                        .to_string(),
                ),
            ),
            zdc_ast::Placement::Server | zdc_ast::Placement::Durable => {}
        }
    }

    // --- expressions ---

    fn expr(&mut self, id: ExprId) -> Label {
        let span = self.hir.exprs[id].span;
        match &self.hir.exprs[id].kind {
            HirExprKind::Number(_) | HirExprKind::Text(_) | HirExprKind::Truth(_) => {
                Label::trusted()
            }
            HirExprKind::Empty => Label::trusted(),
            // §18.1 semantics 9: the operator set it and the browser had no
            // part in it.
            HirExprKind::Environment(_) => Label::trusted(),
            // `address` is the URL bar, and §14G.7.3 names it the
            // language's first untrusted-input source: the visitor typed
            // it. A program never reads it except to initialise the signal
            // `when` dispatches on, and `bind_arm` is where its parts are
            // classified one parameter at a time — so this label is what
            // an *unmatched* read of the whole address carries.
            HirExprKind::Address => Label::untrusted("it is the address a visitor asked for", span),
            HirExprKind::List(items) => {
                let items = items.clone();
                items
                    .into_iter()
                    .fold(Label::trusted(), |acc, item| acc.join(self.expr(item)))
            }
            HirExprKind::Map(entries) => {
                let entries = entries.clone();
                entries.into_iter().fold(Label::trusted(), |acc, (k, v)| {
                    acc.join(self.expr(k)).join(self.expr(v))
                })
            }
            HirExprKind::Ref(res) => self.reference(*res, span),
            HirExprKind::Call { callee, args } => {
                let callee = *callee;
                let args = args.clone();
                let mut label = Label::trusted();
                for (position, arg) in args.iter().enumerate() {
                    let value = match arg {
                        HirArg::Positional(expr) => *expr,
                        HirArg::Named { value, .. } => *value,
                    };
                    let found = self.expr(value);
                    self.pass_argument(callee, position, found.clone());
                    label = label.join(found);
                }
                match callee {
                    Res::Def(def) => label.join(
                        self.results
                            .get(&def)
                            .cloned()
                            .unwrap_or_else(Label::trusted),
                    ),
                    Res::Local(_)
                    | Res::Builtin(_)
                    | Res::Variant { .. }
                    | Res::BuiltinVariant(_) => label,
                }
            }
            // `length of items` — a call in `of` form. The result is as
            // trusted as the operand and as the function's own `give`.
            HirExprKind::OfCall { callee, operand } => {
                let (callee, operand) = (*callee, *operand);
                let found = self.expr(operand);
                self.pass_argument(callee, 0, found.clone());
                match callee {
                    Res::Def(def) => found.join(
                        self.results
                            .get(&def)
                            .cloned()
                            .unwrap_or_else(Label::trusted),
                    ),
                    Res::Local(_)
                    | Res::Builtin(_)
                    | Res::Variant { .. }
                    | Res::BuiltinVariant(_) => found,
                }
            }
            // A built-in operator is a pure function of its operand, so it
            // carries the operand's provenance and adds none.
            HirExprKind::Operator { operand, .. } => {
                let operand = *operand;
                self.expr(operand)
            }
            HirExprKind::Unary { operand, .. } => {
                let operand = *operand;
                self.expr(operand)
            }
            HirExprKind::Binary { lhs, rhs, .. } => {
                let (lhs, rhs) = (*lhs, *rhs);
                self.expr(lhs).join(self.expr(rhs))
            }
            HirExprKind::Field { base, .. } => {
                let base = *base;
                self.expr(base)
            }
            HirExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                self.expr(base).join(self.expr(index))
            }
            // The longer list holds everything the shorter one held and
            // the item as well, so it carries the provenance of both. A
            // rule that took only the list's label would be a laundry:
            // `append attackerText to trusted` would come out trusted.
            HirExprKind::Append { item, list } => {
                let (item, list) = (*item, *list);
                self.expr(item).join(self.expr(list))
            }
        }
    }

    /// Record what a call site passes, so the callee's body is checked
    /// against every argument any site gives it.
    fn pass_argument(&mut self, callee: Res, position: usize, label: Label) {
        let Res::Def(def) = callee else {
            return;
        };
        let DefKind::Function(function) = &self.hir.defs[def].kind else {
            return;
        };
        let Some(param) = function.params.get(position).copied() else {
            return;
        };
        let previous = self
            .params
            .get(&param)
            .cloned()
            .unwrap_or_else(Label::trusted);
        self.params.insert(param, previous.join(label));
    }

    fn reference(&mut self, res: Res, span: Span) -> Label {
        match res {
            Res::Builtin(_) | Res::Variant { .. } | Res::BuiltinVariant(_) => Label::trusted(),
            Res::Local(local) => {
                if let Some(event) = self.payloads.get(&local) {
                    let name = self.hir.locals[local].name.clone();
                    return Label::untrusted(
                        format!("`{name}` is what the browser sent with `on {event}`"),
                        span,
                    );
                }
                self.locals
                    .get(&local)
                    .or_else(|| self.params.get(&local))
                    .cloned()
                    .unwrap_or_else(Label::trusted)
            }
            Res::Def(def) => self.signal_read(def, span),
        }
    }

    /// What reading a signal is worth.
    ///
    /// §18.1 semantics 7: integrity obligations exist only in server
    /// regions, so a client-context read is Trusted and nothing about it is
    /// checked. In a server region two rules apply, and both are §18.1's:
    /// reading a `client` signal *is* `Crossing::Lift`, and reading stored
    /// state that was never declared `trusted` yields a value a browser may
    /// have put there.
    fn signal_read(&mut self, def: DefId, span: Span) -> Label {
        let DefKind::Signal(signal) = &self.hir.defs[def].kind else {
            return Label::trusted();
        };
        let (placement, trusted, is_source) = (signal.placement, signal.trusted, signal.is_source);
        let name = self.hir.defs[def].name.clone();
        match self.context {
            ReadContext::Client | ReadContext::Static => Label::trusted(),
            ReadContext::ViewRootedServer | ReadContext::TriggerRootedServer => match placement {
                zdc_ast::Placement::Client => Label::untrusted(
                    format!("`{name}` is `client` state, and the browser chose it"),
                    span,
                ),
                // `static` is evaluated on the build host and inlined,
                // so its value is one the operator chose at build time and
                // no browser had a hand in it. This is the same reasoning
                // that makes an `in`-clause route parameter trusted, and it
                // is stated once here rather than twice.
                zdc_ast::Placement::Static => Label::trusted(),
                zdc_ast::Placement::Server | zdc_ast::Placement::Durable => {
                    if !is_source {
                        return self
                            .derived
                            .get(&def)
                            .cloned()
                            .unwrap_or_else(Label::trusted);
                    }
                    if trusted {
                        Label::trusted()
                    } else {
                        Label::untrusted(
                            format!(
                                "`{name}` is not declared `trusted`, so a browser may have \
                                 written what is in it"
                            ),
                            span,
                        )
                    }
                }
            },
        }
    }

    // --- statements ---

    /// Walk a block under a program counter label and return the join of
    /// everything it `give`s.
    fn block(&mut self, id: BlockId, pc: Label) -> Label {
        let stmts = self.hir.blocks[id].stmts.clone();
        let mut given = Label::trusted();
        for stmt in &stmts {
            match stmt {
                HirStmt::Pipeline(clause) => match clause {
                    HirPipeline::From(expr) | HirPipeline::TakeFirst(expr) => {
                        given = given.join(self.expr(*expr));
                    }
                    HirPipeline::Keep { var, cond } => {
                        let label = self.expr(*cond);
                        self.locals.insert(*var, label.clone());
                        given = given.join(label);
                    }
                    HirPipeline::Sort { var, key } => {
                        let label = self.expr(*key);
                        self.locals.insert(*var, label.clone());
                        given = given.join(label);
                    }
                    HirPipeline::MapEach { var, to } => {
                        let label = self.expr(*to);
                        self.locals.insert(*var, label.clone());
                        given = given.join(label);
                    }
                },
                HirStmt::Mutation(mutation) => self.mutation(mutation, pc.clone()),
                // A binding is a name for a value, so the name carries the
                // value's provenance. It gives nothing, so `given` is
                // untouched.
                HirStmt::Bind(bind) => {
                    for binding in &bind.bindings {
                        let label = self.expr(binding.value);
                        self.locals.insert(binding.local, label);
                    }
                }
                HirStmt::Give(expr) => given = given.join(self.expr(*expr)).join(pc.clone()),
                HirStmt::When(when) => {
                    let scrutinee = self.expr(when.scrutinee);
                    let inner = pc.clone().join(scrutinee.clone());
                    for arm in &when.arms {
                        self.bind_arm(&arm.pattern_name, &arm.bindings, &scrutinee, arm.span);
                        match &arm.body {
                            HirArmBody::Show(expr) => {
                                given = given.join(self.expr(*expr)).join(inner.clone());
                            }
                            HirArmBody::Block(block) => {
                                given = given.join(self.block(*block, inner.clone()));
                            }
                        }
                    }
                }
                HirStmt::Each(each) => {
                    let items = self.expr(each.iter);
                    self.locals.insert(each.var, items.clone());
                    given = given.join(self.block(each.body, pc.clone().join(items)));
                }
                HirStmt::If(conditional) => {
                    let cond = self.expr(conditional.cond);
                    let inner = pc.clone().join(cond);
                    given = given.join(self.block(conditional.then, inner.clone()));
                    if let Some(otherwise) = conditional.otherwise {
                        given = given.join(self.block(otherwise, inner));
                    }
                }
            }
        }
        given
    }

    /// The three obligations that fire at a write.
    fn mutation(&mut self, mutation: &HirMutation, pc: Label) {
        let place = mutation.place().clone();
        let value = self.expr(mutation.value());
        let mut indexes: Vec<(Label, Span)> = Vec::new();
        for segment in &place.path {
            if let HirPathSeg::Index(expr) = segment {
                let span = self.hir.exprs[*expr].span;
                let label = self.expr(*expr);
                indexes.push((label, span));
            }
        }

        let Some((target, name, trusted, placement)) = self.written_signal(&place) else {
            return;
        };
        let _ = target;

        // §18.1 semantics 4 — a client-rooted write to `server` or
        // `durable` state is a command, and every one of its arguments
        // travels on the wire under the browser's control. The right-hand
        // side is untrusted whatever it says, and folding a literal does
        // not change that: the endpoint name is `signal.op.segments` and
        // nothing else, so a browser can post it with any value it likes.
        let command = self.context == ReadContext::Client
            && matches!(
                placement,
                zdc_ast::Placement::Server | zdc_ast::Placement::Durable
            );
        let over_the_wire = |label: Label, span: Span| -> Label {
            if !command || label.is_untrusted() {
                return label;
            }
            Label::untrusted(
                "a browser sends this write and may send any value with it",
                span,
            )
        };

        let value = over_the_wire(value, self.hir.exprs[mutation.value()].span);
        let indexes: Vec<(Label, Span)> = indexes
            .into_iter()
            .map(|(label, span)| (over_the_wire(label, span), span))
            .collect();

        if !trusted {
            return;
        }

        // A1 / E-INT-02 — the index is what IDOR actually is.
        for (label, span) in &indexes {
            if label.is_untrusted() {
                self.report(
                    format!(
                        "`{name}` is `trusted`, so a browser must not choose which entry of it is \
                         written. This index is untrusted because {} (E-INT-02).",
                        label.reason()
                    ),
                    *span,
                    Some(
                        "`durable per visitor` state has no nameable key, which is the repair \
                         when the entry is meant to be this visitor's own."
                            .to_string(),
                    ),
                );
            }
        }

        // A3 / E-INT-03 — the value written.
        if value.is_untrusted() {
            let span = self.hir.exprs[mutation.value()].span;
            self.report(
                format!(
                    "`{name}` is `trusted`, so no browser may choose what goes in it. This value \
                     is untrusted because {} (E-INT-03).",
                    value.reason()
                ),
                span,
                Some(
                    "A value becomes trusted only by passing through a declaration that says so \
                     — `gives trusted T` on a `foreign` (spec §18.1, semantics 6)."
                        .to_string(),
                ),
            );
        }

        // E-INT-04 — the implicit flow. §17.3.4's pc threading, unchanged.
        if pc.is_untrusted() {
            self.report(
                // The claim and nothing else: "why an implicit flow
                // counts" is one `zdc explain E-INT-04` away, and the
                // inline budget is measured rather than felt.
                format!(
                    "`{name}` is `trusted`, and whether this write happens was decided by an \
                     untrusted value, because {} (E-INT-04).",
                    pc.reason()
                ),
                place.span,
                None,
            );
        }
    }

    /// The signal a place writes through, if it is one this pass can name.
    fn written_signal(
        &self,
        place: &HirPlace,
    ) -> Option<(DefId, String, bool, zdc_ast::Placement)> {
        let Res::Def(def) = place.base else {
            return None;
        };
        let DefKind::Signal(signal) = &self.hir.defs[def].kind else {
            return None;
        };
        Some((
            def,
            self.hir.defs[def].name.clone(),
            signal.trusted,
            signal.placement,
        ))
    }

    // --- the view ---

    fn nodes(&mut self, nodes: &[HirNode]) {
        for node in nodes {
            match node {
                HirNode::Element(element) => {
                    for arg in &element.args {
                        match arg {
                            HirArg::Positional(expr) => {
                                self.expr(*expr);
                            }
                            HirArg::Named { value, .. } => {
                                self.expr(*value);
                            }
                        }
                    }
                    let children = element.children.clone();
                    self.nodes(&children);
                }
                HirNode::Handler(handler) => {
                    if let Some(payload) = handler.payload {
                        self.payloads.insert(payload, handler.event.clone());
                    }
                    self.block(handler.body, Label::trusted());
                }
                HirNode::Each(each) => {
                    let items = self.expr(each.iter);
                    self.locals.insert(each.var, items);
                    let body = each.body.clone();
                    self.nodes(&body);
                }
                HirNode::When(when) => {
                    let scrutinee = self.expr(when.scrutinee);
                    for arm in &when.arms {
                        self.bind_arm(&arm.pattern_name, &arm.bindings, &scrutinee, arm.span);
                        match &arm.body {
                            HirNodeArmBody::Show(element) => {
                                let shown = vec![HirNode::Element((**element).clone())];
                                self.nodes(&shown);
                            }
                            HirNodeArmBody::Nodes(nodes) => {
                                let nodes = nodes.clone();
                                self.nodes(&nodes);
                            }
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
                HirNode::Scope(scope) => {
                    for local in &scope.locals {
                        let label = self.expr(local.init);
                        self.locals.insert(local.local, label);
                    }
                    let body = scope.body.clone();
                    self.nodes(&body);
                }
                // Replaced by instantiation before this pass runs.
                HirNode::Children(_) => {}
            }
        }
    }

    fn report(&mut self, message: String, span: Span, help: Option<String>) {
        if !self.reporting {
            return;
        }
        let error = TypeError {
            message,
            span,
            help,
        };
        if !self.errors.contains(&error) {
            self.errors.push(error);
        }
    }
}
