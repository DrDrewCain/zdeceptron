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

use zdc_hir::{
    BlockId, DefKind, HirArmBody, HirMutation, HirPipeline, HirStmt, HirWhen, LocalId, Res,
};

use crate::expr::Emitter;
use crate::js::precedence;

/// Statements share the expression emitter's error list and name table.
pub struct Statements<'a, 'h> {
    pub emitter: &'a mut Emitter<'h>,
    /// Numbers the `$w` temporaries a statement `when` needs.
    pub temporaries: usize,
}

/// The pair a write goes through, and what it holds.
struct Target {
    getter: String,
    setter: String,
    /// The name the program wrote, for diagnostics.
    declared: String,
    container: zdc_types::Type,
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
            HirStmt::Give(expr) => {
                let value = self.emitter.value(*expr).into_text();
                out.push_str(&format!("{pad}return {value};\n"));
            }
            HirStmt::Mutation(mutation) => {
                if let Some(text) = self.mutation(mutation) {
                    out.push_str(&format!("{pad}{text};\n"));
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

        if !place.path.is_empty() {
            self.emitter.error(
                "A mutation through a path such as `scores at player` needs an immutable-update \
                 helper the runtime does not have and §14B.3 has not settled.",
                place.span,
            );
            return None;
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
                zdc_types::Type::List(_) => format!(
                    "{setter}({getter}().filter(($e) => $e !== {}))",
                    amount.into_text()
                ),
                other => {
                    // unreached: `zdc-types` reports this first, in its own
                    // words.
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
                    // unreached: `zdc-types` reports this first, in its own
                    // words.
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
                    // unreached: `zdc-types` reports this first, in its own
                    // words.
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
                    // unreached: An internal guard. A `starting` client signal
                    // is emitted as a `[get, set]` pair, so its setter exists
                    // whenever it does.
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
                    // unreached: An internal guard, as above: a writable local
                    // signal is emitted with its setter.
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
                HirArmBody::Show(expr) => {
                    let value = self.emitter.value(expr).into_text();
                    out.push_str(&format!("{pad}    return {value};\n"));
                }
                HirArmBody::Block(block) => self.block(block, indent + 4, out),
            }
            out.push_str(&format!("{pad}  }}\n"));
        }
        out.push_str(&format!("{pad}}}\n"));
    }

    /// A run of pipeline clauses becomes one accumulator, `$pN`.
    ///
    /// The binders the comparator introduces are `$a`, `$b` and `$kN`, all
    /// `$`-prefixed and therefore hygienic against any name a program can
    /// spell — `$` is in neither XID_Start nor XID_Continue.
    ///
    /// The accumulator is numbered for the same reason `$wN` and `$kN` are.
    /// It was once the bare name `$p`, and two pipeline runs in one block
    /// then emitted `let $p` twice at the same brace depth, which is a
    /// JavaScript `SyntaxError` — not a wrong value in one function but a
    /// module the engine refuses to parse, so the whole bundle fails to
    /// load. Being unreachable after the first run's `return` does not
    /// help: a redeclaration is rejected before any of it is executed.
    fn pipeline(
        &mut self,
        clauses: &[HirStmt],
        span: zdc_lexer::Span,
        indent: usize,
        out: &mut String,
    ) {
        let pad = " ".repeat(indent);
        let accumulator = format!("$p{}", self.temporaries);
        self.temporaries += 1;
        let mut started = false;

        for clause in clauses {
            let HirStmt::Pipeline(clause) = clause else {
                unreachable!("`block` only groups pipeline statements here");
            };
            if !started && !matches!(clause, HirPipeline::From(_)) {
                // unreached: `zdc-types` reports this first, in its own words.
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
                    out.push_str(&format!("{pad}let {accumulator} = {source};\n"));
                    started = true;
                }
                HirPipeline::Keep { var, cond } => {
                    let name = self.emitter.names.local(*var).to_string();
                    let condition = self.emitter.value(*cond).into_text();
                    out.push_str(&format!(
                        "{pad}{accumulator} = {accumulator}.filter(({name}) => {condition});\n"
                    ));
                }
                HirPipeline::MapEach { var, to } => {
                    let name = self.emitter.names.local(*var).to_string();
                    let mapped = self.emitter.value(*to).into_text();
                    out.push_str(&format!(
                        "{pad}{accumulator} = {accumulator}.map(({name}) => {mapped});\n"
                    ));
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
                    out.push_str(&format!(
                        "{pad}{accumulator} = {accumulator}.slice().sort(($a, $b) => {{\n"
                    ));
                    out.push_str(&format!(
                        "{pad}  const $ka = {extract}($a), $kb = {extract}($b);\n"
                    ));
                    out.push_str(&format!(
                        "{pad}  return $ka < $kb ? -1 : $ka > $kb ? 1 : 0;\n{pad}}});\n"
                    ));
                }
                HirPipeline::TakeFirst(count) => {
                    let count = self.emitter.value(*count).into_text();
                    out.push_str(&format!(
                        "{pad}{accumulator} = {accumulator}.slice(0, {count});\n"
                    ));
                }
            }
        }

        out.push_str(&format!("{pad}return {accumulator};\n"));
    }
}
