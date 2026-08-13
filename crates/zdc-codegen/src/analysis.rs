//! The whole-program facts every emission decision reads.
//!
//! None of them needs types except the last. All the rest are reachability
//! queries over the `Res::Def` and `Res::Local` edges `zdc-resolve` already
//! produces, which is what lets the expensive machinery below be built and
//! tested before `zdc-types` exists (spec §16.3.5).
//!
//! 1. **Which expressions read a signal**, because that is the difference
//!    between baking a value into markup and allocating an effect.
//! 2. **Which binders are reactive**, because a binder whose binding site
//!    outlives its value must be read through the graph. Direct Emission's
//!    claim that a `Res::Local` is never reactive is what froze every list.
//! 3. **Which signals are ever written**, because a never-written signal
//!    needs no setter.
//! 4. **Which definitions one document reaches**, because a routed program
//!    ships one bundle per URL and `/blog`'s code is not in `/work`'s.
//!
//! What the client bundle *contains* is not a guess:
//! `TierSplit::client_members` is the answer for the program as a whole,
//! the walk that produced it stopped at each crossing, and that stop is
//! what makes §14A.1's exclusion provable (spec §17.2.1). The walk here
//! narrows that set to one document; it never widens it.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use zdc_hir::{
    BlockId, Builtin, Def, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirElement,
    HirExprKind, HirMutation, HirNode, HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, LocalId,
    Res,
};
use zdc_types::TypeTable;

use crate::pages::Bindings;

/// Whether reading this definition has to go through the reactive graph.
///
/// Every signal but one: a `static` signal is computed once on the build
/// host and inlined at each read (§14C.3b), so it has no cell, no getter
/// and nothing that could ever change. Treating it as reactive would emit
/// `title()` against a name no root declares.
fn is_reactive_signal(hir: &Hir, def: DefId) -> bool {
    match &hir.defs[def].kind {
        DefKind::Signal(signal) => signal.placement != zdc_ast::Placement::Static,
        // A component is a piece of view, not a value: nothing reads one,
        // so there is no read to route through the reactive graph. A
        // `foreign` is a call, emitted inline, and never a cell either.
        DefKind::Function(_)
        | DefKind::Release(_)
        | DefKind::View(_)
        | DefKind::Record(_)
        | DefKind::Choice(_)
        | DefKind::Component(_)
        | DefKind::Foreign(_) => false,
    }
}

pub struct Analysis {
    /// Binders the address fold replaced with a constant.
    ///
    /// A binder holding a compile-time constant is not reactive and is
    /// not a getter: `/blog/rust`'s `slug` *is* `"rust"` in that document,
    /// which is what §14G.2 revision 1 means by ordinary constant
    /// propagation over an immutable signal.
    folded: HashSet<LocalId>,
    /// Binders whose binding site outlives the value bound to it: `each`
    /// binders and `when` arm binders **in node position**. A function
    /// parameter and a statement-position binder are plain values, because
    /// their scope is one evaluation.
    reactive_locals: HashSet<LocalId>,
    /// Functions that transitively read a signal. Least fixed point over
    /// the call graph, which is exact: the language has no first-class
    /// functions, no dynamic dispatch, and no `eval`.
    reactive_functions: HashSet<DefId>,
    /// Signals with at least one write: a mutation, or a two-way input
    /// binding in the view.
    written: BTreeSet<DefId>,
    /// The same, for a component's own state, which is a local rather than
    /// a definition because it belongs to one instance (§14D.1).
    written_locals: BTreeSet<LocalId>,
    /// Every local that holds a component's state. These are getters like
    /// any other signal, so reading one is a call.
    local_signals: HashSet<LocalId>,
    /// Definitions this document's nodes reach.
    client_closure: BTreeSet<DefId>,
    /// Binders that belong to a `component` declaration rather than to an
    /// instance of it.
    ///
    /// Instantiation copies a component's body once per call site with
    /// fresh binders (§14D.3), so the declaration's own are never emitted.
    /// Naming them anyway would let a component nobody used take `count`
    /// and leave the instance that is emitted with `count$`.
    declaration_locals: HashSet<LocalId>,
    /// Which library functions each definition's own operators dispatched
    /// to, keyed by the definition whose body wrote them.
    ///
    /// Type-directed, so only the checker can answer *which* function: the
    /// HIR records `contains` as an operator and not as a call to
    /// `textContains`. The checker answers it per `ExprId` and an `ExprId`
    /// alone says nothing about who can reach it, so the owner is recorded
    /// here — that is the edge that turns "somewhere in this compilation
    /// unit" into "reachable from this bundle's roots".
    operator_targets: BTreeMap<DefId, BTreeSet<DefId>>,
}

/// The part of the analysis that does not depend on which page is being
/// emitted.
///
/// **Why this exists.** §17.2's split is reachability over the product of
/// the definition set and the root set, and routing puts one root per
/// page on that axis — so the pass is quadratic in definitions × pages by
/// construction. That is measured, accepted, and not worth optimising.
/// What is *not* acceptable is doing anything per page that is itself
/// superlinear in definitions, because that turns the product cubic and
/// starts to bite at realistic page counts.
///
/// The reactive-function fixpoint is exactly such a thing: it is a
/// worklist over the whole call graph, quadratic in functions in the
/// worst case. It is also the same answer for every page — a function is
/// reactive because of what its own body reads, and a function body
/// cannot name a view binder — so it is computed once here and shared.
/// The component-declaration binders, the writes inside function bodies
/// and the checker's operator dispatch are page-independent for the same
/// reason and are hoisted with it.
pub struct Shared {
    reactive_functions: HashSet<DefId>,
    declaration_locals: HashSet<LocalId>,
    written: BTreeSet<DefId>,
    written_locals: BTreeSet<LocalId>,
    /// Which library functions each definition's own operators dispatched
    /// to, keyed by the definition whose body wrote them — the same shape
    /// `Analysis` carries, because the walk that fills it is a property of
    /// the compilation unit rather than of any one page.
    operator_targets: BTreeMap<DefId, BTreeSet<DefId>>,
}

impl Shared {
    /// `types` supplies one edge the HIR does not carry: which library
    /// function each `contains` dispatched to (§17.4.3). The closure walk
    /// needs it, because a bundle that reaches `textContains` through an
    /// operator must still carry `textContains` — §17.4.5's prelude
    /// closure, folded into the walk that was already here rather than run
    /// as a phase of its own.
    pub fn new(hir: &Hir, types: &TypeTable) -> Shared {
        let mut shared = Shared {
            reactive_functions: HashSet::new(),
            declaration_locals: HashSet::new(),
            written: BTreeSet::new(),
            written_locals: BTreeSet::new(),
            operator_targets: BTreeMap::new(),
        };
        // One walk of every body, attributing each dispatched operator to
        // the definition that wrote it. Linear in the number of
        // expressions in the program and independent of the number of
        // roots, which is why it is hoisted here rather than repeated per
        // page: it keeps §17.4.5's closure out of the definitions × roots
        // term `split` already pays.
        for (id, _) in hir.defs.iter() {
            for expr in zdc_graph::exprs_of(hir, id) {
                if let Some(target) = types.operator_target(expr) {
                    shared
                        .operator_targets
                        .entry(id)
                        .or_default()
                        .insert(target);
                }
            }
        }
        for (_, def) in hir.defs.iter() {
            match &def.kind {
                DefKind::Component(component) => {
                    let out = &mut shared.declaration_locals;
                    out.extend(component.params.iter().copied());
                    out.extend(component.children);
                    out.extend(component.states.iter().map(|state| state.local));
                    node_binders(&component.body, out);
                    local_signals(&component.body, out);
                    for node in &component.body {
                        declaration_block_binders(hir, node, out);
                    }
                }
                // Only a view and a component carry nodes, and binders are
                // a property of nodes. Spelled out so a new `DefKind` is a
                // compile error rather than a silently unwalked body.
                DefKind::View(_)
                | DefKind::Signal(_)
                | DefKind::Function(_)
                | DefKind::Release(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Foreign(_) => {}
            }
        }

        // A scratch analysis whose only job is the two whole-program
        // walks. Its `reactive_locals` is empty, which is the right
        // answer for a function body: a function's binders are its
        // parameters and its pipeline binders, and neither is a node
        // binder, so no page can change this verdict.
        let mut scratch = Analysis::empty();
        scratch.solve_reactive_functions(hir);
        for (_, def) in hir.defs.iter() {
            match &def.kind {
                DefKind::Function(function) => scratch.written_in_block(hir, function.body),
                DefKind::Release(release) => scratch.written_in_block(hir, release.body),
                // A component declaration emits nothing; its instances are
                // already in the view. A `foreign` has no body to walk.
                DefKind::View(_)
                | DefKind::Signal(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_)
                | DefKind::Foreign(_) => {}
            }
        }
        shared.reactive_functions = scratch.reactive_functions;
        shared.written = scratch.written;
        shared.written_locals = scratch.written_locals;
        shared
    }
}

impl Analysis {
    fn empty() -> Analysis {
        Analysis {
            folded: HashSet::new(),
            reactive_locals: HashSet::new(),
            reactive_functions: HashSet::new(),
            written: BTreeSet::new(),
            written_locals: BTreeSet::new(),
            local_signals: HashSet::new(),
            client_closure: BTreeSet::new(),
            declaration_locals: HashSet::new(),
            operator_targets: BTreeMap::new(),
        }
    }

    /// The whole program, rooted at the `view`.
    pub fn new(hir: &Hir, types: &TypeTable) -> Analysis {
        let shared = Shared::new(hir, types);
        Analysis::whole(hir, &shared)
    }

    /// The whole program against a [`Shared`] already computed.
    pub fn whole(hir: &Hir, shared: &Shared) -> Analysis {
        let nodes = hir
            .view
            .and_then(|id| match &hir.defs[id].kind {
                DefKind::View(view) => Some(view.nodes.clone()),
                DefKind::Signal(_)
                | DefKind::Function(_)
                | DefKind::Release(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_)
                | DefKind::Foreign(_) => None,
            })
            .unwrap_or_default();
        Analysis::rooted(hir, &nodes, &Bindings::default(), true, shared)
    }

    /// One page of a routed program, rooted at the nodes that page renders
    /// after the address fold (spec §14G.2 revision 1, §17.2.6).
    ///
    /// The seed set is the page's own nodes and nothing else. §16.3.12
    /// seeded every `client` signal as well, which was right when a
    /// program had one root: with one root, unreachable meant unused. With
    /// one root per page it would mean "in every bundle", and the whole
    /// point of splitting is that `/blog`'s code is not in `/work`'s.
    pub fn page(hir: &Hir, nodes: &[HirNode], bindings: &Bindings, shared: &Shared) -> Analysis {
        Analysis::rooted(hir, nodes, bindings, false, shared)
    }

    /// Per-page work is linear in the *page's* nodes plus one pass over
    /// the definition set. Nothing here re-runs a fixpoint, so the split
    /// stays quadratic in definitions × pages rather than becoming cubic.
    fn rooted(
        hir: &Hir,
        roots: &[HirNode],
        bindings: &Bindings,
        seed_signals: bool,
        shared: &Shared,
    ) -> Analysis {
        let mut analysis = Analysis {
            folded: bindings.locals().collect(),
            reactive_functions: shared.reactive_functions.clone(),
            written: shared.written.clone(),
            written_locals: shared.written_locals.clone(),
            declaration_locals: shared.declaration_locals.clone(),
            operator_targets: shared.operator_targets.clone(),
            ..Analysis::empty()
        };
        node_binders(roots, &mut analysis.reactive_locals);
        local_signals(roots, &mut analysis.local_signals);
        // A component's state is a signal, so reading it is a call, exactly
        // as reading a top-level one is.
        analysis
            .reactive_locals
            .extend(analysis.local_signals.iter().copied());
        // A binder the fold replaced is a constant, whatever its binding
        // site would otherwise have made it.
        for local in bindings.locals() {
            analysis.reactive_locals.remove(&local);
        }
        // Only the page's own nodes: the writes inside function bodies
        // are the same for every page and came in with `shared`.
        analysis.written_in_nodes(hir, roots);
        analysis.walk_client_closure(hir, roots, seed_signals);
        analysis
    }

    /// Spec §16.3.3's `reads_signal`. Over-approximating is safe;
    /// under-approximating breaks reactivity, so an unknown is reactive.
    pub fn reads_signal(&self, hir: &Hir, id: ExprId) -> bool {
        match &hir.exprs[id].kind {
            HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty => false,
            // The URL a document was served at is a constant of that
            // document: the build wrote one file per URL, so nothing can
            // move it after the fold.
            HirExprKind::Address => false,
            // **Reactive, and this is the arm that makes the feature
            // work.** The browser's answer changes when the visitor
            // changes their system theme, resizes the window, or turns
            // animation off, so a view that mentions one must re-render
            // when it does. Reporting `false` here would compile to a
            // read taken once at mount — which is exactly the bug the
            // survey of the target site found in six of its eight
            // `matchMedia` call sites.
            HirExprKind::Media(_) => true,
            // Reactive for the same reason: the browser writes it while the
            // page is open, so a binding that reads it has to be a binding
            // and not a value folded once at mount.
            HirExprKind::Scroll => true,
            HirExprKind::List(items) => items.iter().any(|item| self.reads_signal(hir, *item)),
            HirExprKind::Map(entries) => entries
                .iter()
                .any(|(key, value)| self.reads_signal(hir, *key) || self.reads_signal(hir, *value)),
            // `environment` is server-only state; a client walk cannot
            // reach one, but reporting it reactive is the safe direction.
            HirExprKind::Environment(_) => true,
            // A request is reactive exactly when one of its arguments is:
            // it re-runs when they change, and does not when they do not.
            HirExprKind::Outbound { args, .. } => {
                args.iter().any(|arg| self.reads_signal(hir, arg_expr(arg)))
            }
            // A capability is answered once, at build time, so it is never
            // reactive itself; whatever it was asked for still can be.
            HirExprKind::Build { argument, .. } => self.reads_signal(hir, *argument),
            HirExprKind::Ref(res) => self.res_is_reactive(hir, *res),
            HirExprKind::Call { callee, args } => {
                self.res_is_reactive(hir, *callee)
                    || args.iter().any(|arg| self.reads_signal(hir, arg_expr(arg)))
            }
            HirExprKind::OfCall { callee, operand } => {
                self.res_is_reactive(hir, *callee) || self.reads_signal(hir, *operand)
            }
            // A built-in operator is a pure function of its operand.
            HirExprKind::Operator { operand, .. } => self.reads_signal(hir, *operand),
            HirExprKind::Unary { operand, .. } => self.reads_signal(hir, *operand),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.reads_signal(hir, *lhs) || self.reads_signal(hir, *rhs)
            }
            HirExprKind::Field { base, .. } => self.reads_signal(hir, *base),
            HirExprKind::Index { base, index } => {
                self.reads_signal(hir, *base) || self.reads_signal(hir, *index)
            }
            HirExprKind::Append { item, list } => {
                self.reads_signal(hir, *item) || self.reads_signal(hir, *list)
            }
            HirExprKind::Insert { key, value, table } => {
                self.reads_signal(hir, *key)
                    || self.reads_signal(hir, *value)
                    || self.reads_signal(hir, *table)
            }
            HirExprKind::MapInside { source, to, .. } => {
                self.reads_signal(hir, *source) || self.reads_signal(hir, *to)
            }
        }
    }

    /// Whether this expression *is* a getter already, so that passing it in
    /// getter position needs no closure.
    ///
    /// Never `() => X()`. A signal read and a `derived` are the getter, and
    /// double-wrapping is what made every `when` throw at mount.
    pub fn bare_getter(&self, hir: &Hir, id: ExprId) -> Option<Res> {
        match &hir.exprs[id].kind {
            HirExprKind::Ref(res @ Res::Def(def)) => is_reactive_signal(hir, *def).then_some(*res),
            HirExprKind::Ref(res @ Res::Local(local)) => (self.reactive_locals.contains(local)
                && !self.folded.contains(local))
            .then_some(*res),
            // A variant tag and a built-in are constants of the program,
            // so neither is a cell anything could read through.
            HirExprKind::Ref(Res::Builtin(_) | Res::Variant { .. } | Res::BuiltinVariant(_)) => {
                None
            }
            HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            | HirExprKind::Address
            | HirExprKind::Environment(_)
            // A media query *is* a cell, but not one named by a `Res`,
            // which is what this function reports. `Emitter::value`
            // hoists it and hands back the getter call, so the caller
            // wraps it in a closure exactly as it does an expression.
            | HirExprKind::Media(_)
            | HirExprKind::Scroll
            // A capability is answered once, while the build runs, so
            // what it gave is a constant of the bundle and not a cell.
            | HirExprKind::Build { .. }
            // A request *is* a cell, but not one this can hand back: the
            // getter it produces is bound by the emitter under the
            // declaration's own name, and there is no `Res` naming this
            // expression. The declaration's name is what a reader reaches
            // it through, and that is an ordinary `Ref`.
            | HirExprKind::Outbound { .. }
            | HirExprKind::List(_)
            | HirExprKind::Map(_)
            | HirExprKind::Call { .. }
            | HirExprKind::OfCall { .. }
            | HirExprKind::Operator { .. }
            | HirExprKind::Unary { .. }
            | HirExprKind::Binary { .. }
            | HirExprKind::Field { .. }
            | HirExprKind::Index { .. }
            | HirExprKind::Append { .. }
            | HirExprKind::Insert { .. }
            // The container that comes out is built here, so it is a
            // value rather than a cell anything reads through.
            | HirExprKind::MapInside { .. } => None,
        }
    }

    pub fn is_reactive_local(&self, id: LocalId) -> bool {
        self.reactive_locals.contains(&id)
    }

    /// §17.4.5's prelude closure, for one root: every definition this
    /// root's members reach through a type-directed operator, and
    /// everything those reach in turn.
    ///
    /// Which library function `contains` means is the checker's verdict,
    /// and the split runs *before* the checker (§17.1.1) — so the split's
    /// walk cannot carry this edge, and `members` alone names a bundle
    /// that calls `listContains` without ever emitting it. The closure is
    /// completed here instead, which keeps the dependency arrow pointing
    /// the way §17.1.1 proves it runs. It is sound to defer because of the
    /// Phase-0 invariant (§17.4.1): no prelude definition references a
    /// signal, so nothing added here can move a definition between
    /// bundles, introduce a `Remote`, or change any placement fact.
    ///
    /// **The seed is `members`, not the program.** `operator_targets` is
    /// keyed by the definition that wrote the operator, so a `contains`
    /// inside a library function seeds a bundle only when that library
    /// function is itself reachable from this bundle's roots. Seeding from
    /// every operator target in the compilation unit instead is what put
    /// `textContains` and `$split` into `hello.zd`, a program that names
    /// no library function at all — §14A.1 says a bundle provably excludes
    /// what it cannot reach, and "the prelude mentions it somewhere" is
    /// not reachability.
    ///
    /// **Per root, not per unit.** Routing emits one bundle per page, so
    /// the answer has to be a function of the root's member set; there is
    /// no compilation-unit-wide answer that stays correct once two pages
    /// share a prelude.
    ///
    /// Cost is `O(|members| + |added| · sites)` — one pass over the seeds
    /// and a visited-set closure over what they reach. Nothing here scans
    /// the other roots, so the pass is linear in the number of roots.
    pub fn operator_closure(&self, hir: &Hir, members: &BTreeSet<DefId>) -> BTreeSet<DefId> {
        let mut extra: BTreeSet<DefId> = BTreeSet::new();
        let mut seen: BTreeSet<DefId> = members.clone();
        let mut frontier: Vec<DefId> = members
            .iter()
            .filter_map(|def| self.operator_targets.get(def))
            .flatten()
            .copied()
            .collect();
        while let Some(id) = frontier.pop() {
            if !seen.insert(id) {
                continue;
            }
            extra.insert(id);
            // A library function reached by dispatch may dispatch in turn:
            // `indexOf` writes `value contains needle`, and reaching
            // `indexOf` is what makes `textContains` reachable.
            if let Some(targets) = self.operator_targets.get(&id) {
                frontier.extend(targets.iter().copied());
            }
            // The same call edges the split walks, from the same walker, so
            // the two cannot disagree about what a body reaches.
            for site in zdc_graph::sites_of(hir, id) {
                if let zdc_graph::Site::Call { callee, .. } = site {
                    frontier.push(callee);
                }
            }
        }
        extra
    }

    pub fn written(&self) -> &BTreeSet<DefId> {
        &self.written
    }

    pub fn written_locals(&self) -> &BTreeSet<LocalId> {
        &self.written_locals
    }

    /// Whether a local holds a component's own state.
    pub fn is_local_signal(&self, id: LocalId) -> bool {
        self.local_signals.contains(&id)
    }

    /// Whether a binder belongs to a `component` declaration, and so is
    /// never emitted.
    pub fn is_declaration_local(&self, id: LocalId) -> bool {
        self.declaration_locals.contains(&id)
    }

    /// The definitions this document's nodes reach.
    ///
    /// A *narrowing* of the split's client members, never a widening: the
    /// caller intersects the two, so nothing the split stopped at can come
    /// back through here.
    pub fn client_closure(&self) -> &BTreeSet<DefId> {
        &self.client_closure
    }

    fn res_is_reactive(&self, hir: &Hir, res: Res) -> bool {
        match res {
            Res::Def(def) => match hir.defs[def].kind {
                DefKind::Signal(_) => is_reactive_signal(hir, def),
                // A release is a function, and it is reactive for exactly
                // the same reason one is: whether it reads a signal.
                // REL-CLOSED says it must read none, which makes the answer
                // `false` in every program that passes — but the answer is
                // computed rather than assumed, because this pass runs on
                // programs the release rules have already rejected.
                DefKind::Function(_) | DefKind::Release(_) => {
                    self.reactive_functions.contains(&def)
                }
                // A record names a shape and a view names a root; neither
                // is a value that can change. A `foreign` cannot reach a
                // signal at all: the prelude's placement invariant
                // (§17.4.1) is that no library definition mentions one.
                DefKind::View(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_)
                | DefKind::Foreign(_) => false,
            },
            Res::Local(local) => {
                self.reactive_locals.contains(&local) && !self.folded.contains(&local)
            }
            // A variant tag is a constant of the program.
            Res::Variant { .. } | Res::BuiltinVariant(_) | Res::Builtin(_) => false,
        }
    }

    /// Spec §16.3.12's client walk, narrowed to one document: transitive
    /// closure over `Res::Def` edges from this document's nodes.
    ///
    /// **A program with no `view` seeds differently.** §16.3.1 ships
    /// nothing a bundle does not use, and for an application the use is
    /// the page. A module with no `view` has no page, and its use is the
    /// importing file's `for` list — which is outside this compilation
    /// unit and so cannot narrow the walk. Every top-level function is
    /// therefore a seed, because §14D.2 makes every one of them
    /// importable; pruning to the empty set would emit a module whose
    /// whole reason for existing had been optimised away.
    fn walk_client_closure(&mut self, hir: &Hir, roots: &[HirNode], seed_signals: bool) {
        let is_module = hir.view.is_none();
        let mut queue: Vec<DefId> = Vec::new();
        node_references(hir, roots, &mut queue);
        if seed_signals || is_module {
            for (id, def) in hir.defs.iter() {
                match &def.kind {
                    DefKind::Signal(signal)
                        if seed_signals && signal.placement == zdc_ast::Placement::Client =>
                    {
                        queue.push(id);
                    }
                    DefKind::Function(_) if is_module => queue.push(id),
                    // A release is emitted server-side, so it is never a
                    // seed for a *client* closure — not even in a module,
                    // where every importable function is one.
                    DefKind::Release(_)
                    | DefKind::View(_)
                    | DefKind::Signal(_)
                    | DefKind::Function(_)
                    | DefKind::Record(_)
                    | DefKind::Choice(_)
                    | DefKind::Component(_)
                    | DefKind::Foreign(_) => {}
                }
            }
        }
        while let Some(id) = queue.pop() {
            if !self.client_closure.insert(id) {
                continue;
            }
            let mut referenced = Vec::new();
            references_of(hir, &hir.defs[id], &mut referenced);
            queue.extend(referenced);
            // The checker's operator dispatch is an edge the HIR does not
            // carry, so it is followed here for the same reason
            // `operator_closure` exists. Followed *from the definition
            // that wrote the operator*, not seeded from every target in
            // the compilation unit: seeding unit-wide is what put
            // `textContains` into `hello.zd`, a program that names no
            // library function at all.
            if let Some(targets) = self.operator_targets.get(&id) {
                queue.extend(targets.iter().copied());
            }
        }
    }

    fn written_in_nodes(&mut self, hir: &Hir, nodes: &[HirNode]) {
        for node in nodes {
            match node {
                HirNode::Element(element) => self.written_in_element(hir, element),
                HirNode::Each(each) => self.written_in_nodes(hir, &each.body),
                HirNode::When(when) => {
                    for arm in &when.arms {
                        match &arm.body {
                            HirNodeArmBody::Show(element) => self.written_in_element(hir, element),
                            HirNodeArmBody::Nodes(nodes) => self.written_in_nodes(hir, nodes),
                        }
                    }
                }
                HirNode::If(conditional) => {
                    self.written_in_nodes(hir, &conditional.then);
                    if let Some(otherwise) = &conditional.otherwise {
                        self.written_in_nodes(hir, otherwise);
                    }
                }
                HirNode::Scope(scope) => self.written_in_nodes(hir, &scope.body),
                HirNode::Handler(handler) => self.written_in_block(hir, handler.body),
                HirNode::Children(_) => {}
            }
        }
    }

    /// A two-way `Input` or `Checkbox` binding is a write, so the signal
    /// behind one needs its setter even though no `set` statement names it.
    ///
    /// The binding is the first *positional* argument, not the first
    /// argument: `Input hint is "…", name` writes `name` exactly as
    /// `Input name, hint is "…"` does, and `Lowering::leading_argument`
    /// reads it that way. Reading `args.first()` instead left the signal
    /// with no setter and the emission refusing itself.
    fn written_in_element(&mut self, hir: &Hir, element: &HirElement) {
        // Asked of the resolution, never of the spelling: a user component
        // named `Input` resolves to a `Res::Def` and must not be mistaken
        // for the built-in (spec §17.2.2(b)).
        if matches!(element.res, Res::Builtin(Builtin::Element(e)) if e.is_two_way()) {
            if let Some(expr) = leading_positional(element) {
                match hir.exprs[expr].kind {
                    HirExprKind::Ref(Res::Def(def)) => {
                        self.written.insert(def);
                    }
                    HirExprKind::Ref(Res::Local(local)) => {
                        self.written_locals.insert(local);
                    }
                    // Anything else in a binding position is not a place,
                    // so there is nothing to record as written.
                    HirExprKind::Ref(
                        Res::Builtin(_) | Res::Variant { .. } | Res::BuiltinVariant(_),
                    )
                    | HirExprKind::Number(_)
                    | HirExprKind::Address
                    | HirExprKind::Media(_)
                    | HirExprKind::Scroll
                    | HirExprKind::Scroll
                    | HirExprKind::Build { .. }
                    | HirExprKind::Outbound { .. }
                    | HirExprKind::Text(_)
                    | HirExprKind::Truth(_)
                    | HirExprKind::Empty
                    | HirExprKind::Environment(_)
                    | HirExprKind::List(_)
                    | HirExprKind::Map(_)
                    | HirExprKind::Call { .. }
                    | HirExprKind::OfCall { .. }
                    | HirExprKind::Operator { .. }
                    | HirExprKind::Unary { .. }
                    | HirExprKind::Binary { .. }
                    | HirExprKind::Field { .. }
                    | HirExprKind::Index { .. }
                    | HirExprKind::Append { .. }
                    | HirExprKind::Insert { .. }
                    | HirExprKind::MapInside { .. } => {}
                }
            }
        }
        self.written_in_nodes(hir, &element.children);
    }

    fn written_in_block(&mut self, hir: &Hir, id: BlockId) {
        for stmt in &hir.blocks[id].stmts {
            match stmt {
                HirStmt::Mutation(mutation) => match place_of(mutation).base {
                    Res::Def(def) => {
                        self.written.insert(def);
                    }
                    Res::Local(local) => {
                        self.written_locals.insert(local);
                    }
                    // None of these names storage, so none can be written.
                    Res::Builtin(_) | Res::Variant { .. } | Res::BuiltinVariant(_) => {}
                },
                HirStmt::When(when) => {
                    for arm in &when.arms {
                        if let HirArmBody::Block(block) = arm.body {
                            self.written_in_block(hir, block);
                        }
                    }
                }
                HirStmt::Each(each) => self.written_in_block(hir, each.body),
                HirStmt::If(conditional) => {
                    self.written_in_block(hir, conditional.then);
                    if let Some(otherwise) = conditional.otherwise {
                        self.written_in_block(hir, otherwise);
                    }
                }
                // A binding names a value; only a mutation writes one. Nor
                // does a `do`: it runs a `foreign`, which has no body that
                // could reach a place in this program.
                HirStmt::Pipeline(_) | HirStmt::Give(_) | HirStmt::Bind(_) | HirStmt::Do(_) => {}
            }
        }
    }

    /// Least fixed point: a function is reactive if its body reads a signal
    /// or calls a function that does. Iterating handles mutual recursion
    /// without needing a topological sort.
    fn solve_reactive_functions(&mut self, hir: &Hir) {
        loop {
            let mut changed = false;
            for (id, def) in hir.defs.iter() {
                let DefKind::Function(function) = &def.kind else {
                    continue;
                };
                if !self.reactive_functions.contains(&id)
                    && self.block_reads_signal(hir, function.body)
                {
                    self.reactive_functions.insert(id);
                    changed = true;
                }
            }
            if !changed {
                return;
            }
        }
    }

    fn block_reads_signal(&self, hir: &Hir, id: BlockId) -> bool {
        hir.blocks[id].stmts.iter().any(|stmt| match stmt {
            HirStmt::Give(expr) => self.reads_signal(hir, *expr),
            HirStmt::Pipeline(clause) => pipeline_exprs(clause)
                .into_iter()
                .any(|expr| self.reads_signal(hir, expr)),
            HirStmt::Mutation(mutation) => {
                let place = place_of(mutation);
                self.reads_signal(hir, mutation_value(mutation))
                    || place.path.iter().any(|segment| match segment {
                        HirPathSeg::Field(_) => false,
                        HirPathSeg::Index(expr) => self.reads_signal(hir, *expr),
                    })
            }
            HirStmt::When(when) => {
                self.reads_signal(hir, when.scrutinee)
                    || when.arms.iter().any(|arm| match arm.body {
                        HirArmBody::Show(expr) => self.reads_signal(hir, expr),
                        HirArmBody::Block(block) => self.block_reads_signal(hir, block),
                    })
            }
            HirStmt::Bind(bind) => bind
                .bindings
                .iter()
                .any(|binding| self.reads_signal(hir, binding.value)),
            // An effect's arguments are ordinary expressions, so a `do`
            // whose call reads a signal makes its function reactive
            // exactly as a `give` of the same expression would. Answering
            // `false` here would emit an effect that ran once and never
            // again when the signal it reads changed.
            HirStmt::Do(effect) => self.reads_signal(hir, effect.call),
            HirStmt::Each(each) => {
                self.reads_signal(hir, each.iter) || self.block_reads_signal(hir, each.body)
            }
            HirStmt::If(conditional) => {
                self.reads_signal(hir, conditional.cond)
                    || self.block_reads_signal(hir, conditional.then)
                    || conditional
                        .otherwise
                        .is_some_and(|block| self.block_reads_signal(hir, block))
            }
        })
    }
}

/// The leading positional argument of an element, wherever it was written
/// among the named ones.
fn leading_positional(element: &HirElement) -> Option<ExprId> {
    element.args.iter().find_map(|arg| match arg {
        HirArg::Positional(expr) => Some(*expr),
        HirArg::Named { .. } => None,
    })
}

pub fn arg_expr(arg: &HirArg) -> ExprId {
    match arg {
        HirArg::Positional(expr) => *expr,
        HirArg::Named { value, .. } => *value,
    }
}

fn place_of(mutation: &HirMutation) -> &zdc_hir::HirPlace {
    mutation.place()
}

fn mutation_value(mutation: &HirMutation) -> ExprId {
    mutation.value()
}

/// The expressions one pipeline clause holds.
///
/// A `Vec` rather than one `ExprId`, because `fold each` holds two — the
/// seed and the step — and both callers below are `any`-shaped. Reporting
/// only one of them would under-approximate `reads_signal`, which the doc
/// on that function says breaks reactivity, and would leave a definition
/// named only in a fold's seed out of the bundle that calls it.
fn pipeline_exprs(clause: &HirPipeline) -> Vec<ExprId> {
    match clause {
        HirPipeline::From(expr) | HirPipeline::TakeFirst(expr) => vec![*expr],
        HirPipeline::Keep { cond: expr, .. }
        | HirPipeline::Sort { key: expr, .. }
        | HirPipeline::MapEach { to: expr, .. } => vec![*expr],
        HirPipeline::Fold { starting, step, .. } => vec![*starting, *step],
    }
}

/// The binders a run of view nodes declares.
///
/// **Deliberately blind to expressions, and that is not an oversight.**
/// One caller passes `reactive_locals`, so everything collected here is
/// emitted as a *getter call*: the runtime hands an `each` binder and a
/// `when` pattern binding as functions to read. The binder of `map each x
/// in v to e` is neither — it is the parameter of an arrow the emitter
/// writes, holding a plain value — so collecting it here would emit `x()`
/// against a number. `declaration_block_binders` is where an expression
/// binder is collected, because that path feeds name economy alone.
fn node_binders(nodes: &[HirNode], out: &mut HashSet<LocalId>) {
    for node in nodes {
        match node {
            HirNode::Element(element) => node_binders(&element.children, out),
            HirNode::Each(each) => {
                out.insert(each.var);
                node_binders(&each.body, out);
            }
            HirNode::When(when) => {
                for arm in &when.arms {
                    out.extend(arm.bindings.iter().copied());
                    match &arm.body {
                        HirNodeArmBody::Show(element) => node_binders(&element.children, out),
                        HirNodeArmBody::Nodes(nodes) => node_binders(nodes, out),
                    }
                }
            }
            HirNode::If(conditional) => {
                node_binders(&conditional.then, out);
                if let Some(otherwise) = &conditional.otherwise {
                    node_binders(otherwise, out);
                }
            }
            HirNode::Scope(scope) => node_binders(&scope.body, out),
            HirNode::Handler(_) | HirNode::Children(_) => {}
        }
    }
}

/// Every binder inside a component declaration that is not a node binder —
/// the handlers written in its body, the loops and patterns inside those,
/// and the one binder an *expression* can hold.
///
/// This path feeds `declaration_locals` and nothing else, which is what
/// makes the expression walk safe here and wrong in `node_binders`: the
/// set is used to skip naming a declaration's binders, because
/// instantiation copied them per call site and naming the originals would
/// spend `count` on a local nothing emits.
fn declaration_block_binders(hir: &Hir, node: &HirNode, out: &mut HashSet<LocalId>) {
    match node {
        HirNode::Handler(handler) => block_binders(hir, handler.body, out),
        HirNode::Element(element) => {
            for arg in &element.args {
                expr_binders(hir, arg_expr(arg), out);
            }
            for child in &element.children {
                declaration_block_binders(hir, child, out);
            }
        }
        HirNode::Each(each) => {
            expr_binders(hir, each.iter, out);
            for child in &each.body {
                declaration_block_binders(hir, child, out);
            }
        }
        HirNode::When(when) => {
            expr_binders(hir, when.scrutinee, out);
            for arm in &when.arms {
                match &arm.body {
                    HirNodeArmBody::Show(element) => {
                        for arg in &element.args {
                            expr_binders(hir, arg_expr(arg), out);
                        }
                        for child in &element.children {
                            declaration_block_binders(hir, child, out);
                        }
                    }
                    HirNodeArmBody::Nodes(nodes) => {
                        for child in nodes {
                            declaration_block_binders(hir, child, out);
                        }
                    }
                }
            }
        }
        HirNode::If(conditional) => {
            expr_binders(hir, conditional.cond, out);
            for child in conditional
                .then
                .iter()
                .chain(conditional.otherwise.iter().flatten())
            {
                declaration_block_binders(hir, child, out);
            }
        }
        HirNode::Scope(scope) => {
            for child in &scope.body {
                declaration_block_binders(hir, child, out);
            }
        }
        HirNode::Children(_) => {}
    }
}

/// Every binder a block declares, statement binders and the one
/// expression binder alike.
///
/// **The expression walk is not decoration.** Every binder in this
/// language used to belong to a statement or a declaration, so a walk over
/// statements saw all of them; `map each x in v to …` is an expression
/// that binds, so a walk that stopped at statement boundaries would leave
/// a component declaration's binder unrecorded and spend the name it
/// wanted on a local nothing emits.
fn block_binders(hir: &Hir, id: BlockId, out: &mut HashSet<LocalId>) {
    for stmt in &hir.blocks[id].stmts {
        for expr in stmt_exprs(stmt) {
            expr_binders(hir, expr, out);
        }
        match stmt {
            HirStmt::Pipeline(clause) => match clause {
                HirPipeline::Keep { var, .. }
                | HirPipeline::Sort { var, .. }
                | HirPipeline::MapEach { var, .. } => {
                    out.insert(*var);
                }
                HirPipeline::Fold { item, total, .. } => {
                    out.insert(*item);
                    out.insert(*total);
                }
                HirPipeline::From(_) | HirPipeline::TakeFirst(_) => {}
            },
            HirStmt::When(when) => {
                for arm in &when.arms {
                    out.extend(arm.bindings.iter().copied());
                    match arm.body {
                        HirArmBody::Block(block) => block_binders(hir, block, out),
                        HirArmBody::Show(expr) => expr_binders(hir, expr, out),
                    }
                }
            }
            // `with name is value` binds `name`, which is exactly what
            // this walk collects. The value is walked above, for the
            // binder an expression may now hold.
            HirStmt::Bind(bind) => {
                out.extend(bind.bindings.iter().map(|binding| binding.local));
            }
            HirStmt::Each(each) => {
                out.insert(each.var);
                block_binders(hir, each.body, out);
            }
            HirStmt::If(conditional) => {
                block_binders(hir, conditional.then, out);
                if let Some(otherwise) = conditional.otherwise {
                    block_binders(hir, otherwise, out);
                }
            }
            HirStmt::Do(_) | HirStmt::Mutation(_) | HirStmt::Give(_) => {}
        }
    }
}

/// The expressions written directly in one statement. Nested blocks are
/// walked by the caller, which is what owns their scope.
fn stmt_exprs(stmt: &HirStmt) -> Vec<ExprId> {
    match stmt {
        HirStmt::Give(expr) => vec![*expr],
        HirStmt::Do(effect) => vec![effect.call],
        HirStmt::Pipeline(clause) => pipeline_exprs(clause),
        HirStmt::Bind(bind) => bind.bindings.iter().map(|binding| binding.value).collect(),
        HirStmt::Mutation(mutation) => vec![mutation_value(mutation)],
        HirStmt::When(when) => vec![when.scrutinee],
        HirStmt::Each(each) => vec![each.iter],
        HirStmt::If(conditional) => vec![conditional.cond],
    }
}

/// The binders inside an expression. Exactly one form has any.
fn expr_binders(hir: &Hir, id: ExprId, out: &mut HashSet<LocalId>) {
    match &hir.exprs[id].kind {
        HirExprKind::MapInside { var, source, to } => {
            out.insert(*var);
            expr_binders(hir, *source, out);
            expr_binders(hir, *to, out);
        }
        HirExprKind::Number(_)
        | HirExprKind::Text(_)
        | HirExprKind::Truth(_)
        | HirExprKind::Empty
        | HirExprKind::Environment(_)
        | HirExprKind::Address
        | HirExprKind::Media(_)
        | HirExprKind::Scroll
        | HirExprKind::Outbound { .. }
        | HirExprKind::Ref(_) => {}
        HirExprKind::List(items) => {
            for item in items {
                expr_binders(hir, *item, out);
            }
        }
        HirExprKind::Map(entries) => {
            for (key, value) in entries {
                expr_binders(hir, *key, out);
                expr_binders(hir, *value, out);
            }
        }
        HirExprKind::Call { args, .. } => {
            for arg in args {
                expr_binders(hir, arg_expr(arg), out);
            }
        }
        HirExprKind::OfCall { operand, .. }
        | HirExprKind::Operator { operand, .. }
        | HirExprKind::Unary { operand, .. } => expr_binders(hir, *operand, out),
        HirExprKind::Build { argument, .. } => expr_binders(hir, *argument, out),
        HirExprKind::Binary { lhs, rhs, .. } => {
            expr_binders(hir, *lhs, out);
            expr_binders(hir, *rhs, out);
        }
        HirExprKind::Field { base, .. } => expr_binders(hir, *base, out),
        HirExprKind::Index { base, index } => {
            expr_binders(hir, *base, out);
            expr_binders(hir, *index, out);
        }
        HirExprKind::Append { item, list } => {
            expr_binders(hir, *item, out);
            expr_binders(hir, *list, out);
        }
        HirExprKind::Insert { key, value, table } => {
            expr_binders(hir, *key, out);
            expr_binders(hir, *value, out);
            expr_binders(hir, *table, out);
        }
    }
}

/// Every local a component instance declared as its own state.
fn local_signals(nodes: &[HirNode], out: &mut HashSet<LocalId>) {
    for node in nodes {
        match node {
            HirNode::Element(element) => local_signals(&element.children, out),
            HirNode::Each(each) => local_signals(&each.body, out),
            HirNode::When(when) => {
                for arm in &when.arms {
                    match &arm.body {
                        HirNodeArmBody::Show(element) => local_signals(&element.children, out),
                        HirNodeArmBody::Nodes(nodes) => local_signals(nodes, out),
                    }
                }
            }
            HirNode::If(conditional) => {
                local_signals(&conditional.then, out);
                if let Some(otherwise) = &conditional.otherwise {
                    local_signals(otherwise, out);
                }
            }
            HirNode::Scope(scope) => {
                out.extend(scope.locals.iter().map(|local| local.local));
                local_signals(&scope.body, out);
            }
            HirNode::Handler(_) | HirNode::Children(_) => {}
        }
    }
}
/// Every definition this one refers to.
pub fn references_of(hir: &Hir, def: &Def, out: &mut Vec<DefId>) {
    match &def.kind {
        DefKind::Signal(signal) => expr_references(hir, signal.init, out),
        DefKind::Function(function) => block_references(hir, function.body, out),
        DefKind::Release(release) => block_references(hir, release.body, out),
        DefKind::View(view) => node_references(hir, &view.nodes, out),
        // A type declaration emits nothing and refers to nothing: a record
        // is an object literal at each construction site and a variant is a
        // tag string, so neither has a definition to reach. A component
        // reaches nothing either, because instantiation already moved its
        // body into the view.
        // A `foreign` has no body to walk: its whole content is the
        // module and symbol it names, which codegen prints from the
        // declaration itself.
        DefKind::Record(_) | DefKind::Choice(_) | DefKind::Component(_) | DefKind::Foreign(_) => {}
    }
}

pub fn node_references(hir: &Hir, nodes: &[HirNode], out: &mut Vec<DefId>) {
    for node in nodes {
        match node {
            HirNode::Element(element) => element_references(hir, element, out),
            HirNode::Each(each) => {
                expr_references(hir, each.iter, out);
                node_references(hir, &each.body, out);
            }
            HirNode::When(when) => {
                expr_references(hir, when.scrutinee, out);
                for arm in &when.arms {
                    match &arm.body {
                        HirNodeArmBody::Show(element) => element_references(hir, element, out),
                        HirNodeArmBody::Nodes(nodes) => node_references(hir, nodes, out),
                    }
                }
            }
            HirNode::If(conditional) => {
                expr_references(hir, conditional.cond, out);
                node_references(hir, &conditional.then, out);
                if let Some(otherwise) = &conditional.otherwise {
                    node_references(hir, otherwise, out);
                }
            }
            HirNode::Scope(scope) => {
                for local in &scope.locals {
                    expr_references(hir, local.init, out);
                }
                node_references(hir, &scope.body, out);
            }
            HirNode::Handler(handler) => block_references(hir, handler.body, out),
            HirNode::Children(_) => {}
        }
    }
}

fn element_references(hir: &Hir, element: &HirElement, out: &mut Vec<DefId>) {
    for arg in &element.args {
        expr_references(hir, arg_expr(arg), out);
    }
    node_references(hir, &element.children, out);
}

fn block_references(hir: &Hir, id: BlockId, out: &mut Vec<DefId>) {
    for stmt in &hir.blocks[id].stmts {
        match stmt {
            HirStmt::Give(expr) => expr_references(hir, *expr, out),
            HirStmt::Pipeline(clause) => {
                for expr in pipeline_exprs(clause) {
                    expr_references(hir, expr, out);
                }
            }
            // A binding's value is ordinary code: a definition named only
            // on the right of a `with` is reachable, and leaving it out
            // here would emit a bundle that calls what it does not carry.
            HirStmt::Bind(bind) => {
                for binding in &bind.bindings {
                    expr_references(hir, binding.value, out);
                }
            }
            // The same argument the `Bind` arm makes: a `foreign` named
            // only by a `do` is reachable, and leaving it out here would
            // emit a bundle that calls what it does not import.
            HirStmt::Do(effect) => expr_references(hir, effect.call, out),
            HirStmt::Mutation(mutation) => {
                expr_references(hir, mutation_value(mutation), out);
                let place = place_of(mutation);
                if let Res::Def(def) = place.base {
                    out.push(def);
                }
                for segment in &place.path {
                    if let HirPathSeg::Index(expr) = segment {
                        expr_references(hir, *expr, out);
                    }
                }
            }
            HirStmt::When(when) => {
                expr_references(hir, when.scrutinee, out);
                for arm in &when.arms {
                    match arm.body {
                        HirArmBody::Show(expr) => expr_references(hir, expr, out),
                        HirArmBody::Block(block) => block_references(hir, block, out),
                    }
                }
            }
            HirStmt::Each(each) => {
                expr_references(hir, each.iter, out);
                block_references(hir, each.body, out);
            }
            HirStmt::If(conditional) => {
                expr_references(hir, conditional.cond, out);
                block_references(hir, conditional.then, out);
                if let Some(otherwise) = conditional.otherwise {
                    block_references(hir, otherwise, out);
                }
            }
        }
    }
}

pub fn expr_references(hir: &Hir, id: ExprId, out: &mut Vec<DefId>) {
    match &hir.exprs[id].kind {
        HirExprKind::Number(_)
        | HirExprKind::Text(_)
        | HirExprKind::Truth(_)
        | HirExprKind::Empty
        | HirExprKind::Environment(_)
        | HirExprKind::Address
        // The query is a literal and the answer is the browser's, so no
        // definition is referenced.
        | HirExprKind::Media(_)
        | HirExprKind::Scroll => {}
        HirExprKind::List(items) => {
            for item in items {
                expr_references(hir, *item, out);
            }
        }
        HirExprKind::Map(entries) => {
            for (key, value) in entries {
                expr_references(hir, *key, out);
                expr_references(hir, *value, out);
            }
        }
        HirExprKind::Ref(Res::Def(def)) => out.push(*def),
        HirExprKind::Ref(_) => {}
        // A request references whatever its arguments reference. The
        // destination is a literal and references nothing.
        HirExprKind::Outbound { args, .. } => {
            for arg in args {
                expr_references(hir, arg_expr(arg), out);
            }
        }
        HirExprKind::Call { callee, args } => {
            if let Res::Def(def) = callee {
                out.push(*def);
            }
            for arg in args {
                expr_references(hir, arg_expr(arg), out);
            }
        }
        HirExprKind::OfCall { callee, operand } => {
            if let Res::Def(def) = callee {
                out.push(*def);
            }
            expr_references(hir, *operand, out);
        }
        // The capability names no definition — it is the compiler — but
        // the path it is asked for is an ordinary expression that can.
        HirExprKind::Build { argument, .. } => expr_references(hir, *argument, out),
        // Which definition a type-directed operator dispatches to is the
        // checker's answer rather than the HIR's, so it is seeded from
        // `operator_targets` and not found here.
        HirExprKind::Operator { operand, .. } => expr_references(hir, *operand, out),
        HirExprKind::Unary { operand, .. } => expr_references(hir, *operand, out),
        HirExprKind::Binary { lhs, rhs, .. } => {
            expr_references(hir, *lhs, out);
            expr_references(hir, *rhs, out);
        }
        HirExprKind::Field { base, .. } => expr_references(hir, *base, out),
        HirExprKind::Index { base, index } => {
            expr_references(hir, *base, out);
            expr_references(hir, *index, out);
        }
        HirExprKind::Append { item, list } => {
            expr_references(hir, *item, out);
            expr_references(hir, *list, out);
        }
        HirExprKind::Insert { key, value, table } => {
            expr_references(hir, *key, out);
            expr_references(hir, *value, out);
            expr_references(hir, *table, out);
        }
        HirExprKind::MapInside { source, to, .. } => {
            expr_references(hir, *source, out);
            expr_references(hir, *to, out);
        }
    }
}
