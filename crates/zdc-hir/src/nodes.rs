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
    Image,
    Link,
}

impl BuiltinElement {
    /// Every built-in, so a pass may iterate the vocabulary rather than
    /// restate it. Adding a variant without adding it here fails
    /// `the_vocabulary_is_enumerated` below.
    pub const ALL: &'static [BuiltinElement] = &[
        BuiltinElement::Column,
        BuiltinElement::Row,
        BuiltinElement::Text,
        BuiltinElement::Heading,
        BuiltinElement::Button,
        BuiltinElement::Input,
        BuiltinElement::Checkbox,
        BuiltinElement::Spinner,
        BuiltinElement::ErrorBar,
        BuiltinElement::Image,
        BuiltinElement::Link,
    ];

    /// Whether this element writes back into the signal bound to its first
    /// positional argument on every interaction (spec §14B.5).
    pub fn is_two_way(self) -> bool {
        matches!(self, BuiltinElement::Input | BuiltinElement::Checkbox)
    }

    /// The named arguments of *this* element that the browser dereferences
    /// as a URL (spec §14G.1.3(c) sink 7).
    ///
    /// **The `match` has no wildcard arm, and that is the point.** A new
    /// element cannot be added to the vocabulary without deciding, here,
    /// whether it carries a URL — which is the same lesson §16.3.10 draws
    /// about wildcard match arms in the emitter. A list a future element
    /// can silently fall through is not a closed list.
    ///
    /// This is *not* the enforcement boundary. Enforcement is
    /// [`is_url_attribute`], which ranges over the attribute name on every
    /// element, because `named_argument` passes an unrecognised name
    /// through to the attribute of that name: `Text src is …` reaches the
    /// DOM whether or not `Text` was meant to have a `src`. The two are
    /// tied together by a test.
    pub fn url_arguments(self) -> &'static [&'static str] {
        match self {
            BuiltinElement::Column
            | BuiltinElement::Row
            | BuiltinElement::Text
            | BuiltinElement::Heading
            | BuiltinElement::Button
            | BuiltinElement::Input
            | BuiltinElement::Checkbox
            | BuiltinElement::Spinner
            | BuiltinElement::ErrorBar => &[],
            BuiltinElement::Image => &["source"],
            BuiltinElement::Link => &["href"],
        }
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
            BuiltinElement::Image => "Image",
            BuiltinElement::Link => "Link",
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
            "Image" => BuiltinElement::Image,
            "Link" => BuiltinElement::Link,
            _ => return None,
        })
    }
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
    Component(Component),
    Foreign(Foreign),
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
    /// §14C.3b: the path this value is written to at build time, if any.
    ///
    /// Carried on the signal rather than on a declaration of its own,
    /// because it *is* a property of the state: `rss.xml` is the value of
    /// `feed`, so there is nothing to keep in sync with anything.
    pub emits: Option<zdc_ast::Emitted>,
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
