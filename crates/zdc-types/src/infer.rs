//! Inference and checking over the whole program.
//!
//! Hindley–Milner (§5.4) with let-polymorphism, generalising a function
//! after the strongly-connected component it belongs to. Two departures,
//! both forced and both narrow:
//!
//! * A variable carrying a built-in operand constraint is never
//!   generalised. Generalising one would need a qualified type, and §5.4
//!   rules out typeclasses in v1. It is defaulted instead.
//! * A function whose body still has an unresolved `at` is not
//!   generalised either. `at` is overloaded over `List` and `Map`, and
//!   with no container constraint to abstract over, the only way to learn
//!   which one a parameter is, is from the call site.
//!
//! Every error is reported. A walk visits all of a node's children before
//! judging the node, and a failed unification yields `Type::Unknown`,
//! which absorbs everything downstream. Three type errors in one file are
//! three diagnostics from one run.

use std::collections::{HashMap, HashSet};

use zdc_ast::{BinOp, UnaryOp};
use zdc_hir::{
    BlockId, DefId, DefKind, ExprId, Hir, HirArg, HirArm, HirArmBody, HirElement, HirExprKind,
    HirMutation, HirNode, HirNodeArm, HirNodeArmBody, HirPathSeg, HirPipeline, HirPlace, HirStmt,
    LocalId, Res,
};
use zdc_lexer::Span;

use crate::choice::{builtin_choice_of, error_field, Choice, Variant};
use crate::elements::{named_argument, named_argument_is_text, signature, Bound, Slot};
use crate::placement::{read_kind, Contexts, ReadContext, ReadKind, SignalPlacement};
use crate::table::{EmptyKind, IndexKind, TypeTable};
use crate::ty::{Constraint, TyVarId, Type};
use crate::unify::{Mismatch, Solver};
use crate::TypeError;

/// A generalised type: the variables it is polymorphic in, and its shape.
#[derive(Debug, Clone)]
struct Scheme {
    quantified: Vec<TyVarId>,
    ty: Type,
}

impl Scheme {
    fn monomorphic(ty: Type) -> Scheme {
        Scheme {
            quantified: Vec::new(),
            ty,
        }
    }
}

/// An equation that could not be solved when it was written down.
#[derive(Debug, Clone)]
enum Pending {
    /// `base at index`. Which container `base` is decides both the index
    /// type and the result type, and `base` is often a parameter whose
    /// type only the call site knows.
    Index {
        expr: Option<ExprId>,
        base: Type,
        index: Type,
        result: Type,
        /// A place, rather than a value. Reading `at` yields `Option of
        /// T` (§5.4); writing through it does not — see the report.
        lvalue: bool,
        span: Span,
    },
    /// `base.name` where `base` was not yet known.
    Field {
        base: Type,
        name: String,
        result: Type,
        span: Span,
    },
    /// `append E to P` or `remove E from P` where `P` was not yet known.
    ///
    /// §14B.2 makes both collection operations, and `Collection` is a
    /// `List` or a `Map`, so which one decides what the operand means:
    /// appending to a list takes an element, removing from a map takes a
    /// key. Deferred for the same reason `at` is — the place is often a
    /// parameter only the call site pins down.
    Membership {
        verb: &'static str,
        place: Type,
        value: Type,
        span: Span,
        place_span: Span,
    },
    /// A `when` whose scrutinee was not yet known.
    ///
    /// §14F's own example — `function itemOr with maybe, fallback` —
    /// takes apart an `Option` a parameter holds, and only the call site
    /// says which `Option` that is. The arms' bodies are checked when
    /// they are written, against fresh binder variables; which variant
    /// each binder names is settled here.
    When {
        scrutinee: ExprId,
        ty: Type,
        arms: Vec<PendingArm>,
        span: Span,
    },
}

/// One arm of a `when` whose choice is not yet known.
#[derive(Debug, Clone)]
struct PendingArm {
    name: String,
    binders: Vec<(LocalId, Type)>,
    span: Span,
}

/// What a block did.
#[derive(Debug, Clone, Default)]
struct Flow {
    /// Every path through the block reaches a `give`.
    always_gives: bool,
    /// The collection a pipeline in this block produced.
    pipeline: Option<Type>,
}

/// A `when` arm, in whichever position it was written.
struct ArmHead<'a> {
    name: &'a str,
    bindings: &'a [LocalId],
    span: Span,
}

pub(crate) struct Checker<'a> {
    hir: &'a Hir,
    solver: Solver,
    contexts: Contexts,
    errors: Vec<TypeError>,
    table: TypeTable,

    schemes: HashMap<DefId, Scheme>,
    locals: HashMap<LocalId, Type>,

    pending: Vec<Pending>,
    empties: Vec<(ExprId, Type, Span)>,
    /// The type of each field of each named type, invented on first use
    /// and reused after.
    ///
    /// `record` declarations (§14B.1) do not exist, so no record's fields
    /// can be looked up. Sharing one variable per (type, field) is the
    /// most checking available without them: `item.id` means the same
    /// type everywhere in a program even though nothing declared it.
    fields: HashMap<(String, String), Type>,
    /// Every `record` in the program: its fields, in declaration order.
    records: HashMap<String, Vec<(String, Type)>>,
    /// Every `choice` in the program, as the same [`Choice`] the built-ins
    /// produce, so one set of rules governs arms and exhaustiveness.
    choices: HashMap<String, Choice>,

    /// The contexts the body being checked can run in. More than one only
    /// where a colorless function is reached from both, which no example
    /// does and which `zdc-graph` will settle.
    here: Vec<ReadContext>,
    /// The type the enclosing body's `give` must produce.
    result: Type,
}

impl<'a> Checker<'a> {
    pub(crate) fn new(hir: &'a Hir) -> Checker<'a> {
        Checker {
            hir,
            solver: Solver::new(),
            contexts: Contexts::new(hir),
            errors: Vec::new(),
            table: TypeTable::default(),
            schemes: HashMap::new(),
            locals: HashMap::new(),
            pending: Vec::new(),
            empties: Vec::new(),
            fields: HashMap::new(),
            records: HashMap::new(),
            choices: HashMap::new(),
            here: vec![ReadContext::Client],
            result: Type::Unknown,
        }
    }

    pub(crate) fn run(mut self) -> Result<TypeTable, Vec<TypeError>> {
        self.declare_types();
        self.declare_signals();
        self.check_functions();
        self.check_signal_bodies();
        self.check_view();

        self.settle();

        if self.errors.is_empty() {
            Ok(self.table)
        } else {
            Err(self.errors)
        }
    }

    // --- declarations ---

    /// Every `record` and `choice`, before anything that could name one.
    ///
    /// Type declarations are order-independent for the same reason
    /// signals are: a field may name a record declared further down, and a
    /// nominal type is settled by its name rather than by its position.
    fn declare_types(&mut self) {
        let ids: Vec<DefId> = self.hir.defs.iter().map(|(id, _)| id).collect();
        for id in ids {
            let name = self.hir.defs[id].name.clone();
            match self.hir.defs[id].kind.clone() {
                DefKind::Record(record) => {
                    let fields = record
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), self.type_of(&field.ty)))
                        .collect();
                    self.records.insert(name, fields);
                }
                DefKind::Choice(choice) => {
                    let variants = choice
                        .variants
                        .iter()
                        .map(|variant| Variant {
                            name: variant.name.clone(),
                            field_names: variant
                                .fields
                                .iter()
                                .map(|field| field.name.clone())
                                .collect(),
                            fields: variant
                                .fields
                                .iter()
                                .map(|field| self.type_of(&field.ty))
                                .collect(),
                        })
                        .collect();
                    self.choices.insert(
                        name.clone(),
                        Choice {
                            described: name,
                            variants,
                        },
                    );
                }
                _ => {}
            }
        }
    }

    /// The choice a `when` scrutinee eliminates, built-in or declared.
    fn choice_of(&self, ty: &Type) -> Option<Choice> {
        match ty {
            Type::Named(name) => self.choices.get(name).cloned(),
            other => builtin_choice_of(other),
        }
    }

    /// A signal's type is written down, so it is known before any body is
    /// walked. That is what lets two signals read each other.
    fn declare_signals(&mut self) {
        for (id, def) in self.hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            let ty = self.type_of(&signal.ty);
            self.schemes.insert(id, Scheme::monomorphic(ty));
        }
    }

    /// Functions, callees first, generalising each component that can be.
    fn check_functions(&mut self) {
        let order = self.call_graph_components();

        for component in order {
            // Every function in the component gets a monomorphic type
            // first, so a recursive call inside the component sees one.
            for id in &component {
                let DefKind::Function(function) = &self.hir.defs[*id].kind else {
                    continue;
                };
                let params: Vec<Type> = (0..function.params.len())
                    .map(|_| self.solver.fresh())
                    .collect();
                let result = self.solver.fresh();
                for (local, ty) in function.params.iter().zip(params.iter()) {
                    self.locals.insert(*local, ty.clone());
                }
                self.schemes
                    .insert(*id, Scheme::monomorphic(Type::function(params, result)));
            }

            let pending_before = self.pending.len();
            for id in &component {
                self.check_function_body(*id);
            }
            let deferred = self.pending.len() > pending_before;

            if !deferred {
                for id in &component {
                    self.generalize(*id);
                }
            }
        }
    }

    fn check_function_body(&mut self, id: DefId) {
        let DefKind::Function(function) = &self.hir.defs[id].kind else {
            return;
        };
        let body = function.body;
        let Some(Type::Function(_, result)) = self.schemes.get(&id).map(|s| s.ty.clone()) else {
            return;
        };

        self.here = self.body_contexts(id);
        self.result = (*result).clone();

        let flow = self.block(body);

        match flow.pipeline {
            Some(collection) => {
                let span = self.hir.blocks[body].span;
                self.expect(
                    &collection,
                    &self.result.clone(),
                    span,
                    "This pipeline gives",
                );
            }
            None if !flow.always_gives => {
                self.error(
                    format!(
                        "`{}` does not give a value on every path. Every path through a function \
                         must reach a `give`: there is no value in ZDeceptron that stands for \
                         nothing.",
                        self.hir.defs[id].name
                    ),
                    self.hir.defs[id].span,
                );
            }
            None => {}
        }
    }

    fn check_signal_bodies(&mut self) {
        for (id, def) in self.hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            let declared = self.type_of(&signal.ty);

            self.here = self.body_contexts(id);
            self.result = Type::Unknown;

            let found = self.expr(signal.init);
            let span = self.hir.exprs[signal.init].span;
            let what = if signal.is_source {
                format!("`{}` starts as", def.name)
            } else {
                format!("`{}` is derived from", def.name)
            };
            self.expect(&found, &declared, span, &what);
        }
    }

    fn check_view(&mut self) {
        let Some(view) = self.hir.view else {
            return;
        };
        let DefKind::View(view) = &self.hir.defs[view].kind else {
            return;
        };
        self.here = vec![ReadContext::Client];
        self.result = Type::Unknown;
        self.nodes(&view.nodes);
    }

    /// The context or contexts a definition's body runs in.
    ///
    /// A function nothing calls is checked as client code: it is the
    /// least surprising reading, and a function nothing calls has no
    /// cross-placement read to get wrong.
    fn body_contexts(&self, id: DefId) -> Vec<ReadContext> {
        match self.contexts.of(id) {
            Some(context) => vec![context],
            None => {
                let all = self.contexts.all(id);
                if all.is_empty() {
                    vec![ReadContext::Client]
                } else {
                    all
                }
            }
        }
    }

    // --- generalisation ---

    fn generalize(&mut self, id: DefId) {
        let Some(scheme) = self.schemes.get(&id) else {
            return;
        };
        let ty = scheme.ty.clone();

        let mut free = Vec::new();
        self.solver.free_vars(&ty, &mut free);

        // A variable free in something still monomorphic belongs to that
        // thing, not to this scheme. A local does not need checking: the
        // only locals that outlive a body are this function's own
        // parameters, which are already in its own type.
        let mut env: Vec<TyVarId> = Vec::new();
        for (other, scheme) in &self.schemes {
            if *other == id || !scheme.quantified.is_empty() {
                continue;
            }
            self.solver.free_vars(&scheme.ty, &mut env);
        }

        let quantified: Vec<TyVarId> = free
            .into_iter()
            .filter(|var| {
                // A constrained variable would need a qualified type,
                // which §5.4 rules out. It stays monomorphic and is
                // defaulted at the end instead.
                self.solver.constraint_of(*var) == Constraint::Any && !env.contains(var)
            })
            .collect();

        self.schemes.insert(id, Scheme { quantified, ty });
    }

    fn instantiate(&mut self, scheme: &Scheme) -> Type {
        if scheme.quantified.is_empty() {
            return scheme.ty.clone();
        }
        let mapping: HashMap<TyVarId, Type> = scheme
            .quantified
            .iter()
            .map(|var| (*var, self.solver.fresh()))
            .collect();
        self.substitute(&scheme.ty, &mapping)
    }

    fn substitute(&self, ty: &Type, mapping: &HashMap<TyVarId, Type>) -> Type {
        match self.solver.shallow(ty) {
            Type::Var(id) => mapping.get(&id).cloned().unwrap_or(Type::Var(id)),
            Type::List(inner) => Type::list(self.substitute(&inner, mapping)),
            Type::Option(inner) => Type::option(self.substitute(&inner, mapping)),
            Type::Remote(inner) => Type::remote(self.substitute(&inner, mapping)),
            Type::Map(key, value) => Type::map(
                self.substitute(&key, mapping),
                self.substitute(&value, mapping),
            ),
            Type::Function(params, result) => Type::function(
                params
                    .iter()
                    .map(|param| self.substitute(param, mapping))
                    .collect(),
                self.substitute(&result, mapping),
            ),
            settled => settled,
        }
    }

    /// Components of the call graph over functions, callees first.
    fn call_graph_components(&self) -> Vec<Vec<DefId>> {
        let functions: Vec<DefId> = self
            .hir
            .defs
            .iter()
            .filter(|(_, def)| matches!(def.kind, DefKind::Function(_)))
            .map(|(id, _)| id)
            .collect();

        let index: HashMap<DefId, usize> = functions
            .iter()
            .enumerate()
            .map(|(at, id)| (*id, at))
            .collect();

        let edges: Vec<Vec<usize>> = functions
            .iter()
            .map(|id| {
                let mut called = Vec::new();
                crate::placement::called_functions(self.hir, *id, &mut called);
                let mut edges: Vec<usize> = called
                    .into_iter()
                    .filter_map(|callee| index.get(&callee).copied())
                    .collect();
                edges.sort_unstable();
                edges.dedup();
                edges
            })
            .collect();

        components(&edges)
            .into_iter()
            .map(|component| component.into_iter().map(|at| functions[at]).collect())
            .collect()
    }

    // --- statements ---

    fn block(&mut self, id: BlockId) -> Flow {
        let mut flow = Flow::default();
        // The element type the pipeline is currently carrying.
        let mut element: Option<Type> = None;

        let stmts = self.hir.blocks[id].stmts.clone();
        for stmt in &stmts {
            match stmt {
                HirStmt::Pipeline(clause) => self.pipeline(clause, &mut flow, &mut element),
                HirStmt::Mutation(mutation) => self.mutation(mutation),
                HirStmt::Give(expr) => {
                    let found = self.expr(*expr);
                    let want = self.result.clone();
                    let span = self.hir.exprs[*expr].span;
                    self.expect(&found, &want, span, "This `give` gives");
                    flow.always_gives = true;
                }
                HirStmt::When(when) => {
                    let heads: Vec<ArmHead> = when.arms.iter().map(arm_head).collect();
                    let bodies: Vec<&HirArm> = when.arms.iter().collect();
                    let all_give =
                        self.when(
                            when.scrutinee,
                            &heads,
                            when.span,
                            |checker, at| match &bodies[at].body {
                                HirArmBody::Show(expr) => {
                                    let found = checker.expr(*expr);
                                    let want = checker.result.clone();
                                    let span = checker.hir.exprs[*expr].span;
                                    checker.expect(&found, &want, span, "This arm gives");
                                    true
                                }
                                HirArmBody::Block(block) => checker.block(*block).always_gives,
                            },
                        );
                    flow.always_gives |= all_give;
                }
                HirStmt::Each(each) => {
                    let sequence = self.expr(each.iter);
                    let item = self.solver.fresh();
                    let span = self.hir.exprs[each.iter].span;
                    self.expect(&sequence, &Type::list(item.clone()), span, "`each` walks");
                    self.bind(each.var, item);
                    // A list may be empty, so a loop guarantees nothing.
                    self.block(each.body);
                }
                HirStmt::If(conditional) => {
                    let cond = self.expr(conditional.cond);
                    let span = self.hir.exprs[conditional.cond].span;
                    self.expect(&cond, &Type::Truth, span, "An `if` condition is");
                    let then = self.block(conditional.then);
                    let otherwise = conditional.otherwise.map(|block| self.block(block));
                    flow.always_gives |= then.always_gives
                        && otherwise.map(|flow| flow.always_gives).unwrap_or(false);
                }
            }
        }

        flow
    }

    fn pipeline(&mut self, clause: &HirPipeline, flow: &mut Flow, element: &mut Option<Type>) {
        match clause {
            HirPipeline::From(expr) => {
                let source = self.expr(*expr);
                let item = self.solver.fresh();
                let span = self.hir.exprs[*expr].span;
                self.expect(
                    &source,
                    &Type::list(item.clone()),
                    span,
                    "`from` draws from",
                );
                *element = Some(item.clone());
                flow.pipeline = Some(Type::list(item));
            }
            HirPipeline::Keep { var, cond } => {
                if !self.pipeline_binder(*var, element, flow) {
                    self.expr(*cond);
                    return;
                }
                let found = self.expr(*cond);
                let span = self.hir.exprs[*cond].span;
                self.expect(&found, &Type::Truth, span, "The `where` of `keep each` is");
            }
            HirPipeline::Sort { var, key } => {
                if !self.pipeline_binder(*var, element, flow) {
                    self.expr(*key);
                    return;
                }
                // No ordering constraint: §5.4 has no typeclasses, so
                // there is nothing to say "this can be ordered" with.
                self.expr(*key);
            }
            HirPipeline::MapEach { var, to } => {
                if !self.pipeline_binder(*var, element, flow) {
                    self.expr(*to);
                    return;
                }
                let mapped = self.expr(*to);
                *element = Some(mapped.clone());
                flow.pipeline = Some(Type::list(mapped));
            }
            HirPipeline::TakeFirst(count) => {
                let found = self.expr(*count);
                let span = self.hir.exprs[*count].span;
                self.expect(&found, &Type::Whole, span, "`take first` counts");
            }
        }
    }

    /// Bind a pipeline clause's loop name to the element the pipeline is
    /// carrying, or report that there is no pipeline yet.
    fn pipeline_binder(&mut self, var: LocalId, element: &Option<Type>, flow: &mut Flow) -> bool {
        match element {
            Some(item) => {
                self.bind(var, item.clone());
                true
            }
            None => {
                self.bind(var, Type::Unknown);
                self.error(
                    "A pipeline starts with `from`. There is no collection here for this clause \
                     to work on."
                        .to_string(),
                    self.hir.locals[var].span,
                );
                // The body is a pipeline, however broken, so it is not
                // also missing a `give`.
                flow.pipeline = Some(Type::Unknown);
                false
            }
        }
    }

    fn mutation(&mut self, mutation: &HirMutation) {
        match mutation {
            HirMutation::Set { place, value } => {
                let target = self.place(place);
                let found = self.expr(*value);
                let span = self.hir.exprs[*value].span;
                self.expect(&found, &target, span, "This `set` writes");
            }
            HirMutation::Add { value, place } | HirMutation::Subtract { value, place } => {
                let verb = match mutation {
                    HirMutation::Add { .. } => "add",
                    _ => "subtract",
                };
                let target = self.place(place);
                let found = self.expr(*value);
                let span = self.hir.exprs[*value].span;

                // The place is judged first: `add draft to todos` is one
                // mistake — `add` on a collection — and reporting the
                // amount as well would name a consequence as a cause.
                let place_is_numeric = self.demand(
                    &target,
                    Constraint::Numeric,
                    place.span,
                    &format!(
                        "`{verb}` works on numbers only — `append` and `remove` are the \
                         collection forms — and this is"
                    ),
                );
                if !place_is_numeric {
                    return;
                }
                let amount = self.demand(
                    &found,
                    Constraint::Numeric,
                    span,
                    &format!("`{verb}` works on numbers, and this is"),
                );
                if amount {
                    self.expect(&found, &target, span, "This amount is");
                }
            }
            // §14B.2's other half. `append` and `remove` are membership,
            // and the place decides what the operand means: an element for
            // a list, a key for a map.
            HirMutation::Append { value, place } | HirMutation::Remove { value, place } => {
                let verb = match mutation {
                    HirMutation::Append { .. } => "append",
                    _ => "remove",
                };
                let target = self.place(place);
                let found = self.expr(*value);
                let span = self.hir.exprs[*value].span;

                if !self.demand(
                    &target,
                    Constraint::Collection,
                    place.span,
                    &format!(
                        "`{verb}` works on collections only — `add` and `subtract` are the \
                         number forms — and this is"
                    ),
                ) {
                    return;
                }
                let obligation = Pending::Membership {
                    verb,
                    place: target,
                    value: found,
                    span,
                    place_span: place.span,
                };
                if !self.try_membership(&obligation) {
                    self.pending.push(obligation);
                }
            }
        }
    }

    /// The type of somewhere a value can be written.
    ///
    /// A write does not cross a boundary the way a read does: `add 1 to
    /// visits` sends the number, not a `Remote of` anything, so a place
    /// is always the signal's own type. §14G.1.4's table is about reads
    /// and says nothing about this — see the report.
    fn place(&mut self, place: &HirPlace) -> Type {
        let mut current = match place.base {
            Res::Local(local) => self.local(local),
            Res::Def(def) => match &self.hir.defs[def].kind {
                DefKind::Signal(signal) => {
                    if !signal.is_source {
                        self.error(
                            format!(
                                "`{}` is derived with `from`, so nothing can write to it. It is \
                                 recomputed from what it reads.",
                                self.hir.defs[def].name
                            ),
                            place.span,
                        );
                    }
                    self.type_of(&signal.ty)
                }
                _ => {
                    self.error(
                        format!(
                            "`{}` is not somewhere a value can be put.",
                            self.hir.defs[def].name
                        ),
                        place.span,
                    );
                    Type::Unknown
                }
            },
            Res::Variant { .. } => {
                self.error(
                    "A variant is a value, not somewhere a value can be put. Write to the \
                     `state` that holds it."
                        .to_string(),
                    place.span,
                );
                Type::Unknown
            }
            Res::Builtin(_) => Type::Unknown,
        };

        for segment in &place.path {
            current = match segment {
                HirPathSeg::Field(name) => self.field(&current, name, place.span),
                HirPathSeg::Index(index) => {
                    let key = self.expr(*index);
                    self.index(None, &current, &key, true, place.span)
                }
            };
        }
        current
    }

    // --- `when` ---

    /// Check a `when`'s scrutinee, arms and exhaustiveness, calling
    /// `body` for each arm once its binders are bound.
    ///
    /// Returns whether every arm reaches a `give`, which is §16.7 item 7.
    fn when(
        &mut self,
        scrutinee: ExprId,
        arms: &[ArmHead<'_>],
        span: Span,
        mut body: impl FnMut(&mut Self, usize) -> bool,
    ) -> bool {
        let found = self.expr(scrutinee);
        let resolved = self.solver.zonk(&found);
        let scrutinee_span = self.hir.exprs[scrutinee].span;

        // Not a choice, and not yet anything: the scrutinee is a
        // parameter, and only the call site knows what it holds. Check
        // the arms against fresh binders now and settle the variants once
        // the call site has spoken.
        if self.choice_of(&resolved).is_none() && !resolved.is_settled() {
            let mut pending = Vec::with_capacity(arms.len());
            let mut all_give = true;
            for (at, arm) in arms.iter().enumerate() {
                let binders: Vec<(LocalId, Type)> = arm
                    .bindings
                    .iter()
                    .map(|binder| {
                        let ty = self.solver.fresh();
                        self.bind(*binder, ty.clone());
                        (*binder, ty)
                    })
                    .collect();
                let gives = body(self, at);
                self.table.set_arm_gives(arm.span, gives);
                all_give &= gives;
                pending.push(PendingArm {
                    name: arm.name.to_string(),
                    binders,
                    span: arm.span,
                });
            }
            self.pending.push(Pending::When {
                scrutinee,
                ty: found,
                arms: pending,
                span,
            });
            return all_give;
        }

        let Some(choice) = self.choice_of(&resolved) else {
            if !matches!(resolved, Type::Unknown) {
                self.error(
                    format!(
                        "`when` takes apart a choice, and this is `{resolved}`. The choices are \
                         `Option of T` and `Remote of T`."
                    ),
                    scrutinee_span,
                );
            }
            for (at, arm) in arms.iter().enumerate() {
                for binder in arm.bindings {
                    self.bind(*binder, Type::Unknown);
                }
                let gives = body(self, at);
                self.table.set_arm_gives(arm.span, gives);
            }
            return false;
        };

        let binders: Vec<Vec<(LocalId, Type)>> = arms
            .iter()
            .map(|arm| {
                arm.bindings
                    .iter()
                    .map(|binder| (*binder, Type::Unknown))
                    .collect()
            })
            .collect();

        let mut all_give = true;
        for (at, arm) in arms.iter().enumerate() {
            // Bound before the body runs, unlike the deferred path, so
            // the arm sees the field types rather than a variable.
            if let Some(variant) = choice.variant(arm.name) {
                for (at, binder) in arm.bindings.iter().enumerate() {
                    let ty = variant.fields.get(at).cloned().unwrap_or(Type::Unknown);
                    self.bind(*binder, ty);
                }
            } else {
                for binder in arm.bindings {
                    self.bind(*binder, Type::Unknown);
                }
            }
            let gives = body(self, at);
            self.table.set_arm_gives(arm.span, gives);
            all_give &= gives;
        }

        let heads: Vec<PendingArm> = arms
            .iter()
            .zip(binders)
            .map(|(arm, binders)| PendingArm {
                name: arm.name.to_string(),
                binders,
                span: arm.span,
            })
            .collect();
        all_give &= self.match_arms(scrutinee, &choice, &heads, span);
        all_give
    }

    /// Check an arm list against a known choice: every arm names a
    /// variant, binds one name per declared field, appears once, and
    /// between them they cover the choice.
    ///
    /// Returns whether the arms are exhaustive. A binder whose type is
    /// `Unknown` was already bound directly; one that is a variable is
    /// unified with its field here.
    fn match_arms(
        &mut self,
        scrutinee: ExprId,
        choice: &Choice,
        arms: &[PendingArm],
        span: Span,
    ) -> bool {
        let mut matched: HashSet<&str> = HashSet::new();

        for arm in arms {
            let Some(variant) = choice.variant(&arm.name) else {
                self.error(
                    format!(
                        "There is no `{}` in `{}`. Its variants are {}.",
                        arm.name,
                        choice.described,
                        choice.variant_names()
                    ),
                    arm.span,
                );
                continue;
            };

            if !matched.insert(variant.name.as_str()) {
                self.error(
                    format!(
                        "`{}` is matched twice here. The second one can never run.",
                        variant.name
                    ),
                    arm.span,
                );
            }
            if arm.binders.len() != variant.fields.len() {
                self.error(
                    format!(
                        "`{}` has {}, so its pattern binds {}; {} {} bound here. A binder names \
                         the fields in the order they were declared.",
                        variant.name,
                        count(variant.fields.len(), "field"),
                        count(variant.fields.len(), "name"),
                        arm.binders.len(),
                        if arm.binders.len() == 1 { "is" } else { "are" },
                    ),
                    arm.span,
                );
            }
            for (at, (_, ty)) in arm.binders.iter().enumerate() {
                if matches!(ty, Type::Unknown) {
                    continue;
                }
                let field = variant.fields.get(at).cloned().unwrap_or(Type::Unknown);
                self.expect(&field, &ty.clone(), arm.span, "This binder is");
            }
        }

        let missing: Vec<&str> = choice
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .filter(|name| !matched.contains(name))
            .collect();
        let exhaustive = missing.is_empty();
        if !exhaustive {
            let quoted: Vec<String> = missing.iter().map(|name| format!("`{name}`")).collect();
            self.error_with_help(
                format!(
                    "This `when` on `{}` is missing {}. Every arm must be written, in every \
                     context.",
                    choice.described,
                    quoted.join(" and ")
                ),
                span,
                "An arm the compiler can prove unreachable is still written (spec §14G.1.6). \
                 That is what makes a loading state impossible to forget."
                    .to_string(),
            );
        }

        self.table.set_when(scrutinee, choice.clone());
        exhaustive
    }

    // --- view ---

    fn nodes(&mut self, nodes: &[HirNode]) {
        for node in nodes {
            match node {
                HirNode::Element(element) => self.element(element),
                HirNode::Handler(handler) => {
                    let saved = std::mem::replace(&mut self.result, Type::Unknown);
                    self.block(handler.body);
                    self.result = saved;
                }
                HirNode::Each(each) => {
                    let sequence = self.expr(each.iter);
                    let item = self.solver.fresh();
                    let span = self.hir.exprs[each.iter].span;
                    self.expect(&sequence, &Type::list(item.clone()), span, "`each` walks");
                    self.bind(each.var, item);
                    self.nodes(&each.body);
                }
                HirNode::When(when) => {
                    let heads: Vec<ArmHead> = when.arms.iter().map(node_arm_head).collect();
                    let bodies: Vec<&HirNodeArm> = when.arms.iter().collect();
                    self.when(when.scrutinee, &heads, when.span, |checker, at| {
                        match &bodies[at].body {
                            HirNodeArmBody::Show(element) => checker.element(element),
                            HirNodeArmBody::Nodes(nodes) => checker.nodes(nodes),
                        }
                        // A view arm produces nodes, not a value.
                        true
                    });
                }
            }
        }
    }

    fn element(&mut self, element: &HirElement) {
        let Some(signature) = signature(&element.name) else {
            for arg in &element.args {
                self.expr(arg_expr(arg));
            }
            self.nodes(&element.children);
            return;
        };

        let positional: Vec<ExprId> = element
            .args
            .iter()
            .filter_map(|arg| match arg {
                HirArg::Positional(expr) => Some(*expr),
                HirArg::Named { .. } => None,
            })
            .collect();

        match signature.slot {
            Slot::None => {
                for expr in &positional {
                    self.expr(*expr);
                    self.error(
                        format!(
                            "`{}` takes no leading value. Everything it shows comes from a named \
                             argument.",
                            element.name
                        ),
                        self.hir.exprs[*expr].span,
                    );
                }
            }
            Slot::Shown { required } => {
                if positional.is_empty() && required {
                    self.error(
                        format!("`{}` needs a value to show.", element.name),
                        element.span,
                    );
                }
                for (at, expr) in positional.iter().enumerate() {
                    let found = self.expr(*expr);
                    if at > 0 {
                        self.error(
                            format!(
                                "`{}` shows one leading value; the rest of its arguments are \
                                 named.",
                                element.name
                            ),
                            self.hir.exprs[*expr].span,
                        );
                        continue;
                    }
                    self.demand(
                        &found,
                        Constraint::Shown,
                        self.hir.exprs[*expr].span,
                        &format!("`{}` shows text, and this is", element.name),
                    );
                }
            }
            Slot::Bound(bound) => {
                let want = match bound {
                    Bound::Text => Type::Text,
                    Bound::Truth => Type::Truth,
                };
                match positional.first() {
                    None => self.error(
                        format!(
                            "`{}` binds two ways, so it needs the `state` it reads and writes.",
                            element.name
                        ),
                        element.span,
                    ),
                    Some(expr) => {
                        let found = self.expr(*expr);
                        let span = self.hir.exprs[*expr].span;
                        // What it binds to is judged first: binding
                        // `durable` state makes the read `Remote of T`,
                        // and reporting that too would name a
                        // consequence as a second mistake.
                        if self.check_two_way(*expr, &element.name, span) {
                            self.expect(
                                &found,
                                &want,
                                span,
                                &format!("`{}` binds to", element.name),
                            );
                        }
                    }
                }
                for expr in positional.iter().skip(1) {
                    self.expr(*expr);
                    self.error(
                        format!("`{}` binds one value.", element.name),
                        self.hir.exprs[*expr].span,
                    );
                }
            }
        }

        let mut named_seen: HashSet<&str> = HashSet::new();
        for arg in &element.args {
            let HirArg::Named { name, value } = arg else {
                continue;
            };
            named_seen.insert(name.as_str());
            let found = self.expr(*value);
            let span = self.hir.exprs[*value].span;
            if named_argument_is_text(name) {
                self.expect(&found, &Type::Text, span, &format!("`{name}` is"));
            } else {
                self.demand(&found, named_argument(name), span, &format!("`{name}` is"));
            }
        }

        if let Some(required) = signature.required_named {
            if !named_seen.contains(required) {
                self.error(
                    format!(
                        "`{}` needs `{required} is …`; that is where its text comes from.",
                        element.name
                    ),
                    element.span,
                );
            }
        }

        self.nodes(&element.children);
    }

    /// §14B.5: the input elements bind bidirectionally, and the signal
    /// they bind must be a `client` source. Binding a remote one would
    /// make every keystroke a network write.
    fn check_two_way(&mut self, expr: ExprId, element: &str, span: Span) -> bool {
        let HirExprKind::Ref(Res::Def(def)) = self.hir.exprs[expr].kind else {
            self.error(
                format!(
                    "`{element}` writes back into what it is given, so it must be given a `state` \
                     signal rather than a computed value."
                ),
                span,
            );
            return false;
        };
        let DefKind::Signal(signal) = &self.hir.defs[def].kind else {
            self.error(format!("`{element}` must be given a `state` signal."), span);
            return false;
        };
        if !signal.is_source {
            self.error(
                format!(
                    "`{}` is derived with `from`, so `{element}` cannot write back into it.",
                    self.hir.defs[def].name
                ),
                span,
            );
            return false;
        }
        if signal.placement != zdc_ast::Placement::Client {
            self.error_with_help(
                format!(
                    "`{}` is `{}`-placed, and `{element}` writes back on every keystroke.",
                    self.hir.defs[def].name,
                    SignalPlacement::from_ast(signal.placement).describe()
                ),
                span,
                "Bind a `client` signal here and write the remote one from a handler, so the \
                 round trip is visible in the source (spec §14B.5)."
                    .to_string(),
            );
            return false;
        }
        true
    }

    // --- expressions ---

    fn expr(&mut self, id: ExprId) -> Type {
        let span = self.hir.exprs[id].span;
        let ty = match &self.hir.exprs[id].kind {
            // §14A.3 makes both numeric types f64, so an integer literal
            // is left to whichever the context wants and defaults to
            // `Whole`. A literal with a fraction can only be `Decimal`.
            HirExprKind::Number(value) => {
                if value.fract() == 0.0 {
                    self.solver.fresh_constrained(Constraint::Numeric)
                } else {
                    Type::Decimal
                }
            }
            HirExprKind::Text(_) => Type::Text,
            HirExprKind::Truth(_) => Type::Truth,
            // §14B.4. `[]` is the empty list and needs no annotation to be
            // one; what it is a list *of* still comes from context, exactly
            // as `[1, 2]` gets `List of Whole` from its elements.
            HirExprKind::List(items) => {
                let items = items.clone();
                let element = self.solver.fresh();
                for item in items {
                    let found = self.expr(item);
                    let span = self.hir.exprs[item].span;
                    self.expect(&found, &element, span, "This list holds");
                }
                Type::list(element)
            }
            HirExprKind::Map(entries) => {
                let entries = entries.clone();
                let key = self.solver.fresh();
                let value = self.solver.fresh();
                for (at, entry) in entries {
                    let found = self.expr(at);
                    self.expect(
                        &found,
                        &key,
                        self.hir.exprs[at].span,
                        "This map is keyed by",
                    );
                    let found = self.expr(entry);
                    self.expect(&found, &value, self.hir.exprs[entry].span, "This map holds");
                }
                Type::map(key, value)
            }
            HirExprKind::Empty => {
                let ty = self.solver.fresh_constrained(Constraint::Collection);
                self.empties.push((id, ty.clone(), span));
                ty
            }
            // `environment` reads a process environment variable, which
            // is text everywhere. The spec never says so — see the report.
            HirExprKind::Environment(_) => Type::Text,
            HirExprKind::Ref(res) => {
                let res = *res;
                self.read(res, span)
            }
            HirExprKind::Call { callee, args } => {
                let callee = *callee;
                let args = args.clone();
                self.call(callee, &args, span)
            }
            HirExprKind::Unary { op, operand } => {
                let (op, operand) = (*op, *operand);
                let found = self.expr(operand);
                let span = self.hir.exprs[operand].span;
                match op {
                    UnaryOp::Not => {
                        self.expect(&found, &Type::Truth, span, "`not` asks");
                        Type::Truth
                    }
                    UnaryOp::Neg => {
                        self.demand(
                            &found,
                            Constraint::Numeric,
                            span,
                            "Negation needs a number, and this is",
                        );
                        found
                    }
                }
            }
            HirExprKind::Binary { op, lhs, rhs } => {
                let (op, lhs, rhs) = (*op, *lhs, *rhs);
                self.binary(op, lhs, rhs)
            }
            HirExprKind::Field { base, name } => {
                let (base, name) = (*base, name.clone());
                let found = self.expr(base);
                self.field(&found, &name, span)
            }
            HirExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                let container = self.expr(base);
                let key = self.expr(index);
                self.index(Some(id), &container, &key, false, span)
            }
        };
        self.table.set_expr(id, ty.clone());
        ty
    }

    /// §14G.1.4's read table, applied.
    fn read(&mut self, res: Res, span: Span) -> Type {
        match res {
            Res::Local(local) => self.local(local),
            Res::Def(def) => match &self.hir.defs[def].kind {
                DefKind::Signal(signal) => {
                    let value = self.type_of(&signal.ty);
                    let target = SignalPlacement::from_ast(signal.placement);
                    let kinds: Vec<ReadKind> = self
                        .here
                        .iter()
                        .map(|context| read_kind(*context, target))
                        .collect();

                    let Some(first) = kinds.first().cloned() else {
                        return value;
                    };
                    if kinds.iter().any(|kind| *kind != first) {
                        let contexts: Vec<&str> =
                            self.here.iter().map(|context| context.describe()).collect();
                        self.error(
                            format!(
                                "`{}` is read from {}, and it does not mean the same thing in \
                                 both. Splitting the function that reads it is the placement \
                                 pass's decision, not this one's.",
                                self.hir.defs[def].name,
                                contexts.join(" and ")
                            ),
                            span,
                        );
                        return Type::Unknown;
                    }

                    match first {
                        ReadKind::Direct => value,
                        ReadKind::Remote => Type::remote(value),
                        ReadKind::Forbidden(why) => {
                            self.error(
                                format!(
                                    "`{}` is `{}` state and cannot be read here: {why}.",
                                    self.hir.defs[def].name,
                                    target.describe()
                                ),
                                span,
                            );
                            Type::Unknown
                        }
                    }
                }
                DefKind::Function(function) => {
                    let name = self.hir.defs[def].name.clone();
                    // A call is written `name with …` (§4.4), so a
                    // function that declares no parameters has no call
                    // form at all. Saying "call it" would be advice that
                    // cannot be followed.
                    let message = if function.params.is_empty() {
                        format!(
                            "`{name}` is a function, and ZDeceptron has no first-class functions, \
                             so it cannot be used as a value. A call is written `{name} with …`, \
                             and `{name}` declares no parameters, so give it one."
                        )
                    } else {
                        format!(
                            "`{name}` is a function, and ZDeceptron has no first-class functions, \
                             so it cannot be used as a value. Call it with `{name} with …`."
                        )
                    };
                    self.error(message, span);
                    Type::Unknown
                }
                DefKind::View(_) => Type::Unknown,
                DefKind::Record(_) => {
                    let name = self.hir.defs[def].name.clone();
                    self.error(
                        format!(
                            "`{name}` is a record, so it names a shape rather than a value. \
                             Build one by naming its fields: `{name} with …`."
                        ),
                        span,
                    );
                    Type::Unknown
                }
                DefKind::Choice(choice) => {
                    let names: Vec<String> = choice
                        .variants
                        .iter()
                        .map(|variant| variant.name.clone())
                        .collect();
                    let name = self.hir.defs[def].name.clone();
                    self.error(
                        format!(
                            "`{name}` is a choice, so it names a set of variants rather than a \
                             value. Write one of {}.",
                            english_list(&names)
                        ),
                        span,
                    );
                    Type::Unknown
                }
            },
            Res::Variant { choice, index } => match self.variant_of(choice, index) {
                Some((choice_name, variant)) if variant.fields.is_empty() => {
                    Type::Named(choice_name)
                }
                Some((choice_name, variant)) => {
                    let written: Vec<String> = variant
                        .field_names
                        .iter()
                        .map(|field| format!("{field} is …"))
                        .collect();
                    self.error(
                        format!(
                            "`{}` of `{choice_name}` carries {}, so it is built by naming them: \
                             `{} with {}`.",
                            variant.name,
                            count(variant.fields.len(), "field"),
                            variant.name,
                            written.join(", ")
                        ),
                        span,
                    );
                    Type::Unknown
                }
                None => Type::Unknown,
            },
            Res::Builtin(_) => Type::Unknown,
        }
    }

    /// The choice's name and the variant at `index`, as declared.
    fn variant_of(&self, choice: DefId, index: u32) -> Option<(String, Variant)> {
        let name = self.hir.defs[choice].name.clone();
        let declared = self.choices.get(&name)?;
        let variant = declared.variants.get(index as usize)?.clone();
        Some((name, variant))
    }

    fn call(&mut self, callee: Res, args: &[HirArg], span: Span) -> Type {
        // `Todo with title is "x"` and `politeGreeting with name` are the
        // same production (§4.4), so which one this is comes off the
        // definition rather than off the syntax.
        if let Res::Variant { choice, index } = callee {
            let Some((choice_name, variant)) = self.variant_of(choice, index) else {
                return Type::Unknown;
            };
            let fields: Vec<(String, Type)> = variant
                .field_names
                .iter()
                .cloned()
                .zip(variant.fields.iter().cloned())
                .collect();
            self.construct(&variant.name, &fields, args, span);
            return Type::Named(choice_name);
        }
        let Res::Def(def) = callee else {
            for arg in args {
                self.expr(arg_expr(arg));
            }
            self.error(
                "Only a top-level `function` can be called; ZDeceptron has no first-class \
                 functions."
                    .to_string(),
                span,
            );
            return Type::Unknown;
        };
        if matches!(self.hir.defs[def].kind, DefKind::Record(_)) {
            let name = self.hir.defs[def].name.clone();
            let fields = self.records.get(&name).cloned().unwrap_or_default();
            self.construct(&name, &fields, args, span);
            return Type::Named(name);
        }
        let DefKind::Function(function) = &self.hir.defs[def].kind else {
            for arg in args {
                self.expr(arg_expr(arg));
            }
            self.error(
                format!("`{}` is not a function.", self.hir.defs[def].name),
                span,
            );
            return Type::Unknown;
        };

        let names: Vec<String> = function
            .params
            .iter()
            .map(|param| self.hir.locals[*param].name.clone())
            .collect();
        let name = self.hir.defs[def].name.clone();

        let scheme = self.schemes.get(&def).cloned();
        let Some(scheme) = scheme else {
            for arg in args {
                self.expr(arg_expr(arg));
            }
            return Type::Unknown;
        };
        let signature = self.instantiate(&scheme);
        let Type::Function(params, result) = signature else {
            for arg in args {
                self.expr(arg_expr(arg));
            }
            return Type::Unknown;
        };

        // Every argument is visited before the call is judged, so two
        // mistakes in one call are two diagnostics.
        let mut slots: Vec<Option<(ExprId, Type)>> = vec![None; params.len()];
        let mut next = 0usize;
        for arg in args {
            let expr = arg_expr(arg);
            let found = self.expr(expr);
            match arg {
                HirArg::Positional(_) => {
                    if next >= slots.len() {
                        self.error(
                            format!(
                                "`{name}` takes {}, and this call passes more.",
                                count(params.len(), "argument")
                            ),
                            self.hir.exprs[expr].span,
                        );
                        continue;
                    }
                    slots[next] = Some((expr, found));
                    next += 1;
                }
                HirArg::Named { name: given, .. } => {
                    match names.iter().position(|param| param == given) {
                        Some(at) => slots[at] = Some((expr, found)),
                        None => self.error(
                            format!(
                                "`{name}` has no parameter named `{given}`. Its parameters are {}.",
                                english_list(&names)
                            ),
                            self.hir.exprs[expr].span,
                        ),
                    }
                }
            }
        }

        for (at, slot) in slots.iter().enumerate() {
            match slot {
                Some((expr, found)) => {
                    let span = self.hir.exprs[*expr].span;
                    let what = format!("`{}` of `{name}` is", names[at]);
                    self.expect(found, &params[at], span, &what);
                }
                None => self.error(
                    format!("`{name}` is missing an argument for `{}`.", names[at]),
                    span,
                ),
            }
        }

        (*result).clone()
    }

    /// A record literal or a variant with a payload.
    ///
    /// §14G.1.2: **construction is by name.** Every field is written once,
    /// in any order, and a missing or unknown one is reported by name —
    /// which is why a positional argument is refused rather than matched up
    /// silently with whatever field happens to come first.
    fn construct(&mut self, owner: &str, fields: &[(String, Type)], args: &[HirArg], span: Span) {
        let names: Vec<String> = fields.iter().map(|(name, _)| name.clone()).collect();
        let mut given: Vec<Option<ExprId>> = vec![None; fields.len()];

        // Every argument is visited before the value is judged, so two
        // mistakes in one literal are two diagnostics.
        for arg in args {
            let expr = arg_expr(arg);
            let found = self.expr(expr);
            let arg_span = self.hir.exprs[expr].span;
            match arg {
                HirArg::Positional(_) => {
                    self.error(
                        format!(
                        "`{owner}` is built by naming its fields, so write `{} is …`. Its fields \
                         are {}.",
                        names.first().cloned().unwrap_or_else(|| "field".to_string()),
                        english_list(&names)
                    ),
                        arg_span,
                    )
                }
                HirArg::Named { name, .. } => match names.iter().position(|field| field == name) {
                    Some(at) => {
                        if given[at].is_some() {
                            self.error(
                                format!("`{name}` is given twice here. A field is named once."),
                                arg_span,
                            );
                        }
                        given[at] = Some(expr);
                        self.expect(
                            &found,
                            &fields[at].1.clone(),
                            arg_span,
                            &format!("`{name}` of `{owner}` is"),
                        );
                    }
                    None => self.error(
                        format!(
                            "`{owner}` has no field named `{name}`. Its fields are {}.",
                            english_list(&names)
                        ),
                        arg_span,
                    ),
                },
            }
        }

        let missing: Vec<String> = given
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.is_none())
            .map(|(at, _)| names[at].clone())
            .collect();
        if !missing.is_empty() {
            self.error(
                format!(
                    "`{owner}` is missing {}. Every field is given a value, because there is no \
                     value in ZDeceptron that stands for nothing.",
                    english_list(&missing)
                ),
                span,
            );
        }
    }

    fn binary(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId) -> Type {
        let left = self.expr(lhs);
        let right = self.expr(rhs);
        let left_span = self.hir.exprs[lhs].span;
        let right_span = self.hir.exprs[rhs].span;
        let span = Span::new(left_span.start, right_span.end);

        match op {
            BinOp::And | BinOp::Or => {
                let word = if op == BinOp::And { "and" } else { "or" };
                self.expect(&left, &Type::Truth, left_span, &format!("`{word}` joins"));
                self.expect(&right, &Type::Truth, right_span, &format!("`{word}` joins"));
                Type::Truth
            }
            // §16.7 item 2: `===` is value equality for a base type and
            // reference equality for everything else, so codegen needs the
            // operand type — recorded, not restricted.
            BinOp::Is | BinOp::IsNot => {
                let word = if op == BinOp::Is { "is" } else { "is not" };
                self.expect(
                    &right,
                    &left,
                    span,
                    &format!("`{word}` compares two values of one type, and the right side is"),
                );
                Type::Truth
            }
            BinOp::Less | BinOp::Greater | BinOp::LessEq | BinOp::GreaterEq => {
                let what = "Comparison orders numbers, and this is";
                let ok = self.demand(&left, Constraint::Numeric, left_span, what)
                    & self.demand(&right, Constraint::Numeric, right_span, what);
                if ok {
                    self.expect(&right, &left, span, "The right side of this comparison is");
                }
                Type::Truth
            }
            // §16.7 item 1: `+` is addition or concatenation, never both
            // at once, so the operands must agree on which.
            BinOp::Add => {
                let what = "`+` joins numbers or text, and this is";
                let ok = self.demand(&left, Constraint::Addable, left_span, what)
                    & self.demand(&right, Constraint::Addable, right_span, what);
                if !ok {
                    return Type::Unknown;
                }
                self.expect(&right, &left, span, "The right side of this `+` is");
                left
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div => {
                let word = match op {
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    _ => "/",
                };
                let what = format!("`{word}` works on numbers, and this is");
                let ok = self.demand(&left, Constraint::Numeric, left_span, &what)
                    & self.demand(&right, Constraint::Numeric, right_span, &what);
                if !ok {
                    return Type::Unknown;
                }
                self.expect(
                    &right,
                    &left,
                    span,
                    &format!("The right side of this `{word}` is"),
                );
                left
            }
        }
    }

    fn field(&mut self, base: &Type, name: &str, span: Span) -> Type {
        let resolved = self.solver.shallow(base);
        match resolved {
            Type::Unknown => Type::Unknown,
            Type::Error => match error_field(name) {
                Some(ty) => ty,
                None => {
                    self.error(
                        format!("An `Error` has no `{name}`. It carries `message`."),
                        span,
                    );
                    Type::Unknown
                }
            },
            Type::Named(ref type_name) => {
                if let Some(fields) = self.records.get(type_name) {
                    return match fields.iter().find(|(field, _)| field == name) {
                        Some((_, ty)) => ty.clone(),
                        None => {
                            let names: Vec<String> =
                                fields.iter().map(|(field, _)| field.clone()).collect();
                            self.error(
                                format!(
                                    "`{type_name}` has no field named `{name}`. Its fields are \
                                     {}.",
                                    english_list(&names)
                                ),
                                span,
                            );
                            Type::Unknown
                        }
                    };
                }
                if let Some(choice) = self.choices.get(type_name) {
                    let variants = choice.variant_names();
                    self.error(
                        format!(
                            "`{type_name}` is a choice, so it is taken apart with `when` rather \
                             than read from. Its variants are {variants}."
                        ),
                        span,
                    );
                    return Type::Unknown;
                }
                // A type name nothing declares. Sharing one variable per
                // (type, field) is the most checking available without a
                // declaration: `item.id` means the same type everywhere in
                // a program even though nothing wrote it down.
                let key = (type_name.clone(), name.to_string());
                match self.fields.get(&key) {
                    Some(ty) => ty.clone(),
                    None => {
                        let ty = self.solver.fresh();
                        self.fields.insert(key, ty.clone());
                        ty
                    }
                }
            }
            Type::Var(_) => {
                let result = self.solver.fresh();
                self.pending.push(Pending::Field {
                    base: resolved,
                    name: name.to_string(),
                    result: result.clone(),
                    span,
                });
                result
            }
            other => {
                self.error(
                    format!("`{other}` has no fields, so there is no `{name}` to read."),
                    span,
                );
                Type::Unknown
            }
        }
    }

    fn index(
        &mut self,
        expr: Option<ExprId>,
        container: &Type,
        key: &Type,
        lvalue: bool,
        span: Span,
    ) -> Type {
        if !self.demand(container, Constraint::Collection, span, "`at` indexes") {
            return Type::Unknown;
        }
        let result = self.solver.fresh();
        let obligation = Pending::Index {
            expr,
            base: container.clone(),
            index: key.clone(),
            result: result.clone(),
            lvalue,
            span,
        };
        // Solved straight away when the container is already known, which
        // it usually is; deferred only when it is a parameter the call
        // site has not pinned down yet.
        if !self.try_index(&obligation) {
            self.pending.push(obligation);
        }
        result
    }

    // --- deferred equations ---

    /// Solve everything that could not be solved when it was written.
    fn settle(&mut self) {
        self.drain_pending();
        self.solver.default_unconstrained();
        self.drain_pending();
        self.report_unsolved();
        self.check_empties();
        self.fill_table();
    }

    fn drain_pending(&mut self) {
        loop {
            let before = self.pending.len();
            let pending = std::mem::take(&mut self.pending);
            let mut still = Vec::new();
            for obligation in pending {
                let solved = match &obligation {
                    Pending::Index { .. } => self.try_index(&obligation),
                    Pending::Field { .. } => self.try_field(&obligation),
                    Pending::Membership { .. } => self.try_membership(&obligation),
                    Pending::When { .. } => self.try_when(&obligation),
                };
                if !solved {
                    still.push(obligation);
                }
            }
            self.pending = still;
            if self.pending.len() == before {
                return;
            }
        }
    }

    fn try_index(&mut self, obligation: &Pending) -> bool {
        let Pending::Index {
            expr,
            base,
            index,
            result,
            lvalue,
            span,
        } = obligation
        else {
            return false;
        };

        let (kind, key, value) = match self.solver.shallow(base) {
            Type::List(item) => (IndexKind::List, Type::Whole, (*item).clone()),
            Type::Map(key, value) => (IndexKind::Map, (*key).clone(), (*value).clone()),
            Type::Unknown => return true,
            _ => return false,
        };

        if let Some(id) = expr {
            self.table.set_index(*id, kind);
        }

        let what = match kind {
            IndexKind::List => "A list is indexed by position, and this index is",
            IndexKind::Map => "This map key is",
        };
        self.expect(&index.clone(), &key, *span, what);

        // §5.4: reading through an index is bounds-checked, so it gives
        // `Option of T`. Writing through one does not — see the report.
        let produced = if *lvalue { value } else { Type::option(value) };
        self.expect(&produced, &result.clone(), *span, "`at` gives");
        true
    }

    /// `append`/`remove` once the collection is known.
    ///
    /// A list is a sequence, so both take an element; a map is keyed, so
    /// `remove` takes a key and `append` has no meaning at all — a map
    /// entry cannot be added without saying where, which is what `set … at`
    /// is for.
    fn try_membership(&mut self, obligation: &Pending) -> bool {
        let Pending::Membership {
            verb,
            place,
            value,
            span,
            place_span,
        } = obligation
        else {
            return false;
        };

        match self.solver.shallow(place) {
            Type::List(item) => {
                let what = format!("`{verb}` works on the elements of this list, and this is");
                self.expect(&value.clone(), &item, *span, &what);
            }
            Type::Map(key, _) if *verb == "remove" => {
                self.expect(
                    &value.clone(),
                    &key,
                    *span,
                    "`remove` takes the key of the entry to drop, and this is",
                );
            }
            Type::Map(key, value) => self.error(
                format!(
                    "`append` adds to the end of a list, and this is `{}`. A map entry needs a \
                     key, so write `set … at <key> to <value>` instead.",
                    Type::map((*key).clone(), (*value).clone())
                ),
                *place_span,
            ),
            Type::Unknown => {}
            _ => return false,
        }
        true
    }

    fn try_field(&mut self, obligation: &Pending) -> bool {
        let Pending::Field {
            base,
            name,
            result,
            span,
        } = obligation
        else {
            return false;
        };
        if matches!(self.solver.shallow(base), Type::Var(_)) {
            return false;
        }
        let found = self.field(&base.clone(), name, *span);
        self.expect(&found, &result.clone(), *span, "This field");
        true
    }

    fn try_when(&mut self, obligation: &Pending) -> bool {
        let Pending::When {
            scrutinee,
            ty,
            arms,
            span,
        } = obligation
        else {
            return false;
        };
        let resolved = self.solver.zonk(ty);
        if !resolved.is_settled() {
            return false;
        }
        match self.choice_of(&resolved) {
            Some(choice) => {
                self.match_arms(*scrutinee, &choice, arms, *span);
            }
            None if matches!(resolved, Type::Unknown) => {}
            None => self.error(
                format!(
                    "`when` takes apart a choice, and this is `{resolved}`. The choices are \
                     `Option of T` and `Remote of T`."
                ),
                self.hir.exprs[*scrutinee].span,
            ),
        }
        true
    }

    fn report_unsolved(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        for obligation in pending {
            match obligation {
                Pending::Index { span, .. } => self.error(
                    "`at` needs to know whether this is a list or a map, and nothing in the \
                     program says which. Give the state or parameter it comes from a written \
                     type."
                        .to_string(),
                    span,
                ),
                Pending::Membership {
                    verb, place_span, ..
                } => self.error(
                    format!(
                        "`{verb}` needs to know whether this is a list or a map, and nothing in \
                         the program says which. Give the state or parameter it comes from a \
                         written type."
                    ),
                    place_span,
                ),
                Pending::When { scrutinee, .. } => self.error(
                    "The type here is not known, so `when` cannot tell which variants it has. \
                     Give the state or parameter it comes from a written type."
                        .to_string(),
                    self.hir.exprs[scrutinee].span,
                ),
                // A field of a type nothing declared. `record` (§14B.1)
                // does not exist, so there is no declaration this could
                // be checked against and reporting it would be noise.
                Pending::Field { .. } => {}
            }
        }
    }

    fn check_empties(&mut self) {
        let empties = std::mem::take(&mut self.empties);
        for (id, ty, span) in empties {
            match self.solver.shallow(&ty) {
                Type::List(_) => self.table.set_empty(id, EmptyKind::List),
                Type::Map(_, _) => self.table.set_empty(id, EmptyKind::Map),
                Type::Unknown => {}
                _ => self.error(
                    "`empty` is a list or a map, and nothing here says which. Write the type on \
                     the state it starts."
                        .to_string(),
                    span,
                ),
            }
        }
    }

    fn fill_table(&mut self) {
        let ids: Vec<ExprId> = self.hir.exprs.iter().map(|(id, _)| id).collect();
        for id in ids {
            if let Some(ty) = self.table.expr(id).cloned() {
                let settled = self.solver.zonk(&ty);
                self.table.set_expr(id, settled);
            }
        }
        let locals: Vec<(LocalId, Type)> = self
            .locals
            .iter()
            .map(|(id, ty)| (*id, ty.clone()))
            .collect();
        for (id, ty) in locals {
            let settled = self.solver.zonk(&ty);
            self.table.set_local(id, settled);
        }
        let defs: Vec<(DefId, Type)> = self
            .schemes
            .iter()
            .map(|(id, scheme)| (*id, scheme.ty.clone()))
            .collect();
        for (id, ty) in defs {
            let settled = self.solver.zonk(&ty);
            self.table.set_def(id, settled);
        }
    }

    // --- plumbing ---

    fn type_of(&mut self, ty: &zdc_ast::TypeExpr) -> Type {
        match ty {
            zdc_ast::TypeExpr::Named(name) => match name.text.as_str() {
                "Text" => Type::Text,
                "Whole" => Type::Whole,
                "Decimal" => Type::Decimal,
                "Truth" => Type::Truth,
                "Error" => Type::Error,
                // `record` and `choice` (§14B.1) are not implemented, so
                // a name the language will one day define is treated as
                // its own opaque type rather than reported as a mistake.
                other => Type::Named(other.to_string()),
            },
            zdc_ast::TypeExpr::List(inner) => Type::list(self.type_of(inner)),
            zdc_ast::TypeExpr::Option(inner) => Type::option(self.type_of(inner)),
            zdc_ast::TypeExpr::Remote(inner) => Type::remote(self.type_of(inner)),
            zdc_ast::TypeExpr::Map(key, value) => Type::map(self.type_of(key), self.type_of(value)),
        }
    }

    fn bind(&mut self, local: LocalId, ty: Type) {
        self.locals.insert(local, ty);
    }

    fn local(&mut self, local: LocalId) -> Type {
        match self.locals.get(&local) {
            Some(ty) => ty.clone(),
            None => {
                // A binder the walk has not reached: only reachable if a
                // pass upstream produced a body out of order.
                let ty = self.solver.fresh();
                self.locals.insert(local, ty.clone());
                ty
            }
        }
    }

    /// Require two types to be the same. Returns whether they already
    /// were, so a caller can stop rather than pile a second diagnostic on
    /// the same mistake.
    fn expect(&mut self, found: &Type, want: &Type, span: Span, what: &str) -> bool {
        match self.solver.unify(found, want) {
            Ok(()) => true,
            Err(mismatch) => {
                let expected = self.solver.zonk(want);
                let message = match mismatch {
                    // The rejected type is the one that was *expected*,
                    // so what failed is the value, not the expectation.
                    // `state name is client Text starting 1` must blame
                    // the `1`, not the `Text`: a literal that could be
                    // either number is still not text, and saying "`Text`
                    // has to be a number" reads as though the declaration
                    // were the mistake.
                    Mismatch::Constraint { needed, found } if found == expected => format!(
                        "{what} {}, but `{expected}` is expected here.",
                        needed.subject()
                    ),
                    Mismatch::Constraint { needed, found } => {
                        format!("{what} `{found}`, but it has to be {}.", needed.describe())
                    }
                    Mismatch::Infinite => {
                        format!("{what} a value that contains itself, which nothing can be.")
                    }
                    Mismatch::Shape => {
                        let found = self.solver.zonk(found);
                        format!("{what} `{found}`, but `{expected}` is expected here.")
                    }
                };
                self.error(message, span);
                false
            }
        }
    }

    /// Require a type to be one of the built-in operand sets. Returns
    /// whether it was.
    fn demand(&mut self, found: &Type, constraint: Constraint, span: Span, what: &str) -> bool {
        match self.solver.require(found, constraint) {
            Ok(()) => true,
            Err(Mismatch::Constraint { needed, found }) => {
                self.error(
                    format!("{what} `{found}`, but it has to be {}.", needed.describe()),
                    span,
                );
                false
            }
            Err(_) => false,
        }
    }

    fn error(&mut self, message: String, span: Span) {
        self.errors.push(TypeError {
            message,
            span,
            help: None,
        });
    }

    fn error_with_help(&mut self, message: String, span: Span, help: String) {
        self.errors.push(TypeError {
            message,
            span,
            help: Some(help),
        });
    }
}

fn arm_head(arm: &HirArm) -> ArmHead<'_> {
    ArmHead {
        name: &arm.pattern_name,
        bindings: &arm.bindings,
        span: arm.span,
    }
}

fn node_arm_head(arm: &HirNodeArm) -> ArmHead<'_> {
    ArmHead {
        name: &arm.pattern_name,
        bindings: &arm.bindings,
        span: arm.span,
    }
}

fn arg_expr(arg: &HirArg) -> ExprId {
    match arg {
        HirArg::Positional(expr) => *expr,
        HirArg::Named { value, .. } => *value,
    }
}

/// `1 field` / `2 fields`, for a diagnostic that has to say how many.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn english_list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        None => "none".to_string(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
    }
}

/// Strongly-connected components of a graph, callees first.
///
/// Tarjan's algorithm, written iteratively: a program may declare more
/// functions than the stack has frames for, and a compiler that
/// overflows on a large input is a compiler that cannot compile itself.
fn components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let mut found: Vec<Vec<usize>> = Vec::new();

    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        // (node, how many of its edges have been walked)
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some((node, at)) = work.pop() {
            if at == 0 {
                index[node] = next;
                low[node] = next;
                next += 1;
                stack.push(node);
                on_stack[node] = true;
            }

            let mut descended = false;
            for (offset, &child) in edges[node].iter().enumerate().skip(at) {
                if index[child] == usize::MAX {
                    work.push((node, offset + 1));
                    work.push((child, 0));
                    descended = true;
                    break;
                } else if on_stack[child] {
                    low[node] = low[node].min(index[child]);
                }
            }
            if descended {
                continue;
            }

            if low[node] == index[node] {
                let mut component = Vec::new();
                while let Some(top) = stack.pop() {
                    on_stack[top] = false;
                    component.push(top);
                    if top == node {
                        break;
                    }
                }
                found.push(component);
            }

            if let Some((parent, _)) = work.last().copied() {
                low[parent] = low[parent].min(low[node]);
            }
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::components;

    #[test]
    fn a_graph_with_no_edges_is_one_component_per_node() {
        let found = components(&[vec![], vec![], vec![]]);
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn a_callee_comes_before_its_caller() {
        // 0 calls 1, 1 calls 2.
        let found = components(&[vec![1], vec![2], vec![]]);
        let order: Vec<usize> = found.iter().map(|component| component[0]).collect();
        assert_eq!(order, [2, 1, 0]);
    }

    #[test]
    fn a_cycle_is_one_component() {
        // 0 and 1 call each other; 2 stands alone.
        let found = components(&[vec![1], vec![0], vec![]]);
        assert_eq!(found.len(), 2);
        let cycle = found
            .iter()
            .find(|component| component.len() == 2)
            .expect("the cycle is one component");
        assert!(cycle.contains(&0) && cycle.contains(&1));
    }

    #[test]
    fn a_deep_chain_does_not_overflow_the_stack() {
        let depth = 20_000;
        let edges: Vec<Vec<usize>> = (0..depth)
            .map(|at| if at + 1 < depth { vec![at + 1] } else { vec![] })
            .collect();
        assert_eq!(components(&edges).len(), depth);
    }
}
