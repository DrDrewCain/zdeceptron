//! The three whole-program facts every emission decision reads.
//!
//! None of them needs types. All three are reachability queries over the
//! `Res::Def` and `Res::Local` edges `zdc-resolve` already produces, which
//! is what lets the expensive machinery below be built and tested before
//! `zdc-types` exists (spec §16.3.5).
//!
//! 1. **Which expressions read a signal**, because that is the difference
//!    between baking a value into markup and allocating an effect.
//! 2. **Which binders are reactive**, because a binder whose binding site
//!    outlives its value must be read through the graph. Direct Emission's
//!    claim that a `Res::Local` is never reactive is what froze every list.
//! 3. **Which definitions the client bundle needs**, and which signals are
//!    ever written, because a never-written signal needs no setter.

use std::collections::{BTreeSet, HashSet};

use zdc_hir::{
    BlockId, Def, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirElement, HirExprKind,
    HirMutation, HirNode, HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, LocalId, Res,
};

use crate::pages::Bindings;

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
    /// Definitions reachable from the client seed set.
    client_closure: BTreeSet<DefId>,
    /// Binders that belong to a `component` declaration rather than to an
    /// instance of it.
    ///
    /// Instantiation copies a component's body once per call site with
    /// fresh binders (§14D.3), so the declaration's own are never emitted.
    /// Naming them anyway would let a component nobody used take `count`
    /// and leave the instance that is emitted with `count$`.
    declaration_locals: HashSet<LocalId>,
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
/// The component-declaration binders and the writes inside function
/// bodies are page-independent for the same reason and are hoisted with
/// it.
pub struct Shared {
    reactive_functions: HashSet<DefId>,
    declaration_locals: HashSet<LocalId>,
    written: BTreeSet<DefId>,
    written_locals: BTreeSet<LocalId>,
}

impl Shared {
    pub fn new(hir: &Hir) -> Shared {
        let mut shared = Shared {
            reactive_functions: HashSet::new(),
            declaration_locals: HashSet::new(),
            written: BTreeSet::new(),
            written_locals: BTreeSet::new(),
        };
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
                DefKind::View(_)
                | DefKind::Signal(_)
                | DefKind::Function(_)
                | DefKind::Record(_)
                | DefKind::Choice(_) => {}
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
                DefKind::View(_)
                | DefKind::Signal(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_) => {}
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
        }
    }

    /// The whole program, rooted at the `view`.
    pub fn new(hir: &Hir) -> Analysis {
        let nodes = hir.view.and_then(|id| match &hir.defs[id].kind {
            DefKind::View(view) => Some(view.nodes.clone()),
            DefKind::Signal(_)
            | DefKind::Function(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => None,
        });
        let shared = Shared::new(hir);
        match nodes {
            Some(nodes) => Analysis::rooted(hir, &nodes, &Bindings::default(), true, &shared),
            None => Analysis::rooted(hir, &[], &Bindings::default(), true, &shared),
        }
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
            HirExprKind::List(items) => items.iter().any(|item| self.reads_signal(hir, *item)),
            HirExprKind::Map(entries) => entries
                .iter()
                .any(|(key, value)| self.reads_signal(hir, *key) || self.reads_signal(hir, *value)),
            // `environment` is server-only state; a client walk cannot
            // reach one, but reporting it reactive is the safe direction.
            HirExprKind::Environment(_) => true,
            // The fold replaced every `address` a page can reach; one
            // surviving here is a program reading it somewhere the fold
            // does not run, which `expr.rs` reports by name.
            HirExprKind::Address => false,
            HirExprKind::Ref(res) => self.res_is_reactive(hir, *res),
            HirExprKind::Call { callee, args } => {
                self.res_is_reactive(hir, *callee)
                    || args.iter().any(|arg| self.reads_signal(hir, arg_expr(arg)))
            }
            HirExprKind::Unary { operand, .. } => self.reads_signal(hir, *operand),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.reads_signal(hir, *lhs) || self.reads_signal(hir, *rhs)
            }
            HirExprKind::Field { base, .. } => self.reads_signal(hir, *base),
            HirExprKind::Index { base, index } => {
                self.reads_signal(hir, *base) || self.reads_signal(hir, *index)
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
            HirExprKind::Ref(res @ Res::Def(def)) => match &hir.defs[*def].kind {
                DefKind::Signal(signal) => {
                    (signal.placement != zdc_ast::Placement::Static).then_some(*res)
                }
                DefKind::View(_)
                | DefKind::Function(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_) => None,
            },
            HirExprKind::Ref(res @ Res::Local(local)) => (self.reactive_locals.contains(local)
                && !self.folded.contains(local))
            .then_some(*res),
            _ => None,
        }
    }

    pub fn is_reactive_local(&self, id: LocalId) -> bool {
        self.reactive_locals.contains(&id) && !self.folded.contains(&id)
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

    pub fn client_closure(&self) -> &BTreeSet<DefId> {
        &self.client_closure
    }

    fn res_is_reactive(&self, hir: &Hir, res: Res) -> bool {
        match res {
            Res::Def(def) => match hir.defs[def].kind {
                // A `static` signal is one value computed on the build
                // host and inlined, so nothing can change it and reading
                // it allocates no effect (§14C.3b).
                DefKind::Signal(ref signal) => signal.placement != zdc_ast::Placement::Static,
                DefKind::Function(_) => self.reactive_functions.contains(&def),
                // A record names a shape and a view names a root; neither
                // is a value that can change.
                DefKind::View(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_) => false,
            },
            Res::Local(local) => self.reactive_locals.contains(&local),
            // A variant tag is a constant of the program.
            Res::Variant { .. } | Res::Builtin(_) => false,
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
    fn written_in_element(&mut self, hir: &Hir, element: &HirElement) {
        if matches!(element.name.as_str(), "Input" | "Checkbox") {
            if let Some(HirArg::Positional(expr)) = element.args.first() {
                match hir.exprs[*expr].kind {
                    HirExprKind::Ref(Res::Def(def)) => {
                        self.written.insert(def);
                    }
                    HirExprKind::Ref(Res::Local(local)) => {
                        self.written_locals.insert(local);
                    }
                    _ => {}
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
                    _ => {}
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
                HirStmt::Pipeline(_) | HirStmt::Give(_) => {}
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
            HirStmt::Pipeline(clause) => self.reads_signal(hir, pipeline_expr(clause)),
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

    /// Spec §16.3.12's client walk: transitive closure over `Res::Def`
    /// edges from the view and every `client`-placed signal.
    ///
    /// Emitting only the closure is what keeps an unreferenced helper out
    /// of the bundle. The walk would stop at a read of a `server` or
    /// `durable` signal; this milestone refuses those outright, so the stop
    /// is a refusal rather than a boundary.
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
                    DefKind::View(_)
                    | DefKind::Signal(_)
                    | DefKind::Function(_)
                    | DefKind::Record(_)
                    | DefKind::Choice(_)
                    | DefKind::Component(_) => {}
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
        }
    }
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

fn pipeline_expr(clause: &HirPipeline) -> ExprId {
    match clause {
        HirPipeline::From(expr) | HirPipeline::TakeFirst(expr) => *expr,
        HirPipeline::Keep { cond: expr, .. }
        | HirPipeline::Sort { key: expr, .. }
        | HirPipeline::MapEach { to: expr, .. } => *expr,
    }
}

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

/// Every binder inside the statement blocks a component declaration owns —
/// the handlers written in its body, and the loops and patterns inside
/// those.
fn declaration_block_binders(hir: &Hir, node: &HirNode, out: &mut HashSet<LocalId>) {
    match node {
        HirNode::Handler(handler) => block_binders(hir, handler.body, out),
        HirNode::Element(element) => {
            for child in &element.children {
                declaration_block_binders(hir, child, out);
            }
        }
        HirNode::Each(each) => {
            for child in &each.body {
                declaration_block_binders(hir, child, out);
            }
        }
        HirNode::When(when) => {
            for arm in &when.arms {
                match &arm.body {
                    HirNodeArmBody::Show(element) => {
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

fn block_binders(hir: &Hir, id: BlockId, out: &mut HashSet<LocalId>) {
    for stmt in &hir.blocks[id].stmts {
        match stmt {
            HirStmt::Pipeline(clause) => match clause {
                HirPipeline::Keep { var, .. }
                | HirPipeline::Sort { var, .. }
                | HirPipeline::MapEach { var, .. } => {
                    out.insert(*var);
                }
                HirPipeline::From(_) | HirPipeline::TakeFirst(_) => {}
            },
            HirStmt::When(when) => {
                for arm in &when.arms {
                    out.extend(arm.bindings.iter().copied());
                    if let HirArmBody::Block(block) = arm.body {
                        block_binders(hir, block, out);
                    }
                }
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
            HirStmt::Mutation(_) | HirStmt::Give(_) => {}
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
        DefKind::View(view) => node_references(hir, &view.nodes, out),
        // A type declaration emits nothing and refers to nothing: a record
        // is an object literal at each construction site and a variant is a
        // tag string, so neither has a definition to reach. A component
        // reaches nothing either, because instantiation already moved its
        // body into the view.
        DefKind::Record(_) | DefKind::Choice(_) | DefKind::Component(_) => {}
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
            HirStmt::Pipeline(clause) => expr_references(hir, pipeline_expr(clause), out),
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

fn expr_references(hir: &Hir, id: ExprId, out: &mut Vec<DefId>) {
    match &hir.exprs[id].kind {
        HirExprKind::Number(_)
        | HirExprKind::Text(_)
        | HirExprKind::Truth(_)
        | HirExprKind::Empty
        | HirExprKind::Environment(_)
        | HirExprKind::Address => {}
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
        HirExprKind::Call { callee, args } => {
            if let Res::Def(def) = callee {
                out.push(*def);
            }
            for arg in args {
                expr_references(hir, arg_expr(arg), out);
            }
        }
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
    }
}
