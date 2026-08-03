//! Expression emission, per spec §16.3.3.
//!
//! Three positions, one classifier. **Value position** produces the value.
//! **Getter position** — any argument the runtime will `read()` — produces
//! the getter itself when the expression already is one, a closure when it
//! reads a signal, and a bare constant otherwise. Never `() => X()`: a
//! signal read and a `derived` *are* the getter, and double-wrapping hands
//! the runtime a function where it expected a variant.

use zdc_ast::{BinOp, UnaryOp};
use zdc_graph::{Ctx, Region, RootId, TierSplit};
use zdc_hir::{DefKind, ExprId, Hir, HirArg, HirExprKind, Res};
use zdc_types::{Type, TypeTable};

use crate::analysis::Analysis;
use crate::js::{self, precedence, Expr};
use crate::names::Names;
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
    pub names: &'a Names,
    pub analysis: &'a Analysis,
    /// The placement pass's answers. Which root is being emitted decides
    /// how a read is spelled: a browser reads a signal by calling its
    /// getter, a server invocation reads a plain `const` (§17.2.8).
    pub split: &'a TierSplit,
    pub table: &'a TypeTable,
    pub ctx: Ctx,
    pub root: RootId,
    pub unchecked: bool,
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
            HirExprKind::Empty => {
                self.error(
                    "`empty` cannot be compiled yet: whether it is an empty list or an empty map \
                     is a question for the type checker, which does not exist (spec §16.7).",
                    expr.span,
                );
                Expr::primary("undefined")
            }
            // §5.6 confines this to server context, and the split has
            // already rejected every other placement with E0360 — so by
            // the time emission runs, the only remaining question is how
            // to spell it. `$env` is injected by the platform adapter.
            HirExprKind::Environment(key) => {
                let key = key.clone();
                Expr::new(format!("$env({})", js::string(&key)), precedence::MEMBER)
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
                let base = self.value(*base);
                Expr::new(
                    format!("{}.{name}", base.operand(precedence::MEMBER)),
                    precedence::MEMBER,
                )
            }
            HirExprKind::Index { base, .. } => {
                let span = self.hir.exprs[*base].span;
                self.error(
                    "`at` cannot be compiled yet: indexing returns `Option of T`, and choosing \
                     between the list and the map helper needs the type checker (spec §16.7).",
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
                Res::Builtin(_) => unreachable!("a built-in is never a getter"),
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
                // In the browser a signal is read by calling it, source
                // or derived alike — `const [count, setCount] = signal(0)`,
                // `const doubled = derived(...)` and `const greeting =
                // $remote(...)` all bind a getter. In a server invocation
                // there is no graph and no getter: every member of the root
                // is a plain `const`, including the values the client
                // lifted up to it.
                DefKind::Signal(_) => {
                    if self.ctx.region == Region::Client {
                        Expr::new(format!("{}()", self.names.def(def)), precedence::MEMBER)
                    } else {
                        Expr::primary(self.names.def(def).to_string())
                    }
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
            },
            Res::Local(local) => {
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

    /// A call, with named arguments reordered into declaration order.
    fn call(&mut self, callee: Res, args: &[HirArg], span: zdc_lexer::Span) -> Expr {
        let Res::Def(def) = callee else {
            self.error(
                "Only a top-level `function` can be called; ZDeceptron has no first-class \
                 functions.",
                span,
            );
            return Expr::primary("undefined");
        };
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

    /// The type the checker settled for an expression **in this root's
    /// context**. Code generation always knows which root it is emitting,
    /// and therefore which context to ask for (§17.1.4 item 3).
    fn settled(&self, id: ExprId) -> Option<&Type> {
        self.table
            .expr_in(id, self.ctx.read_context())
            .or_else(|| self.table.expr(id))
            .filter(|ty| !matches!(ty, Type::Unknown))
    }

    fn both_numeric_or_both_text(&self, lhs: ExprId, rhs: ExprId) -> bool {
        let (Some(left), Some(right)) = (self.settled(lhs), self.settled(rhs)) else {
            return false;
        };
        let numeric = |ty: &Type| matches!(ty, Type::Whole | Type::Decimal);
        (numeric(left) && numeric(right))
            || (matches!(left, Type::Text) && matches!(right, Type::Text))
    }

    fn comparable_by_value(&self, id: ExprId) -> bool {
        self.settled(id)
            .is_some_and(|ty| matches!(ty, Type::Text | Type::Whole | Type::Decimal | Type::Truth))
    }

    fn binary(&mut self, op: BinOp, lhs: ExprId, rhs: ExprId, span: zdc_lexer::Span) -> Expr {
        // Two operators are correct emission only once the checker has
        // proved the operand types (§16.7). Emitting them anyway would
        // reintroduce the exact JavaScript defects §5.4 claims to have
        // excluded by construction, so they are refused rather than
        // guessed at.
        if !self.unchecked {
            match op {
                // §16.7 items 1 and 2, answered. `+` is f64 addition when
                // both operands are proved numeric and concatenation when
                // both are proved `Text`; `===` is value equality for a
                // base type and reference equality for everything else.
                // Emitting either without the proof reintroduces exactly
                // the coercion §5.4 claims to have excluded.
                BinOp::Add => {
                    if !self.both_numeric_or_both_text(lhs, rhs) {
                        self.error(
                            "`+` needs both operands proved numeric or both proved `Text`, and \
                             the checker did not settle them that way. Pass `--unchecked` to \
                             emit it anyway (spec §16.7).",
                            span,
                        );
                    }
                }
                BinOp::Is | BinOp::IsNot if !self.comparable_by_value(lhs) => {
                    self.error(
                        "`is` compiles to `===`, which is value equality only for `Text`, \
                             `Whole`, `Decimal` and `Truth`. The checker did not settle this \
                             operand to one of those, so `===` would compare references. Pass \
                             `--unchecked` to emit it anyway (spec §16.7).",
                        span,
                    );
                }
                _ => {}
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
