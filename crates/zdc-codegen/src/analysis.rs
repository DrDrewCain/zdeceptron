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
//! 3. **Which signals are ever written**, because a never-written signal
//!    needs no setter.
//!
//! What the client bundle *contains* used to be a fourth answer here, as a
//! reachability walk from a guessed seed set. It is not a guess any more:
//! `TierSplit::client_members` is the answer, the walk that produced it
//! stopped at each crossing, and that stop is what makes §14A.1's
//! exclusion provable (spec §17.2.1).

use std::collections::{BTreeSet, HashSet};

use zdc_hir::{
    BlockId, Builtin, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirElement, HirExprKind,
    HirMutation, HirNode, HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, LocalId, Res,
};

pub struct Analysis {
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
    /// Binders that belong to a `component` declaration rather than to an
    /// instance of it.
    ///
    /// Instantiation copies a component's body once per call site with
    /// fresh binders (§14D.3), so the declaration's own are never emitted.
    /// Naming them anyway would let a component nobody used take `count`
    /// and leave the instance that is emitted with `count$`.
    declaration_locals: HashSet<LocalId>,
}

impl Analysis {
    pub fn new(hir: &Hir) -> Analysis {
        let mut analysis = Analysis {
            reactive_locals: HashSet::new(),
            reactive_functions: HashSet::new(),
            written: BTreeSet::new(),
            written_locals: BTreeSet::new(),
            local_signals: HashSet::new(),
            declaration_locals: HashSet::new(),
        };
        for (_, def) in hir.defs.iter() {
            match &def.kind {
                DefKind::View(view) => {
                    node_binders(&view.nodes, &mut analysis.reactive_locals);
                    local_signals(&view.nodes, &mut analysis.local_signals);
                }
                DefKind::Component(component) => {
                    let out = &mut analysis.declaration_locals;
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
                DefKind::Signal(_)
                | DefKind::Function(_)
                | DefKind::Record(_)
                | DefKind::Choice(_) => {}
            }
        }
        // A component's state is a signal, so reading it is a call, exactly
        // as reading a top-level one is.
        analysis
            .reactive_locals
            .extend(analysis.local_signals.iter().copied());
        analysis.collect_written(hir);
        analysis.solve_reactive_functions(hir);
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
            HirExprKind::Ref(res @ Res::Def(def)) => {
                matches!(hir.defs[*def].kind, DefKind::Signal(_)).then_some(*res)
            }
            HirExprKind::Ref(res @ Res::Local(local)) => {
                self.reactive_locals.contains(local).then_some(*res)
            }
            _ => None,
        }
    }

    pub fn is_reactive_local(&self, id: LocalId) -> bool {
        self.reactive_locals.contains(&id)
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

    fn res_is_reactive(&self, hir: &Hir, res: Res) -> bool {
        match res {
            Res::Def(def) => match hir.defs[def].kind {
                DefKind::Signal(_) => true,
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

    fn collect_written(&mut self, hir: &Hir) {
        for (_, def) in hir.defs.iter() {
            match &def.kind {
                DefKind::Function(function) => self.written_in_block(hir, function.body),
                DefKind::View(view) => self.written_in_nodes(hir, &view.nodes),
                // A component declaration emits nothing; its instances are
                // already in the view.
                DefKind::Signal(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_) => {}
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
    fn written_in_element(&mut self, hir: &Hir, element: &HirElement) {
        // Asked of the resolution, never of the spelling: a user component
        // named `Input` resolves to a `Res::Def` and must not be mistaken
        // for the built-in (spec §17.2.2(b)).
        if matches!(element.res, Res::Builtin(Builtin::Element(e)) if e.is_two_way()) {
            if let Some(HirArg::Positional(expr)) = element.args.first() {
                match hir.exprs[*expr].kind {
                    HirExprKind::Ref(Res::Def(def)) => {
                        self.written.insert(def);
                    }
                    HirExprKind::Ref(Res::Local(local)) => {
                        self.written_locals.insert(local);
                    }
                    // Anything else in a binding position is not a place,
                    // so there is nothing to record as written.
                    HirExprKind::Ref(Res::Builtin(_) | Res::Variant { .. })
                    | HirExprKind::Number(_)
                    | HirExprKind::Text(_)
                    | HirExprKind::Truth(_)
                    | HirExprKind::Empty
                    | HirExprKind::Environment(_)
                    | HirExprKind::List(_)
                    | HirExprKind::Map(_)
                    | HirExprKind::Call { .. }
                    | HirExprKind::Unary { .. }
                    | HirExprKind::Binary { .. }
                    | HirExprKind::Field { .. }
                    | HirExprKind::Index { .. } => {}
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
                    // Neither names storage, so neither can be written.
                    Res::Builtin(_) | Res::Variant { .. } => {}
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
