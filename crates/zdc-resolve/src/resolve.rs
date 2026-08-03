use crate::collect::{collect, GlobalTable, ResolveError, BUILTIN_VARIANTS};
use crate::scope::Scopes;
use std::collections::HashSet;
use zdc_ast as ast;
use zdc_hir::{
    Builtin, Choice, Def, DefId, DefKind, ExprId, Field, Function, Hir, HirArg, HirArm, HirArmBody,
    HirBlock, HirEach, HirEachNode, HirElement, HirExpr, HirExprKind, HirHandler, HirIf,
    HirMutation, HirNode, HirNodeArm, HirNodeArmBody, HirPathSeg, HirPipeline, HirPlace, HirStmt,
    HirWhen, HirWhenNode, Local, LocalId, Record, Res, Signal, Variant, View,
};

/// The view elements the language provides.
///
/// A stopgap until user-defined components exist (spec §14D), at which
/// point an element name becomes an ordinary lookup in the global table
/// and this constant is the single place that changes.
const BUILTIN_ELEMENTS: &[&str] = &[
    "Column", "Row", "Text", "Heading", "Button", "Input", "Checkbox", "Spinner", "ErrorBar",
];

/// The variant names every program can match, whatever it declares: the
/// ones `Option` and `Remote` provide. A `choice` adds its own on top and
/// may not redeclare one of these (spec §14G.1.2).
const BUILTIN_PATTERNS: &[&str] = BUILTIN_VARIANTS;

/// Lowers a parsed program into HIR, resolving every identifier.
///
/// Resolution never stops at the first error. Each walk visits all of a
/// node's children before deciding whether the node itself resolved, so
/// a program with three undefined names yields three diagnostics from
/// one run. A node whose children did not all resolve is dropped from
/// the tree, which is harmless: the HIR is returned only when no error
/// was recorded at all.
pub struct Resolver<'a> {
    program: &'a ast::Program,
    hir: Hir,
    scopes: Scopes,
    globals: GlobalTable,
    errors: Vec<ResolveError>,
    /// The definition each declaration became, indexed by its position
    /// in `Program::decls`.
    defs: Vec<DefId>,
}

impl<'a> Resolver<'a> {
    pub fn new(program: &'a ast::Program) -> Self {
        Resolver {
            program,
            hir: Hir::new(),
            scopes: Scopes::new(),
            globals: GlobalTable::default(),
            errors: Vec::new(),
            defs: Vec::new(),
        }
    }

    pub fn resolve(mut self) -> Result<Hir, Vec<ResolveError>> {
        // Copied out so the walks below borrow the program rather than
        // `self`, which they also need mutably.
        let program = self.program;
        self.globals = collect(program)?;

        // Every declaration gets its definition before any body is
        // looked at. This is what makes top-level declarations
        // order-independent: a signal may read one declared further down
        // the file, because the signal graph is a graph, not a sequence.
        for decl in &program.decls {
            let (name, span) = match decl {
                ast::Decl::State(state) => (state.name.text.clone(), state.name.span),
                ast::Decl::Function(function) => (function.name.text.clone(), function.name.span),
                ast::Decl::Record(record) => (record.name.text.clone(), record.name.span),
                ast::Decl::Choice(choice) => (choice.name.text.clone(), choice.name.span),
                ast::Decl::View(view) => ("view".to_string(), view.span),
            };
            let id = self.hir.defs.alloc(Def {
                name,
                span,
                kind: pending(),
            });
            self.defs.push(id);
        }

        for (index, decl) in program.decls.iter().enumerate() {
            let kind = match decl {
                ast::Decl::State(state) => self.signal(state).map(DefKind::Signal),
                ast::Decl::Function(function) => self.function(function).map(DefKind::Function),
                ast::Decl::Record(record) => Some(DefKind::Record(Record {
                    fields: self.fields(&record.name.text, &record.fields),
                })),
                ast::Decl::Choice(choice) => Some(DefKind::Choice(Choice {
                    variants: choice
                        .variants
                        .iter()
                        .map(|variant| Variant {
                            name: variant.name.text.clone(),
                            fields: self.fields(&variant.name.text, &variant.fields),
                            span: variant.span,
                        })
                        .collect(),
                })),
                ast::Decl::View(view) => Some(DefKind::View(self.view(view))),
            };
            if let Some(kind) = kind {
                self.hir.defs[self.defs[index]].kind = kind;
            }
        }

        self.hir.view = self.globals.view.map(|index| self.defs[index]);

        if self.errors.is_empty() {
            Ok(self.hir)
        } else {
            Err(self.errors)
        }
    }

    // --- declarations ---

    fn signal(&mut self, state: &ast::StateDecl) -> Option<Signal> {
        // `starting` declares a source the program sets directly;
        // `from` declares a value the compiler recomputes (spec §4.5).
        let (is_source, expr) = match &state.init {
            ast::Init::Starting(expr) => (true, expr),
            ast::Init::From(expr) => (false, expr),
        };
        let init = self.expr(expr)?;
        Some(Signal {
            secret: state.secret,
            placement: state.placement,
            ty: state.ty.clone(),
            is_source,
            init,
        })
    }

    /// The fields of a record or of a variant's payload, in declaration
    /// order.
    ///
    /// A repeated name is reported rather than kept: construction is by
    /// name (§14G.1.2), so a second `title` would give one field two values
    /// and elimination would still bind positionally to the first.
    fn fields(&mut self, owner: &str, fields: &[ast::FieldDecl]) -> Vec<Field> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out = Vec::with_capacity(fields.len());
        for field in fields {
            if !seen.insert(field.name.text.as_str()) {
                self.error(
                    format!(
                        "`{owner}` declares `{}` twice. Each field of a record or a variant is \
                         named once, because a value is built by naming its fields.",
                        field.name.text
                    ),
                    field.name.span,
                );
            }
            out.push(Field {
                name: field.name.text.clone(),
                ty: field.ty.clone(),
                span: field.span,
            });
        }
        out
    }

    fn function(&mut self, function: &ast::FunctionDecl) -> Option<Function> {
        self.scopes.push();
        let params = self.bind_all(&function.params);
        let body = self.block(&function.body);
        self.scopes.pop();
        Some(Function { params, body })
    }

    fn view(&mut self, view: &ast::ViewDecl) -> View {
        self.scopes.push();
        let nodes = self.nodes(&view.nodes);
        self.scopes.pop();
        View { nodes }
    }

    // --- statements ---

    fn block(&mut self, block: &ast::Block) -> zdc_hir::BlockId {
        self.scopes.push();
        let stmts: Vec<HirStmt> = block
            .stmts
            .iter()
            .map(|stmt| self.stmt(stmt))
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect();
        self.scopes.pop();
        self.hir.blocks.alloc(HirBlock {
            stmts,
            span: block.span,
        })
    }

    fn stmt(&mut self, stmt: &ast::Stmt) -> Option<HirStmt> {
        Some(match stmt {
            ast::Stmt::Pipeline(clause) => HirStmt::Pipeline(self.pipeline(clause)?),
            ast::Stmt::Mutation(mutation) => HirStmt::Mutation(self.mutation(mutation)?),
            ast::Stmt::Give(expr) => HirStmt::Give(self.expr(expr)?),
            ast::Stmt::When(when) => {
                let scrutinee = self.expr(&when.scrutinee);
                let arms = all_or_none(when.arms.iter().map(|arm| self.arm(arm)).collect());
                HirStmt::When(HirWhen {
                    scrutinee: scrutinee?,
                    arms: arms?,
                    span: when.span,
                })
            }
            ast::Stmt::Each(each) => {
                // The sequence is resolved before the loop name is
                // bound, so `each item in item` iterates the outer
                // `item` rather than itself.
                let iter = self.expr(&each.iter);
                self.scopes.push();
                let var = self.bind(&each.var);
                let body = self.block(&each.body);
                self.scopes.pop();
                HirStmt::Each(HirEach {
                    var,
                    iter: iter?,
                    body,
                    span: each.span,
                })
            }
            ast::Stmt::If(conditional) => {
                let cond = self.expr(&conditional.cond);
                let then = self.block(&conditional.then);
                let otherwise = conditional
                    .otherwise
                    .as_ref()
                    .map(|block| self.block(block));
                HirStmt::If(HirIf {
                    cond: cond?,
                    then,
                    otherwise,
                    span: conditional.span,
                })
            }
        })
    }

    /// A pipeline clause's loop name is in scope for that clause's
    /// expression and nowhere else.
    fn pipeline(&mut self, clause: &ast::PipelineClause) -> Option<HirPipeline> {
        Some(match clause {
            ast::PipelineClause::From(expr) => HirPipeline::From(self.expr(expr)?),
            ast::PipelineClause::TakeFirst(expr) => HirPipeline::TakeFirst(self.expr(expr)?),
            ast::PipelineClause::Keep { var, cond } => {
                let (var, cond) = self.clause_binder(var, cond);
                HirPipeline::Keep { var, cond: cond? }
            }
            ast::PipelineClause::Sort { var, key } => {
                let (var, key) = self.clause_binder(var, key);
                HirPipeline::Sort { var, key: key? }
            }
            ast::PipelineClause::MapEach { var, to } => {
                let (var, to) = self.clause_binder(var, to);
                HirPipeline::MapEach { var, to: to? }
            }
        })
    }

    fn clause_binder(
        &mut self,
        var: &ast::Ident,
        expr: &ast::Expr,
    ) -> (LocalId, Option<zdc_hir::ExprId>) {
        self.scopes.push();
        let var = self.bind(var);
        let expr = self.expr(expr);
        self.scopes.pop();
        (var, expr)
    }

    fn mutation(&mut self, mutation: &ast::Mutation) -> Option<HirMutation> {
        Some(match mutation {
            ast::Mutation::Set { place, value } => {
                let place = self.place(place);
                let value = self.expr(value);
                HirMutation::Set {
                    place: place?,
                    value: value?,
                }
            }
            ast::Mutation::Add { value, place } => {
                let value = self.expr(value);
                let place = self.place(place);
                HirMutation::Add {
                    value: value?,
                    place: place?,
                }
            }
            ast::Mutation::Subtract { value, place } => {
                let value = self.expr(value);
                let place = self.place(place);
                HirMutation::Subtract {
                    value: value?,
                    place: place?,
                }
            }
            ast::Mutation::Append { value, place } => {
                let value = self.expr(value);
                let place = self.place(place);
                HirMutation::Append {
                    value: value?,
                    place: place?,
                }
            }
            ast::Mutation::Remove { value, place } => {
                let value = self.expr(value);
                let place = self.place(place);
                HirMutation::Remove {
                    value: value?,
                    place: place?,
                }
            }
        })
    }

    fn place(&mut self, place: &ast::Place) -> Option<HirPlace> {
        let base = self.value_name(&place.base);
        let path = all_or_none(
            place
                .path
                .iter()
                .map(|segment| match segment {
                    ast::PathSeg::Field(name) => Some(HirPathSeg::Field(name.text.clone())),
                    ast::PathSeg::Index(expr) => self.expr(expr).map(HirPathSeg::Index),
                })
                .collect(),
        );
        Some(HirPlace {
            base: base?,
            path: path?,
            span: place.span,
        })
    }

    fn arm(&mut self, arm: &ast::Arm) -> Option<HirArm> {
        let pattern_name = self.pattern_name(&arm.pattern.name);
        self.scopes.push();
        let bindings = self.bind_all(&arm.pattern.bindings);
        let body = match &arm.body {
            ast::ArmBody::Show(expr) => self.expr(expr).map(HirArmBody::Show),
            ast::ArmBody::Block(block) => Some(HirArmBody::Block(self.block(block))),
        };
        self.scopes.pop();
        Some(HirArm {
            pattern_name: pattern_name?,
            bindings,
            body: body?,
            span: arm.span,
        })
    }

    // --- view nodes ---

    fn nodes(&mut self, nodes: &[ast::Node]) -> Vec<HirNode> {
        nodes
            .iter()
            .map(|node| self.node(node))
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect()
    }

    fn node(&mut self, node: &ast::Node) -> Option<HirNode> {
        Some(match node {
            ast::Node::Element(element) => HirNode::Element(self.element(element)?),
            ast::Node::Handler(handler) => HirNode::Handler(HirHandler {
                event: handler.event.text.clone(),
                body: self.block(&handler.body),
                span: handler.span,
            }),
            ast::Node::Each(each) => {
                let iter = self.expr(&each.iter);
                self.scopes.push();
                let var = self.bind(&each.var);
                let body = self.nodes(&each.body);
                self.scopes.pop();
                HirNode::Each(HirEachNode {
                    var,
                    iter: iter?,
                    body,
                    span: each.span,
                })
            }
            ast::Node::When(when) => {
                let scrutinee = self.expr(&when.scrutinee);
                let arms = all_or_none(when.arms.iter().map(|arm| self.node_arm(arm)).collect());
                HirNode::When(HirWhenNode {
                    scrutinee: scrutinee?,
                    arms: arms?,
                    span: when.span,
                })
            }
        })
    }

    fn element(&mut self, element: &ast::Element) -> Option<HirElement> {
        let res = self.element_name(&element.name);
        let args = all_or_none(element.args.iter().map(|arg| self.arg(arg)).collect());
        let children = self.nodes(&element.children);
        Some(HirElement {
            name: element.name.text.clone(),
            res: res?,
            args: args?,
            children,
            span: element.span,
        })
    }

    fn node_arm(&mut self, arm: &ast::NodeArm) -> Option<HirNodeArm> {
        let pattern_name = self.pattern_name(&arm.pattern.name);
        self.scopes.push();
        let bindings = self.bind_all(&arm.pattern.bindings);
        let body = match &arm.body {
            ast::NodeArmBody::Show(element) => self
                .element(element)
                .map(|element| HirNodeArmBody::Show(Box::new(element))),
            ast::NodeArmBody::Nodes(nodes) => Some(HirNodeArmBody::Nodes(self.nodes(nodes))),
        };
        self.scopes.pop();
        Some(HirNodeArm {
            pattern_name: pattern_name?,
            bindings,
            body: body?,
            span: arm.span,
        })
    }

    // --- expressions ---

    fn expr(&mut self, expr: &ast::Expr) -> Option<ExprId> {
        let span = expr.span();
        let kind = match expr {
            ast::Expr::Number { value, .. } => HirExprKind::Number(*value),
            ast::Expr::Text { value, .. } => HirExprKind::Text(value.clone()),
            ast::Expr::Truth { value, .. } => HirExprKind::Truth(*value),
            ast::Expr::Empty { .. } => HirExprKind::Empty,
            ast::Expr::List { items, .. } => HirExprKind::List(all_or_none(
                items.iter().map(|item| self.expr(item)).collect(),
            )?),
            ast::Expr::Map { entries, .. } => HirExprKind::Map(all_or_none(
                entries
                    .iter()
                    .map(|(key, value)| {
                        // Both halves are visited before either is judged,
                        // so two undefined names in one entry are two
                        // diagnostics.
                        let key = self.expr(key);
                        let value = self.expr(value);
                        Some((key?, value?))
                    })
                    .collect(),
            )?),
            ast::Expr::Environment { key, .. } => HirExprKind::Environment(key.clone()),
            ast::Expr::Var { name, .. } => HirExprKind::Ref(self.value_name(name)?),
            ast::Expr::Call { name, args, .. } => {
                let callee = self.value_name(name);
                let args = all_or_none(args.iter().map(|arg| self.arg(arg)).collect());
                HirExprKind::Call {
                    callee: callee?,
                    args: args?,
                }
            }
            ast::Expr::Unary { op, operand, .. } => HirExprKind::Unary {
                op: *op,
                operand: self.expr(operand)?,
            },
            ast::Expr::Binary { op, lhs, rhs, .. } => {
                let lhs = self.expr(lhs);
                let rhs = self.expr(rhs);
                HirExprKind::Binary {
                    op: *op,
                    lhs: lhs?,
                    rhs: rhs?,
                }
            }
            ast::Expr::Field { base, name, .. } => HirExprKind::Field {
                base: self.expr(base)?,
                name: name.text.clone(),
            },
            ast::Expr::Index { base, index, .. } => {
                let base = self.expr(base);
                let index = self.expr(index);
                HirExprKind::Index {
                    base: base?,
                    index: index?,
                }
            }
        };
        Some(self.hir.exprs.alloc(HirExpr { kind, span }))
    }

    fn arg(&mut self, arg: &ast::Arg) -> Option<HirArg> {
        Some(match arg {
            ast::Arg::Positional(expr) => HirArg::Positional(self.expr(expr)?),
            ast::Arg::Named { name, value } => HirArg::Named {
                name: name.text.clone(),
                value: self.expr(value)?,
            },
        })
    }

    // --- names ---

    /// A name used as a value: a local first, so an inner binding
    /// shadows a top-level one, then a top-level declaration.
    fn value_name(&mut self, ident: &ast::Ident) -> Option<Res> {
        if let Some(local) = self.scopes.lookup(&ident.text) {
            return Some(Res::Local(local));
        }
        if let Some(index) = self.globals.lookup(&ident.text) {
            return Some(Res::Def(self.defs[index]));
        }
        // A variant name is a value (`All`) and a constructor (`Archived
        // with reason is …`) alike, so it is looked up here as well as in
        // pattern position.
        if let Some((index, at)) = self.globals.variant(&ident.text) {
            return Some(Res::Variant {
                choice: self.defs[index],
                index: at,
            });
        }
        self.error(
            format!(
                "`{}` is not defined. Declare it with `state`, `function`, `record`, or \
                 `choice`, or check the spelling.",
                ident.text
            ),
            ident.span,
        );
        None
    }

    /// A name used as a view element. Element position is not value
    /// position, so a local named `Row` does not hide the element `Row`.
    fn element_name(&mut self, ident: &ast::Ident) -> Option<Res> {
        if BUILTIN_ELEMENTS.contains(&ident.text.as_str()) {
            return Some(Res::Builtin(Builtin::Element));
        }
        self.error(
            format!(
                "`{}` is not a view element. The view elements are {}.",
                ident.text,
                english_list(BUILTIN_ELEMENTS)
            ),
            ident.span,
        );
        None
    }

    /// The variant a `when` arm matches. Which choice it belongs to is a
    /// question for the type checker, so only the name is checked here.
    fn pattern_name(&mut self, ident: &ast::Ident) -> Option<String> {
        if BUILTIN_PATTERNS.contains(&ident.text.as_str())
            || self.globals.declares_variant(&ident.text)
        {
            return Some(ident.text.clone());
        }
        self.error(
            format!(
                "`{}` is not a variant name. A `when` arm matches {}, or a variant a `choice` \
                 in this file declares.",
                ident.text,
                english_list(BUILTIN_PATTERNS)
            ),
            ident.span,
        );
        None
    }

    /// Bind a run of names in the current scope: a function's
    /// parameters, or a pattern's binders.
    ///
    /// A pattern binds one fresh name per named field of the variant it
    /// matches (spec §14G.1.2), so this is a list in both cases. Two
    /// fields bound to the same name would make one of them
    /// unreachable, so that is reported rather than silently shadowed.
    fn bind_all(&mut self, idents: &[ast::Ident]) -> Vec<LocalId> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut ids = Vec::with_capacity(idents.len());
        for ident in idents {
            if !seen.insert(ident.text.as_str()) {
                self.error(
                    format!(
                        "`{}` is bound twice here. Each name after `with` must be different, or \
                         one of them can never be read.",
                        ident.text
                    ),
                    ident.span,
                );
            }
            ids.push(self.bind(ident));
        }
        ids
    }

    fn bind(&mut self, ident: &ast::Ident) -> LocalId {
        let id = self.hir.locals.alloc(Local {
            name: ident.text.clone(),
            span: ident.span,
        });
        self.scopes.declare(&ident.text, id);
        id
    }

    fn error(&mut self, message: String, span: zdc_lexer::Span) {
        self.errors.push(ResolveError { message, span });
    }
}

/// The kind a definition holds between being allocated and being
/// resolved.
///
/// Definitions are allocated for the whole program before any body is
/// walked, so that a body can refer to one declared later. An empty view
/// is the placeholder because it owns no arena index: if a bug ever left
/// one un-overwritten, the result is an empty view rather than a
/// definition pointing at an arena slot that means something else.
fn pending() -> DefKind {
    DefKind::View(View { nodes: Vec::new() })
}

/// Combine per-item results only after every item has been visited.
///
/// Collecting straight into `Option<Vec<_>>` would stop at the first
/// `None` and hide the errors in everything after it, which is exactly
/// the behaviour resolution must not have.
fn all_or_none<T>(resolved: Vec<Option<T>>) -> Option<Vec<T>> {
    resolved.into_iter().collect()
}

/// `a`, `b`, and `c` — for listing the valid names in a diagnostic.
fn english_list(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hir_of(src: &str) -> Result<Hir, Vec<ResolveError>> {
        let program = zdc_parser::parse(src).expect("parses");
        Resolver::new(&program).resolve()
    }

    fn errors_of(src: &str) -> Vec<String> {
        hir_of(src)
            .expect_err("expected resolution to fail")
            .into_iter()
            .map(|error| error.message)
            .collect()
    }

    #[test]
    fn a_signal_reading_another_signal_resolves_to_a_def() {
        let hir = hir_of("state a is client Whole starting 1\nstate b is client Whole from a\n")
            .expect("resolves");
        assert_eq!(hir.defs.len(), 2);
    }

    /// The behaviour the collection pass exists for: a declaration may
    /// read one written below it.
    #[test]
    fn forward_reference_resolves() {
        let hir = hir_of("state b is client Whole from a\nstate a is client Whole starting 1\n")
            .expect("forward references are legal");
        assert_eq!(hir.defs.len(), 2);
    }

    #[test]
    fn a_reference_points_at_the_definition_it_names() {
        let hir = hir_of("state b is client Whole from a\nstate a is client Whole starting 1\n")
            .expect("resolves");
        let DefKind::Signal(b) = &hir.defs[hir.defs.iter().next().expect("a definition").0].kind
        else {
            panic!("expected a signal")
        };
        let HirExprKind::Ref(Res::Def(target)) = hir.exprs[b.init].kind else {
            panic!("expected a resolved reference")
        };
        assert_eq!(hir.defs[target].name, "a");
    }

    #[test]
    fn an_undefined_name_is_reported_with_its_span() {
        let errors = hir_of("state a is client Whole from missing\n").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("missing"),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("not"),
            "the message should say the name was not found: {}",
            errors[0].message
        );
        let start = "state a is client Whole from ".len() as u32;
        assert_eq!(errors[0].span, zdc_lexer::Span::new(start, start + 7));
    }

    #[test]
    fn a_function_parameter_is_in_scope_in_its_body() {
        hir_of("function double with n\n    give n + n\n").expect("resolves");
    }

    #[test]
    fn a_parameter_is_not_in_scope_outside_its_function() {
        let errors =
            hir_of("function f with n\n    give n\nstate a is client Whole from n\n").unwrap_err();
        assert_eq!(errors.len(), 1, "n must not leak out of f");
    }

    #[test]
    fn a_loop_variable_is_scoped_to_its_clause() {
        hir_of("function f with xs\n    from xs\n    keep each item where item.live\n")
            .expect("resolves");
    }

    #[test]
    fn a_loop_variable_does_not_escape_its_clause() {
        let errors = hir_of(
            "function f with xs\n    from xs\n    keep each item where item.live\n    give item\n",
        )
        .unwrap_err();
        assert_eq!(errors.len(), 1, "item must not outlive its clause");
    }

    #[test]
    fn every_pipeline_clause_binds_its_own_loop_name() {
        hir_of(
            "function f with xs\n\
             \x20   from xs\n\
             \x20   keep each a where a.live\n\
             \x20   sort each b by b.rank\n\
             \x20   map each c to c.name\n\
             \x20   take first 5\n",
        )
        .expect("resolves");
    }

    #[test]
    fn an_inner_binding_shadows_an_outer_one() {
        hir_of("function f with item\n    each item in item\n        give item\n")
            .expect("shadowing is legal");
    }

    #[test]
    fn a_loop_sequence_is_resolved_before_the_loop_name_is_bound() {
        // Without that ordering `each item in item` would iterate
        // itself, and this program would resolve with `f` having no
        // parameter at all.
        let errors = hir_of("function f\n    each item in item\n        give item\n").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("item"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn every_error_is_reported_not_just_the_first() {
        let errors =
            hir_of("state a is client Whole from nope\nstate b is client Whole from alsonope\n")
                .unwrap_err();
        assert_eq!(
            errors.len(),
            2,
            "resolution must not stop at the first error"
        );
    }

    /// Both sides of a binary expression are visited before the
    /// expression is judged, so two mistakes in one line are two
    /// diagnostics.
    #[test]
    fn both_halves_of_one_expression_are_reported() {
        let errors = hir_of("state a is client Whole from nope + alsonope\n").unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    /// The same rule across a list: every argument is visited before the
    /// call is judged.
    #[test]
    fn every_bad_argument_in_one_call_is_reported() {
        let errors = errors_of("view\n    Text nope, other is alsonope\n");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn a_when_arm_binds_one_name_per_named_field() {
        let hir = hir_of(
            "state entry is client Whole starting 1\n\
             function f\n\
             \x20   when entry\n\
             \x20       Failed with why, moment\n\
             \x20           give why\n",
        )
        .expect("resolves");

        let DefKind::Function(f) = &hir.defs[hir.defs.iter().nth(1).expect("a function").0].kind
        else {
            panic!("expected a function")
        };
        let HirStmt::When(when) = &hir.blocks[f.body].stmts[0] else {
            panic!("expected a when statement")
        };
        assert_eq!(when.arms[0].bindings.len(), 2);
        assert_eq!(hir.locals[when.arms[0].bindings[1]].name, "moment");
    }

    #[test]
    fn an_arm_binding_does_not_escape_its_arm() {
        let errors = hir_of(
            "state entry is client Whole starting 1\n\
             function f\n\
             \x20   when entry\n\
             \x20       Ready with value\n\
             \x20           give value\n\
             \x20   give value\n",
        )
        .unwrap_err();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn a_pattern_may_not_bind_one_name_twice() {
        let errors = errors_of(
            "state entry is client Whole starting 1\n\
             function f\n\
             \x20   when entry\n\
             \x20       Failed with why, why\n\
             \x20           give why\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bound twice"), "got: {}", errors[0]);
    }

    #[test]
    fn a_function_may_not_bind_one_parameter_twice() {
        let errors = errors_of("function f with a, a\n    give a\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bound twice"), "got: {}", errors[0]);
    }

    #[test]
    fn an_unknown_variant_name_names_the_ones_that_exist() {
        let errors = errors_of(
            "state entry is client Whole starting 1\n\
             view\n\
             \x20   when entry\n\
             \x20       Loadng show Spinner\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`Loading`"), "got: {}", errors[0]);
    }

    #[test]
    fn an_unknown_element_name_names_the_ones_that_exist() {
        let errors = errors_of("view\n    Colunm\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`Column`"), "got: {}", errors[0]);
    }

    /// A view element name is not a value name, so an element is looked
    /// up among the elements even where a local of the same name exists.
    #[test]
    fn a_local_does_not_hide_an_element_of_the_same_name() {
        hir_of(
            "state names is client Whole starting 1\n\
             view\n\
             \x20   each Text in names\n\
             \x20       Text Text\n",
        )
        .expect("resolves");
    }

    #[test]
    fn a_loop_name_in_a_view_reaches_the_handlers_inside_it() {
        hir_of(
            "state votes is durable Whole starting 0\n\
             state items is client Whole starting 0\n\
             view\n\
             \x20   each item in items\n\
             \x20       Row item.name\n\
             \x20           on click\n\
             \x20               add 1 to votes at item.id\n",
        )
        .expect("resolves");
    }

    #[test]
    fn a_view_loop_name_does_not_escape_the_loop() {
        let errors = errors_of(
            "state items is client Whole starting 0\n\
             view\n\
             \x20   each item in items\n\
             \x20       Text item\n\
             \x20   Text item\n",
        );
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn the_view_is_recorded_on_the_program() {
        let hir = hir_of("view\n    Column\n").expect("resolves");
        let view = hir.view.expect("a view");
        assert!(matches!(hir.defs[view].kind, DefKind::View(_)));
    }

    #[test]
    fn a_program_without_a_view_has_none() {
        let hir = hir_of("state a is client Whole starting 1\n").expect("resolves");
        assert_eq!(hir.view, None);
    }

    #[test]
    fn a_signal_records_whether_it_is_a_source() {
        let hir = hir_of("state a is client Whole starting 1\nstate b is client Whole from a\n")
            .expect("resolves");
        let kinds: Vec<bool> = hir
            .defs
            .iter()
            .map(|(_, def)| match &def.kind {
                DefKind::Signal(signal) => signal.is_source,
                other => panic!("expected a signal, got {other:?}"),
            })
            .collect();
        assert_eq!(kinds, [true, false]);
    }

    #[test]
    fn a_collection_error_is_returned_before_any_body_is_walked() {
        let errors = errors_of(
            "state a is client Whole starting 1\n\
             state a is client Whole starting 2\n\
             state b is client Whole from nope\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("already declared"), "got: {}", errors[0]);
    }

    // --- record and choice declarations (spec §14B.1, §14G.1.2) ---

    #[test]
    fn a_record_becomes_a_definition_with_its_fields_in_order() {
        let hir = hir_of("record Todo\n    id is Whole\n    title is Text\n").expect("resolves");
        let (_, def) = hir.defs.iter().next().expect("a definition");
        let DefKind::Record(record) = &def.kind else {
            panic!("expected a record, got {:?}", def.kind)
        };
        assert_eq!(def.name, "Todo");
        let names: Vec<&str> = record
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        assert_eq!(names, ["id", "title"]);
    }

    /// A payload-free variant is a value, so a bare name resolves to it.
    #[test]
    fn a_variant_name_resolves_to_its_choice_and_position() {
        let hir = hir_of(
            "choice Status\n\
             \x20   Active\n\
             \x20   Archived with reason is Text\n\
             state s is client Status starting Archived with reason is \"old\"\n",
        )
        .expect("resolves");
        let (_, def) = hir
            .defs
            .iter()
            .find(|(_, def)| def.name == "s")
            .expect("the signal");
        let DefKind::Signal(signal) = &def.kind else {
            panic!("expected a signal")
        };
        let HirExprKind::Call {
            callee: Res::Variant { index, .. },
            ..
        } = hir.exprs[signal.init].kind
        else {
            panic!(
                "expected a variant construction, got {:?}",
                hir.exprs[signal.init].kind
            )
        };
        assert_eq!(index, 1, "`Archived` is the second variant");
    }

    /// A `when` arm may name a variant a `choice` in this file declared.
    #[test]
    fn a_when_arm_may_match_a_declared_variant() {
        hir_of(
            "choice Status\n\
             \x20   Active\n\
             \x20   Archived with reason is Text\n\
             state s is client Status starting Active\n\
             function f\n\
             \x20   when s\n\
             \x20       Active show 1\n\
             \x20       Archived with why show 2\n",
        )
        .expect("resolves");
    }

    #[test]
    fn two_choices_may_not_declare_the_same_variant() {
        let errors = errors_of("choice A\n    Same\nchoice B\n    Same\n");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("already a variant"),
            "got: {}",
            errors[0]
        );
    }

    /// `when` matches by name, so a program-declared `Ready` would make a
    /// `Remote` arm mean two things (§14G.1.2).
    #[test]
    fn a_choice_may_not_redeclare_a_builtin_variant() {
        let errors = errors_of("choice Fetch\n    Ready\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`Option`"), "got: {}", errors[0]);
    }

    /// Construction is by name, so a field named twice would give one field
    /// two values.
    #[test]
    fn a_record_may_not_declare_one_field_twice() {
        let errors = errors_of("record Todo\n    id is Whole\n    id is Text\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("twice"), "got: {}", errors[0]);
    }

    /// A record shares the top-level namespace with signals and functions,
    /// because `Todo with …` is spelled exactly like a call.
    #[test]
    fn a_record_may_not_share_a_name_with_a_signal() {
        let errors =
            errors_of("state Todo is client Whole starting 1\nrecord Todo\n    id is Whole\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("already declared"), "got: {}", errors[0]);
    }

    #[test]
    fn a_collection_literal_resolves_every_element() {
        let errors = errors_of("state xs is client List of Whole starting [nope, alsonope]\n");
        assert_eq!(errors.len(), 2, "every element is visited: {errors:?}");
    }

    #[test]
    fn a_map_literal_resolves_both_halves_of_every_entry() {
        let errors =
            errors_of("state m is client Map of Whole to Whole starting [nope to alsonope]\n");
        assert_eq!(errors.len(), 2, "{errors:?}");
    }

    #[test]
    fn no_message_names_a_rust_type() {
        let sources = [
            "state a is client Whole from nope\n",
            "view\n    Colunm\n",
            "state a is client Whole starting 1\nview\n    when a\n        Nope show Spinner\n",
            "function f with a, a\n    give a\n",
            "choice A\n    Same\nchoice B\n    Same\n",
            "record Todo\n    id is Whole\n    id is Text\n",
            "choice Fetch\n    Ready\n",
        ];
        let forbidden = ["Ident", "TokenKind", "Expr", "DefId", "LocalId", "HirExpr"];

        for src in sources {
            for message in errors_of(src) {
                for needle in forbidden {
                    assert!(
                        !message.contains(needle),
                        "message for {src:?} leaked `{needle}`: {message}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_list_of_names_reads_as_english() {
        assert_eq!(english_list(&["a"]), "`a`");
        assert_eq!(english_list(&["a", "b"]), "`a`, and `b`");
        assert_eq!(english_list(&["a", "b", "c"]), "`a`, `b`, and `c`");
    }
}
