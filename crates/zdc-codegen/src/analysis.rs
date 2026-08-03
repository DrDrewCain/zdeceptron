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

use std::collections::{BTreeSet, HashMap, HashSet};

use zdc_hir::{
    BlockId, Def, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirElement, HirExprKind,
    HirMutation, HirNode, HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, LocalId, Res,
};
use zdc_types::TypeTable;

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
    /// Definitions reachable from the client seed set.
    client_closure: BTreeSet<DefId>,
    /// Which library function each `contains` dispatched to.
    operator_targets: HashMap<ExprId, DefId>,
}

impl Analysis {
    /// `types` supplies one edge the HIR does not carry: which library
    /// function each `contains` dispatched to (§17.4.3). The closure walk
    /// needs it, because a bundle that reaches `textContains` through an
    /// operator must still carry `textContains` — §17.4.5's prelude
    /// closure, folded into the walk that was already here rather than run
    /// as a phase of its own.
    pub fn new(hir: &Hir, types: &TypeTable) -> Analysis {
        let mut analysis = Analysis {
            reactive_locals: HashSet::new(),
            reactive_functions: HashSet::new(),
            written: BTreeSet::new(),
            client_closure: BTreeSet::new(),
            operator_targets: HashMap::new(),
        };
        for (_, def) in hir.defs.iter() {
            if let DefKind::View(view) = &def.kind {
                node_binders(&view.nodes, &mut analysis.reactive_locals);
            }
        }
        for (expr, def) in types.operator_targets() {
            analysis.operator_targets.insert(expr, def);
        }
        analysis.collect_written(hir);
        analysis.solve_reactive_functions(hir);
        analysis.walk_client_closure(hir);
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

    pub fn client_closure(&self) -> &BTreeSet<DefId> {
        &self.client_closure
    }

    fn res_is_reactive(&self, hir: &Hir, res: Res) -> bool {
        match res {
            Res::Def(def) => match hir.defs[def].kind {
                DefKind::Signal(_) => true,
                DefKind::Function(_) => self.reactive_functions.contains(&def),
                // A record names a shape and a view names a root; neither
                // is a value that can change. A `foreign` cannot reach a
                // signal at all: the prelude's placement invariant
                // (§17.4.1) is that no library definition mentions one.
                DefKind::View(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Foreign(_) => false,
            },
            Res::Local(local) => self.reactive_locals.contains(&local),
            // A variant tag is a constant of the program.
            Res::Variant { .. } | Res::BuiltinVariant(_) | Res::Builtin(_) => false,
        }
    }

    fn collect_written(&mut self, hir: &Hir) {
        for (_, def) in hir.defs.iter() {
            match &def.kind {
                DefKind::Function(function) => self.written_in_block(hir, function.body),
                DefKind::View(view) => self.written_in_nodes(hir, &view.nodes),
                DefKind::Signal(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Foreign(_) => {}
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
        if matches!(element.name.as_str(), "Input" | "Checkbox") {
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
                // A binding names a value; only a mutation writes one.
                HirStmt::Pipeline(_) | HirStmt::Give(_) | HirStmt::Bind(_) => {}
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
            HirStmt::Bind(bind) => bind
                .bindings
                .iter()
                .any(|binding| self.reads_signal(hir, binding.value)),
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
    fn walk_client_closure(&mut self, hir: &Hir) {
        let mut queue: Vec<DefId> = Vec::new();
        for (id, def) in hir.defs.iter() {
            match &def.kind {
                DefKind::View(_) => queue.push(id),
                DefKind::Signal(signal) if signal.placement == zdc_ast::Placement::Client => {
                    queue.push(id);
                }
                _ => {}
            }
        }

        while let Some(id) = queue.pop() {
            if !self.client_closure.insert(id) {
                continue;
            }
            let mut referenced = Vec::new();
            references_of(hir, &hir.defs[id], &self.operator_targets, &mut referenced);
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
            HirNode::Handler(_) => {}
        }
    }
}

/// Every definition this one refers to.
fn references_of(hir: &Hir, def: &Def, targets: &HashMap<ExprId, DefId>, out: &mut Vec<DefId>) {
    match &def.kind {
        DefKind::Signal(signal) => expr_references(hir, signal.init, targets, out),
        DefKind::Function(function) => block_references(hir, function.body, targets, out),
        DefKind::View(view) => node_references(hir, &view.nodes, targets, out),
        // A type declaration emits nothing and refers to nothing: a record
        // is an object literal at each construction site and a variant is a
        // tag string, so neither has a definition to reach. A `foreign` is
        // a leaf by construction — it has no body to walk.
        DefKind::Record(_) | DefKind::Choice(_) | DefKind::Foreign(_) => {}
    }
}

fn node_references(
    hir: &Hir,
    nodes: &[HirNode],
    targets: &HashMap<ExprId, DefId>,
    out: &mut Vec<DefId>,
) {
    for node in nodes {
        match node {
            HirNode::Element(element) => element_references(hir, element, targets, out),
            HirNode::Each(each) => {
                expr_references(hir, each.iter, targets, out);
                node_references(hir, &each.body, targets, out);
            }
            HirNode::When(when) => {
                expr_references(hir, when.scrutinee, targets, out);
                for arm in &when.arms {
                    match &arm.body {
                        HirNodeArmBody::Show(element) => {
                            element_references(hir, element, targets, out)
                        }
                        HirNodeArmBody::Nodes(nodes) => node_references(hir, nodes, targets, out),
                    }
                }
            }
            HirNode::Handler(handler) => block_references(hir, handler.body, targets, out),
        }
    }
}

fn element_references(
    hir: &Hir,
    element: &HirElement,
    targets: &HashMap<ExprId, DefId>,
    out: &mut Vec<DefId>,
) {
    for arg in &element.args {
        expr_references(hir, arg_expr(arg), targets, out);
    }
    node_references(hir, &element.children, targets, out);
}

fn block_references(
    hir: &Hir,
    id: BlockId,
    targets: &HashMap<ExprId, DefId>,
    out: &mut Vec<DefId>,
) {
    for stmt in &hir.blocks[id].stmts {
        match stmt {
            HirStmt::Give(expr) => expr_references(hir, *expr, targets, out),
            HirStmt::Pipeline(clause) => expr_references(hir, pipeline_expr(clause), targets, out),
            HirStmt::Mutation(mutation) => {
                expr_references(hir, mutation_value(mutation), targets, out);
                let place = place_of(mutation);
                if let Res::Def(def) = place.base {
                    out.push(def);
                }
                for segment in &place.path {
                    if let HirPathSeg::Index(expr) = segment {
                        expr_references(hir, *expr, targets, out);
                    }
                }
            }
            HirStmt::When(when) => {
                expr_references(hir, when.scrutinee, targets, out);
                for arm in &when.arms {
                    match arm.body {
                        HirArmBody::Show(expr) => expr_references(hir, expr, targets, out),
                        HirArmBody::Block(block) => block_references(hir, block, targets, out),
                    }
                }
            }
            HirStmt::Bind(bind) => {
                for binding in &bind.bindings {
                    expr_references(hir, binding.value, targets, out);
                }
            }
            HirStmt::Each(each) => {
                expr_references(hir, each.iter, targets, out);
                block_references(hir, each.body, targets, out);
            }
            HirStmt::If(conditional) => {
                expr_references(hir, conditional.cond, targets, out);
                block_references(hir, conditional.then, targets, out);
                if let Some(otherwise) = conditional.otherwise {
                    block_references(hir, otherwise, targets, out);
                }
            }
        }
    }
}

fn expr_references(hir: &Hir, id: ExprId, targets: &HashMap<ExprId, DefId>, out: &mut Vec<DefId>) {
    match &hir.exprs[id].kind {
        HirExprKind::Number(_)
        | HirExprKind::Text(_)
        | HirExprKind::Truth(_)
        | HirExprKind::Empty
        | HirExprKind::Environment(_) => {}
        HirExprKind::List(items) => {
            for item in items {
                expr_references(hir, *item, targets, out);
            }
        }
        HirExprKind::Map(entries) => {
            for (key, value) in entries {
                expr_references(hir, *key, targets, out);
                expr_references(hir, *value, targets, out);
            }
        }
        HirExprKind::Ref(Res::Def(def)) => out.push(*def),
        HirExprKind::Ref(_) => {}
        HirExprKind::Call { callee, args } => {
            if let Res::Def(def) = callee {
                out.push(*def);
            }
            for arg in args {
                expr_references(hir, arg_expr(arg), targets, out);
            }
        }
        HirExprKind::OfCall { callee, operand } => {
            if let Res::Def(def) = callee {
                out.push(*def);
            }
            expr_references(hir, *operand, targets, out);
        }
        HirExprKind::Operator { operand, .. } => expr_references(hir, *operand, targets, out),
        HirExprKind::Unary { operand, .. } => expr_references(hir, *operand, targets, out),
        HirExprKind::Binary { lhs, rhs, .. } => {
            // §17.4.5's prelude closure. A `contains` reaches its library
            // function through the checker's dispatch verdict rather than
            // through any name in the source, so without this edge the
            // walk stops one short and the bundle calls something it never
            // emitted.
            if let Some(target) = targets.get(&id) {
                out.push(*target);
            }
            expr_references(hir, *lhs, targets, out);
            expr_references(hir, *rhs, targets, out);
        }
        HirExprKind::Field { base, .. } => expr_references(hir, *base, targets, out),
        HirExprKind::Index { base, index } => {
            expr_references(hir, *base, targets, out);
            expr_references(hir, *index, targets, out);
        }
        HirExprKind::Append { item, list } => {
            expr_references(hir, *item, targets, out);
            expr_references(hir, *list, targets, out);
        }
    }
}
