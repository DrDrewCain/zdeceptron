//! Expression emission, per spec §16.3.3.
//!
//! Three positions, one classifier. **Value position** produces the value.
//! **Getter position** — any argument the runtime will `read()` — produces
//! the getter itself when the expression already is one, a closure when it
//! reads a signal, and a bare constant otherwise. Never `() => X()`: a
//! signal read and a `derived` *are* the getter, and double-wrapping hands
//! the runtime a function where it expected a variant.

use zdc_ast::{BinOp, UnaryOp};
use zdc_hir::{DefId, DefKind, ExprId, Hir, HirArg, HirExprKind, Res};
use zdc_types::{EmptyKind, Type, TypeTable};

use crate::analysis::Analysis;
use crate::events;
use crate::js::{self, precedence, Expr};
use crate::names::Names;
use crate::pages::{Binding, Bindings};
use crate::view::RuntimeImports;
use crate::CodegenError;

/// What an expression is worth in getter position.
#[derive(Debug, Clone)]
pub enum Operand {
    /// A compile-time literal: bakeable straight into markup.
    Literal(Literal),
    /// No signal read anywhere in the subtree, but not a literal either.
    /// One assignment at clone time; no effect is allocated.
    Static(String),
    /// A getter. Either the signal itself or a closure around a read.
    Reactive(String),
}

#[derive(Debug, Clone)]
pub enum Literal {
    Number(f64),
    Text(String),
    Truth(bool),
}

impl Literal {
    /// What `String(value)` would produce, for baking into markup.
    pub fn as_text(&self) -> String {
        match self {
            Literal::Number(n) => js::number_to_text(*n),
            Literal::Text(text) => text.clone(),
            Literal::Truth(truth) => truth.to_string(),
        }
    }

    pub fn as_js(&self) -> String {
        match self {
            Literal::Number(n) => js::number(*n),
            Literal::Text(text) => js::string(text),
            Literal::Truth(truth) => truth.to_string(),
        }
    }
}

pub struct Emitter<'a> {
    pub hir: &'a Hir,
    /// Every type the checker found (§16.7). Codegen is a consumer of a
    /// verdict, never a producer of one: `+`, `is`, `empty` and `when` are
    /// all decisions this table settles, and there is no path through this
    /// module that guesses one.
    pub types: &'a TypeTable,
    pub names: &'a Names,
    pub analysis: &'a Analysis,
    /// The binders this document's address fold replaced with constants
    /// (spec §14G.2 revision 1). Empty for an unrouted program.
    pub bindings: &'a Bindings,
    /// The runtime symbols the emission has used so far, so the import
    /// list names exactly what the module calls.
    pub used: RuntimeImports,
    pub errors: Vec<CodegenError>,
}

impl<'a> Emitter<'a> {
    pub fn error(&mut self, message: impl Into<String>, span: zdc_lexer::Span) {
        self.errors.push(CodegenError {
            message: message.into(),
            span,
        });
    }

    /// An expression in value position.
    pub fn value(&mut self, id: ExprId) -> Expr {
        let expr = &self.hir.exprs[id];
        match &expr.kind {
            HirExprKind::Number(n) => Expr::primary(js::number(*n)),
            HirExprKind::Text(text) => Expr::primary(js::string(text)),
            HirExprKind::Truth(truth) => Expr::primary(truth.to_string()),
            // §16.7 item 6: which container `empty` is comes off the
            // checker's verdict, never off the syntax.
            HirExprKind::Empty => match self.types.empty_kind(id) {
                Some(EmptyKind::List) => Expr::primary("[]"),
                Some(EmptyKind::Map) => Expr::primary("new Map()"),
                None => {
                    self.error(
                        "`empty` is a list or a map, and nothing here says which. Write the type \
                         on the state it starts.",
                        expr.span,
                    );
                    Expr::primary("undefined")
                }
            },
            HirExprKind::List(items) => {
                let items = items.clone();
                let emitted: Vec<String> = items
                    .into_iter()
                    .map(|item| self.value(item).into_text())
                    .collect();
                Expr::primary(format!("[{}]", emitted.join(", ")))
            }
            // A `Map` is a JavaScript `Map`, not an object: §5.4's keys are
            // values of a declared key type, and an object would coerce
            // every one of them to a string.
            HirExprKind::Map(entries) => {
                let entries = entries.clone();
                let emitted: Vec<String> = entries
                    .into_iter()
                    .map(|(key, value)| {
                        let key = self.value(key).into_text();
                        let value = self.value(value).into_text();
                        format!("[{key}, {value}]")
                    })
                    .collect();
                Expr::primary(format!("new Map([{}])", emitted.join(", ")))
            }
            HirExprKind::Environment(key) => {
                self.error(
                    format!(
                        "`environment \"{key}\"` is only legal in `server` and `durable` context \
                         (spec §5.6), and this build emits a client bundle only."
                    ),
                    expr.span,
                );
                Expr::primary("undefined")
            }
            HirExprKind::Address => {
                self.error(
                    "`address` is read by the signal that holds it, which the build folds to one \
                     value per document. Write `state page is client Option of <route> starting \
                     address` and dispatch on `page` with `when`.",
                    expr.span,
                );
                Expr::primary("undefined")
            }
            HirExprKind::Ref(res) => self.reference(*res, expr.span),
            HirExprKind::Call { callee, args } => self.call(*callee, args, expr.span),
            HirExprKind::Unary { op, operand } => {
                let inner = self.value(*operand);
                let symbol = match op {
                    UnaryOp::Not => "!",
                    UnaryOp::Neg => "-",
                };
                Expr::new(
                    format!("{symbol}{}", inner.operand(precedence::UNARY)),
                    precedence::UNARY,
                )
            }
            HirExprKind::Binary { op, lhs, rhs } => self.binary(*op, *lhs, *rhs, expr.span),
            HirExprKind::Field { base, name } => {
                // A field of an event payload is a property of the
                // browser's event under a different spelling — `press.x`
                // is `press.clientX`. The checker already settled which
                // payload this is, so this is a lookup rather than a guess.
                let accessor = match self.types.expr(*base) {
                    Some(Type::Event(payload)) => events::accessor(*payload, name),
                    _ => None,
                };
                let field = accessor.unwrap_or(name.as_str()).to_string();
                let base = self.value(*base);
                Expr::new(
                    format!("{}.{field}", base.operand(precedence::MEMBER)),
                    precedence::MEMBER,
                )
            }
            HirExprKind::Index { base, .. } => {
                let span = self.hir.exprs[*base].span;
                self.error(
                    "`at` cannot be compiled yet. The checker says which container this is, but \
                     indexing yields `Option of T` (spec §5.4) and the runtime has no `$at` to \
                     build one with — that is §14F's standard library, not a type question \
                     (spec §16.7 item 5).",
                    span,
                );
                Expr::primary("undefined")
            }
        }
    }

    /// An expression in getter position, classified per §16.3.3.
    pub fn operand(&mut self, id: ExprId) -> Operand {
        match &self.hir.exprs[id].kind {
            HirExprKind::Number(n) => return Operand::Literal(Literal::Number(*n)),
            HirExprKind::Text(text) => return Operand::Literal(Literal::Text(text.clone())),
            HirExprKind::Truth(truth) => return Operand::Literal(Literal::Truth(*truth)),
            _ => {}
        }

        // Already a getter: pass it through untouched.
        if let Some(res) = self.analysis.bare_getter(self.hir, id) {
            let name = match res {
                Res::Def(def) => self.names.def(def).to_string(),
                Res::Local(local) => self.names.local(local).to_string(),
                // `bare_getter` answers only for signals and reactive
                // binders, and neither of these is one.
                Res::Builtin(_) | Res::Variant { .. } => {
                    unreachable!("a built-in and a variant are never getters")
                }
            };
            return Operand::Reactive(name);
        }

        let reactive = self.analysis.reads_signal(self.hir, id);
        let value = self.value(id).into_text();
        if reactive {
            Operand::Reactive(format!("() => {value}"))
        } else {
            Operand::Static(value)
        }
    }

    fn reference(&mut self, res: Res, span: zdc_lexer::Span) -> Expr {
        match res {
            Res::Def(def) => match &self.hir.defs[def].kind {
                // A client signal is read by calling it, source or derived
                // alike — `const [count, setCount] = signal(0)` and
                // `const doubled = derived(...)` both bind a getter.
                // A `static` signal is a build-time constant inlined
                // into the module, so it is a binding rather than a cell
                // and reading it is not a call (§14C.3b).
                DefKind::Signal(signal) if signal.placement == zdc_ast::Placement::Static => {
                    Expr::primary(self.names.def(def).to_string())
                }
                DefKind::Signal(_) => {
                    Expr::new(format!("{}()", self.names.def(def)), precedence::MEMBER)
                }
                DefKind::Function(_) => {
                    self.error(
                        format!(
                            "`{}` is a function, and ZDeceptron has no first-class functions: \
                             call it with `{}` and its arguments.",
                            self.hir.defs[def].name, self.hir.defs[def].name
                        ),
                        span,
                    );
                    Expr::primary("undefined")
                }
                DefKind::View(_) => {
                    self.error("The view is not a value.", span);
                    Expr::primary("undefined")
                }
                DefKind::Component(_) => {
                    self.error(
                        format!(
                            "`{}` is a component, so it is a run of view nodes rather than a \
                             value.",
                            self.hir.defs[def].name
                        ),
                        span,
                    );
                    Expr::primary("undefined")
                }
                DefKind::Record(_) | DefKind::Choice(_) => {
                    self.error(
                        format!("`{}` names a type, not a value.", self.hir.defs[def].name),
                        span,
                    );
                    Expr::primary("undefined")
                }
            },
            // A payload-free variant: `{ tag, fields }`, exactly what
            // `when` dispatches on (§16.3).
            Res::Variant { choice, index } => self.variant(choice, index, &[], span),
            Res::Local(local) => {
                // A binder the address fold replaced *is* its value in
                // this document, so it needs no cell and no read.
                if let Some(binding) = self.bindings.get(local) {
                    return self.folded(binding, span);
                }
                if self.analysis.is_reactive_local(local) {
                    // The row outlives any one version of its item, so the
                    // binder is a getter and reading it is a call.
                    Expr::new(format!("{}()", self.names.local(local)), precedence::MEMBER)
                } else {
                    Expr::primary(self.names.local(local).to_string())
                }
            }
            Res::Builtin(_) => {
                self.error("A built-in name is not a value.", span);
                Expr::primary("undefined")
            }
        }
    }

    /// A binder the address fold replaced with a constant.
    fn folded(&mut self, binding: &Binding, span: zdc_lexer::Span) -> Expr {
        match binding {
            Binding::Literal(literal) => Expr::primary(literal.as_js()),
            Binding::Route { variant, values } => {
                let Some((choice, _)) = self.hir.routes else {
                    self.error("A route value needs a `route` declaration.", span);
                    return Expr::primary("undefined");
                };
                let DefKind::Choice(declared) = &self.hir.defs[choice].kind else {
                    self.error("A route value belongs to a `route`.", span);
                    return Expr::primary("undefined");
                };
                let Some(declared) = declared.variants.get(*variant) else {
                    self.error("A route value belongs to a `route`.", span);
                    return Expr::primary("undefined");
                };
                let name = js::string(&declared.name);
                self.used.dom.insert("variant");
                let mut emitted = vec![name];
                emitted.extend(values.iter().map(|value| js::string(value)));
                Expr::new(
                    format!("variant({})", emitted.join(", ")),
                    precedence::MEMBER,
                )
            }
        }
    }

    /// The URL a `Link`'s route value renders, per §14G.2's bijection.
    ///
    /// One function, and it delegates the rendering itself to
    /// `RouteTable::url`, which is also what the collision check, the
    /// per-page split and the manifest use. Four renderings of a URL
    /// would be four chances to disagree about where a link goes.
    /// Whether this expression denotes one of the program's own routes.
    ///
    /// The destination slot takes a route value or a URL, and which one
    /// is in hand is answered structurally here rather than by type,
    /// because the URL a route renders is a compile-time question and
    /// this is where it is answered.
    pub fn is_route_value(&self, id: ExprId) -> bool {
        let Some((choice, _)) = &self.hir.routes else {
            return false;
        };
        match &self.hir.exprs[id].kind {
            HirExprKind::Ref(Res::Variant { choice: owner, .. }) => owner == choice,
            HirExprKind::Call {
                callee: Res::Variant { choice: owner, .. },
                ..
            } => owner == choice,
            HirExprKind::Ref(Res::Local(local)) => {
                matches!(self.bindings.get(*local), Some(Binding::Route { .. }))
            }
            HirExprKind::Ref(Res::Def(_))
            | HirExprKind::Ref(Res::Builtin(_))
            | HirExprKind::Call { .. }
            | HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            | HirExprKind::List(_)
            | HirExprKind::Map(_)
            | HirExprKind::Environment(_)
            | HirExprKind::Address
            | HirExprKind::Unary { .. }
            | HirExprKind::Binary { .. }
            | HirExprKind::Field { .. }
            | HirExprKind::Index { .. } => false,
        }
    }

    pub fn route_url(&mut self, id: ExprId) -> Operand {
        let span = self.hir.exprs[id].span;
        let Some((choice, table)) = &self.hir.routes else {
            self.error(
                "`Link` navigates to a route, and this program declares none.",
                span,
            );
            return Operand::Literal(Literal::Text(String::new()));
        };
        let (choice, table) = (*choice, table.clone());

        let (index, args) = match &self.hir.exprs[id].kind {
            HirExprKind::Ref(Res::Variant {
                choice: owner,
                index,
            }) if *owner == choice => (*index as usize, Vec::new()),
            HirExprKind::Call {
                callee:
                    Res::Variant {
                        choice: owner,
                        index,
                    },
                args,
            } if *owner == choice => (*index as usize, args.clone()),
            HirExprKind::Ref(Res::Local(local)) => match self.bindings.get(*local) {
                Some(Binding::Route { variant, values }) => {
                    return Operand::Literal(Literal::Text(table.url(*variant, values)));
                }
                Some(Binding::Literal(_)) | None => {
                    self.error(
                        "`Link` takes a route value written where the link is, as in `Link Home` \
                         or `Link (BlogPost with slug is post.slug)`. A route held in a binder \
                         cannot be linked to yet, because the URL it renders is not known where \
                         the anchor is written.",
                        span,
                    );
                    return Operand::Literal(Literal::Text(String::new()));
                }
            },
            HirExprKind::Ref(Res::Variant { .. })
            | HirExprKind::Ref(Res::Def(_))
            | HirExprKind::Ref(Res::Builtin(_))
            | HirExprKind::Call { .. }
            | HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            | HirExprKind::List(_)
            | HirExprKind::Map(_)
            | HirExprKind::Environment(_)
            | HirExprKind::Address
            | HirExprKind::Unary { .. }
            | HirExprKind::Binary { .. }
            | HirExprKind::Field { .. }
            | HirExprKind::Index { .. } => {
                self.error(
                    "`Link` takes a route value, as in `Link Home` or \
                     `Link (BlogPost with slug is post.slug)`.",
                    span,
                );
                return Operand::Literal(Literal::Text(String::new()));
            }
        };

        let Some(variant) = table.variants.get(index).cloned() else {
            self.error("This route has no URL.", span);
            return Operand::Literal(Literal::Text(String::new()));
        };
        let DefKind::Choice(declared) = &self.hir.defs[choice].kind else {
            return Operand::Literal(Literal::Text(String::new()));
        };
        let fields: Vec<String> = declared.variants[index]
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect();

        // Parameters in declaration order, because that is the order the
        // URL puts them in.
        let mut operands: Vec<Operand> = Vec::with_capacity(fields.len());
        for field in &fields {
            let found = args.iter().find_map(|arg| match arg {
                HirArg::Named { name, value } if name == field => Some(*value),
                HirArg::Named { .. } | HirArg::Positional(_) => None,
            });
            match found {
                Some(expr) => operands.push(self.operand(expr)),
                None => {
                    self.error(
                        format!("This link is missing a value for the route parameter `{field}`."),
                        span,
                    );
                    return Operand::Literal(Literal::Text(String::new()));
                }
            }
        }

        url_operand(&table, index, &variant.path, &operands)
    }

    /// One variant value: `variant('Archived', reason)`.
    ///
    /// Fields are emitted in *declaration* order however the literal wrote
    /// them, because a pattern binds positionally over the same order
    /// (§14G.1.2) and `whenInto` hands `fields` straight to the arm.
    fn variant(
        &mut self,
        choice: DefId,
        index: u32,
        args: &[HirArg],
        span: zdc_lexer::Span,
    ) -> Expr {
        let DefKind::Choice(declared) = &self.hir.defs[choice].kind else {
            self.error("A variant belongs to a `choice`.", span);
            return Expr::primary("undefined");
        };
        let Some(variant) = declared.variants.get(index as usize) else {
            self.error("A variant belongs to a `choice`.", span);
            return Expr::primary("undefined");
        };
        let name = variant.name.clone();
        let fields: Vec<String> = variant.fields.iter().map(|f| f.name.clone()).collect();

        let Some(values) = self.by_declaration_order(&name, &fields, args, span) else {
            return Expr::primary("undefined");
        };
        self.used.dom.insert("variant");
        let mut emitted = vec![js::string(&name)];
        emitted.extend(values);
        Expr::new(
            format!("variant({})", emitted.join(", ")),
            precedence::MEMBER,
        )
    }

    /// One record value: a plain object with its fields in declaration
    /// order, so every instance shares one hidden class (§16.7 item 9).
    fn record(&mut self, def: DefId, args: &[HirArg], span: zdc_lexer::Span) -> Expr {
        let DefKind::Record(declared) = &self.hir.defs[def].kind else {
            self.error("A record literal names a `record`.", span);
            return Expr::primary("undefined");
        };
        let name = self.hir.defs[def].name.clone();
        let fields: Vec<String> = declared.fields.iter().map(|f| f.name.clone()).collect();

        let Some(values) = self.by_declaration_order(&name, &fields, args, span) else {
            return Expr::primary("undefined");
        };
        let pairs: Vec<String> = fields
            .iter()
            .zip(values)
            .map(|(field, value)| format!("{field}: {value}"))
            .collect();
        Expr::primary(format!("{{ {} }}", pairs.join(", ")))
    }

    /// The argument values of a record or variant literal, reordered into
    /// declaration order.
    ///
    /// The checker has already reported a missing, repeated or unknown
    /// field, so anything wrong here means codegen ran without a verdict;
    /// it refuses rather than emitting an object with a hole in it.
    fn by_declaration_order(
        &mut self,
        owner: &str,
        fields: &[String],
        args: &[HirArg],
        span: zdc_lexer::Span,
    ) -> Option<Vec<String>> {
        let mut slots: Vec<Option<ExprId>> = vec![None; fields.len()];
        for arg in args {
            let HirArg::Named { name, value } = arg else {
                self.error(format!("`{owner}` is built by naming its fields."), span);
                return None;
            };
            match fields.iter().position(|field| field == name) {
                Some(at) => slots[at] = Some(*value),
                None => {
                    self.error(format!("`{owner}` has no field named `{name}`."), span);
                    return None;
                }
            }
        }
        let mut values = Vec::with_capacity(fields.len());
        for (at, slot) in slots.iter().enumerate() {
            match slot {
                Some(expr) => values.push(self.value(*expr).into_text()),
                None => {
                    self.error(
                        format!("`{owner}` is missing a value for `{}`.", fields[at]),
                        span,
                    );
                    return None;
                }
            }
        }
        Some(values)
    }

    /// A call, with named arguments reordered into declaration order.
    fn call(&mut self, callee: Res, args: &[HirArg], span: zdc_lexer::Span) -> Expr {
        // `Todo with title is "x"` shares this production with a call, so
        // the definition decides which it is (§4.4, §14B.4).
        if let Res::Variant { choice, index } = callee {
            return self.variant(choice, index, args, span);
        }
        let Res::Def(def) = callee else {
            self.error(
                "Only a top-level `function` can be called; ZDeceptron has no first-class \
                 functions.",
                span,
            );
            return Expr::primary("undefined");
        };
        if matches!(self.hir.defs[def].kind, DefKind::Record(_)) {
            return self.record(def, args, span);
        }
        let DefKind::Function(function) = &self.hir.defs[def].kind else {
            self.error(
                format!("`{}` is not a function.", self.hir.defs[def].name),
                span,
            );
            return Expr::primary("undefined");
        };

        let params: Vec<String> = function
            .params
            .iter()
            .map(|param| self.hir.locals[*param].name.clone())
            .collect();
        let name = self.names.def(def).to_string();

        let mut ordered: Vec<Option<ExprId>> = vec![None; params.len()];
        let mut next_positional = 0;
        for arg in args {
            match arg {
                HirArg::Positional(expr) => {
                    if next_positional >= ordered.len() {
                        self.error(
                            format!(
                                "`{}` takes {} argument(s), and this call passes more.",
                                self.hir.defs[def].name,
                                params.len()
                            ),
                            span,
                        );
                        return Expr::primary("undefined");
                    }
                    ordered[next_positional] = Some(*expr);
                    next_positional += 1;
                }
                HirArg::Named {
                    name: arg_name,
                    value,
                } => match params.iter().position(|param| param == arg_name) {
                    Some(index) => ordered[index] = Some(*value),
                    None => {
                        self.error(
                            format!(
                                "`{}` has no parameter named `{arg_name}`. Its parameters are {}.",
                                self.hir.defs[def].name,
                                params.join(", ")
                            ),
                            span,
                        );
                        return Expr::primary("undefined");
                    }
                },
            }
        }

        let mut emitted = Vec::with_capacity(ordered.len());
        for (index, slot) in ordered.iter().enumerate() {
            match slot {
                Some(expr) => emitted.push(self.value(*expr).into_text()),
                None => {
                    self.error(
                        format!(
                            "`{}` is missing an argument for `{}`.",
                            self.hir.defs[def].name, params[index]
                        ),
                        span,
                    );
                    return Expr::primary("undefined");
                }
            }
        }

        Expr::new(
            format!("{name}({})", emitted.join(", ")),
            precedence::MEMBER,
        )
    }

    fn binary(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId, span: zdc_lexer::Span) -> Expr {
        // §16.7 item 2. `===` is value equality for the base types and
        // *reference* equality for everything else, and the runtime has no
        // structural comparison to fall back on, so comparing two lists
        // would silently answer a different question than it looks like.
        // The checker has proved the operand type by now, so this is a
        // decision rather than a guess.
        if matches!(op, BinOp::Is | BinOp::IsNot) {
            let compared = self.types.expr(lhs).cloned().unwrap_or(Type::Unknown);
            if !is_compared_by_value(&compared) {
                self.error(
                    format!(
                        "`is` compares `{compared}` by identity rather than by contents, because \
                         the runtime has no structural comparison. Compare a `Text`, `Whole`, \
                         `Decimal` or `Truth` field instead (spec §16.7)."
                    ),
                    span,
                );
                return Expr::primary("undefined");
            }
        }

        let (symbol, level) = match op {
            BinOp::Or => ("||", precedence::OR),
            BinOp::And => ("&&", precedence::AND),
            // `===`, not `Object.is`: `-0 === 0` is true, which is the
            // right answer in an f64 language and what Elm and Dart do.
            // `signal.js`'s `Object.is` is change detection, a different
            // relation with a different job.
            BinOp::Is => ("===", precedence::EQUALITY),
            BinOp::IsNot => ("!==", precedence::EQUALITY),
            BinOp::Less => ("<", precedence::RELATIONAL),
            BinOp::Greater => (">", precedence::RELATIONAL),
            BinOp::LessEq => ("<=", precedence::RELATIONAL),
            BinOp::GreaterEq => (">=", precedence::RELATIONAL),
            BinOp::Add => ("+", precedence::ADDITIVE),
            BinOp::Sub => ("-", precedence::ADDITIVE),
            BinOp::Mul => ("*", precedence::MULTIPLICATIVE),
            BinOp::Div => ("/", precedence::MULTIPLICATIVE),
        };

        let left = self.value(lhs);
        let right = self.value(rhs);
        // Every operator here is left-associative, so the right operand
        // needs a parenthesis one level earlier than the left one does.
        Expr::new(
            format!(
                "{} {symbol} {}",
                left.operand(level),
                right.operand(level + 1)
            ),
            level,
        )
    }
}

/// A route's URL as an operand: a literal when every parameter is one,
/// and a getter when any of them reads a signal.
///
/// A link inside an `each` is the case that matters — the row's binder is
/// a getter, so the `href` has to be one too, or every row would link to
/// whatever the first row held when the template was cloned.
fn url_operand(
    table: &zdc_hir::RouteTable,
    index: usize,
    path: &str,
    values: &[Operand],
) -> Operand {
    let literals: Option<Vec<String>> = values
        .iter()
        .map(|value| match value {
            Operand::Literal(literal) => Some(literal.as_text()),
            Operand::Static(_) | Operand::Reactive(_) => None,
        })
        .collect();
    if let Some(literals) = literals {
        return Operand::Literal(Literal::Text(table.url(index, &literals)));
    }

    let mut source = js::string(path.trim_end_matches('/'));
    let mut reactive = false;
    for value in values {
        let piece = match value {
            Operand::Literal(literal) => js::string(&format!("/{}", literal.as_text())),
            Operand::Static(js) => format!("'/' + String({js})"),
            Operand::Reactive(getter) => {
                reactive = true;
                format!("'/' + String(({getter})())")
            }
        };
        source.push_str(" + ");
        source.push_str(&piece);
    }
    if reactive {
        Operand::Reactive(format!("() => {source}"))
    } else {
        Operand::Static(source)
    }
}

/// Whether `===` answers the question `is` asks for this type.
///
/// It does for the base types, and it does not for a `List`, a `Map`, a
/// record or a variant, where it compares identity (§16.3.3, §16.7 item 2).
fn is_compared_by_value(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Text | Type::Whole | Type::Decimal | Type::Truth | Type::Error
    )
}
