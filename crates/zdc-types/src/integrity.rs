//! `trusted` — the integrity direction (spec §18.1).
//!
//! `secret` answers *who may learn this value*. This answers the other
//! question — *who chose it* — and §14G.7.3 records that routing is what
//! forces the language to ask it: "`address` is the language's **first
//! untrusted-input source**."
//!
//! # Why the source set is not configured
//!
//! It is not a list. It is one arm of a classifier the compiler already
//! runs: a route parameter is trusted exactly when it carries an `in`
//! clause, and untrusted exactly when it does not (§18.1 semantics 5).
//!
//! A parameter **with** an `in` clause is trusted because the compiler
//! renders the full URL of every enumerated value and emits one document
//! per URL. A visitor who reaches that document reached one the build
//! host wrote, so the parameter's value is one the build host chose. The
//! match is a *proof*, not a check — which is the one site in this design
//! where an obligation is discharged by a proof rather than by code a
//! person wrote and can forget.
//!
//! A parameter **without** an `in` clause is born untrusted. There is no
//! step at which it is laundered: it is untrusted at the binder, and the
//! only way to make it trusted is to give it an `in`.
//!
//! # What is checked, and what is not
//!
//! Two of §18.1's three authority sites:
//!
//! * **A1** — an index expression over a `trusted` signal. This is what
//!   IDOR actually is, and §18.1.6 names it the useful v1 slot.
//! * **A3** — the value written into a `trusted` place.
//!
//! A2 — an argument to a `foreign` parameter declared `trusted` — needs
//! `foreign`, which this compiler does not have. It is the one site that
//! catches path traversal, and it is missing for that reason and no
//! other.
//!
//! §18.1 semantics 4 asks for something stronger than A3: *every*
//! client-rooted write to a `trusted` place is rejected, because every
//! command argument travels the wire and a browser can send any value it
//! likes. That rule is correct and it is deliberately not enforced here,
//! because the mechanism it names does not exist: this compiler emits no
//! commands and crosses no boundary, so there is no wire for a browser to
//! write to. Enforcing it now would be a rule with no mechanism behind
//! it, and it must be turned on with `Crossing::Command`.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use zdc_hir::{
    BlockId, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirElement, HirExprKind, HirNode,
    HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, LocalId, Res,
};
use zdc_lexer::Span;

use crate::TypeError;

/// Check every integrity obligation in a program.
pub fn check(hir: &Hir) -> Result<(), Vec<TypeError>> {
    let mut pass = Integrity {
        hir,
        untrusted: HashSet::new(),
        trusted_signals: BTreeSet::new(),
        errors: Vec::new(),
    };
    pass.declare();
    pass.run();
    if pass.errors.is_empty() {
        Ok(())
    } else {
        Err(pass.errors)
    }
}

struct Integrity<'a> {
    hir: &'a Hir,
    /// Every binder holding a value a browser chose.
    untrusted: HashSet<LocalId>,
    /// Every signal declared `trusted`.
    trusted_signals: BTreeSet<DefId>,
    errors: Vec<TypeError>,
}

impl Integrity<'_> {
    /// Phase 1 — declare. §17.3's central argument reused verbatim:
    /// declaring rather than inferring is what removes the fixpoint over
    /// the set of writers of a cell (§18.1 semantics 2).
    fn declare(&mut self) {
        for (id, def) in self.hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            if !signal.trusted {
                continue;
            }
            // §18.1 semantics 9. `static` is evaluated on the build host
            // with no browser attached and `client` is owned by the
            // browser outright, so the word is redundant on one and
            // meaningless on the other. Neither needs a rule of its own;
            // both fall out of what the placements are.
            match signal.placement {
                zdc_ast::Placement::Static => self.errors.push(TypeError {
                    message: format!(
                        "`{}` is `static`, and `static` state is already trusted: it is computed \
                         on the build host, where no browser has any part in it. Remove \
                         `trusted`.",
                        def.name
                    ),
                    span: def.span,
                    help: None,
                }),
                zdc_ast::Placement::Client => self.errors.push(TypeError {
                    message: format!(
                        "`{}` is `client`, and `client` state cannot be trusted: it lives in \
                         browser memory, which is the browser's to choose. There is no such thing \
                         as protecting a browser from itself. Declare it `server` or `durable`.",
                        def.name
                    ),
                    span: def.span,
                    help: None,
                }),
                zdc_ast::Placement::Server | zdc_ast::Placement::Durable => {
                    self.trusted_signals.insert(id);
                }
            }
        }
    }

    fn run(&mut self) {
        for (_, def) in self.hir.defs.iter() {
            match &def.kind {
                DefKind::View(view) => self.nodes(&view.nodes, Trust::Trusted),
                DefKind::Function(function) => self.block(function.body, Trust::Trusted),
                DefKind::Signal(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_) => {}
            }
        }
    }

    // --- nodes ---

    fn nodes(&mut self, nodes: &[HirNode], pc: Trust) {
        for node in nodes {
            match node {
                HirNode::Element(element) => self.element(element, pc),
                HirNode::Each(each) => {
                    // A binder ranging over an untrusted list holds
                    // untrusted elements.
                    if self.expr(each.iter).is_untrusted() {
                        self.untrusted.insert(each.var);
                    }
                    self.nodes(&each.body, pc);
                }
                HirNode::When(when) => {
                    let scrutinee = self.expr(when.scrutinee);
                    for arm in &when.arms {
                        self.bind_arm(&arm.pattern_name, &arm.bindings, scrutinee);
                        match &arm.body {
                            HirNodeArmBody::Show(element) => {
                                self.element(element, pc.join(scrutinee))
                            }
                            HirNodeArmBody::Nodes(nodes) => self.nodes(nodes, pc.join(scrutinee)),
                        }
                    }
                }
                HirNode::If(conditional) => {
                    let condition = self.expr(conditional.cond);
                    self.nodes(&conditional.then, pc.join(condition));
                    if let Some(otherwise) = &conditional.otherwise {
                        self.nodes(otherwise, pc.join(condition));
                    }
                }
                HirNode::Scope(scope) => {
                    for local in &scope.locals {
                        if self.expr(local.init).is_untrusted() {
                            self.untrusted.insert(local.local);
                        }
                    }
                    self.nodes(&scope.body, pc);
                }
                HirNode::Handler(handler) => self.block(handler.body, pc),
                HirNode::Children(_) => {}
            }
        }
    }

    fn element(&mut self, element: &HirElement, pc: Trust) {
        for arg in &element.args {
            let expr = match arg {
                HirArg::Positional(expr) => *expr,
                HirArg::Named { value, .. } => *value,
            };
            self.expr(expr);
        }
        self.nodes(&element.children, pc);
    }

    /// Bind a `when` arm's pattern binders.
    ///
    /// This is the whole of the source set. An arm naming a route variant
    /// binds one name per route parameter, in declaration order, and the
    /// parameter's `in` clause decides which side of the lattice its
    /// binder starts on (§18.1 semantics 5).
    fn bind_arm(&mut self, pattern: &str, bindings: &[LocalId], scrutinee: Trust) {
        if let Some((def, table)) = &self.hir.routes {
            if let DefKind::Choice(choice) = &self.hir.defs[*def].kind {
                if let Some(index) = choice
                    .variants
                    .iter()
                    .position(|variant| variant.name == pattern)
                {
                    if let Some(variant) = table.variants.get(index) {
                        for (binder, param) in bindings.iter().zip(&variant.params) {
                            if param.enumerated_in.is_none() {
                                self.untrusted.insert(*binder);
                            }
                        }
                        return;
                    }
                }
            }
        }
        // Any other pattern: a binder is as trusted as what it came out
        // of. `Some with here` over `address` carries the route value
        // through unchanged, and the arm inside it is where the route
        // parameters are actually bound.
        if scrutinee.is_untrusted() {
            for binder in bindings {
                self.untrusted.insert(*binder);
            }
        }
    }

    // --- statements ---

    fn block(&mut self, id: BlockId, pc: Trust) {
        for stmt in &self.hir.blocks[id].stmts {
            match stmt {
                HirStmt::Give(expr) => {
                    self.expr(*expr);
                }
                HirStmt::Pipeline(clause) => {
                    let (var, expr) = match clause {
                        HirPipeline::From(expr) | HirPipeline::TakeFirst(expr) => (None, *expr),
                        HirPipeline::Keep { var, cond } => (Some(*var), *cond),
                        HirPipeline::Sort { var, key } => (Some(*var), *key),
                        HirPipeline::MapEach { var, to } => (Some(*var), *to),
                    };
                    if let Some(var) = var {
                        // The binder is bound before the clause body is
                        // judged, and it holds an element of whatever the
                        // pipeline is carrying.
                        if pc.is_untrusted() {
                            self.untrusted.insert(var);
                        }
                    }
                    self.expr(expr);
                }
                HirStmt::Mutation(mutation) => self.mutation(mutation, pc),
                HirStmt::When(when) => {
                    let scrutinee = self.expr(when.scrutinee);
                    for arm in &when.arms {
                        self.bind_arm(&arm.pattern_name, &arm.bindings, scrutinee);
                        match arm.body {
                            HirArmBody::Show(expr) => {
                                self.expr(expr);
                            }
                            HirArmBody::Block(block) => self.block(block, pc.join(scrutinee)),
                        }
                    }
                }
                HirStmt::Each(each) => {
                    if self.expr(each.iter).is_untrusted() {
                        self.untrusted.insert(each.var);
                    }
                    self.block(each.body, pc);
                }
                HirStmt::If(conditional) => {
                    let condition = self.expr(conditional.cond);
                    self.block(conditional.then, pc.join(condition));
                    if let Some(otherwise) = conditional.otherwise {
                        self.block(otherwise, pc.join(condition));
                    }
                }
            }
        }
    }

    /// The two authority sites that exist in v1.
    fn mutation(&mut self, mutation: &zdc_hir::HirMutation, pc: Trust) {
        let place = mutation.place();
        let value = self.expr(mutation.value());

        let trusted = match place.base {
            Res::Def(def) => self.trusted_signals.contains(&def).then_some(def),
            Res::Local(_) | Res::Builtin(_) | Res::Variant { .. } => None,
        };
        let Some(target) = trusted else {
            // Still walk the index expressions, so a name inside one is
            // classified even when the target is not trusted.
            for segment in &place.path {
                if let HirPathSeg::Index(expr) = segment {
                    self.expr(*expr);
                }
            }
            return;
        };
        let name = self.hir.defs[target].name.clone();

        // A1 — an index over a `trusted` signal. This is IDOR: the key
        // decides *whose* row is written, so a browser choosing the key
        // is a browser choosing the victim.
        for segment in &place.path {
            let HirPathSeg::Index(expr) = segment else {
                continue;
            };
            if self.expr(*expr).is_untrusted() {
                self.errors.push(self.authority(
                    "E-INT-02",
                    format!(
                        "`{name}` is `trusted`, and this indexes it with a value a browser chose. \
                         Which entry is written is then the browser's decision rather than the \
                         program's."
                    ),
                    place.span,
                ));
            }
        }

        // A3 — the value written into a `trusted` place.
        if value.is_untrusted() {
            self.errors.push(self.authority(
                "E-INT-03",
                format!(
                    "`{name}` is `trusted`, and this writes a value a browser chose into it. A \
                     browser must not choose what a trusted place holds."
                ),
                place.span,
            ));
        }

        // §18.1 semantics 11 — implicit flows reuse the pc unchanged.
        if pc.is_untrusted() {
            self.errors.push(self.authority(
                "E-INT-04",
                format!(
                    "`{name}` is `trusted`, and this writes to it under a condition a browser \
                     chose. *Whether* the write happens is then the browser's decision, which is \
                     the same decision as what it holds."
                ),
                place.span,
            ));
        }
    }

    fn authority(&self, code: &str, message: String, span: Span) -> TypeError {
        TypeError {
            message: format!("{code}: {message}"),
            span,
            help: Some(
                "A route parameter is trusted when it carries an `in` naming a `static` signal: \
                 the build renders one document per enumerated value, so reaching the document \
                 proves the value is one the build chose. A parameter with no `in` is untrusted, \
                 and nothing launders it (spec §18.1 semantics 5)."
                    .to_string(),
            ),
        }
    }

    // --- expressions ---

    fn expr(&mut self, id: ExprId) -> Trust {
        match &self.hir.exprs[id].kind {
            HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            // §18.1 semantics 9: the operator set it and the browser had
            // no part in it.
            | HirExprKind::Environment(_) => Trust::Trusted,
            // `address` itself is the URL bar. A program never reads it
            // except to initialise the signal `when` dispatches on, and
            // that dispatch is where its parts are classified.
            HirExprKind::Address => Trust::Untrusted,
            HirExprKind::List(items) => {
                let items = items.clone();
                let mut trust = Trust::Trusted;
                for item in items {
                    trust = trust.join(self.expr(item));
                }
                trust
            }
            HirExprKind::Map(entries) => {
                let entries = entries.clone();
                let mut trust = Trust::Trusted;
                for (key, value) in entries {
                    trust = trust.join(self.expr(key)).join(self.expr(value));
                }
                trust
            }
            HirExprKind::Ref(Res::Local(local)) => {
                if self.untrusted.contains(local) {
                    Trust::Untrusted
                } else {
                    Trust::Trusted
                }
            }
            HirExprKind::Ref(_) => Trust::Trusted,
            HirExprKind::Call { args, .. } => {
                let args = args.clone();
                let mut trust = Trust::Trusted;
                for arg in &args {
                    let expr = match arg {
                        HirArg::Positional(expr) => *expr,
                        HirArg::Named { value, .. } => *value,
                    };
                    trust = trust.join(self.expr(expr));
                }
                // §14E.3 extended, not excepted: a callee's integrity
                // result is the join of its arguments. The only way to
                // raise it is `gives trusted T` on a `foreign`, which is
                // a grant at a conspicuous declaration.
                trust
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
                // The shape of the answer depends on the key as much as
                // on the container, so both join in.
                self.expr(base).join(self.expr(index))
            }
        }
    }
}

/// The lattice: `Trusted ⊑ Untrusted`.
///
/// One join and nothing else is ever done to it, which is what lets the
/// same walk carry a second lattice without branching on a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trust {
    Trusted,
    Untrusted,
}

impl Trust {
    fn join(self, other: Trust) -> Trust {
        match (self, other) {
            (Trust::Trusted, Trust::Trusted) => Trust::Trusted,
            (Trust::Untrusted, _) | (_, Trust::Untrusted) => Trust::Untrusted,
        }
    }

    fn is_untrusted(self) -> bool {
        self == Trust::Untrusted
    }
}

/// Every signal a program declared `trusted`, for whoever reports the
/// program's authority sites.
pub fn trusted_signals(hir: &Hir) -> BTreeMap<DefId, Span> {
    let mut found = BTreeMap::new();
    for (id, def) in hir.defs.iter() {
        if let DefKind::Signal(signal) = &def.kind {
            if signal.trusted {
                found.insert(id, def.span);
            }
        }
    }
    found
}
