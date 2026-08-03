//! `reference_sites` — spec §17.2.8.
//!
//! `zdc-codegen`'s `references_of` already walked exactly the right edges:
//! pipelines, node arms, handlers, mutation index expressions. What it did
//! wrong was flatten them to a `Vec<DefId>`, which cannot tell a read from
//! a write from a call — and those are three different crossings. This is
//! the same walk, *classified*.

use zdc_hir::{
    Builtin, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirElement, HirExprKind, HirMutation,
    HirNode, HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, Res,
};
use zdc_lexer::Span;

use crate::root::{MutOp, MutSite, PathKeySeg};

/// One reference, classified. Emitted in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum Site {
    /// A call to a top-level function.
    Call { callee: DefId, span: Span },
    /// A read of a signal. The `ExprId` is the read site, which is what
    /// the crossing is keyed on and what `zdc-types` asks about.
    Read {
        signal: DefId,
        expr: ExprId,
        span: Span,
    },
    /// A mutation whose place base is a signal.
    Write {
        signal: DefId,
        site: MutSite,
        op: MutOp,
        path: Vec<PathKeySeg>,
        span: Span,
    },
    /// A two-way `Input`/`Checkbox` binding: a write on every keystroke,
    /// with no `set` statement to point at.
    Bind {
        signal: DefId,
        site: MutSite,
        span: Span,
    },
    /// A mutation whose place base is not a signal at all — E0314.
    NotAPlace { name: String, span: Span },
    /// `environment "K"`, legal only in `Region::Server` (§5.6) — E0360.
    Environment { span: Span },
}

/// Every reference a definition's own body makes, in source order.
///
/// A signal contributes its initialiser, a function its body, the view its
/// nodes. Nothing here knows about regions: classification is the split's
/// job and needs the context, which is a property of the root rather than
/// of the definition.
pub fn sites_of(hir: &Hir, id: DefId) -> Vec<Site> {
    let mut walk = Walk {
        hir,
        owner: id,
        ordinal: 0,
        out: Vec::new(),
    };
    match &hir.defs[id].kind {
        DefKind::Signal(signal) => walk.expr(signal.init),
        DefKind::Function(function) => walk.block(function.body),
        DefKind::View(view) => walk.nodes(&view.nodes),
    }
    walk.out
}

struct Walk<'a> {
    hir: &'a Hir,
    owner: DefId,
    /// One counter over the whole owner, so a mutation statement and a
    /// two-way binding are addressable by the same kind of identity
    /// (§17.2.5 fatal 2).
    ordinal: u32,
    out: Vec<Site>,
}

impl Walk<'_> {
    fn next_site(&mut self) -> MutSite {
        let site = MutSite {
            owner: self.owner,
            ordinal: self.ordinal,
        };
        self.ordinal += 1;
        site
    }

    fn expr(&mut self, id: ExprId) {
        let span = self.hir.exprs[id].span;
        match &self.hir.exprs[id].kind {
            HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty => {}
            HirExprKind::Environment(_) => self.out.push(Site::Environment { span }),
            HirExprKind::Ref(Res::Def(def)) => {
                // A `Ref` naming a function is "no first-class functions",
                // which `zdc-types` already reports; it contributes no
                // edge here rather than a second diagnostic (§17.2.8).
                if matches!(self.hir.defs[*def].kind, DefKind::Signal(_)) {
                    self.out.push(Site::Read {
                        signal: *def,
                        expr: id,
                        span,
                    });
                }
            }
            HirExprKind::Ref(_) => {}
            HirExprKind::Call { callee, args } => {
                if let Res::Def(def) = callee {
                    if matches!(self.hir.defs[*def].kind, DefKind::Function(_)) {
                        self.out.push(Site::Call { callee: *def, span });
                    }
                }
                let args: Vec<ExprId> = args.iter().map(arg_expr).collect();
                for arg in args {
                    self.expr(arg);
                }
            }
            HirExprKind::Unary { operand, .. } => {
                let operand = *operand;
                self.expr(operand);
            }
            HirExprKind::Binary { lhs, rhs, .. } => {
                let (lhs, rhs) = (*lhs, *rhs);
                self.expr(lhs);
                self.expr(rhs);
            }
            HirExprKind::Field { base, .. } => {
                let base = *base;
                self.expr(base);
            }
            HirExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                self.expr(base);
                self.expr(index);
            }
        }
    }

    fn block(&mut self, id: zdc_hir::BlockId) {
        let stmts = self.hir.blocks[id].stmts.clone();
        for stmt in &stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Pipeline(clause) => match clause {
                HirPipeline::From(expr) | HirPipeline::TakeFirst(expr) => self.expr(*expr),
                HirPipeline::Keep { cond: expr, .. }
                | HirPipeline::Sort { key: expr, .. }
                | HirPipeline::MapEach { to: expr, .. } => self.expr(*expr),
            },
            HirStmt::Give(expr) => self.expr(*expr),
            HirStmt::Mutation(mutation) => self.mutation(mutation),
            HirStmt::When(when) => {
                self.expr(when.scrutinee);
                for arm in &when.arms {
                    match &arm.body {
                        HirArmBody::Show(expr) => self.expr(*expr),
                        HirArmBody::Block(block) => self.block(*block),
                    }
                }
            }
            HirStmt::Each(each) => {
                self.expr(each.iter);
                self.block(each.body);
            }
            HirStmt::If(conditional) => {
                self.expr(conditional.cond);
                self.block(conditional.then);
                if let Some(otherwise) = conditional.otherwise {
                    self.block(otherwise);
                }
            }
        }
    }

    fn mutation(&mut self, mutation: &HirMutation) {
        let (op, place, value) = match mutation {
            HirMutation::Set { place, value } => (MutOp::Set, place, *value),
            HirMutation::Add { place, value } => (MutOp::Incr, place, *value),
            HirMutation::Subtract { place, value } => (MutOp::Decr, place, *value),
        };

        let site = self.next_site();
        match place.base {
            Res::Def(def) if matches!(self.hir.defs[def].kind, DefKind::Signal(_)) => {
                let path = place
                    .path
                    .iter()
                    .map(|segment| match segment {
                        HirPathSeg::Field(name) => PathKeySeg::Field(name.clone()),
                        HirPathSeg::Index(_) => PathKeySeg::Index,
                    })
                    .collect();
                self.out.push(Site::Write {
                    signal: def,
                    site,
                    op,
                    path,
                    span: place.span,
                });
            }
            // §17.2.5's E0314. A parameter is a value rather than a place,
            // and `zdc-codegen` silently dropped this today.
            Res::Local(local) => self.out.push(Site::NotAPlace {
                name: self.hir.locals[local].name.clone(),
                span: place.span,
            }),
            Res::Def(def) => self.out.push(Site::NotAPlace {
                name: self.hir.defs[def].name.clone(),
                span: place.span,
            }),
            Res::Builtin(_) => self.out.push(Site::NotAPlace {
                name: "a built-in".to_string(),
                span: place.span,
            }),
        }

        // The right-hand side and every index expression are ordinary
        // reads in the *enclosing* context, not in the command's — that is
        // §17.2.7's Command rule, and it is what makes `add 1 to visits`
        // compile to `$call('visits.incr', 1)`.
        self.expr(value);
        for segment in &place.path {
            if let HirPathSeg::Index(expr) = segment {
                self.expr(*expr);
            }
        }
    }

    fn nodes(&mut self, nodes: &[HirNode]) {
        for node in nodes {
            match node {
                HirNode::Element(element) => self.element(element),
                HirNode::Each(each) => {
                    self.expr(each.iter);
                    self.nodes(&each.body);
                }
                HirNode::When(when) => {
                    self.expr(when.scrutinee);
                    for arm in &when.arms {
                        match &arm.body {
                            HirNodeArmBody::Show(element) => self.element(element),
                            HirNodeArmBody::Nodes(nodes) => self.nodes(nodes),
                        }
                    }
                }
                HirNode::Handler(handler) => self.block(handler.body),
            }
        }
    }

    fn element(&mut self, element: &HirElement) {
        // Asked of the resolution, not of the spelling (§17.2.2(b)).
        if let Res::Builtin(Builtin::Element(builtin)) = element.res {
            if builtin.is_two_way() {
                if let Some(HirArg::Positional(expr)) = element.args.first() {
                    if let HirExprKind::Ref(Res::Def(def)) = self.hir.exprs[*expr].kind {
                        if matches!(self.hir.defs[def].kind, DefKind::Signal(_)) {
                            let site = self.next_site();
                            self.out.push(Site::Bind {
                                signal: def,
                                site,
                                span: element.span,
                            });
                        }
                    }
                }
            }
        }
        let args: Vec<ExprId> = element.args.iter().map(arg_expr).collect();
        for arg in args {
            self.expr(arg);
        }
        let children = element.children.clone();
        self.nodes(&children);
    }
}

pub fn arg_expr(arg: &HirArg) -> ExprId {
    match arg {
        HirArg::Positional(expr) => *expr,
        HirArg::Named { value, .. } => *value,
    }
}
