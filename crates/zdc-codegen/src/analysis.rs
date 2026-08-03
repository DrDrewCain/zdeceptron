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

/// Whether reading this definition has to go through the reactive graph.
///
/// Every signal but one: a `static` signal is computed once on the build
/// host and inlined at each read (§14C.3b), so it has no cell, no getter
/// and nothing that could ever change. Treating it as reactive would emit
/// `title()` against a name no root declares.
fn is_reactive_signal(hir: &Hir, def: DefId) -> bool {
    match &hir.defs[def].kind {
        DefKind::Signal(signal) => signal.placement != zdc_ast::Placement::Static,
        DefKind::Function(_) | DefKind::View(_) | DefKind::Record(_) | DefKind::Choice(_) => false,
    }
}

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
}

impl Analysis {
    pub fn new(hir: &Hir) -> Analysis {
        let mut analysis = Analysis {
            reactive_locals: HashSet::new(),
            reactive_functions: HashSet::new(),
            written: BTreeSet::new(),
        };
        for (_, def) in hir.defs.iter() {
            if let DefKind::View(view) = &def.kind {
                node_binders(&view.nodes, &mut analysis.reactive_locals);
            }
        }
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
            HirExprKind::Ref(res @ Res::Def(def)) => is_reactive_signal(hir, *def).then_some(*res),
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

    fn res_is_reactive(&self, hir: &Hir, res: Res) -> bool {
        match res {
            Res::Def(def) => match hir.defs[def].kind {
                DefKind::Signal(_) => is_reactive_signal(hir, def),
                DefKind::Function(_) => self.reactive_functions.contains(&def),
                // A record names a shape and a view names a root; neither
                // is a value that can change.
                DefKind::View(_) | DefKind::Record(_) | DefKind::Choice(_) => false,
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
                DefKind::Signal(_) | DefKind::Record(_) | DefKind::Choice(_) => {}
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
                HirNode::Handler(handler) => self.written_in_block(hir, handler.body),
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
                if let HirExprKind::Ref(Res::Def(def)) = hir.exprs[*expr].kind {
                    self.written.insert(def);
                }
            }
        }
        self.written_in_nodes(hir, &element.children);
    }

    fn written_in_block(&mut self, hir: &Hir, id: BlockId) {
        for stmt in &hir.blocks[id].stmts {
            match stmt {
                HirStmt::Mutation(mutation) => {
                    if let Res::Def(def) = place_of(mutation).base {
                        self.written.insert(def);
                    }
                }
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
            HirNode::Handler(_) => {}
        }
    }
}
