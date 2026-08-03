//! Statements: function bodies and event handlers, per spec §16.3.10.
//!
//! Two things here are load-bearing and easy to get wrong in the other
//! direction. `.slice()` before `.sort()` is mandatory, because ZD values
//! are immutable and `signal.write` compares with `Object.is`, so an
//! in-place sort would both mutate a shared value and defeat change
//! detection. And a mutation emits the *read* form — `setCount(count() + 1)`
//! rather than `setCount(v => v + 1)` — because an event handler is not a
//! tracking context, so the read registers no dependency edge and allocates
//! nothing per click.

use zdc_graph::MutCrossing;
use zdc_hir::{
    BlockId, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirExprKind, HirMutation,
    HirPipeline, HirStmt, HirWhen, LocalId, Res,
};

use crate::expr::Emitter;
use crate::js::precedence;

/// The function currently being emitted, when its body gives the result
/// of calling itself.
///
/// JavaScript engines do not eliminate tail calls — the ES6 proposal for
/// it shipped in no major engine — so a fold written as tail recursion
/// costs one frame per element exactly as an index recursion does. This
/// is what makes §17.4.10's accumulator worth writing: the emitter turns
/// a call in tail position into a jump, and the depth of a fold stops
/// depending on the length of what it folds.
///
/// Only *self* calls. Mutual recursion needs a trampoline, which costs an
/// allocation per step and would be paid by every call in the program to
/// help the ones that are not self-recursive; §17.4.9's library has none.
#[derive(Debug, Clone)]
pub struct TailSelfCall {
    pub def: DefId,
    /// The emitted parameter names, in declaration order.
    pub params: Vec<LocalId>,
}

/// Statements share the expression emitter's error list and name table.
pub struct Statements<'a, 'h> {
    pub emitter: &'a mut Emitter<'h>,
    /// Numbers the `$w` temporaries a statement `when` needs.
    pub temporaries: usize,
    /// Set when a statement emitted an `await`.
    ///
    /// A cross-region write is a network call, so the block that contains
    /// one has to be `async` and has to have somewhere for a rejection to
    /// go. Recorded here rather than recovered by searching the emitted
    /// text for `await`, because that string also appears inside any
    /// string literal a program happens to contain.
    pub awaited: bool,
    /// Set only when emitting a function body wrapped in `$tail`.
    pub tail: Option<TailSelfCall>,
}

/// The pair a write goes through, and what it holds.
struct Target {
    getter: String,
    setter: String,
    /// The name the program wrote, for diagnostics.
    declared: String,
    container: zdc_types::Type,
}

/// Whether this body ever gives the result of calling `def` itself.
///
/// Decided before a line is emitted, because the answer chooses whether
/// the body is wrapped in a loop at all: a function that does not recurse
/// keeps exactly the emission it had, which is what keeps §16.4's worked
/// output byte-identical.
pub fn gives_a_self_call(hir: &Hir, def: DefId, block: BlockId) -> bool {
    hir.blocks[block].stmts.iter().any(|stmt| match stmt {
        HirStmt::Give(expr) => is_self_call(hir, def, *expr),
        HirStmt::When(when) => when.arms.iter().any(|arm| match arm.body {
            HirArmBody::Show(expr) => is_self_call(hir, def, expr),
            HirArmBody::Block(body) => gives_a_self_call(hir, def, body),
        }),
        HirStmt::Each(each) => gives_a_self_call(hir, def, each.body),
        HirStmt::If(conditional) => {
            gives_a_self_call(hir, def, conditional.then)
                || conditional
                    .otherwise
                    .is_some_and(|body| gives_a_self_call(hir, def, body))
        }
        HirStmt::Pipeline(_) | HirStmt::Mutation(_) | HirStmt::Bind(_) => false,
    })
}

/// A call to `def` and nothing wrapped around it. `sumFrom with …` and
/// `first of …` are both calls; `1 + (sumFrom with …)` is not, because
/// the addition still has to happen after the call comes back.
fn is_self_call(hir: &Hir, def: DefId, expr: ExprId) -> bool {
    let callee = match &hir.exprs[expr].kind {
        HirExprKind::Call { callee, .. } | HirExprKind::OfCall { callee, .. } => *callee,
        _ => return false,
    };
    callee == Res::Def(def)
}

/// What a mutation does to the value already in the place.
#[derive(Debug, Clone, Copy)]
enum Operator {
    /// `set` — the new value replaces the old one.
    Replace,
    /// `add` and `subtract` — numbers only (§14B.2).
    Arithmetic(char),
    /// `append` — collections only (§14B.2).
    Append,
    /// `remove` — collections only (§14B.2).
    Remove,
}

impl Statements<'_, '_> {
    /// A block as a brace-free run of statements at `indent` spaces.
    pub fn block(&mut self, id: BlockId, indent: usize, out: &mut String) {
        let stmts = self.emitter.hir.blocks[id].stmts.clone();
        let mut index = 0;
        while index < stmts.len() {
            if matches!(stmts[index], HirStmt::Pipeline(_)) {
                let start = index;
                while index < stmts.len() && matches!(stmts[index], HirStmt::Pipeline(_)) {
                    index += 1;
                }
                let span = self.emitter.hir.blocks[id].span;
                self.pipeline(&stmts[start..index], span, indent, out);
                continue;
            }
            self.stmt(&stmts[index], indent, out);
            index += 1;
        }
    }

    fn stmt(&mut self, stmt: &HirStmt, indent: usize, out: &mut String) {
        let pad = " ".repeat(indent);
        match stmt {
            HirStmt::Give(expr) => self.give(*expr, indent, out),
            HirStmt::Mutation(mutation) => {
                if let Some(text) = self.mutation(mutation) {
                    out.push_str(&format!("{pad}{text};\n"));
                }
            }
            // `const`, not `let`: a binding names one value and never
            // takes another. §14B.2's five verbs write a `state`, and
            // resolution has already refused a binding that shadows
            // anything, so nothing can reassign this name.
            HirStmt::Bind(bind) => {
                for binding in &bind.bindings {
                    let value = self.emitter.value(binding.value).into_text();
                    let name = self.emitter.names.local(binding.local).to_string();
                    out.push_str(&format!("{pad}const {name} = {value};\n"));
                }
            }
            HirStmt::If(conditional) => {
                let cond = self.emitter.value(conditional.cond).into_text();
                out.push_str(&format!("{pad}if ({cond}) {{\n"));
                self.block(conditional.then, indent + 2, out);
                match conditional.otherwise {
                    Some(otherwise) => {
                        out.push_str(&format!("{pad}}} else {{\n"));
                        self.block(otherwise, indent + 2, out);
                        out.push_str(&format!("{pad}}}\n"));
                    }
                    None => out.push_str(&format!("{pad}}}\n")),
                }
            }
            HirStmt::Each(each) => {
                // A statement `each` is unrelated to the node-position one:
                // no keys, no DOM, and its binder is a plain value.
                let iter = self.emitter.value(each.iter).into_text();
                let name = self.emitter.names.local(each.var).to_string();
                out.push_str(&format!("{pad}for (const {name} of {iter}) {{\n"));
                self.block(each.body, indent + 2, out);
                out.push_str(&format!("{pad}}}\n"));
            }
            HirStmt::When(when) => self.when(when, indent, out),
            HirStmt::Pipeline(_) => {
                unreachable!("a pipeline run is emitted as a whole by `block`")
            }
        }
    }

    /// `give E` — a `return`, or a jump back to the top of the function
    /// when `E` is a call to the function itself.
    fn give(&mut self, expr: ExprId, indent: usize, out: &mut String) {
        if let Some(jump) = self.tail_jump(expr, indent) {
            out.push_str(&jump);
            return;
        }
        let pad = " ".repeat(indent);
        let value = self.emitter.value(expr).into_text();
        out.push_str(&format!("{pad}return {value};\n"));
    }

    /// `give f with a is x` inside `f` becomes "give the parameters their
    /// next values and go round again".
    ///
    /// The new values are computed into `$`-prefixed temporaries before
    /// any parameter is written, because an argument is written in terms
    /// of the parameters it is replacing — `index is index + 1` reads
    /// `index` after `numbers` may already have been reassigned. The
    /// braces are what let those temporaries be `const` and what stop two
    /// jumps in one block from declaring the same name twice.
    fn tail_jump(&mut self, expr: ExprId, indent: usize) -> Option<String> {
        let tail = self.tail.clone()?;
        if !is_self_call(self.emitter.hir, tail.def, expr) {
            return None;
        }
        let (args, span) = {
            let hir_expr = &self.emitter.hir.exprs[expr];
            let args = match &hir_expr.kind {
                HirExprKind::Call { args, .. } => args.clone(),
                HirExprKind::OfCall { operand, .. } => vec![HirArg::Positional(*operand)],
                _ => return None,
            };
            (args, hir_expr.span)
        };
        let values = self.emitter.ordered_arguments(tail.def, &args, span)?;
        let names: Vec<String> = tail
            .params
            .iter()
            .map(|param| self.emitter.names.local(*param).to_string())
            .collect();

        let pad = " ".repeat(indent);
        let mut out = format!("{pad}{{\n");
        for (index, value) in values.iter().enumerate() {
            out.push_str(&format!("{pad}  const $t{index} = {value};\n"));
        }
        for (index, name) in names.iter().enumerate() {
            out.push_str(&format!("{pad}  {name} = $t{index};\n"));
        }
        out.push_str(&format!("{pad}  continue $tail;\n{pad}}}\n"));
        Some(out)
    }

    /// `set X to E` -> `setX(<E>)`, and the `add`/`subtract` forms with the
    /// read spelled out rather than an updater closure.
    fn mutation(&mut self, mutation: &HirMutation) -> Option<String> {
        let place = mutation.place();
        let value = mutation.value();
        let operator = match mutation {
            HirMutation::Set { .. } => Operator::Replace,
            HirMutation::Add { .. } => Operator::Arithmetic('+'),
            HirMutation::Subtract { .. } => Operator::Arithmetic('-'),
            HirMutation::Append { .. } => Operator::Append,
            HirMutation::Remove { .. } => Operator::Remove,
        };

        // What this mutation *is* was decided by the split: a local write,
        // a store write, or a command the browser asks the server to
        // perform (§17.2.7). Emission only spells the decision out.
        //
        // Asked of the signal as well as of the place, because a place
        // span is not unique: instantiation copies a component's body per
        // call site and keeps the spans, so the same `set` line can be two
        // writes to two differently-placed signals. A write whose base is
        // a local is a component's own state, which is `client` and always
        // local (§14D.1), so the split records no crossing for it.
        let crossing = match place.base {
            Res::Def(def) => self
                .emitter
                .split
                .mutation_at(place.span, self.emitter.ctx, def)
                .cloned(),
            // A local is a component's own state, handled above. A builtin
            // and a variant are not storage, so neither is a place a
            // mutation can name, and the split records no crossing for
            // them. Spelled out rather than wildcarded so that a new `Res`
            // is a compile error here.
            Res::Local(_) | Res::Builtin(_) | Res::Variant { .. } | Res::BuiltinVariant(_) => None,
        };

        if let Some(MutCrossing::Command { root }) = crossing {
            return self.command(root, place, value);
        }

        if !place.path.is_empty() {
            self.emitter.error(
                "A mutation through a path such as `scores at player` needs an immutable-update \
                 helper the runtime does not have and §14B.3 has not settled.",
                place.span,
            );
            return None;
        }

        if let Some(MutCrossing::StoreWrite { key, .. }) = crossing {
            let name = self.emitter.hir.defs[key].name.clone();
            let amount = self.emitter.value(value).into_text();
            // The same five words `MutOp::word` renders into a command
            // name, taken off the HIR so the two cannot drift apart.
            let call = match mutation {
                HirMutation::Set { .. } => "set",
                HirMutation::Add { .. } => "incr",
                HirMutation::Subtract { .. } => "decr",
                HirMutation::Append { .. } => "append",
                HirMutation::Remove { .. } => "remove",
            };
            self.awaited = true;
            return Some(format!(
                "await $store.{call}({}, {amount})",
                crate::js::string(&name)
            ));
        }

        let target = self.target(place)?;
        let Target {
            getter,
            setter,
            declared,
            container,
        } = target;
        let amount = self.emitter.value(value);

        Some(match operator {
            Operator::Replace => format!("{setter}({})", amount.into_text()),
            Operator::Arithmetic(symbol) => format!(
                "{setter}({getter}() {symbol} {})",
                amount.operand(precedence::MULTIPLICATIVE)
            ),
            // §14B.2's membership forms. Both build a new collection rather
            // than mutating in place: ZD values are immutable and
            // `signal.write` compares with `Object.is`, so writing through
            // the old value would defeat change detection entirely.
            Operator::Append => {
                format!("{setter}([...{getter}(), {}])", amount.into_text())
            }
            Operator::Remove => match container {
                zdc_types::Type::Map(_, _) => format!(
                    "{setter}(new Map([...{getter}()].filter(($e) => $e[0] !== {})))",
                    amount.into_text()
                ),
                zdc_types::Type::List(_) => {
                    // `.filter` is an array method, and the signal may
                    // hold a list an `append` expression built.
                    let source = self.emitter.forced(format!("{getter}()"));
                    format!(
                        "{setter}({source}.filter(($e) => $e !== {}))",
                        amount.into_text()
                    )
                }
                other => {
                    self.emitter.error(
                        format!(
                            "`remove` works on a list or a map, and `{declared}` is `{other}`."
                        ),
                        place.span,
                    );
                    return None;
                }
            },
        })
    }

    /// A cross-region write. The right-hand side and every index were
    /// evaluated here, in this region, and are shipped as the command's
    /// arguments; only the place resolution and the store operator run on
    /// the other side (§17.2.7's command rule).
    /// The operator is not passed in: the endpoint's name already carries
    /// it (`visits.incr`), because that is what identifies the command.
    fn command(
        &mut self,
        root: zdc_graph::RootId,
        place: &zdc_hir::HirPlace,
        value: zdc_hir::ExprId,
    ) -> Option<String> {
        let Some(endpoint) = self.emitter.split.endpoint_of(root) else {
            self.emitter.error(
                "This write crosses a region boundary, but the split recorded no endpoint for it.",
                place.span,
            );
            return None;
        };
        let name = endpoint.name.clone();

        let mut args = vec![self.emitter.value(value).into_text()];
        for segment in &place.path {
            if let zdc_hir::HirPathSeg::Index(index) = segment {
                args.push(self.emitter.value(*index).into_text());
            }
        }
        // **Awaited.** Without this the handler fires the request and
        // discards the promise: three writes in one handler produce three
        // requests whose order is whatever the network decides, whose
        // failures are unobservable, and whose partial application is
        // invisible. There is no transaction across endpoints — see the
        // note on `handler_source` — but a write that cannot be waited on
        // cannot be part of one either.
        self.awaited = true;
        Some(format!(
            "await $call({}, {})",
            crate::js::string(&name),
            args.join(", ")
        ))
    }

    /// The getter, setter and value type behind a mutation's place.
    ///
    /// Two things can be written: a top-level `state`, and a component's
    /// own state. The second is a local rather than a definition — a
    /// component's state belongs to one instance (§14D.1) — but it is the
    /// same `[read, write]` pair once emitted, so the rest of `mutation`
    /// does not need to know which it got.
    fn target(&mut self, place: &zdc_hir::HirPlace) -> Option<Target> {
        match place.base {
            Res::Def(def) => {
                let DefKind::Signal(signal) = &self.emitter.hir.defs[def].kind else {
                    self.emitter.error(
                        format!(
                            "`{}` is not state, so it cannot be mutated.",
                            self.emitter.hir.defs[def].name
                        ),
                        place.span,
                    );
                    return None;
                };
                let is_source = signal.is_source;
                let declared = self.emitter.hir.defs[def].name.clone();
                if !is_source {
                    self.emitter.error(
                        format!(
                            "`{declared}` is declared with `from`, so the compiler recomputes it; \
                             only a `starting` signal can be written (spec §4.5)."
                        ),
                        place.span,
                    );
                    return None;
                }
                let Some(setter) = self.emitter.names.setter(def).map(str::to_string) else {
                    // Reachable only if the write analysis missed this
                    // site, which would mean the emitted module had no
                    // setter to call.
                    self.emitter.error(
                        format!("`{declared}` is written here but was not given a setter."),
                        place.span,
                    );
                    return None;
                };
                Some(Target {
                    getter: self.emitter.names.def(def).to_string(),
                    setter,
                    declared,
                    container: self
                        .emitter
                        .types
                        .def(def)
                        .cloned()
                        .unwrap_or(zdc_types::Type::Unknown),
                })
            }
            Res::Local(local) if self.emitter.analysis.is_local_signal(local) => {
                let declared = self.emitter.hir.locals[local].name.clone();
                let Some(setter) = self.emitter.names.local_setter(local).map(str::to_string)
                else {
                    self.emitter.error(
                        format!("`{declared}` is written here but was not given a setter."),
                        place.span,
                    );
                    return None;
                };
                Some(Target {
                    getter: self.emitter.names.local(local).to_string(),
                    setter,
                    declared,
                    container: self
                        .emitter
                        .types
                        .local(local)
                        .cloned()
                        .unwrap_or(zdc_types::Type::Unknown),
                })
            }
            _ => {
                self.emitter.error(
                    "Only a `state` declaration can be mutated; a parameter or loop name holds \
                     one evaluation's value.",
                    place.span,
                );
                None
            }
        }
    }

    /// A statement `when` is a `switch` on the tag, with the arm's binders
    /// destructured out of `fields` positionally (§16.3.10).
    ///
    /// Exhaustiveness is the checker's verdict (§14G.1.6), and `zdc build`
    /// runs it before this, so there is no fall-through case to write: an
    /// unmatched tag is unreachable by construction.
    fn when(&mut self, when: &HirWhen, indent: usize, out: &mut String) {
        let pad = " ".repeat(indent);
        let scrutinee = self.emitter.value(when.scrutinee).into_text();
        let temporary = format!("$w{}", self.temporaries);
        self.temporaries += 1;

        out.push_str(&format!("{pad}const {temporary} = {scrutinee};\n"));
        out.push_str(&format!("{pad}switch ({temporary}.tag) {{\n"));
        for arm in &when.arms {
            out.push_str(&format!("{pad}  case '{}': {{\n", arm.pattern_name));
            if !arm.bindings.is_empty() {
                let binders: Vec<String> = arm
                    .bindings
                    .iter()
                    .map(|binding: &LocalId| self.emitter.names.local(*binding).to_string())
                    .collect();
                out.push_str(&format!(
                    "{pad}    const [{}] = {temporary}.fields;\n",
                    binders.join(", ")
                ));
            }
            match arm.body {
                // `show` in statement position is the arm's result, and a
                // statement `when` is the last thing a function body does.
                HirArmBody::Show(expr) => self.give(expr, indent + 4, out),
                HirArmBody::Block(block) => self.block(block, indent + 4, out),
            }
            out.push_str(&format!("{pad}  }}\n"));
        }
        out.push_str(&format!("{pad}}}\n"));
    }

    /// A run of pipeline clauses becomes one accumulator, `$p`.
    ///
    /// The binders the comparator introduces are `$a`, `$b` and `$kN`, all
    /// `$`-prefixed and therefore hygienic against any name a program can
    /// spell — `$` is in neither XID_Start nor XID_Continue.
    fn pipeline(
        &mut self,
        clauses: &[HirStmt],
        span: zdc_lexer::Span,
        indent: usize,
        out: &mut String,
    ) {
        let pad = " ".repeat(indent);
        let mut started = false;

        for clause in clauses {
            let HirStmt::Pipeline(clause) = clause else {
                unreachable!("`block` only groups pipeline statements here");
            };
            if !started && !matches!(clause, HirPipeline::From(_)) {
                self.emitter.error(
                    "A pipeline must start with `from`, naming the sequence the later clauses \
                     work on.",
                    span,
                );
                return;
            }
            match clause {
                HirPipeline::From(expr) => {
                    let source = self.emitter.value(*expr).into_text();
                    // Every clause below is an array method, so the
                    // sequence has to be an array before the first one.
                    let source = self.emitter.forced(source);
                    out.push_str(&format!("{pad}let $p = {source};\n"));
                    started = true;
                }
                HirPipeline::Keep { var, cond } => {
                    let name = self.emitter.names.local(*var).to_string();
                    let condition = self.emitter.value(*cond).into_text();
                    out.push_str(&format!("{pad}$p = $p.filter(({name}) => {condition});\n"));
                }
                HirPipeline::MapEach { var, to } => {
                    let name = self.emitter.names.local(*var).to_string();
                    let mapped = self.emitter.value(*to).into_text();
                    out.push_str(&format!("{pad}$p = $p.map(({name}) => {mapped});\n"));
                }
                HirPipeline::Sort { var, key } => {
                    // `.slice()` first, and it is mandatory: ZD values are
                    // immutable and `signal.write` compares with
                    // `Object.is`, so an in-place sort would both mutate a
                    // shared value and defeat change detection.
                    let name = self.emitter.names.local(*var).to_string();
                    let key = self.emitter.value(*key).into_text();
                    let extract = format!("$k{}", self.temporaries);
                    self.temporaries += 1;
                    out.push_str(&format!("{pad}const {extract} = ({name}) => {key};\n"));
                    out.push_str(&format!("{pad}$p = $p.slice().sort(($a, $b) => {{\n"));
                    out.push_str(&format!(
                        "{pad}  const $ka = {extract}($a), $kb = {extract}($b);\n"
                    ));
                    out.push_str(&format!(
                        "{pad}  return $ka < $kb ? -1 : $ka > $kb ? 1 : 0;\n{pad}}});\n"
                    ));
                }
                HirPipeline::TakeFirst(count) => {
                    let count = self.emitter.value(*count).into_text();
                    out.push_str(&format!("{pad}$p = $p.slice(0, {count});\n"));
                }
            }
        }

        out.push_str(&format!("{pad}return $p;\n"));
    }
}
