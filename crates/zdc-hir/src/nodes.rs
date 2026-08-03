//! The HIR node types.
//!
//! Every node carries the span of the source it came from: later passes
//! report their errors against HIR rather than AST, so a node without a
//! span is a diagnostic that cannot point anywhere.

use crate::ids::{Arena, BlockId, DefId, ExprId, LocalId};
use zdc_lexer::Span;

/// What a resolved name points at.
///
/// After resolution no reference is a string. Later passes match on one
/// of these variants instead of on spelling, so a rename cannot silently
/// change which declaration a pass believes it is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Res {
    /// A top-level `state`, `function`, or `view`.
    Def(DefId),
    /// A parameter, loop variable, or pattern binding.
    Local(LocalId),
    /// A name the language provides rather than the program.
    Builtin(Builtin),
}

/// The kind of built-in a `Res::Builtin` names.
///
/// A stopgap until user-defined components (spec §14D) and record and
/// choice declarations (§14B.1) exist, at which point built-in elements
/// and types become ordinary definitions with a `DefId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    /// A view element the language provides, such as `Row` or `Text`.
    Element,
    /// A type name the language provides, such as `Text` or `Whole`.
    Type,
}

/// A whole resolved program.
#[derive(Debug, Clone, PartialEq)]
pub struct Hir {
    pub defs: Arena<DefId, Def>,
    pub locals: Arena<LocalId, Local>,
    pub exprs: Arena<ExprId, HirExpr>,
    pub blocks: Arena<BlockId, HirBlock>,
    /// The `view` declaration, if the program has one.
    pub view: Option<DefId>,
}

impl Hir {
    pub fn new() -> Self {
        Hir {
            defs: Arena::new(),
            locals: Arena::new(),
            exprs: Arena::new(),
            blocks: Arena::new(),
            view: None,
        }
    }
}

impl Default for Hir {
    fn default() -> Self {
        Hir::new()
    }
}

/// A top-level declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Def {
    pub name: String,
    pub span: Span,
    pub kind: DefKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DefKind {
    Signal(Signal),
    Function(Function),
    View(View),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub secret: bool,
    pub placement: zdc_ast::Placement,
    /// Types are not resolved by this pass; they are checked by the next
    /// one, which is where a type name has a meaning to check against.
    pub ty: zdc_ast::TypeExpr,
    /// `true` for `starting` (a mutable source), `false` for `from` (a
    /// derived value). Spec §4.5.
    pub is_source: bool,
    pub init: ExprId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub params: Vec<LocalId>,
    pub body: BlockId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub nodes: Vec<HirNode>,
}

/// A binding introduced inside a body: a parameter, a loop variable, or
/// one of a pattern's binders.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExprKind {
    Number(f64),
    Text(String),
    Truth(bool),
    Empty,
    /// A resolved reference. The string is gone.
    Ref(Res),
    Call {
        callee: Res,
        args: Vec<HirArg>,
    },
    Environment(String),
    Unary {
        op: zdc_ast::UnaryOp,
        operand: ExprId,
    },
    Binary {
        op: zdc_ast::BinOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    /// A field name stays a string: which record it selects from is not
    /// known until types are.
    Field {
        base: ExprId,
        name: String,
    },
    Index {
        base: ExprId,
        index: ExprId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirArg {
    Positional(ExprId),
    Named { name: String, value: ExprId },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    Pipeline(HirPipeline),
    Mutation(HirMutation),
    Give(ExprId),
    When(HirWhen),
    Each(HirEach),
    If(HirIf),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirPipeline {
    From(ExprId),
    Keep { var: LocalId, cond: ExprId },
    Sort { var: LocalId, key: ExprId },
    MapEach { var: LocalId, to: ExprId },
    TakeFirst(ExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirMutation {
    Set { place: HirPlace, value: ExprId },
    Add { value: ExprId, place: HirPlace },
    Subtract { value: ExprId, place: HirPlace },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirPlace {
    pub base: Res,
    pub path: Vec<HirPathSeg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirPathSeg {
    Field(String),
    Index(ExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirWhen {
    pub scrutinee: ExprId,
    pub arms: Vec<HirArm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirArm {
    /// The variant matched. Which choice it belongs to is a question for
    /// the type checker, so the name is still a string here.
    pub pattern_name: String,
    /// One binder per named field of the matched variant, in declaration
    /// order (spec §14G.1.2). Empty for a payload-free variant such as
    /// `Loading`.
    pub bindings: Vec<LocalId>,
    pub body: HirArmBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirArmBody {
    Show(ExprId),
    Block(BlockId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirEach {
    pub var: LocalId,
    pub iter: ExprId,
    pub body: BlockId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirIf {
    pub cond: ExprId,
    pub then: BlockId,
    pub otherwise: Option<BlockId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirNode {
    Element(HirElement),
    Each(HirEachNode),
    When(HirWhenNode),
    Handler(HirHandler),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirElement {
    pub name: String,
    pub res: Res,
    pub args: Vec<HirArg>,
    pub children: Vec<HirNode>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirEachNode {
    pub var: LocalId,
    pub iter: ExprId,
    pub body: Vec<HirNode>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirWhenNode {
    pub scrutinee: ExprId,
    pub arms: Vec<HirNodeArm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirNodeArm {
    pub pattern_name: String,
    /// One binder per named field, in declaration order (spec §14G.1.2).
    pub bindings: Vec<LocalId>,
    pub body: HirNodeArmBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirNodeArmBody {
    Show(Box<HirElement>),
    Nodes(Vec<HirNode>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirHandler {
    pub event: String,
    pub body: BlockId,
    pub span: Span,
}
