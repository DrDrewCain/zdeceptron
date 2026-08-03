//! What the type checker hands to whoever runs next.
//!
//! §16.7 is a list a code generator already wrote against, so it is a
//! contract rather than a suggestion. Every blocking entry on that list
//! is answered here, by the same name it has there.

use std::collections::HashMap;

use zdc_hir::{DefId, ExprId, LocalId};

use crate::choice::Choice;
use crate::placement::ReadContext;
use crate::ty::Type;

/// Which container an `at` indexes (§16.7 item 5). They need different
/// runtime helpers, so codegen cannot emit one without knowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexKind {
    List,
    Map,
    /// §17.4.3 puts `Text` in the `at` row: `name at 0` gives `Option of
    /// Text`. It indexes by code point rather than by UTF-16 unit, which
    /// is why it needs a helper of its own rather than `[]`.
    Text,
}

/// Which primitive a `length of` or `text of` means (§17.4.3).
///
/// The operator is one word and its meaning is chosen by the head
/// constructor of its operand, which only this pass knows. A `List` and a
/// `Map` answer `length` with different properties and a `Text` with
/// neither, so codegen cannot pick one without a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorKind {
    TextLength,
    ListLength,
    MapLength,
    TextOfWhole,
    TextOfDecimal,
    TextOfTruth,
    /// `text of` applied to a `Text`, which is the identity.
    TextOfText,
}

/// Which container an `empty` creates (§16.7 item 6): `[]` or `new Map()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyKind {
    List,
    Map,
}

/// Every type the program was found to have.
///
/// **`exprs` is keyed by `(ExprId, ReadContext)`** — §17.1.4 item 3.
/// `Ref(d)` has one `ExprId` and two types when `d` is read from two
/// regions, and without the re-key the second write silently clobbers the
/// first. `indexes`, `empties` and `whens` stay keyed as they are: none of
/// them is affected by `Remote` wrapping. `arm_gives` was re-keyed for a
/// different reason — see its own note.
///
/// `locals` and `defs` are **not** re-keyed here, and the reason is worth
/// stating: this checker shares one `Scheme` per definition across every
/// context, so a local and a definition each have exactly one type. When
/// schemes become per-context — which is what the spec's own §17.1.4
/// example program needs — those two maps move with them. See the report.
#[derive(Debug, Clone, Default)]
pub struct TypeTable {
    exprs: HashMap<(ExprId, ReadContext), Type>,
    locals: HashMap<LocalId, Type>,
    defs: HashMap<DefId, Type>,
    indexes: HashMap<ExprId, IndexKind>,
    operators: HashMap<ExprId, OperatorKind>,
    /// Which library function a `contains` dispatched to (§17.4.3).
    ///
    /// Unlike `length of`, all three targets are written in ZDeceptron, so
    /// the answer is a definition rather than a JavaScript form — and the
    /// bundle has to carry it, which is why codegen's closure walk needs
    /// this map to find the edge.
    operator_targets: HashMap<ExprId, DefId>,
    empties: HashMap<ExprId, EmptyKind>,
    /// Keyed by the scrutinee, which is unique to its `when`.
    whens: HashMap<ExprId, Choice>,
    /// Keyed by `(scrutinee, arm index)`, **not** by the arm's span.
    ///
    /// A span is not unique to an arm. `zdc-resolve`'s instantiation
    /// copies a component's body once per call site and carries each
    /// span across verbatim, so two instances of one component share
    /// every span in it — the same aliasing that let one instance's
    /// `secret` place discharge another's `public` obligation in
    /// `ifc.rs`. The scrutinee's `ExprId` is freshly allocated per
    /// instance (`instantiate.rs`'s `expr` ends in `exprs.alloc`), so
    /// pairing it with the arm's position is unique where a span is not.
    arm_gives: HashMap<(ExprId, usize), bool>,
}

impl TypeTable {
    /// The type of an expression (§16.7 items 1, 2 and 9).
    ///
    /// `+` is addition when this is `Whole` or `Decimal` and
    /// concatenation when it is `Text`; `is` is `===` for a base type and
    /// structural for everything else.
    pub fn expr(&self, id: ExprId) -> Option<&Type> {
        // Deterministic: the contexts are tried in a fixed order rather
        // than in whatever order the map happens to iterate in.
        [
            ReadContext::Client,
            ReadContext::ViewRootedServer,
            ReadContext::TriggerRootedServer,
            ReadContext::Static,
        ]
        .into_iter()
        .find_map(|context| self.exprs.get(&(id, context)))
    }

    /// The type an expression has *in one context*. Code generation
    /// always knows which root it is emitting, and therefore which
    /// context to ask for (§17.1.4 item 3).
    pub fn expr_in(&self, id: ExprId, context: ReadContext) -> Option<&Type> {
        self.exprs.get(&(id, context))
    }

    pub fn local(&self, id: LocalId) -> Option<&Type> {
        self.locals.get(&id)
    }

    /// A signal's value type, or a function's type.
    pub fn def(&self, id: DefId) -> Option<&Type> {
        self.defs.get(&id)
    }

    /// Whether an `at` indexes a `List` or a `Map` (§16.7 item 5).
    pub fn index_kind(&self, id: ExprId) -> Option<IndexKind> {
        self.indexes.get(&id).copied()
    }

    /// Whether an `empty` is a list or a map (§16.7 item 6).
    pub fn empty_kind(&self, id: ExprId) -> Option<EmptyKind> {
        self.empties.get(&id).copied()
    }

    /// Which primitive a `length of` or `text of` means (§17.4.3).
    pub fn operator_kind(&self, id: ExprId) -> Option<OperatorKind> {
        self.operators.get(&id).copied()
    }

    /// Which library function a `contains` dispatched to (§17.4.3).
    pub fn operator_target(&self, id: ExprId) -> Option<DefId> {
        self.operator_targets.get(&id).copied()
    }

    /// Every dispatched operator target in the program, so codegen's
    /// closure walk can follow the edges §17.4.5 calls the prelude
    /// closure.
    pub fn operator_targets(&self) -> impl Iterator<Item = (ExprId, DefId)> + '_ {
        self.operator_targets.iter().map(|(id, def)| (*id, *def))
    }

    /// The choice type a `when` eliminates, keyed by its scrutinee, with
    /// every variant's declared field list in order (§16.7 item 4).
    ///
    /// `whenInto`'s `arm.length` contract is satisfiable from this: the
    /// arity of arm `n` is `variants[n].fields.len()`.
    pub fn when_choice(&self, scrutinee: ExprId) -> Option<&Choice> {
        self.whens.get(&scrutinee)
    }

    /// Whether a `when` arm in statement position always reaches a `give`
    /// (§16.7 item 7), named by its scrutinee and its position among the
    /// arms — the pair that stays distinct across two instances of one
    /// component, where a span does not.
    pub fn arm_always_gives(&self, scrutinee: ExprId, at: usize) -> Option<bool> {
        self.arm_gives.get(&(scrutinee, at)).copied()
    }

    /// Every `when` in the program, as (scrutinee, choice).
    pub fn whens(&self) -> impl Iterator<Item = (ExprId, &Choice)> {
        self.whens.iter().map(|(id, choice)| (*id, choice))
    }

    /// Every `at` in the program, as (expression, container).
    pub fn indexes(&self) -> impl Iterator<Item = (ExprId, IndexKind)> + '_ {
        self.indexes.iter().map(|(id, kind)| (*id, *kind))
    }

    /// Every `empty` in the program, as (expression, container).
    pub fn empties(&self) -> impl Iterator<Item = (ExprId, EmptyKind)> + '_ {
        self.empties.iter().map(|(id, kind)| (*id, *kind))
    }

    /// Every expression whose type was recorded.
    pub fn expr_types(&self) -> impl Iterator<Item = (ExprId, &Type)> {
        self.exprs.iter().map(|((id, _), ty)| (*id, ty))
    }

    /// Every `(expression, context)` pair whose type was recorded.
    pub fn expr_types_in_context(&self) -> impl Iterator<Item = ((ExprId, ReadContext), &Type)> {
        self.exprs.iter().map(|(key, ty)| (*key, ty))
    }

    pub(crate) fn set_expr(&mut self, id: ExprId, context: ReadContext, ty: Type) {
        self.exprs.insert((id, context), ty);
    }

    pub(crate) fn set_local(&mut self, id: LocalId, ty: Type) {
        self.locals.insert(id, ty);
    }

    pub(crate) fn set_def(&mut self, id: DefId, ty: Type) {
        self.defs.insert(id, ty);
    }

    pub(crate) fn set_index(&mut self, id: ExprId, kind: IndexKind) {
        self.indexes.insert(id, kind);
    }

    pub(crate) fn set_operator(&mut self, id: ExprId, kind: OperatorKind) {
        self.operators.insert(id, kind);
    }

    pub(crate) fn set_operator_target(&mut self, id: ExprId, def: DefId) {
        self.operator_targets.insert(id, def);
    }

    pub(crate) fn set_empty(&mut self, id: ExprId, kind: EmptyKind) {
        self.empties.insert(id, kind);
    }

    pub(crate) fn set_when(&mut self, scrutinee: ExprId, choice: Choice) {
        self.whens.insert(scrutinee, choice);
    }

    pub(crate) fn set_arm_gives(&mut self, scrutinee: ExprId, at: usize, gives: bool) {
        self.arm_gives.insert((scrutinee, at), gives);
    }
}
