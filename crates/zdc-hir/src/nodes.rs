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
    /// A top-level `state`, `function`, `record`, `choice`, or `view`.
    Def(DefId),
    /// A parameter, loop variable, or pattern binding.
    Local(LocalId),
    /// A name the language provides rather than the program.
    Builtin(Builtin),
    /// One variant of a user-declared `choice`, by the choice it belongs to
    /// and its position in the declaration.
    ///
    /// A variant name is a value (`All`) or a constructor (`Archived with
    /// reason is "old"`), and both need the choice as well as the name, so
    /// resolution settles it here rather than leaving codegen to search.
    Variant { choice: DefId, index: u32 },
}

/// The kind of built-in a `Res::Builtin` names.
///
/// A stopgap until user-defined components (spec §14D) and record and
/// choice declarations (§14B.1) exist, at which point built-in elements
/// and types become ordinary definitions with a `DefId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    /// A view element the language provides, such as `Row` or `Text`.
    Element(BuiltinElement),
    /// A type name the language provides, such as `Text` or `Whole`.
    Type,
}

/// Which view element a [`Builtin::Element`] names (spec §17.2.2(b)).
///
/// Carrying the element rather than a bare marker is what lets a pass ask
/// "is this the two-way `Input`?" without matching on a string. A string
/// match is a live soundness hole the moment §14D lets a program declare
/// `component Input`: a user component resolves to [`Res::Def`] and can
/// never be confused with the built-in, but only if the question is asked
/// of the resolution rather than of the spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinElement {
    Column,
    Row,
    Text,
    Heading,
    Button,
    Input,
    Checkbox,
    Spinner,
    ErrorBar,
}

impl BuiltinElement {
    /// Whether this element writes back into the signal bound to its first
    /// positional argument on every interaction (spec §14B.5).
    pub fn is_two_way(self) -> bool {
        matches!(self, BuiltinElement::Input | BuiltinElement::Checkbox)
    }

    pub fn name(self) -> &'static str {
        match self {
            BuiltinElement::Column => "Column",
            BuiltinElement::Row => "Row",
            BuiltinElement::Text => "Text",
            BuiltinElement::Heading => "Heading",
            BuiltinElement::Button => "Button",
            BuiltinElement::Input => "Input",
            BuiltinElement::Checkbox => "Checkbox",
            BuiltinElement::Spinner => "Spinner",
            BuiltinElement::ErrorBar => "ErrorBar",
        }
    }

    pub fn from_name(name: &str) -> Option<BuiltinElement> {
        Some(match name {
            "Column" => BuiltinElement::Column,
            "Row" => BuiltinElement::Row,
            "Text" => BuiltinElement::Text,
            "Heading" => BuiltinElement::Heading,
            "Button" => BuiltinElement::Button,
            "Input" => BuiltinElement::Input,
            "Checkbox" => BuiltinElement::Checkbox,
            "Spinner" => BuiltinElement::Spinner,
            "ErrorBar" => BuiltinElement::ErrorBar,
            _ => return None,
        })
    }
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
    Record(Record),
    Choice(Choice),
    Component(Component),
}

/// A `component` declaration (spec §14D.1).
///
/// The body is kept as written, never as instantiated. Each call site gets
/// its own copy, because a component's own `state` is per instance and its
/// parameters carry the caller's placements — so the graph the later passes
/// traverse is the *inlined* one (§14D.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub params: Vec<LocalId>,
    /// The binder for the nodes nested under this component at its call
    /// site, if it declared one.
    pub children: Option<LocalId>,
    /// The component's own state, in declaration order. Every one is
    /// `client`-placed: §14D.1 admits no other, because `server` state is
    /// per invocation and `durable` state is shared, so neither has a
    /// per-instance meaning.
    pub states: Vec<LocalSignal>,
    pub body: Vec<HirNode>,
}

/// A signal whose lifetime is one component instance rather than the
/// program.
///
/// It is a `Local` rather than a `Def` on purpose: a `Def` is emitted once
/// at module scope, and a component inside an `each` needs one signal per
/// row. Binding it as a local puts the declaration inside whichever region
/// closure the instance lands in, which is exactly per-instance.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalSignal {
    pub local: LocalId,
    pub placement: zdc_ast::Placement,
    pub ty: zdc_ast::TypeExpr,
    pub is_source: bool,
    pub init: ExprId,
    pub span: Span,
}

/// A `record` declaration: a product type with named fields (§14B.1).
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// In declaration order. Codegen emits object literals in this order so
    /// every instance of a record shares one hidden class (§16.7 item 9).
    pub fields: Vec<Field>,
}

/// A `choice` declaration: a tagged union (§14B.1, §14G.1.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub variants: Vec<Variant>,
}

/// One variant of a `choice`, with its named fields in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Span,
}

/// One `name is type` field of a record or of a variant's payload.
///
/// The type is not resolved here, for the same reason a signal's is not:
/// this pass resolves names to definitions, and a type name has a meaning
/// to check only once there is a checker.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: zdc_ast::TypeExpr,
    pub span: Span,
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
    /// §14C.3b: the path this value is written to at build time, if any.
    ///
    /// Carried on the signal rather than on a declaration of its own,
    /// because it *is* a property of the state: `rss.xml` is the value of
    /// `feed`, so there is nothing to keep in sync with anything.
    pub emits: Option<zdc_ast::Emitted>,
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
    /// `[a, b]` — spec §14B.4.
    List(Vec<ExprId>),
    /// `["a" to 1]` — spec §14B.4, in written order.
    Map(Vec<(ExprId, ExprId)>),
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
    Append { value: ExprId, place: HirPlace },
    Remove { value: ExprId, place: HirPlace },
}

impl HirMutation {
    pub fn place(&self) -> &HirPlace {
        match self {
            HirMutation::Set { place, .. }
            | HirMutation::Add { place, .. }
            | HirMutation::Subtract { place, .. }
            | HirMutation::Append { place, .. }
            | HirMutation::Remove { place, .. } => place,
        }
    }

    pub fn value(&self) -> ExprId {
        match self {
            HirMutation::Set { value, .. }
            | HirMutation::Add { value, .. }
            | HirMutation::Subtract { value, .. }
            | HirMutation::Append { value, .. }
            | HirMutation::Remove { value, .. } => *value,
        }
    }
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
    If(HirIfNode),
    Handler(HirHandler),
    /// `children`, before instantiation replaces it with the nodes nested
    /// under the call site. No `Children` survives into a `view`.
    Children(Span),
    /// One component instance: its own state, and the body that reads it.
    ///
    /// Produced by instantiation, never by the parser. It is not a region
    /// boundary — the locals are declared in whatever region the instance
    /// lands in, so an instance inside an `each` row gets its state inside
    /// that row's closure.
    Scope(HirScope),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirScope {
    pub locals: Vec<LocalSignal>,
    pub body: Vec<HirNode>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirIfNode {
    pub cond: ExprId,
    pub then: Vec<HirNode>,
    pub otherwise: Option<Vec<HirNode>>,
    pub span: Span,
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
