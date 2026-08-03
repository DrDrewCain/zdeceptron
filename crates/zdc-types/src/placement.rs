//! The interface between this pass and the placement pass (Plan 4,
//! `zdc-graph`).
//!
//! Placement is not the type checker's job. The *type of a cross-placement
//! read* is, because `Remote of T` is a type (§5.2, amended by §14G.1.4).
//! This module is the whole of the boundary: one table, one question asked
//! of the placement pass, and a stub that answers it from the HIR alone
//! until `zdc-graph` exists.
//!
//! # What `zdc-graph` supplies
//!
//! [`Placements`], and nothing else. It answers, for every definition:
//!
//! * every [`ReadContext`] the definition's body must be checked in —
//!   at most four, one in every current program;
//! * the [`ReadKind`] at each read site, which is §14G.1.4's table
//!   *already applied* rather than a second copy of it.
//!
//! The syntax-driven stub this module used to carry is gone. It was exact
//! for the three placements the grammar has and for the one root the
//! language has, and it could not see a `Lift`, a `Store` or a trigger.

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
    /// §14C.3b. Evaluated once on the build host and inlined into the
    /// bundle: reading it from the browser crosses no boundary, so §5.2's
    /// Rule 1 is satisfied rather than excepted.
    Static,
    Server,
    Durable,
    /// §14G.3a. No `per visitor` syntax exists yet; see `Static`.
    DurablePerVisitor,
}

impl SignalPlacement {
    pub fn from_ast(placement: zdc_ast::Placement) -> SignalPlacement {
        match placement {
            zdc_ast::Placement::Client => SignalPlacement::Client,
            zdc_ast::Placement::Static => SignalPlacement::Static,
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

/// What the placement pass answers — spec §17.1.4.
///
/// The dependency runs one way: **types depend on placement**, never the
/// reverse (§17.1.1). So this is a trait rather than a type imported from
/// `zdc-graph`: the split consults no inference result anywhere, and a
/// crate-level cycle would say otherwise.
///
/// §17.1.4 states the interface as two inherent methods on `TierSplit`.
/// Stating it as a trait here is the same interface with the dependency
/// arrow drawn the way §17.1.1 proves it runs; `zdc-graph` implements it
/// for `TierSplit` and re-exports these names unchanged.
pub trait Placements {
    /// Every context a definition's body must be checked in. Never empty
    /// for a definition that exists: §17.2.6's orphan roots guarantee it.
    fn read_contexts(&self, def: DefId) -> Vec<ReadContext>;

    /// Replaces re-deriving §14G.1.4 inside `Checker::read`. The split
    /// already applied the table; this is a lookup, not a computation.
    fn read_kind_at(&self, expr: zdc_hir::ExprId, context: ReadContext) -> ReadKind;
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
        // Nothing reaches a component declaration: instantiation replaced
        // every call site with the body itself, so its calls are already
        // counted where they landed. A `foreign` has no body to walk.
        DefKind::Component(_) | DefKind::Record(_) | DefKind::Choice(_) | DefKind::Foreign(_) => {}
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
        | HirExprKind::Environment(_)
        // `address` is written by the browser at load, so it reads
        // nothing and calls nothing.
        | HirExprKind::Address => {}
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
            HirNode::If(conditional) => {
                expr_callees(hir, conditional.cond, found);
                nodes_callees(hir, &conditional.then, found);
                if let Some(otherwise) = &conditional.otherwise {
                    nodes_callees(hir, otherwise, found);
                }
            }
            HirNode::Scope(scope) => {
                for local in &scope.locals {
                    expr_callees(hir, local.init, found);
                }
                nodes_callees(hir, &scope.body, found);
            }
            // Replaced by instantiation before this pass runs.
            HirNode::Children(_) => {}
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
