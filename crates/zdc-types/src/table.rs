//! What the type checker hands to whoever runs next.
//!
//! §16.7 is a list a code generator already wrote against, so it is a
//! contract rather than a suggestion. Every blocking entry on that list
//! is answered here, by the same name it has there.

use std::collections::HashMap;

use zdc_hir::{DefId, ExprId, LocalId};
use zdc_lexer::Span;

use crate::choice::Choice;
use crate::placement::ReadContext;
use crate::ty::Type;

/// Which container an `at` indexes (§16.7 item 5). They need different
/// runtime helpers, so codegen cannot emit one without knowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexKind {
    List,
    Map,
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
/// first. `indexes`, `empties`, `whens` and `arm_gives` stay keyed as they
/// are: none of them is affected by `Remote` wrapping.
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
