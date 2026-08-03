//! Component instantiation — §14D.1 and §14D.3.
//!
//! A component has no placement of its own. Like a function it runs
//! wherever its inputs are, so the only honest way to compile one is to
//! put its body where it was written and let the arguments carry their own
//! placements in with them. This pass does exactly that: every call site
//! is replaced by a fresh copy of the component's body, with each
//! parameter reference replaced by the very expression the caller passed.
//!
//! Three properties that §14D.1 states as rules fall out of that rather
//! than needing enforcement of their own:
//!
//! * **Colourlessness.** A component passed a `durable` signal does not
//!   become server-placed. After this pass there is no component left to
//!   have a placement — only the caller's expression, in the caller's
//!   context, which is what the placement pass was always going to read.
//! * **`Remote of T` cannot be laundered.** The read that used to be
//!   written inside the component is now written in the view, so the type
//!   checker introduces `Remote` for it exactly as it would have if the
//!   line had been typed there (§5.2). Passing a remote value through a
//!   parameter changes nothing, because the parameter is gone.
//! * **Secrecy flows through parameters.** The argument expression is the
//!   thing that ends up in the body, so a secret reaches the same taint
//!   analysis it would have reached without the component (§5.3).
//!
//! It is also monomorphisation, which is what §17.2 needs: a component
//! reached from two regions has two copies, so `zdc-graph` can give each
//! one the inherited attribute of the region it actually landed in.

use std::collections::HashMap;

use zdc_hir::{
    Component, DefId, DefKind, ExprId, Hir, HirArg, HirArm, HirArmBody, HirBlock, HirEach,
    HirEachNode, HirElement, HirExpr, HirExprKind, HirHandler, HirIf, HirIfNode, HirMutation,
    HirNode, HirNodeArm, HirNodeArmBody, HirPathSeg, HirPipeline, HirPlace, HirScope, HirStmt,
    HirWhen, HirWhenNode, Local, LocalId, LocalSignal, Res,
};
use zdc_lexer::Span;

use crate::collect::ResolveError;

/// Expand every component instance in the view.
pub fn instantiate(hir: &mut Hir) -> Result<(), Vec<ResolveError>> {
    let Some(view) = hir.view else { return Ok(()) };
    let DefKind::View(declaration) = &hir.defs[view].kind else {
        return Ok(());
    };
    let nodes = declaration.nodes.clone();

    let mut pass = Instantiate {
        hir,
        errors: Vec::new(),
        stack: Vec::new(),
    };
    let mut frame = Frame::default();
    let expanded = pass.nodes(&nodes, &mut frame);
    let errors = std::mem::take(&mut pass.errors);

    if let DefKind::View(declaration) = &mut hir.defs[view].kind {
        declaration.nodes = expanded;
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// What one copy of a component body rewrites.
#[derive(Default)]
struct Frame {
    /// Whether this walk is inside a component body.
    ///
    /// The view's own nodes are walked to find call sites, not to be
    /// rewritten, so nothing outside a component is copied or renamed. A
    /// pass that freshened every binder unconditionally would rename the
    /// view's own `each` bindings for no reason, and every emitted name
    /// downstream would move.
    copying: bool,
    /// Parameter to the expression the caller wrote. The expression is the
    /// caller's own, reused rather than copied, so the arguments of nested
    /// components are already substituted by the time they are seen.
    params: HashMap<LocalId, ExprId>,
    /// The nodes nested under the call site, already expanded.
    children: Vec<HirNode>,
    /// Every binder inside the body gets a fresh identity per instance, so
    /// two instances of one component never share a signal or a loop
    /// variable.
    locals: HashMap<LocalId, LocalId>,
}

struct Instantiate<'h> {
    hir: &'h mut Hir,
    errors: Vec<ResolveError>,
    /// The components currently being expanded, outermost first.
    stack: Vec<DefId>,
}

impl Instantiate<'_> {
    fn nodes(&mut self, nodes: &[HirNode], frame: &mut Frame) -> Vec<HirNode> {
        let mut out = Vec::with_capacity(nodes.len());
        for node in nodes {
            self.node(node, frame, &mut out);
        }
        out
    }

    /// One node, appended to `out`.
    ///
    /// It appends rather than returns because two node kinds expand to a
    /// *run* of nodes: `children` becomes whatever the call site nested,
    /// and a component with no state of its own becomes its body spliced
    /// into the caller's list.
    fn node(&mut self, node: &HirNode, frame: &mut Frame, out: &mut Vec<HirNode>) {
        match node {
            HirNode::Element(element) => match self.component_of(element.res) {
                Some(id) => self.expand(id, element, frame, out),
                None => out.push(HirNode::Element(HirElement {
                    name: element.name.clone(),
                    res: element.res,
                    args: element
                        .args
                        .iter()
                        .map(|arg| self.arg(arg, frame))
                        .collect(),
                    children: self.nodes(&element.children, frame),
                    span: element.span,
                })),
            },
            HirNode::Each(each) => {
                let iter = self.expr(each.iter, frame);
                let var = self.rebind(each.var, frame);
                let body = self.nodes(&each.body, frame);
                out.push(HirNode::Each(HirEachNode {
                    var,
                    iter,
                    body,
                    span: each.span,
                }));
            }
            HirNode::When(when) => {
                let scrutinee = self.expr(when.scrutinee, frame);
                let arms = when
                    .arms
                    .iter()
                    .map(|arm| {
                        let bindings = arm
                            .bindings
                            .iter()
                            .map(|binding| self.rebind(*binding, frame))
                            .collect();
                        let body = match &arm.body {
                            HirNodeArmBody::Show(element) => {
                                let mut shown = Vec::new();
                                self.node(
                                    &HirNode::Element((**element).clone()),
                                    frame,
                                    &mut shown,
                                );
                                match shown.pop() {
                                    Some(HirNode::Element(element)) if shown.is_empty() => {
                                        HirNodeArmBody::Show(Box::new(element))
                                    }
                                    // A component in `show` position expands
                                    // to a run of nodes, which is what the
                                    // multi-root arm form is for.
                                    Some(last) => {
                                        shown.push(last);
                                        HirNodeArmBody::Nodes(shown)
                                    }
                                    None => HirNodeArmBody::Nodes(Vec::new()),
                                }
                            }
                            HirNodeArmBody::Nodes(nodes) => {
                                HirNodeArmBody::Nodes(self.nodes(nodes, frame))
                            }
                        };
                        HirNodeArm {
                            pattern_name: arm.pattern_name.clone(),
                            bindings,
                            body,
                            span: arm.span,
                        }
                    })
                    .collect();
                out.push(HirNode::When(HirWhenNode {
                    scrutinee,
                    arms,
                    span: when.span,
                }));
            }
            HirNode::If(conditional) => {
                let cond = self.expr(conditional.cond, frame);
                let then = self.nodes(&conditional.then, frame);
                let otherwise = conditional
                    .otherwise
                    .as_ref()
                    .map(|nodes| self.nodes(nodes, frame));
                out.push(HirNode::If(HirIfNode {
                    cond,
                    then,
                    otherwise,
                    span: conditional.span,
                }));
            }
            HirNode::Handler(handler) => {
                // The payload binder is rebound before the body is copied,
                // for the reason every other binder is: two instances of a
                // component are two closures, so `press` in one must not be
                // the same local as `press` in the other.
                let payload = handler.payload.map(|local| self.rebind(local, frame));
                let body = self.block(handler.body, frame);
                out.push(HirNode::Handler(HirHandler {
                    event: handler.event.clone(),
                    payload,
                    event_span: handler.event_span,
                    body,
                    span: handler.span,
                }));
            }
            // The call site's nodes, already expanded in the caller's own
            // frame — which is why a component cannot see through them
            // into the names its caller happened to have in scope.
            HirNode::Children(_) => out.extend(frame.children.iter().cloned()),
            HirNode::Scope(scope) => {
                let locals = scope
                    .locals
                    .iter()
                    .map(|local| self.local_signal(local, frame))
                    .collect();
                let body = self.nodes(&scope.body, frame);
                out.push(HirNode::Scope(HirScope {
                    locals,
                    body,
                    span: scope.span,
                }));
            }
        }
    }

    /// Replace one call site with a copy of the component's body.
    fn expand(
        &mut self,
        id: DefId,
        element: &HirElement,
        frame: &mut Frame,
        out: &mut Vec<HirNode>,
    ) {
        if self.stack.contains(&id) {
            let mut path: Vec<String> = self
                .stack
                .iter()
                .skip_while(|seen| **seen != id)
                .map(|seen| self.hir.defs[*seen].name.clone())
                .collect();
            path.push(self.hir.defs[id].name.clone());
            self.errors.push(ResolveError {
                message: format!(
                    "These components contain each other: {}. A component is written where it is \
                     used, so one that contains itself describes a view with no end.",
                    path.join(" → ")
                ),
                span: element.span,
            });
            return;
        }

        let DefKind::Component(component) = &self.hir.defs[id].kind else {
            return;
        };
        let component: Component = component.clone();
        let name = self.hir.defs[id].name.clone();

        let Some(params) = self.bind_arguments(&name, &component, element, frame) else {
            return;
        };
        // Declaring `children` and being given none is not an error: an
        // empty run is a run, and the body renders nothing there. Being
        // given some without declaring any is, because they have nowhere
        // to go.
        let children = self.nodes(&element.children, frame);
        if !children.is_empty() && component.children.is_none() {
            self.errors.push(ResolveError {
                message: format!(
                    "`{name}` does not take `children`, so these nested nodes have nowhere to go. \
                     Write `component {name} with children` and place them with `children` in \
                     its body."
                ),
                span: element.span,
            });
            return;
        }

        let mut inner = Frame {
            copying: true,
            params,
            children,
            locals: HashMap::new(),
        };

        self.stack.push(id);
        let locals: Vec<LocalSignal> = component
            .states
            .iter()
            .map(|state| self.local_signal(state, &mut inner))
            .collect();
        let body = self.nodes(&component.body, &mut inner);
        self.stack.pop();

        // A component with no state of its own needs no scope: splicing
        // its nodes straight in keeps the emitted region flat, and a scope
        // that declares nothing would be a wrapper the program never asked
        // for.
        if locals.is_empty() {
            out.extend(body);
            return;
        }
        out.push(HirNode::Scope(HirScope {
            locals,
            body,
            span: element.span,
        }));
    }

    /// Match a call site's arguments to the component's parameters.
    ///
    /// The same rules as any element's arguments (§14D.1): positional
    /// first, in declaration order, then named. Every parameter must end
    /// up with exactly one value, because a component's body reads them
    /// unconditionally.
    fn bind_arguments(
        &mut self,
        name: &str,
        component: &Component,
        element: &HirElement,
        frame: &mut Frame,
    ) -> Option<HashMap<LocalId, ExprId>> {
        let mut bound: HashMap<LocalId, ExprId> = HashMap::new();
        let mut positional = 0usize;
        let mut failed = false;

        for arg in &element.args {
            match arg {
                HirArg::Positional(expr) => {
                    let Some(param) = component.params.get(positional) else {
                        self.errors.push(ResolveError {
                            message: format!(
                                "`{name}` takes {}, and this is one more.",
                                count(component.params.len(), "argument")
                            ),
                            span: self.hir.exprs[*expr].span,
                        });
                        failed = true;
                        continue;
                    };
                    positional += 1;
                    bound.insert(*param, self.expr(*expr, frame));
                }
                HirArg::Named {
                    name: written,
                    value,
                } => {
                    let found = component
                        .params
                        .iter()
                        .find(|param| self.hir.locals[**param].name == *written)
                        .copied();
                    let Some(param) = found else {
                        self.errors.push(ResolveError {
                            message: format!(
                                "`{name}` has no parameter called `{written}`. It takes {}.",
                                english_list(&self.parameter_names(component))
                            ),
                            span: self.hir.exprs[*value].span,
                        });
                        failed = true;
                        continue;
                    };
                    let value = self.expr(*value, frame);
                    if bound.insert(param, value).is_some() {
                        self.errors.push(ResolveError {
                            message: format!(
                                "`{written}` is given twice here. Each parameter takes one value."
                            ),
                            span: self.hir.exprs[value].span,
                        });
                        failed = true;
                    }
                }
            }
        }

        let missing: Vec<String> = component
            .params
            .iter()
            .filter(|param| !bound.contains_key(param))
            .map(|param| self.hir.locals[*param].name.clone())
            .collect();
        if !missing.is_empty() {
            self.errors.push(ResolveError {
                message: format!(
                    "`{name}` needs {}, which this does not give it.",
                    english_list(&missing)
                ),
                span: element.span,
            });
            failed = true;
        }

        (!failed).then_some(bound)
    }

    fn parameter_names(&self, component: &Component) -> Vec<String> {
        component
            .params
            .iter()
            .map(|param| self.hir.locals[*param].name.clone())
            .collect()
    }

    fn local_signal(&mut self, state: &LocalSignal, frame: &mut Frame) -> LocalSignal {
        let init = self.expr(state.init, frame);
        LocalSignal {
            local: self.rebind(state.local, frame),
            placement: state.placement,
            ty: state.ty.clone(),
            is_source: state.is_source,
            init,
            span: state.span,
        }
    }

    /// Whether this element names a component, and which one.
    fn component_of(&self, res: Res) -> Option<DefId> {
        let Res::Def(id) = res else { return None };
        matches!(self.hir.defs[id].kind, DefKind::Component(_)).then_some(id)
    }

    // --- copying ---------------------------------------------------------

    /// A fresh identity for a binder inside a component body.
    fn rebind(&mut self, local: LocalId, frame: &mut Frame) -> LocalId {
        if !frame.copying {
            return local;
        }
        let declared = self.hir.locals[local].clone();
        let fresh = self.hir.locals.alloc(Local {
            name: declared.name,
            span: declared.span,
        });
        frame.locals.insert(local, fresh);
        fresh
    }

    fn arg(&mut self, arg: &HirArg, frame: &mut Frame) -> HirArg {
        match arg {
            HirArg::Positional(expr) => HirArg::Positional(self.expr(*expr, frame)),
            HirArg::Named { name, value } => HirArg::Named {
                name: name.clone(),
                value: self.expr(*value, frame),
            },
        }
    }

    /// Copy one expression into this instance.
    ///
    /// A parameter reference is not copied — it *becomes* the caller's own
    /// expression, which is the whole mechanism by which placement and
    /// taint cross the boundary unchanged.
    fn expr(&mut self, id: ExprId, frame: &mut Frame) -> ExprId {
        if !frame.copying {
            return id;
        }
        let span = self.hir.exprs[id].span;
        let kind = match self.hir.exprs[id].kind.clone() {
            HirExprKind::Ref(Res::Local(local)) => {
                if let Some(substituted) = frame.params.get(&local) {
                    return *substituted;
                }
                HirExprKind::Ref(Res::Local(self.rename(local, frame)))
            }
            kind @ (HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            | HirExprKind::Environment(_)
            | HirExprKind::Address
            | HirExprKind::Ref(_)) => kind,
            HirExprKind::List(items) => HirExprKind::List(
                items
                    .into_iter()
                    .map(|item| self.expr(item, frame))
                    .collect(),
            ),
            HirExprKind::Map(entries) => HirExprKind::Map(
                entries
                    .into_iter()
                    .map(|(key, value)| (self.expr(key, frame), self.expr(value, frame)))
                    .collect(),
            ),
            HirExprKind::Call { callee, args } => HirExprKind::Call {
                callee,
                args: args.iter().map(|arg| self.arg(arg, frame)).collect(),
            },
            // The `of` forms carry their callee as a `Res` and one operand,
            // so they copy exactly as `Call` does: the callee is a
            // top-level name that instantiation never rebinds, and the
            // operand is an ordinary expression of the body.
            HirExprKind::OfCall { callee, operand } => HirExprKind::OfCall {
                callee,
                operand: self.expr(operand, frame),
            },
            HirExprKind::Operator { op, operand } => HirExprKind::Operator {
                op,
                operand: self.expr(operand, frame),
            },
            // The capability is a `Copy` tag resolution already fixed, so
            // only the operand is copied — exactly as `OfCall` copies its
            // operand and leaves its callee alone.
            HirExprKind::Build {
                capability,
                argument,
            } => HirExprKind::Build {
                capability,
                argument: self.expr(argument, frame),
            },
            HirExprKind::Unary { op, operand } => HirExprKind::Unary {
                op,
                operand: self.expr(operand, frame),
            },
            HirExprKind::Binary { op, lhs, rhs } => HirExprKind::Binary {
                op,
                lhs: self.expr(lhs, frame),
                rhs: self.expr(rhs, frame),
            },
            HirExprKind::Field { base, name } => HirExprKind::Field {
                base: self.expr(base, frame),
                name,
            },
            HirExprKind::Index { base, index } => HirExprKind::Index {
                base: self.expr(base, frame),
                index: self.expr(index, frame),
            },
        };
        self.hir.exprs.alloc(HirExpr { kind, span })
    }

    fn rename(&self, local: LocalId, frame: &Frame) -> LocalId {
        frame.locals.get(&local).copied().unwrap_or(local)
    }

    fn block(&mut self, id: zdc_hir::BlockId, frame: &mut Frame) -> zdc_hir::BlockId {
        if !frame.copying {
            return id;
        }
        let block = self.hir.blocks[id].clone();
        let stmts = block
            .stmts
            .iter()
            .map(|stmt| self.stmt(stmt, frame))
            .collect();
        self.hir.blocks.alloc(HirBlock {
            stmts,
            span: block.span,
        })
    }

    fn stmt(&mut self, stmt: &HirStmt, frame: &mut Frame) -> HirStmt {
        match stmt {
            HirStmt::Give(expr) => HirStmt::Give(self.expr(*expr, frame)),
            HirStmt::Pipeline(clause) => HirStmt::Pipeline(match clause {
                HirPipeline::From(expr) => HirPipeline::From(self.expr(*expr, frame)),
                HirPipeline::TakeFirst(expr) => HirPipeline::TakeFirst(self.expr(*expr, frame)),
                HirPipeline::Keep { var, cond } => {
                    let var = self.rebind(*var, frame);
                    HirPipeline::Keep {
                        var,
                        cond: self.expr(*cond, frame),
                    }
                }
                HirPipeline::Sort { var, key } => {
                    let var = self.rebind(*var, frame);
                    HirPipeline::Sort {
                        var,
                        key: self.expr(*key, frame),
                    }
                }
                HirPipeline::MapEach { var, to } => {
                    let var = self.rebind(*var, frame);
                    HirPipeline::MapEach {
                        var,
                        to: self.expr(*to, frame),
                    }
                }
            }),
            HirStmt::Mutation(mutation) => HirStmt::Mutation(match mutation {
                HirMutation::Set { place, value } => HirMutation::Set {
                    place: self.place(place, frame),
                    value: self.expr(*value, frame),
                },
                HirMutation::Add { value, place } => HirMutation::Add {
                    value: self.expr(*value, frame),
                    place: self.place(place, frame),
                },
                HirMutation::Subtract { value, place } => HirMutation::Subtract {
                    value: self.expr(*value, frame),
                    place: self.place(place, frame),
                },
                HirMutation::Append { value, place } => HirMutation::Append {
                    value: self.expr(*value, frame),
                    place: self.place(place, frame),
                },
                HirMutation::Remove { value, place } => HirMutation::Remove {
                    value: self.expr(*value, frame),
                    place: self.place(place, frame),
                },
            }),
            HirStmt::When(when) => {
                let scrutinee = self.expr(when.scrutinee, frame);
                let arms = when
                    .arms
                    .iter()
                    .map(|arm| {
                        let bindings = arm
                            .bindings
                            .iter()
                            .map(|binding| self.rebind(*binding, frame))
                            .collect();
                        let body = match arm.body {
                            HirArmBody::Show(expr) => HirArmBody::Show(self.expr(expr, frame)),
                            HirArmBody::Block(block) => HirArmBody::Block(self.block(block, frame)),
                        };
                        HirArm {
                            pattern_name: arm.pattern_name.clone(),
                            bindings,
                            body,
                            span: arm.span,
                        }
                    })
                    .collect();
                HirStmt::When(HirWhen {
                    scrutinee,
                    arms,
                    span: when.span,
                })
            }
            HirStmt::Each(each) => {
                let iter = self.expr(each.iter, frame);
                let var = self.rebind(each.var, frame);
                HirStmt::Each(HirEach {
                    var,
                    iter,
                    body: self.block(each.body, frame),
                    span: each.span,
                })
            }
            HirStmt::If(conditional) => HirStmt::If(HirIf {
                cond: self.expr(conditional.cond, frame),
                then: self.block(conditional.then, frame),
                otherwise: conditional.otherwise.map(|block| self.block(block, frame)),
                span: conditional.span,
            }),
        }
    }

    /// A place inside a component body.
    ///
    /// Writing through a parameter is how `VoteCard` casts a vote, and it
    /// is the one substitution that cannot always be made: a place needs a
    /// name to write to, so the argument has to be one. Passing an
    /// expression and then writing to it is refused here rather than
    /// silently dropped.
    fn place(&mut self, place: &HirPlace, frame: &mut Frame) -> HirPlace {
        let base = match place.base {
            Res::Local(local) => match frame.params.get(&local) {
                Some(argument) => match self.hir.exprs[*argument].kind {
                    HirExprKind::Ref(res) => res,
                    _ => {
                        let name = self.hir.locals[local].name.clone();
                        self.errors.push(ResolveError {
                            message: format!(
                                "`{name}` is written to inside this component, so the value \
                                 passed for it has to be a name that can be written to — a \
                                 `state`, or another component's parameter. This call site \
                                 passes a computed value instead."
                            ),
                            span: self.hir.exprs[*argument].span,
                        });
                        place.base
                    }
                },
                None => Res::Local(self.rename(local, frame)),
            },
            other => other,
        };
        HirPlace {
            base,
            path: place
                .path
                .iter()
                .map(|segment| match segment {
                    HirPathSeg::Field(name) => HirPathSeg::Field(name.clone()),
                    HirPathSeg::Index(expr) => HirPathSeg::Index(self.expr(*expr, frame)),
                })
                .collect(),
            span: place.span,
        }
    }
}

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
        None => "no arguments".to_string(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
    }
}

/// A span that belongs to no source, for a diagnostic with nowhere better.
#[allow(dead_code)]
fn nowhere() -> Span {
    Span::new(0, 0)
}
