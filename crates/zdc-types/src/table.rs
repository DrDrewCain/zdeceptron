//! What the type checker hands to whoever runs next.
//!
//! §16.7 is a list a code generator already wrote against, so it is a
//! contract rather than a suggestion. Every blocking entry on that list
//! is answered here, by the same name it has there.

use std::collections::HashMap;

use zdc_hir::{DefId, ExprId, LocalId};
use zdc_lexer::Span;

use crate::choice::Choice;
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
#[derive(Debug, Clone, Default)]
pub struct TypeTable {
    exprs: HashMap<ExprId, Type>,
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
    /// Keyed by arm span, which is unique to its arm.
    arm_gives: HashMap<Span, bool>,
}

impl TypeTable {
    /// The type of an expression (§16.7 items 1, 2 and 9).
    ///
    /// `+` is addition when this is `Whole` or `Decimal` and
    /// concatenation when it is `Text`; `is` is `===` for a base type and
    /// structural for everything else.
    pub fn expr(&self, id: ExprId) -> Option<&Type> {
        self.exprs.get(&id)
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
    /// (§16.7 item 7). Keyed by the arm's span.
    pub fn arm_always_gives(&self, arm: Span) -> Option<bool> {
        self.arm_gives.get(&arm).copied()
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
        self.exprs.iter().map(|(id, ty)| (*id, ty))
    }

    pub(crate) fn set_expr(&mut self, id: ExprId, ty: Type) {
        self.exprs.insert(id, ty);
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

    pub(crate) fn set_arm_gives(&mut self, arm: Span, gives: bool) {
        self.arm_gives.insert(arm, gives);
    }
}
