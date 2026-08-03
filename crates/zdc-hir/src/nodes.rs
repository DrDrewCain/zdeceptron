//! The HIR node types.
//!
//! Every node carries the span of the source it came from: later passes
//! report their errors against HIR rather than AST, so a node without a
//! span is a diagnostic that cannot point anywhere.

use crate::ids::{Arena, ArenaId, BlockId, DefId, ExprId, LocalId};
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
    /// One variant of a choice the language provides — `Some`, `None`,
    /// `Loading`, `Ready`, `Failed`.
    ///
    /// §17.4.2: `BUILTIN_PATTERNS` recognised these in *pattern* position
    /// only, so no function could ever return an `Option`. A library that
    /// cannot write `Some with value is v` cannot be written at all, which
    /// is what this variant fixes.
    BuiltinVariant(BuiltinVariant),
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
    Element,
    /// A type name the language provides, such as `Text` or `Whole`.
    Type,
}

/// One variant of `Option of T` or `Remote of T`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinVariant {
    Some,
    None,
    Loading,
    Ready,
    Failed,
}

impl BuiltinVariant {
    pub fn from_name(name: &str) -> Option<BuiltinVariant> {
        Some(match name {
            "Some" => BuiltinVariant::Some,
            "None" => BuiltinVariant::None,
            "Loading" => BuiltinVariant::Loading,
            "Ready" => BuiltinVariant::Ready,
            "Failed" => BuiltinVariant::Failed,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            BuiltinVariant::Some => "Some",
            BuiltinVariant::None => "None",
            BuiltinVariant::Loading => "Loading",
            BuiltinVariant::Ready => "Ready",
            BuiltinVariant::Failed => "Failed",
        }
    }

    /// The names of the fields this variant carries, in declaration order.
    pub fn field_names(self) -> &'static [&'static str] {
        match self {
            BuiltinVariant::Some => &["value"],
            BuiltinVariant::Ready => &["value"],
            BuiltinVariant::Failed => &["error"],
            BuiltinVariant::None | BuiltinVariant::Loading => &[],
        }
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
    /// How many leading definitions came from the prelude (§17.4.1).
    ///
    /// The prelude is resolved into *these* arenas rather than its own, so
    /// a user reference to `valueOr` is an ordinary `Res::Def` and every
    /// later pass needs no rule for it. It is allocated first and
    /// contiguously, so one number separates the library from the program —
    /// which is what lets an editor list only the user's declarations and
    /// lets a diagnostic tell "the user wrote this" from "the library did".
    pub prelude_defs: usize,
    /// How many leading expressions came from the prelude. Spans below
    /// this index index the prelude's own source files, not the user's.
    pub prelude_exprs: usize,
    /// How many leading binders came from the prelude.
    pub prelude_locals: usize,
}

impl Hir {
    pub fn new() -> Self {
        Hir {
            defs: Arena::new(),
            locals: Arena::new(),
            exprs: Arena::new(),
            blocks: Arena::new(),
            view: None,
            prelude_defs: 0,
            prelude_exprs: 0,
            prelude_locals: 0,
        }
    }

    /// Whether this definition came from the prelude rather than from the
    /// file being compiled.
    pub fn is_prelude_def(&self, id: DefId) -> bool {
        id.index() < self.prelude_defs
    }

    /// Whether this expression came from the prelude.
    pub fn is_prelude_expr(&self, id: ExprId) -> bool {
        id.index() < self.prelude_exprs
    }

    /// Whether this binder came from the prelude.
    pub fn is_prelude_local(&self, id: LocalId) -> bool {
        id.index() < self.prelude_locals
    }

    /// Every definition the file being compiled declared, in source order.
    pub fn user_defs(&self) -> impl Iterator<Item = (DefId, &Def)> {
        self.defs.iter().filter(|(id, _)| !self.is_prelude_def(*id))
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
    Foreign(Foreign),
}

/// A `foreign` declaration: a platform function with no ZDeceptron body
/// (§14E, §17.4.2).
///
/// §14F.2 says the standard library is written in ZDeceptron and that
/// failing to write a piece of it "is a finding about the language, not a
/// reason to reach for the FFI". §17.4.10 records which pieces those are:
/// building a `Text` out of nothing, constructing a collection, f64
/// formatting, Unicode case tables, and the clock. Every other prelude
/// operation is an ordinary `Function` above these.
#[derive(Debug, Clone, PartialEq)]
pub struct Foreign {
    pub site: zdc_ast::ForeignSite,
    pub module: String,
    pub symbol: String,
    pub form: zdc_ast::CallForm,
    pub params: Vec<LocalId>,
    /// The asserted parameter types, positionally matching `params`.
    pub param_types: Vec<zdc_ast::TypeExpr>,
    pub result: zdc_ast::TypeExpr,
}

impl Foreign {
    /// Whether this names the language's own primitive layer rather than a
    /// package on the platform (§17.4.10).
    pub fn is_primitive(&self) -> bool {
        self.module.starts_with("zd:")
    }
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// How every call to this function must be written (§17.4.2). A
    /// `with` function called with `of`, or the reverse, is an error that
    /// names the one valid form.
    pub form: zdc_ast::CallForm,
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
    /// `length of items` — a call in the `of` form (§14F.1, §17.4.2).
    ///
    /// Kept apart from `Call` because the two are not interchangeable: the
    /// declaration decides which spelling a callable answers to, and
    /// collapsing them here would lose the only thing that distinguishes
    /// them by the time the checker could report it.
    OfCall {
        callee: Res,
        operand: ExprId,
    },
    /// `length of` and `text of` — the two members of §17.4.3's closed
    /// dispatched set that no ZDeceptron body can define, whichever type
    /// they are applied to.
    ///
    /// Which primitive this means is chosen by the head constructor of its
    /// operand, so the checker settles it and records the answer; codegen
    /// reads that verdict rather than guessing one.
    Operator {
        op: OperatorName,
        operand: ExprId,
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
    /// `append item to list` — the list construction form.
    ///
    /// The one operation that makes a list *longer*. `rest of` makes one
    /// shorter and, before this, nothing made one longer, so no function
    /// could hand back a collection it had not been given — which is what
    /// kept `split`, `reverse` and `values` in the primitive layer.
    Append {
        item: ExprId,
        list: ExprId,
    },
}

/// A built-in unary operator written with `of`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorName {
    /// `length of` — over `Text`, `List of T`, and `Map of K to V`.
    Length,
    /// `text of` — over every base type.
    TextOf,
}

impl OperatorName {
    pub fn from_name(name: &str) -> Option<OperatorName> {
        Some(match name {
            "length" => OperatorName::Length,
            "text" => OperatorName::TextOf,
            _ => return None,
        })
    }

    /// How it reads in a diagnostic, as the program wrote it.
    pub fn describe(self) -> &'static str {
        match self {
            OperatorName::Length => "length of",
            OperatorName::TextOf => "text of",
        }
    }
}

/// The `of`-operator names no user declaration may take, because a
/// program writing `length of` must always mean the same thing (§4.1).
pub const BUILTIN_OF_OPERATORS: &[&str] = &["length", "text"];

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
    /// `with total is 0` — spec §17.4.10's local binding.
    Bind(HirBind),
}

/// One `with` statement's run of bindings, in written order.
///
/// Not a scope of its own: a binding is in scope from the statement after
/// it to the end of the block it was written in, which is the block's
/// scope and no new one. This is the same decision §14D's `HirScope`
/// records for a component instance — a construct that binds names
/// without being a region boundary — and it is why nothing downstream of
/// resolution needs a rule for bindings at all.
#[derive(Debug, Clone, PartialEq)]
pub struct HirBind {
    pub bindings: Vec<HirBinding>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirBinding {
    pub local: LocalId,
    pub value: ExprId,
    pub span: Span,
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
