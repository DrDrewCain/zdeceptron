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
    /// The `route` declaration, if the program has one, and the URL each
    /// of its variants renders (spec §14G.2).
    ///
    /// A route lowers to an ordinary [`DefKind::Choice`] — it *is* a
    /// choice, plus a bijection onto URLs — so `when` dispatch, variant
    /// construction, exhaustiveness and field binding are the machinery
    /// that already exists rather than a second copy of it. This table is
    /// the bijection, and nothing else about a route is special.
    pub routes: Option<(DefId, RouteTable)>,
}

/// The URL side of a `route` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteTable {
    /// One entry per variant, in declaration order — the same order
    /// [`Choice::variants`] is in, so the two are indexed alike.
    pub variants: Vec<RouteVariantInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteVariantInfo {
    /// The literal prefix, beginning with `/`.
    pub path: String,
    pub path_span: Span,
    pub params: Vec<RouteParam>,
    pub span: Span,
}

/// One route parameter: a variant field that also appears in the URL.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteParam {
    pub name: String,
    /// The `static` signal holding every value this parameter ranges
    /// over, if it is enumerable.
    ///
    /// `None` makes the parameter **untrusted** (spec §18.1 semantics 5):
    /// nothing proved the value came from anywhere but the URL bar.
    /// `Some` makes it trusted, because a successful match against a
    /// compiler-rendered enumeration is a proof rather than a check.
    pub enumerated_in: Option<DefId>,
    pub span: Span,
}

impl RouteTable {
    /// The URL a variant renders with these parameter values.
    ///
    /// One function, used by the collision check, by `Link`, by the page
    /// emitter and by the manifest, so no two of them can disagree about
    /// what a route's URL is.
    pub fn url(&self, index: usize, values: &[String]) -> String {
        let Some(variant) = self.variants.get(index) else {
            return String::new();
        };
        let mut out = variant.path.trim_end_matches('/').to_string();
        for value in values {
            out.push('/');
            out.push_str(value);
        }
        if out.is_empty() {
            out.push('/');
        }
        out
    }
}

impl Hir {
    pub fn new() -> Self {
        Hir {
            defs: Arena::new(),
            locals: Arena::new(),
            exprs: Arena::new(),
            blocks: Arena::new(),
            view: None,
            routes: None,
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
    /// Declared `trusted` (spec §18.1). Integrity is *declared* on state
    /// and *derived* on values, exactly as §17.3 declares secrecy, which
    /// is what keeps the check free of a fixpoint over the set of writers.
    /// It is an obligation checked at every write into this signal and at
    /// every index over it, never a fact that flows out of it.
    pub trusted: bool,
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
    /// The document's metadata, already reduced to the literals it must
    /// be. It is written into `index.html` at build time, so it cannot
    /// read a signal: there is nothing at run time to write it into.
    pub metadata: Metadata,
    pub nodes: Vec<HirNode>,
}

/// What `view title is "…", description is "…", language is "…"` said.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Metadata {
    /// The `<title>`. `None` means the source file's stem is used.
    pub title: Option<String>,
    /// The `<meta name="description">`, omitted when absent.
    pub description: Option<String>,
    /// `<html lang>`, which defaults to `en`.
    pub language: Option<String>,
}

/// The named arguments a `view` accepts, so the diagnostic and the
/// reader agree on the list.
pub const VIEW_METADATA: &[&str] = &["title", "description", "language"];

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
    /// `address` — the URL this document was served at, as
    /// `Option of <route>` (spec §14G.2).
    Address,
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

/// The one element whose leading argument is a URL: `Link`.
pub const DESTINATION_ELEMENT: &str = "Link";

/// The attribute a `Link`'s leading argument *is*, named here rather than
/// left implicit in the slot.
///
/// # Why the destination is a named argument in the HIR
///
/// A `Link`'s destination is written first — `Link "https://example.com"`,
/// `Link Home` — and a leading argument is otherwise lowered by the slot,
/// which is a position rather than a name. Every pass that ranges over
/// *URL-bearing attributes* ranges over attribute **names**: it asks
/// whether an argument is `href`, `src`, `srcset` and so on. A destination
/// carried only by its position would be invisible to every one of them —
/// and it would be invisible for the commonest way there is to write a
/// link, so the rule would look enforced and would not be.
///
/// So the destination is not a nameless slot in the HIR. `zdc-resolve`
/// puts it under this name the moment it lowers the element, and from
/// there it is an ordinary [`HirArg::Named`] carrying the attribute it
/// becomes. A name-keyed URL rule sees it without knowing that `Link`
/// exists, and codegen sends it down the same path a named URL argument
/// takes. The source syntax is unchanged and stays single: writing
/// `href is …` on a `Link` is a resolve error naming the one phrasing.
pub const DESTINATION_ARGUMENT: &str = "href";

/// The destination argument of an element, if it has one.
///
/// The counterpart of [`destination_as_href`]: every pass that wants
/// *where this link goes* asks here rather than reaching for the leading
/// positional argument, which no longer holds it.
pub fn destination_of(element: &HirElement) -> Option<ExprId> {
    if element.name != DESTINATION_ELEMENT {
        return None;
    }
    element.args.iter().find_map(|arg| match arg {
        HirArg::Named { name, value } if name == DESTINATION_ARGUMENT => Some(*value),
        HirArg::Named { .. } | HirArg::Positional(_) => None,
    })
}

/// Rewrite a `Link`'s leading destination into the `href` it becomes.
///
/// Only the first positional argument is rewritten. A second one is not a
/// destination and is left where it is, so the type checker still reports
/// it as the extra leading value it is rather than as a missing `href`.
pub fn destination_as_href(element: &str, mut args: Vec<HirArg>) -> Vec<HirArg> {
    if element != DESTINATION_ELEMENT {
        return args;
    }
    for arg in &mut args {
        if let HirArg::Positional(value) = *arg {
            *arg = HirArg::Named {
                name: DESTINATION_ARGUMENT.to_string(),
                value,
            };
            break;
        }
    }
    args
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
    /// The binder of `on click with press`, if the handler asked for the
    /// event. A `Local` rather than anything new: it is a name bound over
    /// a body, which is what every other binder in the language is, so
    /// scoping, naming and emission all reuse the machinery that exists.
    pub payload: Option<LocalId>,
    /// Where the event name was written, for the diagnostic that has to
    /// name an event the language does not know.
    pub event_span: Span,
    pub body: BlockId,
    pub span: Span,
}
