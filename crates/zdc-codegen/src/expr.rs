//! Expression emission, per spec §16.3.3.
//!
//! Three positions, one classifier. **Value position** produces the value.
//! **Getter position** — any argument the runtime will `read()` — produces
//! the getter itself when the expression already is one, a closure when it
//! reads a signal, and a bare constant otherwise. Never `() => X()`: a
//! signal read and a `derived` *are* the getter, and double-wrapping hands
//! the runtime a function where it expected a variant.

use std::collections::{BTreeMap, BTreeSet};

use zdc_ast::{BinOp, UnaryOp};
use zdc_graph::{Ctx, Region, RootId, TierSplit};
use zdc_hir::{
    Builtin, BuiltinVariant, DefId, DefKind, ExprId, Hir, HirArg, HirExprKind, OperatorName, Res,
};
use zdc_types::{EmptyKind, IndexKind, OperatorKind, Type, TypeTable};

use crate::analysis::Analysis;
use crate::events;
use crate::intrinsics::{self, JsForm, TEXT_OF_TRUTH};
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
            Literal::Text(text) => js::string(text).to_string(),
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
    /// The placement pass's answers. Which root is being emitted decides
    /// how a read is spelled: a browser reads a signal by calling its
    /// getter, a server invocation reads a plain `const` (§17.2.8).
    pub split: &'a TierSplit,
    pub ctx: Ctx,
    pub root: RootId,
    /// What the build host computed for each `static` signal, as JSON
    /// (§17.4.8). Outside the `BUILD` root a `static` read *is* its value,
    /// so this is where that value comes from.
    pub statics: &'a BTreeMap<DefId, String>,
    /// Which hoisted `static` values this emission actually read.
    ///
    /// A hoisted value is declared once and named, so the declaration has
    /// to be emitted — and emitted only when something reads it, or a
    /// bundle would carry values no page mentions. Recorded here as the
    /// reads happen rather than derived from the split, because the split
    /// makes a `static` an *inlined* member and inlined members are not
    /// in the client's member list at all: there was nothing to inline
    /// into a declaration until now.
    pub read_statics: BTreeSet<DefId>,
    pub errors: Vec<CodegenError>,
    /// The complete durable write set of every event handler, collected as
    /// the handlers are emitted.
    ///
    /// This is the thing a general-purpose database client cannot have. It
    /// goes into the manifest so a deploy adapter can check it against the
    /// caps its target imposes on an atomic batch — DynamoDB's
    /// `TransactWriteItems` and Deno KV's `atomic()` both have one — before
    /// a request hits them rather than after.
    pub transactions: Vec<crate::HandlerWrites>,
    /// The CSS media queries this module reads, each with the index of
    /// the cell hoisted for it.
    ///
    /// One cell per **distinct query**, not one per read: `matchMedia`
    /// returns a live `MediaQueryList` and subscribing twice to the same
    /// query would install two listeners that always agree. The query is a
    /// literal (see `zdc_ast::Expr::Media`), so the set is known by the
    /// end of emission and the cells are declared in the preamble beside
    /// the templates.
    pub media: BTreeMap<String, usize>,
    /// Whether the program reads `scroll`.
    ///
    /// A flag rather than a map, because there is one document and one
    /// answer: `media` needs a cell per distinct query and this needs a
    /// cell per program.
    pub scroll: bool,
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
            HirExprKind::Text(text) => Expr::primary(js::string(text).to_string()),
            HirExprKind::Truth(truth) => Expr::primary(truth.to_string()),
            // `$request(url, [[name, getter], …])` — #19.
            //
            // Each argument is wrapped in a closure rather than passed as
            // a value, and that is not uniformity for its own sake: the
            // runtime reads them **inside** an effect, so reading them
            // here would bind the request to the values it had when the
            // module loaded and it would never re-run.
            //
            // The destination reaches the emitted module as a JavaScript
            // string literal and nothing else. There is no concatenation
            // anywhere on this path, which is what makes "the origin in
            // the policy is the origin in the source" checkable.
            HirExprKind::Outbound { destination, args } => {
                let (destination, args) = (destination.clone(), args.clone());
                self.used.request.insert("request as $request");
                // The destination was parsed by `zdc_hir::destination`
                // before this program was allowed to exist, so an `Err`
                // here is unreachable and a same-origin one contributes
                // nothing.
                if let Ok(zdc_hir::Destination::CrossOrigin(origin)) =
                    zdc_hir::destination(&destination)
                {
                    self.used.connect.insert(origin);
                }
                let params: Vec<String> = args
                    .iter()
                    .map(|arg| {
                        let (name, value) = match arg {
                            HirArg::Named { name, value } => (name.clone(), *value),
                            // Resolution refuses one, so this is a name
                            // nothing can be spelled with rather than a
                            // default anybody could reach.
                            HirArg::Positional(value) => (String::new(), *value),
                        };
                        let body = self.value(value).into_text();
                        format!("[{}, () => {}]", js::string(&name), js::arrow_body(&body))
                    })
                    .collect();
                Expr::primary(format!(
                    "$request({}, [{}])",
                    js::string(&destination),
                    params.join(", ")
                ))
            }
            // §16.7 item 6: which container `empty` is comes off the
            // checker's verdict, never off the syntax.
            HirExprKind::Empty => match self.types.empty_kind(id) {
                Some(EmptyKind::List) => Expr::primary("[]"),
                Some(EmptyKind::Map) => Expr::primary("new Map()"),
                None => {
                    // unreached: `zdc-types` reports this first, in its own
                    // words. A settled program has an `empty` the checker gave
                    // a container to; an unsettled one never reaches codegen.
                    self.error(
                        "`empty` is a list or a map, and nothing here says which. Write the type \
                         on the state it starts.",
                        expr.span,
                    );
                    Expr::primary("undefined")
                }
            },
            // `?:` — the one JavaScript form this language now has a
            // spelling for. Each part is emitted as an operand of the
            // conditional, so an `||` inside an arm keeps its brackets and
            // the parse is the one the source asked for.
            &HirExprKind::Conditional {
                condition,
                value,
                otherwise,
            } => {
                let condition = self.value(condition);
                let value = self.value(value);
                let otherwise = self.value(otherwise);
                Expr::new(
                    format!(
                        "{} ? {} : {}",
                        condition.operand(js::precedence::CONDITIONAL + 1),
                        value.operand(js::precedence::CONDITIONAL),
                        otherwise.operand(js::precedence::CONDITIONAL)
                    ),
                    js::precedence::CONDITIONAL,
                )
            }
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
            // §5.6 confines this to server context, and the split has
            // already rejected every other placement with E0360 — so by
            // the time emission runs, the only remaining question is how
            // to spell it. `$env` is injected by the platform adapter.
            HirExprKind::Environment(key) => {
                let key = key.clone();
                Expr::new(format!("$env({})", js::string(&key)), precedence::MEMBER)
            }
            // The split has already refused every context but the browser
            // with E0362, so the only remaining question is how to spell
            // it. It spells as a *read of a hoisted cell*, because the
            // answer changes while the page is open and a view that shows
            // one has to change with it.
            HirExprKind::Media(query) => {
                let query = query.clone();
                let next = self.media.len();
                let index = *self.media.entry(query).or_insert(next);
                self.used.media.insert("mediaMatch");
                Expr::new(format!("$q{index}()"), precedence::MEMBER)
            }
            HirExprKind::Scroll => {
                self.scroll = true;
                self.used.viewport.insert("scrollFraction");
                Expr::new("$scroll()".to_string(), precedence::MEMBER)
            }
            HirExprKind::Address => {
                // unreached: `zdc-types` reports this first, in its own words — a
                // bare `address` with no `route` is a type error before it is a
                // fold question.
                self.error(
                    "`address` is read by the signal that holds it, which the build folds to one \
                     value per document. Write `state page is client Option of <route> starting \
                     address` and dispatch on `page` with `when`.",
                    expr.span,
                );
                Expr::primary("undefined")
            }
            // The split has already refused every context but build-time
            // evaluation with E0361, so the only remaining question is how
            // to spell it. `$build` is the compiler's own object, injected
            // into the sandbox that runs the build root, and it exists in
            // no other bundle.
            HirExprKind::Build {
                capability,
                argument,
            } => {
                let inner = self.value(*argument);
                Expr::new(
                    format!("$build.{}({})", capability.name(), inner.into_text()),
                    precedence::MEMBER,
                )
            }
            HirExprKind::Ref(res) => self.reference(*res, expr.span),
            HirExprKind::Call { callee, args } => self.call(*callee, args, expr.span),
            HirExprKind::OfCall { callee, operand } => {
                let args = [HirArg::Positional(*operand)];
                self.call(*callee, &args, expr.span)
            }
            HirExprKind::Operator { op, operand } => self.operator(id, *op, *operand, expr.span),
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
            HirExprKind::Binary { op, lhs, rhs } => self.binary(id, *op, *lhs, *rhs, expr.span),
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
            // §5.4: indexing is bounds-checked, so it gives an `Option of
            // T`. Which helper builds one comes off the checker's verdict
            // (§16.7 item 5), never off the syntax.
            HirExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                let Some(kind) = self.types.index_kind(id) else {
                    // unreached: `zdc-types` reports this first, in its own words. An
                    // index it could not classify is an index whose type it refused.
                    self.error(
                        "`at` needs to know whether this is a text, a list or a map, and nothing \
                         in the program says which.",
                        expr.span,
                    );
                    return Expr::primary("undefined");
                };
                let helper = match kind {
                    IndexKind::List => "$listAt",
                    IndexKind::Map => "$mapAt",
                    IndexKind::Text => "$textAt",
                };
                let container = self.value(base).into_text();
                let key = self.value(index).into_text();
                self.use_helper(helper);
                Expr::new(format!("{helper}({container}, {key})"), precedence::MEMBER)
            }
            // `append item to list`. The list operand is emitted raw, so
            // an append of an append is a link onto a link and costs one
            // allocation rather than one copy — see `$force` for why that
            // is what makes building a list linear rather than quadratic.
            HirExprKind::Append { item, list } => {
                let (item, list) = (*item, *list);
                let base = self.value(list).into_text();
                let element = self.value(item).into_text();
                self.use_helper("$append");
                Expr::new(format!("$append({base}, {element})"), precedence::MEMBER)
            }
            // `set key to value in table`, and it links exactly as
            // `append` does (#233).
            //
            // This note used to argue the opposite, and the argument is
            // worth keeping because half of it is still true: a `Map`
            // cannot share a prefix the way an append chain does, since
            // a later `set` may overwrite a key an earlier one wrote and
            // position alone cannot say which won. What that rules out
            // is `$Ap`'s shape, where a link is an addition and order is
            // read off position. It does not rule out a chain of
            // *writes*: a copy is a replay of writes anyway, so a chain
            // that remembers them in order and replays them oldest-first
            // on first read produces the same map, in the same order,
            // for one flatten instead of one copy per write. The map
            // operand is emitted raw so a write onto a write is a link
            // onto a link, which is what makes a fold that builds a map
            // and reads it at the end linear rather than quadratic.
            // `$mapSet`'s note says which folds those are and which
            // shape is still quadratic, and `depth.rs` measures both.
            HirExprKind::Insert { key, value, table } => {
                let (key, value, table) = (*key, *value, *table);
                let base = self.value(table).into_text();
                let written_key = self.value(key).into_text();
                let written_value = self.value(value).into_text();
                self.use_helper("$mapSet");
                Expr::new(
                    format!("$mapSet({base}, {written_key}, {written_value})"),
                    precedence::MEMBER,
                )
            }
            // `map each x in maybe to …` — a conditional over the variant
            // tag, inline, with no runtime helper of its own.
            //
            // **The emitted arrow is a JavaScript closure and not a
            // ZDeceptron value, and that distinction is the whole design.**
            // The pipeline's `map each row to …` has emitted
            // `.map((row) => …)` since the language had pipelines; nothing
            // is passed here either, because the body was a syntactic
            // expression at the site.
            //
            // Two arrows and one name, deliberately. The outer parameter
            // holds the container so it is evaluated once — `maybe` may be
            // a call — and the inner one shadows it with the payload, so
            // the body sees the binder bound to what was inside. The
            // shadowing is safe whatever the program called its binder:
            // `.tag`, `.fields[0]` and the pass-through arm are all read
            // in the outer scope, and the body is the only thing read in
            // the inner one.
            //
            // No new runtime module, so §16's size gate is untouched:
            // `variant` is `runtime/dom.js`'s, which every bundle already
            // links.
            HirExprKind::MapInside { var, source, to } => {
                let (var, source, to) = (*var, *source, *to);
                let Some(tag) = self.types.expr(source).and_then(payload_tag) else {
                    // unreached: `zdc-types` reports a container that holds
                    // neither zero nor one first, in its own words.
                    self.error(
                        "`map each … in` transforms what is inside an `Option` or a `Remote`, \
                         and this is neither.",
                        expr.span,
                    );
                    return Expr::primary("undefined");
                };
                let name = self.names.local(var).to_string();
                let container = self.value(source).into_text();
                let body = js::arrow_body(&self.value(to).into_text());
                self.used.dom.insert("variant");
                let tag = js::string(tag).to_string();
                Expr::new(
                    format!(
                        "(({name}) => {name}.tag === {tag} ? variant({tag}, \
                         (({name}) => {body})({name}.fields[0])) : {name})({container})"
                    ),
                    precedence::MEMBER,
                )
            }
        }
    }

    /// Wrap an emitted list in `$force`, so that an append chain reaches
    /// array indexing and the array methods as a real array.
    ///
    /// The three call sites are the three places a list is taken apart by
    /// something other than `at`, `length of` or iteration: the pipeline's
    /// `from`, `remove`'s filter, and a node-position `each`. Everything
    /// else goes through `$listAt`, which forces for itself, or through
    /// `$Ap`'s own `length`, iterator and `toJSON`.
    pub fn forced(&mut self, source: String) -> String {
        self.use_helper("$force");
        format!("$force({source})")
    }

    /// A `$`-prefixed preamble helper, and whatever it needs from the
    /// runtime.
    pub fn use_helper(&mut self, name: &'static str) {
        if !self.used.helpers.insert(name) {
            return;
        }
        if let Some((_, needs_variant)) = intrinsics::helper(name) {
            if needs_variant {
                self.used.dom.insert("variant");
            }
        }
        for required in intrinsics::requires(name) {
            self.use_helper(required);
        }
    }

    /// `length of x` and `text of x`, per the checker's dispatch verdict.
    fn operator(
        &mut self,
        id: ExprId,
        op: OperatorName,
        operand: ExprId,
        span: zdc_lexer::Span,
    ) -> Expr {
        let Some(kind) = self.types.operator_kind(id) else {
            // unreached: `zdc-types` reports this first, in its own words. An
            // operator it could not dispatch is one it refused to type.
            self.error(
                format!(
                    "`{}` needs to know what kind of value this is, and nothing in the program \
                     says.",
                    op.describe()
                ),
                span,
            );
            return Expr::primary("undefined");
        };
        let form = match kind {
            OperatorKind::TextLength => JsForm::Helper("$textLength"),
            OperatorKind::ListLength => JsForm::Field("length"),
            OperatorKind::MapLength => JsForm::Field("size"),
            OperatorKind::TextOfWhole | OperatorKind::TextOfDecimal => JsForm::Helper("$textOf"),
            OperatorKind::TextOfTruth => JsForm::Helper(TEXT_OF_TRUTH),
            OperatorKind::TextOfText => JsForm::Identity,
        };
        let inner = self.value(operand);
        self.apply(form, inner)
    }

    /// Emit one primitive against an already-emitted operand.
    fn apply(&mut self, form: JsForm, operand: Expr) -> Expr {
        match form {
            JsForm::Identity => operand,
            JsForm::Field(field) => Expr::new(
                format!("{}.{field}", operand.operand(precedence::MEMBER)),
                precedence::MEMBER,
            ),
            JsForm::Helper(name) => {
                self.use_helper(name);
                Expr::new(
                    format!("{name}({})", operand.into_text()),
                    precedence::MEMBER,
                )
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
                // binders, and none of these is one.
                Res::Builtin(_) | Res::Variant { .. } | Res::BuiltinVariant(_) => {
                    unreachable!("a built-in and a variant are never getters")
                }
            };
            return Operand::Reactive(name);
        }

        let reactive = self.analysis.reads_signal(self.hir, id);
        let value = self.value(id).into_text();
        if reactive {
            // The getter every reactive binding is built from, so a
            // record literal reaching it would be the same brace-at-the-
            // start-of-a-body defect as the pipeline's (#194). No `.zd`
            // program reaches it with one today: a component argument is
            // substituted at the use rather than held as a getter, and
            // every attribute is `Text`, `Truth` or a number, so this is
            // a guard on the emission and not a fix for a live case.
            Operand::Reactive(format!("() => {}", js::arrow_body(&value)))
        } else {
            Operand::Static(value)
        }
    }

    /// The same operand, for the one position where it becomes **text a
    /// person reads**: a text slot's value (#297).
    ///
    /// # Why a `Truth` is `yes`/`no` here, and why the conversion is
    /// emitted rather than shipped
    ///
    /// `Text flag` used to render `true`, which is not a word in this
    /// language. Its truth literals are `yes` and `no`, the formatter
    /// writes `yes` and `no`, and a reader who typed `yes` was shown
    /// JavaScript's spelling of their own literal.
    ///
    /// **The choice was already made, one operator over.** §17.4.3's
    /// closed dispatched set sends `text of` a `Truth` to `textOfTruth`,
    /// and §17.4.9 gives that function's body as `if value / give "yes" /
    /// give "no"`. So the question here was never *which words* — the
    /// language has already answered that and the compiler already ships
    /// the answer — but whether a text slot may disagree with `text of`
    /// about them. It may not: `Text flag` and `Text (text of flag)` write
    /// into the same text node from the same value, and two spellings of
    /// one conversion is exactly the shape §16.3.5's single URL allowlist
    /// argues against, where a rule stated twice is a defect *even while
    /// the two copies agree*.
    ///
    /// **Refusing `Text <Truth>` instead was the other candidate, and it
    /// costs more than it saves.** It has a real argument — `yes`/`no` are
    /// rarely the words a page wants, and English is not the only
    /// audience — but that argument is against the words themselves, and
    /// the words are the source language's, not this site's. A refusal
    /// here would also have to come from somewhere: `Constraint::Shown`
    /// admits `Truth` and is the *same* constraint `text of` imposes
    /// (`zdc_types::Constraint::Shown`, `ty.rs`), so narrowing it would
    /// either take `text of flag` with it or leave two constraints
    /// differing in one type. A program that wants other words already has
    /// the phrase for them, and `examples/preferences.zd` is written that
    /// way: `if dark / Text "…" / otherwise / Text "…"`. The default only
    /// has to be this language's word rather than the host's.
    ///
    /// **Nothing is added to the shipped runtime.** The issue's own
    /// sketch put the conversion in `dom.js`'s text binding, one line —
    /// but `dom.js` is paid for by every program, including the ones with
    /// no `Truth` anywhere, and `zdc-bench`'s null-program ceiling is the
    /// standing reason not to spend bytes there. `$textOfTruth` is an
    /// emitted preamble helper that already exists for `text of`, so a
    /// program that shows a truth links one arrow function and a program
    /// that does not links nothing. `bindText` still only stringifies.
    pub fn shown_operand(&mut self, id: ExprId) -> Operand {
        let operand = self.operand(id);
        if !self.is_truth(id) {
            return operand;
        }
        match operand {
            // Folded rather than called: a written `yes` is known here, so
            // the word goes straight into the template's markup and the
            // program links no helper at all.
            Operand::Literal(Literal::Truth(truth)) => {
                Operand::Literal(Literal::Text(intrinsics::truth_word(truth).to_string()))
            }
            // unreached: the checker settled this expression as `Truth`, so
            // a literal of it is a `Truth` literal.
            literal @ Operand::Literal(_) => literal,
            Operand::Static(value) => {
                self.use_helper(TEXT_OF_TRUTH);
                Operand::Static(format!("{TEXT_OF_TRUTH}({value})"))
            }
            // What is wanted is the *read*, and `operand` above has just
            // finished turning a read into a getter — so the two shapes it
            // can produce are unwound rather than called through a second
            // arrow. `() => $textOfTruth((() => !one())())` is what calling
            // through one looks like, and the inner arrow is allocated
            // again on every recomputation, which is §16.3.3's "never
            // `() => X()`" one layer out. The third branch is unreachable
            // from this function's two producers and is written anyway,
            // because it is the correct emission for any getter a later
            // one invents.
            Operand::Reactive(getter) => {
                self.use_helper(TEXT_OF_TRUTH);
                let read = if let Some(body) = getter.strip_prefix("() => ") {
                    body.to_string()
                } else if js::ident(&getter).is_some() {
                    format!("{getter}()")
                } else {
                    format!("({getter})()")
                };
                Operand::Reactive(format!("() => {TEXT_OF_TRUTH}({read})"))
            }
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
                DefKind::Signal(signal) => {
                    // §14C.3b: a `static` read crosses no boundary, because
                    // the value is *in* the bundle. In the build root it is
                    // an ordinary `const`; everywhere else it is the literal
                    // the build host printed, and there is no cell, no
                    // getter, and nothing that could ever change.
                    if signal.placement == zdc_ast::Placement::Static {
                        if self.ctx.region == Region::Static {
                            return Expr::primary(self.names.def(def).to_string());
                        }
                        let Some(json) = self.statics.get(&def) else {
                            // unreached: `evaluate` reports this first, in its own words. A
                            // `static` with no value means the build host failed, which is
                            // E9 or E10 there; `check` stubs the map for the same reason.
                            self.error(
                                format!(
                                    "`{}` is `static`, so its value is computed on the build host \
                                     and inlined here — but no value was computed for it (spec \
                                     §17.4.8).",
                                    self.hir.defs[def].name
                                ),
                                span,
                            );
                            return Expr::primary("undefined");
                        };
                        // **Hoisted when it is big.** A `static` is a
                        // constant, and inlining a constant is the right
                        // call for a number or a short string: no
                        // indirection, no name, nothing to look up. It is
                        // catastrophic for a list. A blog's fourteen posts
                        // read nine times on one page put the same
                        // ninety-eight kilobytes into the bundle nine
                        // times, and that page came to a megabyte of which
                        // seven eighths was one value repeated.
                        //
                        // So a literal past `HOIST_ABOVE` is declared once
                        // and named. It is still a constant — a `const`,
                        // not a cell, no getter, nothing that can change —
                        // and the cost when it is read once is the twenty
                        // bytes of the declaration.
                        if crate::hoisted(json) {
                            self.read_statics.insert(def);
                            return Expr::primary(self.names.def(def).to_string());
                        }
                        return Expr::primary(js::literal(json));
                    }
                    if self.ctx.region == Region::Client {
                        Expr::new(format!("{}()", self.names.def(def)), precedence::MEMBER)
                    } else {
                        Expr::primary(self.names.def(def).to_string())
                    }
                }
                DefKind::Function(_) | DefKind::Foreign(_) | DefKind::Release(_) => {
                    // unreached: `zdc-types` reports this first, in its own words,
                    // and says the same thing at greater length.
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
                    // unreached: No expression can name the view: `view` is a
                    // keyword, and the view definition is not in the value
                    // namespace.
                    self.error("The view is not a value.", span);
                    Expr::primary("undefined")
                }
                DefKind::Component(_) => {
                    // unreached: `zdc-resolve` reports this first, in its own
                    // words.
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
                    // unreached: `zdc-types` reports this first, in its own
                    // words.
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
            Res::BuiltinVariant(variant) => self.builtin_variant(variant, &[], span),
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
                // unreached: `zdc-resolve` reports this first, in its own
                // words.
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
                    // unreached: `zdc-types` reports this first, in its own words. A
                    // route value with no `route` declaration is an undefined name.
                    self.error("A route value needs a `route` declaration.", span);
                    return Expr::primary("undefined");
                };
                let DefKind::Choice(declared) = &self.hir.defs[choice].kind else {
                    // unreached: `zdc-types` reports this first, in its own words.
                    self.error("A route value belongs to a `route`.", span);
                    return Expr::primary("undefined");
                };
                let Some(declared) = declared.variants.get(*variant) else {
                    // unreached: `zdc-types` reports this first, in its own words.
                    self.error("A route value belongs to a `route`.", span);
                    return Expr::primary("undefined");
                };
                let name = js::string(&declared.name).to_string();
                self.used.dom.insert("variant");
                let mut emitted = vec![name];
                emitted.extend(values.iter().map(|value| js::string(value).to_string()));
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
            | HirExprKind::Ref(Res::BuiltinVariant(_))
            | HirExprKind::Call { .. }
            | HirExprKind::OfCall { .. }
            | HirExprKind::Operator { .. }
            | HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            | HirExprKind::Conditional { .. }
            | HirExprKind::List(_)
            | HirExprKind::Map(_)
            | HirExprKind::Environment(_)
            | HirExprKind::Address
            | HirExprKind::Media(_)
            | HirExprKind::Scroll
            | HirExprKind::Build { .. }
            | HirExprKind::Unary { .. }
            | HirExprKind::Binary { .. }
            | HirExprKind::Field { .. }
            | HirExprKind::Index { .. }
            | HirExprKind::Append { .. }
            // A request answers with `Remote of Text`, and a route value
            // is a variant of the program's own `route` choice. Neither
            // is the other.
            | HirExprKind::Outbound { .. }
            | HirExprKind::Insert { .. }
            | HirExprKind::MapInside { .. } => false,
        }
    }

    pub fn route_url(&mut self, id: ExprId) -> Operand {
        let span = self.hir.exprs[id].span;
        let Some((choice, table)) = &self.hir.routes else {
            // unreached: `zdc-types` reports this first, in its own words —
            // `Link` with no `route` in the program has no `Site` to expect.
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
                    // unreached: `zdc-types` reports this first, in its own words.
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
            | HirExprKind::Ref(Res::BuiltinVariant(_))
            | HirExprKind::Call { .. }
            | HirExprKind::OfCall { .. }
            | HirExprKind::Operator { .. }
            | HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            | HirExprKind::Conditional { .. }
            | HirExprKind::List(_)
            | HirExprKind::Map(_)
            | HirExprKind::Environment(_)
            | HirExprKind::Address
            | HirExprKind::Media(_)
            | HirExprKind::Scroll
            | HirExprKind::Build { .. }
            | HirExprKind::Unary { .. }
            | HirExprKind::Binary { .. }
            | HirExprKind::Field { .. }
            | HirExprKind::Index { .. }
            | HirExprKind::Append { .. }
            | HirExprKind::Outbound { .. }
            | HirExprKind::Insert { .. }
            | HirExprKind::MapInside { .. } => {
                // unreached: `zdc-types` reports this first, in its own words.
                self.error(
                    "`Link` takes a route value, as in `Link Home` or \
                     `Link (BlogPost with slug is post.slug)`.",
                    span,
                );
                return Operand::Literal(Literal::Text(String::new()));
            }
        };

        let Some(variant) = table.variants.get(index).cloned() else {
            // unreached: `zdc-types` reports this first, in its own words. A
            // route variant with no URL is refused where the route is declared.
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
                    // unreached: `zdc-types` reports this first, in its own words. A
                    // link missing a parameter is a missing argument to the variant.
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
            // unreached: An internal guard. `Res::Variant` is built by
            // `zdc-resolve` only from a `choice`, so the definition it names
            // is one.
            self.error("A variant belongs to a `choice`.", span);
            return Expr::primary("undefined");
        };
        let Some(variant) = declared.variants.get(index as usize) else {
            // unreached: An internal guard. `Res::Variant` carries an index
            // `zdc-resolve` took from the variant list it is indexing.
            self.error("A variant belongs to a `choice`.", span);
            return Expr::primary("undefined");
        };
        let name = variant.name.clone();
        let fields: Vec<String> = variant.fields.iter().map(|f| f.name.clone()).collect();

        let Some(values) = self.by_declaration_order(&name, &fields, args, span) else {
            return Expr::primary("undefined");
        };
        self.used.dom.insert("variant");
        let mut emitted = vec![js::string(&name).to_string()];
        emitted.extend(values);
        Expr::new(
            format!("variant({})", emitted.join(", ")),
            precedence::MEMBER,
        )
    }

    /// One value of a built-in variant: `variant('Some', v)`.
    ///
    /// The same shape a declared variant gets, because `when` dispatches
    /// on the tag alone and cannot tell the two apart — which is the point
    /// of §14G.1.2 giving the built-ins field names in the first place.
    fn builtin_variant(
        &mut self,
        variant: BuiltinVariant,
        args: &[HirArg],
        span: zdc_lexer::Span,
    ) -> Expr {
        let fields: Vec<String> = variant
            .field_names()
            .iter()
            .map(|field| (*field).to_string())
            .collect();
        let Some(values) = self.by_declaration_order(variant.name(), &fields, args, span) else {
            return Expr::primary("undefined");
        };
        self.used.dom.insert("variant");
        let mut emitted = vec![js::string(variant.name()).to_string()];
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
            // unreached: An internal guard. `record` is the only definition
            // kind `Emitter::record` is called for.
            self.error("A record literal names a `record`.", span);
            return Expr::primary("undefined");
        };
        let name = self.hir.defs[def].name.clone();
        let fields: Vec<String> = declared.fields.iter().map(|f| f.name.clone()).collect();

        let Some(values) = self.by_declaration_order(&name, &fields, args, span) else {
            return Expr::primary("undefined");
        };
        // `js::property`, as a foreign's argument object already
        // uses: a field name is a program's own identifier, and an object
        // literal key is the one place it is written as syntax.
        let pairs: Vec<String> = fields
            .iter()
            .zip(values)
            .map(|(field, value)| format!("{}: {value}", js::property(field)))
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
                // unreached: `zdc-types` reports this first, in its own words.
                self.error(format!("`{owner}` is built by naming its fields."), span);
                return None;
            };
            match fields.iter().position(|field| field == name) {
                Some(at) => slots[at] = Some(*value),
                None => {
                    // unreached: `zdc-types` reports this first, in its own
                    // words.
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
                    // unreached: `zdc-types` reports this first, in its own
                    // words.
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
        if let Res::BuiltinVariant(variant) = callee {
            return self.builtin_variant(variant, args, span);
        }
        // `Pair with first is …, second is …`. Emitted exactly as a
        // record literal is, and for the same reason: two fields, always
        // in the same order, so every pair in a bundle shares one hidden
        // class (§16.7 item 9). It is why a pair needs nothing of
        // `runtime/wire.js`: an object with named fields is the shape a
        // record already crosses on.
        if callee == Res::Builtin(Builtin::Pair) {
            let fields: Vec<String> = Type::PAIR_FIELDS.iter().map(|f| (*f).to_string()).collect();
            let Some(values) = self.by_declaration_order("Pair", &fields, args, span) else {
                return Expr::primary("undefined");
            };
            let pairs: Vec<String> = fields
                .iter()
                .zip(values)
                .map(|(field, value)| format!("{}: {value}", js::property(field)))
                .collect();
            return Expr::primary(format!("{{ {} }}", pairs.join(", ")));
        }
        let Res::Def(def) = callee else {
            // unreached: `zdc-types` reports this first, in these same
            // words — `infer.rs` carries a copy of the sentence.
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
        let Some(arguments) = self.ordered_arguments(def, args, span) else {
            return Expr::primary("undefined");
        };
        // Inside an argument list a comma is the separator, so every
        // argument is already grouped by the call's own parentheses and
        // none of them needs any of its own.
        let emitted: Vec<String> = arguments.iter().map(|arg| arg.text.clone()).collect();
        let name = self.names.def(def).to_string();

        // §17.4.7: a `zd:` primitive is emitted as its JavaScript form
        // rather than as a call to a definition, because there is no
        // definition — it has no body. Everything else is an ordinary
        // call to a function this bundle also carries.
        if let DefKind::Foreign(foreign) = &self.hir.defs[def].kind {
            let module = foreign.module().map(str::to_string);
            let symbol = foreign.export.as_str().to_string();
            let owns_view = foreign.owns_view();
            let constructs = foreign.constructs();
            let is_method = foreign.is_method();
            let is_property = foreign.is_property();
            let writes_property = foreign.writes_property();
            // A method and a property are never a `zd:` primitive: the
            // primitive layer is module-qualified by construction, and a
            // receiver is not a module. Looking one up would ask a table
            // keyed on a module about a declaration that names none.
            let intrinsic = module
                .as_deref()
                .and_then(|module| intrinsics::intrinsic(module, &symbol));
            let Some(form) = intrinsic else {
                // Not a `zd:` primitive, so it is a real module and the
                // bundle imports it. §14E.2 links a foreign into whichever
                // bundles actually call it, which is why the record is
                // made here — at a call — rather than from the
                // declaration list.
                if owns_view {
                    // unreached: `zdc-types` reports this first, in its own
                    // words — `infer.rs` rules on a call to a `gives view`
                    // foreign before an emission is attempted. Kept because
                    // this is the site that would otherwise emit a call
                    // whose result is `undefined`.
                    self.error(
                        format!(
                            "`{}` gives a view, so it owns a DOM node and is written as a view \
                             element rather than called for a result (spec §14E.1).",
                            self.hir.defs[def].name
                        ),
                        span,
                    );
                    return Expr::primary("undefined");
                }
                // The export was refused at parse time, and it is checked
                // again here because this is the *emission* site: the
                // parser guards one construct's syntax, and this guards
                // the position the name is written into. Two passes with
                // one rule between them, never two rules.
                if js::ident(&symbol).is_none() {
                    // unreached: the parser reports this first, in its own
                    // words — `foreign_export` refuses a literal that is
                    // not a JavaScript identifier, so no `ExportName`
                    // holding one exists to reach an emission. Kept
                    // because this is the position the name is written
                    // into, and a guard here is what makes that a
                    // property of the bytes rather than of the grammar.
                    self.error(
                        format!(
                            "`{}` would be imported as `{symbol}`, which is not a JavaScript \
                             identifier. An `import` clause needs a name as syntax, so there is \
                             no escaping that makes this safe (spec §14E.1).",
                            self.hir.defs[def].name
                        ),
                        span,
                    );
                    return Expr::primary("undefined");
                }
                // `of Handle as "domElement"` — the symbol is a property
                // of the call's first argument, and the emission is member
                // access and **nothing else**: no parentheses, because
                // `renderer.domElement` is a canvas and
                // `renderer.domElement()` is a `TypeError`.
                //
                // Emitted before the method form rather than folded into
                // it, so that the one difference between the two — whether
                // an argument list is written — is one branch a reader can
                // see rather than an `if` buried inside a `format!`.
                if is_property {
                    let Some(receiver) = arguments.into_iter().next() else {
                        // unreached: `zdc-resolve` reports this first, in
                        // its own words — a property with no parameters has
                        // nothing to read from and is refused at the
                        // declaration.
                        self.error(
                            format!(
                                "`{}` is a property and takes no arguments, so there is nothing \
                                 to read it off (spec §14E.1).",
                                self.hir.defs[def].name
                            ),
                            span,
                        );
                        return Expr::primary("undefined");
                    };
                    return Expr::new(
                        format!("{}.{symbol}", receiver.operand(precedence::MEMBER)),
                        precedence::MEMBER,
                    );
                }
                // `set Handle as "roughness"` — the symbol is a property
                // of the call's first argument and the second argument is
                // written into it. One `=`, and the whole of the form's
                // safety is that neither side is spelled by the program:
                // the member name is a validated identifier from the
                // declaration, and the two operands are emitted
                // expressions like any other argument.
                //
                // The receiver is an operand of member access, so anything
                // binding more loosely than a dot gets its parentheses.
                // The value needs none of its own — assignment is
                // right-associative, so every expression this compiler can
                // emit binds tighter than the `=` it sits to the right of.
                if writes_property {
                    let mut written = arguments.iter();
                    let (Some(receiver), Some(value)) = (written.next(), written.next()) else {
                        // unreached: `zdc-resolve` reports this first, in
                        // its own words — a write with fewer than two
                        // parameters has no object or no value and is
                        // refused at the declaration.
                        self.error(
                            format!(
                                "`{}` writes a property and needs the object and the value, and \
                                 one of them is missing (spec §14E.1).",
                                self.hir.defs[def].name
                            ),
                            span,
                        );
                        return Expr::primary("undefined");
                    };
                    return Expr::new(
                        format!(
                            "{}.{symbol} = {}",
                            receiver.operand(precedence::MEMBER),
                            value.operand(precedence::ASSIGNMENT)
                        ),
                        precedence::ASSIGNMENT,
                    );
                }
                // `on Handle as "add"` — the symbol is a method on the
                // call's first argument, and **nothing is imported**: a
                // method comes with the object. The receiver is emitted as
                // an operand of member access, so an expression that binds
                // more loosely than a dot gets its parentheses; the rest of
                // the arguments are inside the call's own.
                if is_method {
                    let mut written = arguments.iter();
                    let Some(receiver) = written.next() else {
                        // unreached: `zdc-resolve` reports this first, in
                        // its own words — a method with no parameters has
                        // no receiver and is refused at the declaration.
                        self.error(
                            format!(
                                "`{}` is a method and takes no arguments, so there is nothing to \
                                 look it up on (spec §14E.1).",
                                self.hir.defs[def].name
                            ),
                            span,
                        );
                        return Expr::primary("undefined");
                    };
                    let rest: Vec<String> = written.map(|argument| argument.text.clone()).collect();
                    return Expr::new(
                        format!(
                            "{}.{symbol}({})",
                            receiver.operand(precedence::MEMBER),
                            rest.join(", ")
                        ),
                        precedence::MEMBER,
                    );
                }
                let Some(module) = module else {
                    // unreached: every non-method source carries a module,
                    // and `is_method` is the only other variant.
                    self.error(
                        format!(
                            "`{}` names no module to import it from (spec §14E.1).",
                            self.hir.defs[def].name
                        ),
                        span,
                    );
                    return Expr::primary("undefined");
                };
                self.used.foreign.insert(def, (module, symbol));
                // `gives new Handle` — the export is a class, so the call
                // constructs. `new X(…)` with its argument list is
                // `NewExpression` with arguments, which binds exactly as
                // tightly as a call does, so the precedence is the same one
                // an ordinary call is emitted at and no member access after
                // it needs parentheses.
                if constructs {
                    return Expr::new(
                        format!("new {name}({})", emitted.join(", ")),
                        precedence::MEMBER,
                    );
                }
                return Expr::new(
                    format!("{name}({})", emitted.join(", ")),
                    precedence::MEMBER,
                );
            };
            return match form {
                // **The operand keeps its own precedence.** Neither of
                // these two forms wraps it in call syntax — `Identity`
                // emits it unchanged and `Field` appends `.length` — so
                // the grouping the source wrote is carried by the operand
                // itself or by nothing at all.
                JsForm::Identity | JsForm::Field(_) => {
                    let operand = arguments
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| Expr::primary(String::new()));
                    self.apply(form, operand)
                }
                JsForm::Helper(helper) => {
                    self.use_helper(helper);
                    Expr::new(
                        format!("{helper}({})", emitted.join(", ")),
                        precedence::MEMBER,
                    )
                }
            };
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
        self.types
            .expr_in(id, self.ctx.read_context())
            .or_else(|| self.types.expr(id))
            .filter(|ty| !matches!(ty, Type::Unknown))
    }

    /// Whether the checker settled this expression as a `Truth`.
    ///
    /// Asked at the one place a value becomes text for a person to read
    /// (#297), where the host's word for a truth is not this language's.
    /// It is a question about the *value*, not about the element, which is
    /// why it lives beside `settled` rather than in the view lowering: the
    /// answer is the checker's and codegen is only reading it.
    pub fn is_truth(&self, id: ExprId) -> bool {
        matches!(self.settled(id), Some(Type::Truth))
    }

    /// One call's arguments, in the callee's declaration order.
    ///
    /// Shared with the self-tail-call rewrite in `stmt`, which needs the
    /// same reordering to know what to give each parameter on the next
    /// turn of the loop. A second copy of it would be a second place for
    /// `f with b is 2, a is 1` to come out in the wrong order.
    /// **`Expr`, not `String`.** An argument is handed back with the
    /// precedence it was emitted at, because one caller does not wrap it in
    /// call syntax: a `JsForm::Identity` or `JsForm::Field` primitive puts
    /// its operand straight into the surrounding expression, and there the
    /// operand's own binding is the whole question. Returning text made
    /// that caller invent a precedence, it invented `PRIMARY`, and
    /// `r * (decimalOf of (10 + d * 3))` came out as `r * 10 + d * 3` —
    /// which typechecks, builds, and computes 32 where the source says 44.
    pub(crate) fn ordered_arguments(
        &mut self,
        def: DefId,
        args: &[HirArg],
        span: zdc_lexer::Span,
    ) -> Option<Vec<Expr>> {
        let parameters = match &self.hir.defs[def].kind {
            DefKind::Function(function) => function.params.clone(),
            DefKind::Foreign(foreign) => foreign.params.clone(),
            // A release is called exactly like a function, so call sites do
            // not advertise that a boundary was crossed (§19.1).
            DefKind::Release(release) => release.params.clone(),
            // Nothing else is callable. Written out rather than
            // wildcarded so that a new callable `DefKind` has to be
            // given its parameter list here on purpose.
            DefKind::Signal(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => {
                // unreached: `zdc-types` reports this first, in its own
                // words, with the identical sentence — `infer.rs`'s
                // call-site rule runs before anything here does.
                self.error(
                    format!("`{}` is not a function.", self.hir.defs[def].name),
                    span,
                );
                return None;
            }
        };

        let params: Vec<String> = parameters
            .iter()
            .map(|param| self.hir.locals[*param].name.clone())
            .collect();

        let mut ordered: Vec<Option<ExprId>> = vec![None; params.len()];
        let mut next_positional = 0;
        for arg in args {
            match arg {
                HirArg::Positional(expr) => {
                    if next_positional >= ordered.len() {
                        // unreached: `zdc-types` reports this first, in its
                        // own words.
                        self.error(
                            format!(
                                "`{}` takes {} argument(s), and this call passes more.",
                                self.hir.defs[def].name,
                                params.len()
                            ),
                            span,
                        );
                        return None;
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
                        // unreached: `zdc-types` reports this first, in its
                        // own words.
                        self.error(
                            format!(
                                "`{}` has no parameter named `{arg_name}`. Its parameters are {}.",
                                self.hir.defs[def].name,
                                params.join(", ")
                            ),
                            span,
                        );
                        return None;
                    }
                },
            }
        }

        let mut emitted = Vec::with_capacity(ordered.len());
        for (index, slot) in ordered.iter().enumerate() {
            match slot {
                Some(expr) => emitted.push(self.value(*expr)),
                None => {
                    // unreached: `zdc-types` reports this first, in its own
                    // words.
                    self.error(
                        format!(
                            "`{}` is missing an argument for `{}`.",
                            self.hir.defs[def].name, params[index]
                        ),
                        span,
                    );
                    return None;
                }
            }
        }

        Some(emitted)
    }

    fn binary(
        &mut self,
        id: ExprId,
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
        span: zdc_lexer::Span,
    ) -> Expr {
        // §17.4.3: `contains` is one word over three library functions,
        // all three written in ZDeceptron. Which one is the checker's
        // verdict, and the answer is a definition rather than a JavaScript
        // form, so this emits a call to code the bundle also carries.
        if op == BinOp::Contains {
            let Some(target) = self.types.operator_target(id) else {
                // unreached: `zdc-types` reports this first, in its own words. A
                // `contains` it could not classify is one it refused to type.
                self.error(
                    "`contains` needs to know whether this is a text, a list or a map, and \
                     nothing in the program says which.",
                    span,
                );
                return Expr::primary("undefined");
            };
            let container = self.value(lhs).into_text();
            let value = self.value(rhs).into_text();
            return Expr::new(
                format!("{}({container}, {value})", self.names.def(target)),
                precedence::MEMBER,
            );
        }

        // §16.7 item 2. `===` is value equality for the base types and
        // *reference* equality for everything else, and the runtime has no
        // structural comparison to fall back on, so comparing two lists
        // would silently answer a different question than it looks like.
        // The checker has proved the operand type by now, so this is a
        // decision rather than a guess.
        if matches!(op, BinOp::Is | BinOp::IsNot) {
            let compared = self.settled(lhs).cloned().unwrap_or(Type::Unknown);
            if !is_compared_by_value(&compared) {
                // unreached: `zdc-types` reports this first, in its own words. Its
                // operand rule for `is` refuses a record before the emitter sees
                // one, which is what makes the ignored test above a *disagreement*
                // rather than a shared refusal.
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
            // Emitted as a call above; it never reaches the symbol table
            // because there is no JavaScript operator that means it.
            BinOp::Contains => unreachable!("`contains` is emitted as a library call"),
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

    let mut source = js::string(path.trim_end_matches('/')).to_string();
    let mut reactive = false;
    for value in values {
        let piece = match value {
            Operand::Literal(literal) => js::string(&format!("/{}", literal.as_text())).to_string(),
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
        Operand::Reactive(format!("() => {}", js::arrow_body(&source)))
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

/// The tag `map each … in` transforms through, for the two containers that
/// hold zero or one.
///
/// Named rather than defaulted: a container this does not know is a
/// compiler bug and says so, instead of quietly emitting a test against
/// `'Some'` on something that has no such arm.
fn payload_tag(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::Option(_) => Some("Some"),
        Type::Remote(_) => Some("Ready"),
        // Written out rather than wildcarded, and this is the one match
        // over `Type` in the emitter where that earns its keep. A *third*
        // container — anything holding zero or one of something — would
        // inherit `None` here and silently emit nothing, which is the
        // failure the doc comment above says this function exists to
        // prevent. A scalar inheriting `None` is right; a container
        // inheriting it is the bug, and only naming them tells the two
        // apart.
        Type::Text
        | Type::Markup
        | Type::Whole
        | Type::Decimal
        | Type::Truth
        | Type::Error
        | Type::Handle
        | Type::Code
        | Type::Event(_)
        | Type::Named(_)
        | Type::List(_)
        | Type::Map(_, _)
        | Type::Pair(_, _)
        | Type::Function(_, _)
        | Type::Var(_)
        | Type::Unknown
        | Type::Nothing => None,
    }
}
