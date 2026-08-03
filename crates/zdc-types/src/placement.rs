//! The interface between this pass and the placement pass (Plan 4,
//! `zdc-graph`).
//!
//! Placement is not the type checker's job. The *type of a cross-placement
//! read* is, because `Remote of T` is a type (§5.2, amended by §14G.1.4).
//! This module is the whole of the boundary: one table, one question asked
//! of the placement pass, and a stub that answers it from the HIR alone
//! until `zdc-graph` exists.
//!
//! # What `zdc-graph` must supply
//!
//! Replace [`Contexts`] with the real thing. It must answer, for every
//! definition whose body contains reads:
//!
//! * [`ReadContext`] for that body — which row of §14G.1.4's table applies.
//! * [`SignalPlacement`] for every signal — which column applies.
//!
//! The stub below computes both from syntax. It is exact for the three
//! placements the grammar has and for the one root the language has, and
//! it is wrong the moment either of those grows. See the crate's report
//! for the precise list.

use std::collections::{HashMap, HashSet};

use zdc_hir::{
    BlockId, DefId, DefKind, Hir, HirArg, HirArmBody, HirElement, HirExprKind, HirNode,
    HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, Res,
};

/// Where a signal's value lives — §14G.1.4's columns.
///
/// Two of the five do not exist in the grammar yet. They are named here
/// anyway so the table below is the spec's table and not a subset of it:
/// a reviewer can check the code against §14G.1.4 line by line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalPlacement {
    Client,
    /// §14C.3b. No `static` keyword exists; `zdc-graph` supplies this.
    Static,
    Server,
    Durable,
    /// §14G.3a. No `per visitor` syntax exists; `zdc-graph` supplies this.
    DurablePerVisitor,
}

impl SignalPlacement {
    pub fn from_ast(placement: zdc_ast::Placement) -> SignalPlacement {
        match placement {
            zdc_ast::Placement::Client => SignalPlacement::Client,
            zdc_ast::Placement::Server => SignalPlacement::Server,
            zdc_ast::Placement::Durable => SignalPlacement::Durable,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            SignalPlacement::Client => "client",
            SignalPlacement::Static => "static",
            SignalPlacement::Server => "server",
            SignalPlacement::Durable => "durable",
            SignalPlacement::DurablePerVisitor => "durable per visitor",
        }
    }
}

/// Where a read happens — §14G.1.4's rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadContext {
    /// The view, a `client` signal's initializer, or an event handler.
    Client,
    /// `static` evaluation, which runs on the build host (§14G.1.5).
    Static,
    /// A `server` or `durable` derivation the view is the root of: the
    /// client supplies its `client` inputs as RPC arguments.
    ViewRootedServer,
    /// A `server` or `durable` derivation a cadence or path trigger is
    /// the root of (§14G.4). It has no client and no session.
    TriggerRootedServer,
}

impl ReadContext {
    pub fn describe(self) -> &'static str {
        match self {
            ReadContext::Client => "client context",
            ReadContext::Static => "`static` context",
            ReadContext::ViewRootedServer => "a server derivation rooted at the view",
            ReadContext::TriggerRootedServer => "a server derivation rooted at a trigger",
        }
    }
}

/// What a read of a signal yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadKind {
    /// The signal's own type, unchanged.
    Direct,
    /// `Remote of T` — the network is in the type because the network is
    /// there (§5.2).
    Remote,
    /// The read is not allowed at all, and the message says why.
    Forbidden(&'static str),
}

/// §14G.1.4's read table, transcribed.
///
/// The four rows and five columns are the spec's, in the spec's order.
pub fn read_kind(reader: ReadContext, target: SignalPlacement) -> ReadKind {
    use ReadContext as R;
    use SignalPlacement as P;

    match (reader, target) {
        // view / `client` signal
        (R::Client, P::Client) | (R::Client, P::Static) => ReadKind::Direct,
        (R::Client, P::Server) | (R::Client, P::Durable) | (R::Client, P::DurablePerVisitor) => {
            ReadKind::Remote
        }

        // `static` context
        (R::Static, P::Static) => ReadKind::Direct,
        (R::Static, _) => ReadKind::Forbidden(
            "`static` evaluation happens at build time, when no browser, no invocation and no \
             store exist, so it can only read other `static` state",
        ),

        // `server` / `durable` rooted at the view
        (R::ViewRootedServer, _) => ReadKind::Direct,

        // `server` / `durable` rooted at a trigger
        (R::TriggerRootedServer, P::Client) => ReadKind::Forbidden(
            "a trigger runs with no browser attached, so there is no client state for it to read",
        ),
        (R::TriggerRootedServer, P::DurablePerVisitor) => ReadKind::Forbidden(
            "a trigger runs with no session, so there is no visitor whose partition it could read",
        ),
        (R::TriggerRootedServer, _) => ReadKind::Direct,
    }
}

/// Which [`ReadContext`] each definition's body is checked in.
///
/// **This is the stub.** Functions are colorless (§5.1): a function runs
/// wherever its inputs are, so its context is a property of its callers,
/// not of itself. The real answer is the placement closure `zdc-graph`
/// computes. What this does instead is walk the call graph from the two
/// roots the language currently has — the view, and each signal's own
/// initializer — and record which contexts reach each function.
///
/// That is exact today and no longer will be once triggers (§14G.4) or
/// `static` (§14C.3b) exist, because both add roots this cannot see.
#[derive(Debug, Default)]
pub struct Contexts {
    per_def: HashMap<DefId, HashSet<ReadContext>>,
}

impl Contexts {
    pub fn new(hir: &Hir) -> Contexts {
        let mut contexts = Contexts::default();

        // The roots. A signal's initializer is checked in the context its
        // own placement names; the view and everything under it is client.
        let mut seeds: Vec<(DefId, ReadContext)> = Vec::new();
        for (id, def) in hir.defs.iter() {
            let context = match &def.kind {
                DefKind::View(_) => ReadContext::Client,
                DefKind::Signal(signal) => match signal.placement {
                    zdc_ast::Placement::Client => ReadContext::Client,
                    // No trigger syntax exists, so every server or durable
                    // derivation is rooted at the view.
                    zdc_ast::Placement::Server | zdc_ast::Placement::Durable => {
                        ReadContext::ViewRootedServer
                    }
                },
                // Reached through a call, never as a root.
                DefKind::Function(_) => continue,
                // A type declaration has no body, so nothing runs in it and
                // it is placement-agnostic (§14B.1): a `Todo` is a `Todo`
                // wherever it lives. A `foreign` has no body either, and
                // its own `is client`/`is server`/`is anywhere` says where
                // it may run (§14E.2), so it is never a root.
                DefKind::Record(_) | DefKind::Choice(_) | DefKind::Foreign(_) => continue,
            };
            seeds.push((id, context));
        }

        for (root, context) in seeds {
            contexts.mark(root, context);
            let mut frontier = vec![root];
            let mut seen: HashSet<DefId> = HashSet::from([root]);
            while let Some(id) = frontier.pop() {
                for callee in callees(hir, id) {
                    contexts.mark(callee, context);
                    if seen.insert(callee) {
                        frontier.push(callee);
                    }
                }
            }
        }

        contexts
    }

    fn mark(&mut self, id: DefId, context: ReadContext) {
        self.per_def.entry(id).or_default().insert(context);
    }

    /// The context a definition's body is checked in, or `None` when more
    /// than one reaches it.
    ///
    /// A function reached from two contexts has two read types for the
    /// same expression, which one inferred type cannot hold. Nothing in
    /// the checked-in examples does this; when something does, the answer
    /// is `zdc-graph` splitting the function per placement, not a change
    /// here.
    pub fn of(&self, id: DefId) -> Option<ReadContext> {
        let reached = self.per_def.get(&id)?;
        let mut found = reached.iter();
        let first = *found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Every context that reaches a definition, for the diagnostic that
    /// names them.
    pub fn all(&self, id: DefId) -> Vec<ReadContext> {
        let mut all: Vec<ReadContext> = self
            .per_def
            .get(&id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
        all.sort_by_key(|context| context.describe());
        all
    }
}

/// Every function a definition's body calls, directly.
///
/// Exact: the language has no first-class functions, no dynamic dispatch
/// and no `eval`, so a call edge is a `Res::Def` naming a `function`.
pub(crate) fn called_functions(hir: &Hir, id: DefId, out: &mut Vec<DefId>) {
    out.extend(callees(hir, id));
}

fn callees(hir: &Hir, id: DefId) -> Vec<DefId> {
    let mut found = Vec::new();
    match &hir.defs[id].kind {
        DefKind::Signal(signal) => expr_callees(hir, signal.init, &mut found),
        DefKind::Function(function) => block_callees(hir, function.body, &mut found),
        DefKind::View(view) => nodes_callees(hir, &view.nodes, &mut found),
        DefKind::Record(_) | DefKind::Choice(_) | DefKind::Foreign(_) => {}
    }
    found.retain(|id| matches!(hir.defs[*id].kind, DefKind::Function(_)));
    found
}

fn expr_callees(hir: &Hir, id: zdc_hir::ExprId, found: &mut Vec<DefId>) {
    match &hir.exprs[id].kind {
        HirExprKind::Number(_)
        | HirExprKind::Text(_)
        | HirExprKind::Truth(_)
        | HirExprKind::Empty
        | HirExprKind::Environment(_) => {}
        HirExprKind::List(items) => {
            for item in items {
                expr_callees(hir, *item, found);
            }
        }
        HirExprKind::Map(entries) => {
            for (key, value) in entries {
                expr_callees(hir, *key, found);
                expr_callees(hir, *value, found);
            }
        }
        HirExprKind::Ref(Res::Def(def)) => found.push(*def),
        HirExprKind::Ref(_) => {}
        HirExprKind::Call { callee, args } => {
            if let Res::Def(def) = callee {
                found.push(*def);
            }
            for arg in args {
                expr_callees(hir, arg_expr(arg), found);
            }
        }
        HirExprKind::OfCall { callee, operand } => {
            if let Res::Def(def) = callee {
                found.push(*def);
            }
            expr_callees(hir, *operand, found);
        }
        // A built-in operator's target is a primitive, never a definition
        // with a body, so it adds no call edge.
        HirExprKind::Operator { operand, .. } => expr_callees(hir, *operand, found),
        HirExprKind::Unary { operand, .. } => expr_callees(hir, *operand, found),
        HirExprKind::Binary { lhs, rhs, .. } => {
            expr_callees(hir, *lhs, found);
            expr_callees(hir, *rhs, found);
        }
        HirExprKind::Field { base, .. } => expr_callees(hir, *base, found),
        HirExprKind::Index { base, index } => {
            expr_callees(hir, *base, found);
            expr_callees(hir, *index, found);
        }
        HirExprKind::Append { item, list } => {
            expr_callees(hir, *item, found);
            expr_callees(hir, *list, found);
        }
    }
}

fn block_callees(hir: &Hir, id: BlockId, found: &mut Vec<DefId>) {
    for stmt in &hir.blocks[id].stmts {
        match stmt {
            HirStmt::Pipeline(clause) => match clause {
                HirPipeline::From(expr) | HirPipeline::TakeFirst(expr) => {
                    expr_callees(hir, *expr, found)
                }
                HirPipeline::Keep { cond: expr, .. }
                | HirPipeline::Sort { key: expr, .. }
                | HirPipeline::MapEach { to: expr, .. } => expr_callees(hir, *expr, found),
            },
            HirStmt::Mutation(mutation) => {
                expr_callees(hir, mutation.value(), found);
                for segment in &mutation.place().path {
                    if let HirPathSeg::Index(expr) = segment {
                        expr_callees(hir, *expr, found);
                    }
                }
            }
            HirStmt::Give(expr) => expr_callees(hir, *expr, found),
            HirStmt::Bind(bind) => {
                for binding in &bind.bindings {
                    expr_callees(hir, binding.value, found);
                }
            }
            HirStmt::When(when) => {
                expr_callees(hir, when.scrutinee, found);
                for arm in &when.arms {
                    match &arm.body {
                        HirArmBody::Show(expr) => expr_callees(hir, *expr, found),
                        HirArmBody::Block(block) => block_callees(hir, *block, found),
                    }
                }
            }
            HirStmt::Each(each) => {
                expr_callees(hir, each.iter, found);
                block_callees(hir, each.body, found);
            }
            HirStmt::If(conditional) => {
                expr_callees(hir, conditional.cond, found);
                block_callees(hir, conditional.then, found);
                if let Some(otherwise) = conditional.otherwise {
                    block_callees(hir, otherwise, found);
                }
            }
        }
    }
}

fn nodes_callees(hir: &Hir, nodes: &[HirNode], found: &mut Vec<DefId>) {
    for node in nodes {
        match node {
            HirNode::Element(element) => element_callees(hir, element, found),
            HirNode::Handler(handler) => block_callees(hir, handler.body, found),
            HirNode::Each(each) => {
                expr_callees(hir, each.iter, found);
                nodes_callees(hir, &each.body, found);
            }
            HirNode::When(when) => {
                expr_callees(hir, when.scrutinee, found);
                for arm in &when.arms {
                    match &arm.body {
                        HirNodeArmBody::Show(element) => element_callees(hir, element, found),
                        HirNodeArmBody::Nodes(nodes) => nodes_callees(hir, nodes, found),
                    }
                }
            }
        }
    }
}

fn element_callees(hir: &Hir, element: &HirElement, found: &mut Vec<DefId>) {
    for arg in &element.args {
        expr_callees(hir, arg_expr(arg), found);
    }
    nodes_callees(hir, &element.children, found);
}

fn arg_expr(arg: &HirArg) -> zdc_hir::ExprId {
    match arg {
        HirArg::Positional(expr) => *expr,
        HirArg::Named { value, .. } => *value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_read_table_matches_the_spec_row_for_the_view() {
        assert_eq!(
            read_kind(ReadContext::Client, SignalPlacement::Client),
            ReadKind::Direct
        );
        assert_eq!(
            read_kind(ReadContext::Client, SignalPlacement::Static),
            ReadKind::Direct
        );
        for remote in [
            SignalPlacement::Server,
            SignalPlacement::Durable,
            SignalPlacement::DurablePerVisitor,
        ] {
            assert_eq!(read_kind(ReadContext::Client, remote), ReadKind::Remote);
        }
    }

    #[test]
    fn a_view_rooted_server_derivation_reads_everything_directly() {
        for target in [
            SignalPlacement::Client,
            SignalPlacement::Static,
            SignalPlacement::Server,
            SignalPlacement::Durable,
            SignalPlacement::DurablePerVisitor,
        ] {
            assert_eq!(
                read_kind(ReadContext::ViewRootedServer, target),
                ReadKind::Direct
            );
        }
    }

    #[test]
    fn a_trigger_rooted_derivation_may_not_read_client_or_per_visitor_state() {
        assert!(matches!(
            read_kind(ReadContext::TriggerRootedServer, SignalPlacement::Client),
            ReadKind::Forbidden(_)
        ));
        assert!(matches!(
            read_kind(
                ReadContext::TriggerRootedServer,
                SignalPlacement::DurablePerVisitor
            ),
            ReadKind::Forbidden(_)
        ));
        assert_eq!(
            read_kind(ReadContext::TriggerRootedServer, SignalPlacement::Server),
            ReadKind::Direct
        );
    }

    #[test]
    fn static_context_reads_only_static_state() {
        assert_eq!(
            read_kind(ReadContext::Static, SignalPlacement::Static),
            ReadKind::Direct
        );
        for other in [
            SignalPlacement::Client,
            SignalPlacement::Server,
            SignalPlacement::Durable,
            SignalPlacement::DurablePerVisitor,
        ] {
            assert!(matches!(
                read_kind(ReadContext::Static, other),
                ReadKind::Forbidden(_)
            ));
        }
    }
}
