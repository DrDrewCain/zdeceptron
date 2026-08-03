#![forbid(unsafe_code)]

//! Plain data types for the ZDeceptron syntax tree.
//!
//! No logic lives here. The parser builds these; later passes lower them
//! into HIR.

use zdc_lexer::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    State(StateDecl),
    Function(FunctionDecl),
    View(ViewDecl),
    Record(RecordDecl),
    Choice(ChoiceDecl),
    Component(ComponentDecl),
    Use(UseDecl),
    Foreign(ForeignDecl),
    Route(RouteDecl),
    Release(ReleaseDecl),
}

// --- routing (spec §14G.2) ---

/// `route Site` — the set of URLs this program answers to.
///
/// A route is a `choice` plus a bijection between its values and URLs
/// (§14G.2). It is declared, not derived from a directory layout: a
/// file-based convention would put the URL space in the file system,
/// which invariant 5 forbids as configuration the compiler cannot check.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteDecl {
    pub name: Ident,
    pub variants: Vec<RouteVariantDecl>,
    pub span: Span,
}

/// `BlogPost is "/blog" with slug is Text in postSlugs`
#[derive(Debug, Clone, PartialEq)]
pub struct RouteVariantDecl {
    pub name: Ident,
    /// The literal prefix, exactly as written. `[slug]` meta-syntax inside
    /// a string is refused for the same reason §6 refuses embedded markup.
    pub path: String,
    pub path_span: Span,
    pub params: Vec<RouteParamDecl>,
    pub span: Span,
}

/// `slug is Text in postSlugs` — one route parameter.
///
/// `in` takes a bare name, never an expression (§14G.2 revision 4): an
/// undelimited expression before a comma-separated list is swallowed by
/// the greedy argument list.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteParamDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    /// The `static` signal this parameter ranges over, if it is
    /// enumerable. A parameter with no `in` is not enumerable, and §18.1
    /// semantics 5 makes it **untrusted**.
    pub enumerated_in: Option<Ident>,
    pub span: Span,
}

// --- modules (spec §14D.2) ---

/// `use "./model" for Item, Status` — the names this file borrows from
/// another one.
///
/// The path is relative to the importing file and the `.zd` extension is
/// implied. One phrasing per construct (§4.1): no wildcard, no aliasing,
/// and no re-export in v1.
#[derive(Debug, Clone, PartialEq)]
pub struct UseDecl {
    pub path: String,
    pub path_span: Span,
    pub names: Vec<Ident>,
    pub span: Span,
}

// --- components (spec §14D.1) ---

/// `component VoteCard with item, votes` — a named run of view nodes,
/// used at the call site exactly as a built-in element is.
///
/// `children` is not in `params`. It is not passed at the call site; it is
/// the nodes nested *under* the call site, so it is recorded separately
/// and positional arguments never have to step over it.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDecl {
    pub name: Ident,
    pub params: Vec<Ident>,
    /// Where `children` was written in the parameter list, if it was.
    pub children: Option<Span>,
    pub body: Vec<ComponentItem>,
    pub span: Span,
}

/// One line of a component's body: either its own state, or a view node.
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentItem {
    State(StateDecl),
    Node(Node),
}

// --- type declarations (spec §4.4 `typeDecl`, §14B.1 as amended by §14G.1.2) ---

/// One `name is type` line, in a `record` body or a variant's payload.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `record Todo` — a product type whose fields are named.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordDecl {
    pub name: Ident,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

/// `choice Status` — a tagged union whose variants carry named fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceDecl {
    pub name: Ident,
    pub variants: Vec<VariantDecl>,
    pub span: Span,
}

/// One variant of a `choice`.
///
/// §14G.1.2: `variant := IDENT ["with" variantField ("," variantField)*]`,
/// and a `variantField` is `IDENT "is" type` — the same `name is type` line
/// a record field is, which is why both use [`FieldDecl`].
#[derive(Debug, Clone, PartialEq)]
pub struct VariantDecl {
    pub name: Ident,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

// --- state ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Client,
    /// §14C.3b. Read once at build time and inlined into the bundle.
    /// Reading it from the client yields `T` rather than `Remote of T`,
    /// because no boundary is crossed — Rule 1 (§5.2) is satisfied, not
    /// excepted.
    Static,
    Server,
    Durable,
}

impl Placement {
    /// The one English spelling, for diagnostics that name the placement
    /// a program wrote.
    pub fn word(self) -> &'static str {
        match self {
            Placement::Client => "client",
            Placement::Static => "static",
            Placement::Server => "server",
            Placement::Durable => "durable",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Init {
    /// `starting <expr>` — a source signal, mutable.
    Starting(Expr),
    /// `from <expr>` — a derived signal, recomputed, not directly mutable.
    From(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateDecl {
    pub secret: bool,
    /// `trusted state orders …` — spec §18.1.1.
    ///
    /// The integrity direction's one declaration-level grant on state
    /// (`G-SIG` clause 1, spec §21.7.3). It is the *obligation* marker too:
    /// declaring it is what makes every write to the place (A3) and every
    /// index into it (A1) a checked site.
    pub trusted: bool,
    pub name: Ident,
    pub placement: Placement,
    pub ty: TypeExpr,
    pub init: Init,
    /// §14C.3b's sub-requirement: where this value is **written** at build
    /// time, relative to the bundle root.
    ///
    /// `rss.xml` and `llms.txt` are generated *files*, not endpoints, and
    /// deriving them from the same state the pages are built from is what
    /// keeps them from drifting. Only a `static` signal may carry one,
    /// because only a `static` signal has a value at build time.
    pub emits: Option<Emitted>,
    pub span: Span,
}

/// A build-time output path, and where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emitted {
    pub path: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Named(Ident),
    List(Box<TypeExpr>),
    Map(Box<TypeExpr>, Box<TypeExpr>),
    Option(Box<TypeExpr>),
    Remote(Box<TypeExpr>),
}

// --- functions and statements ---

/// How a callable's arguments are written, and therefore how every call to
/// it must be written.
///
/// §17.4.2: a function is called in exactly one form, and the declaration
/// decides which. `length with posts` where `length` was declared
/// `function length of value` is an error naming the one valid form, and
/// vice versa — which is what keeps §4.1's one-phrasing rule while giving
/// unary accessors the `of` spelling §14F.1 asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallForm {
    /// `f with a, b` — any number of parameters.
    With,
    /// `length of items` — exactly one, a unary accessor.
    Of,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub name: Ident,
    pub form: CallForm,
    pub params: Vec<Ident>,
    pub body: Block,
    pub span: Span,
}

/// Where a `foreign` may run (§14E.2).
///
/// **This answers one question and only one: which output bundles may this
/// library be linked into.** It is not a purity classification, it never
/// was, and reading it as one is residual risk R1 — see [`ForeignResult`],
/// which is the classification built for the other question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignSite {
    Client,
    Server,
    Anywhere,
}

/// What a `foreign` declares about **its result**, on the `gives` line
/// (§21.9).
///
/// Two questions were spelled with one word until §21.8. [`ForeignSite`]
/// answers *where may this be linked*; this answers *is the result a
/// function of the arguments*. They are independent — a query-string
/// reader is honestly `is anywhere` and is not pure, and a password hash
/// is honestly `is server` and is — so they get separate declarations.
///
/// **The default is [`ForeignResult::Opaque`]**, and the default is the
/// design: an unmarked `foreign` is never mistaken for pure. The failure
/// mode of the other default is a silent leak, which is the same reason
/// `Authority` defaults to `Untrusted`.
///
/// Deliberately an enum rather than two `bool`s: `gives pure trusted T` is
/// not a state the type can hold, so no consumer has to decide what it
/// would mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForeignResult {
    /// `gives T` — no claim. The result is whatever the JavaScript did,
    /// which for all the compiler knows is the wall clock or the request
    /// URL.
    #[default]
    Opaque,
    /// `gives pure T` — grant `G-FGN-P`: the result is a function of the
    /// arguments, so its integrity is their join.
    ///
    /// Asserted by a human and checked by nobody. §14E.4's dev-mode check
    /// validates the shape of a return value and cannot detect impurity.
    /// What changed in §21.9 is not that the claim became checkable — it is
    /// that it became *declared* rather than inferred from an unrelated
    /// property.
    Pure,
    /// `gives trusted T` — grant `G-FGN-T`: the result is Trusted whatever
    /// the arguments were. Strictly stronger than [`ForeignResult::Pure`],
    /// and strictly more of a human's word.
    Trusted,
}

impl ForeignResult {
    /// The one valid spelling of the modifier, or `None` where there is no
    /// modifier to spell.
    pub fn describe(self) -> Option<&'static str> {
        match self {
            ForeignResult::Opaque => None,
            ForeignResult::Pure => Some("pure"),
            ForeignResult::Trusted => Some("trusted"),
        }
    }
}

impl ForeignSite {
    pub fn describe(self) -> &'static str {
        match self {
            ForeignSite::Client => "client",
            ForeignSite::Server => "server",
            ForeignSite::Anywhere => "anywhere",
        }
    }
}

/// One parameter of a `foreign`: a name and the type it asserts.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignParam {
    pub name: Ident,
    /// `takes key is trusted Text` — a **requirement on the caller**,
    /// discharged at obligation site A2 (spec §18.1 semantics 8). The same
    /// word on a `release` clause is a *grant*; §19.10.2 records why the
    /// two live in different syntactic slots.
    pub trusted: bool,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `foreign textLength is anywhere` — spec §14E.1, as amended by §17.4.2.
///
/// The types are *asserted*, not inferred: there is no body to infer them
/// from. §17.4.10 lists the seventeen operations that need one, and every
/// `foreign` outside that list is the program's own claim about a platform
/// function.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignDecl {
    pub name: Ident,
    pub site: ForeignSite,
    /// The module the symbol comes from. A `zd:` prefix names the
    /// language's own primitive layer (§17.4.10) rather than a package.
    pub module: String,
    pub symbol: String,
    pub form: CallForm,
    pub params: Vec<ForeignParam>,
    /// What the `gives` line claims about the result — `gives T`,
    /// `gives pure T` or `gives trusted T` (spec §21.9).
    pub result_grant: ForeignResult,
    pub result: TypeExpr,
    pub span: Span,
}

// --- declassification (spec §19.1, §19.10.2) ---

/// `limit 10 per visitor` — the per-evaluation budget clause.
///
/// **This bounds nothing cumulatively.** It counts evaluations of *one*
/// declaration against *one* anonymous session: `k` declarations give `kN`,
/// clearing a cookie mints a fresh budget, and until `DurableStore` exists
/// it is not enforced at all. Spec §21.8.7 and residual risk R3.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseLimit {
    pub count: u32,
    pub span: Span,
}

/// `release judge with guess, answer` — spec §19.1 as amended by §19.10.2.
///
/// ```text
/// releaseDecl := "release" IDENT ["with" params] NEWLINE INDENT
///                  "gives" type NEWLINE
///                  { "trusted" IDENT NEWLINE }
///                  [ "limit" NUMBER "per" "visitor" NEWLINE ]
///                  stmt+ DEDENT
/// ```
///
/// Clause order is fixed — `gives`, then endorsements, then `limit`, then
/// statements — so `releaseDecl` stays LL(1) and the parser never
/// backtracks.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseDecl {
    pub name: Ident,
    pub params: Vec<Ident>,
    /// The declared bandwidth per evaluation (spec §19.2 rule 4).
    pub gives: TypeExpr,
    /// The parameters named by a `trusted` clause — `endorsed(f)` in
    /// REL-ARG. Site-local and result-transparent: an endorsement discharges
    /// REL-ARG at this release's call sites and does nothing anywhere else
    /// (spec §19.10.3(a)).
    pub endorsed: Vec<Ident>,
    pub limit: Option<ReleaseLimit>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Pipeline(PipelineClause),
    Mutation(Mutation),
    Give(Expr),
    When(WhenStmt),
    Each(EachStmt),
    If(IfStmt),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineClause {
    From(Expr),
    Keep { var: Ident, cond: Expr },
    Sort { var: Ident, key: Expr },
    MapEach { var: Ident, to: Expr },
    TakeFirst(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mutation {
    Set {
        place: Place,
        value: Expr,
    },
    /// Numbers only (spec §14B.2).
    Add {
        value: Expr,
        place: Place,
    },
    /// Numbers only (spec §14B.2).
    Subtract {
        value: Expr,
        place: Place,
    },
    /// Collections only (spec §14B.2).
    Append {
        value: Expr,
        place: Place,
    },
    /// Collections only (spec §14B.2).
    Remove {
        value: Expr,
        place: Place,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub base: Ident,
    pub path: Vec<PathSeg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathSeg {
    Field(Ident),
    Index(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenStmt {
    pub scrutinee: Expr,
    pub arms: Vec<Arm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pattern: Pattern,
    pub body: ArmBody,
    pub span: Span,
}

/// A `when` arm's pattern: a variant name and the names it binds.
///
/// A variant declares *named fields* (`Archived with reason is Text`), and
/// a pattern binds a fresh name to each of them positionally
/// (`Archived with why, moment`). A pattern may therefore bind several
/// names, so this is a list rather than a single optional binder — the
/// grammar is `pattern := IDENT ["with" IDENT ("," IDENT)*]` (spec
/// §14G.1.2). A payload-free variant such as `Loading` binds none, and
/// the list is empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub name: Ident,
    pub bindings: Vec<Ident>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArmBody {
    Show(Expr),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EachStmt {
    pub var: Ident,
    pub iter: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub cond: Expr,
    pub then: Block,
    pub otherwise: Option<Block>,
    pub span: Span,
}

// --- view ---

#[derive(Debug, Clone, PartialEq)]
pub struct ViewDecl {
    /// The document's metadata: `view title is "…", description is "…"`.
    /// Named arguments, exactly as an element's are.
    pub args: Vec<Arg>,
    pub nodes: Vec<Node>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Each(EachNode),
    When(WhenNode),
    Handler(Handler),
    /// `if open` with an indented body, and an optional `otherwise`.
    ///
    /// §4.4 gave `if` to statements only, and §14D.1's own `Disclosure`
    /// writes one in node position. The view needs it for the same reason
    /// a block does: showing a node conditionally is not the same question
    /// as matching a variant, and spelling it `when` would need a `choice`
    /// nobody declared.
    If(IfNode),
    /// `children` — the nodes nested under this component at its call site.
    Children(Span),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfNode {
    pub cond: Expr,
    pub then: Vec<Node>,
    pub otherwise: Option<Vec<Node>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub name: Ident,
    pub args: Vec<Arg>,
    pub children: Vec<Node>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Positional(Expr),
    Named { name: Ident, value: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EachNode {
    pub var: Ident,
    pub iter: Expr,
    pub body: Vec<Node>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenNode {
    pub scrutinee: Expr,
    pub arms: Vec<NodeArm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeArm {
    pub pattern: Pattern,
    pub body: NodeArmBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeArmBody {
    Show(Element),
    Nodes(Vec<Node>),
}

/// `on click` — a listener on the element it is nested in.
///
/// `payload` is the optional binder of `on click with press`: the event the
/// browser raised, as a value. It reuses the `with`-introduces-binders
/// phrasing `function f with a`, `component C with label` and
/// `Archived with reason` already have, so it costs no reserved word
/// (§4.1, §14G.7.7). Omitting it is the whole of the old form, which is
/// why every existing program is unaffected.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    pub event: Ident,
    pub payload: Option<Ident>,
    pub body: Block,
    pub span: Span,
}

// --- expressions ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Is,
    IsNot,
    /// `body contains query` — §14F.1's one addition to the closed infix
    /// set. Which of `textContains`, `listContains` and `mapContains` it
    /// means is chosen by the head constructor of the left operand
    /// (§17.4.3), which only the type checker knows.
    Contains,
    Less,
    Greater,
    LessEq,
    GreaterEq,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number {
        value: f64,
        span: Span,
    },
    Text {
        value: String,
        span: Span,
    },
    Truth {
        value: bool,
        span: Span,
    },
    Empty {
        span: Span,
    },
    /// `["red", "green"]` — spec §14B.4. `[]` is the empty list; the empty
    /// map has no bracket form, because `[]` cannot be both.
    List {
        items: Vec<Expr>,
        span: Span,
    },
    /// `["a" to 1, "b" to 2]` — spec §14B.4, reusing the `to` of
    /// `Map of K to V` so one word means one thing in type and value
    /// position alike.
    Map {
        entries: Vec<(Expr, Expr)>,
        span: Span,
    },
    Var {
        name: Ident,
        span: Span,
    },
    Call {
        name: Ident,
        args: Vec<Arg>,
        span: Span,
    },
    /// `length of posts` — §14F.1's `of` prefix for unary accessors, and
    /// §17.4.2's `ofExpr`. Right-associative, so `text of day of moment`
    /// is `text of (day of moment)`.
    Of {
        name: Ident,
        operand: Box<Expr>,
        span: Span,
    },
    Environment {
        key: String,
        span: Span,
    },
    /// `address` — the URL this document was served at, as a value of the
    /// program's `route` type wrapped in `Option` (spec §14G.2).
    ///
    /// A signal initialised from it is immutable: the browser writes it at
    /// load and the program never does, which is what makes per-URL
    /// constant folding ordinary constant propagation rather than a new
    /// evaluation mode (§14G.2 revision 1).
    Address {
        span: Span,
    },
    /// `build read "content/hello.md"` — a compiler-provided capability.
    ///
    /// The capability keeps its written spelling here: whether it names
    /// one of the closed set is a resolution question, so the parser does
    /// not answer it and a misspelling gets a diagnostic that can list the
    /// alternatives.
    Build {
        capability: Ident,
        argument: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Field {
        base: Box<Expr>,
        name: Ident,
        span: Span,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Number { span, .. }
            | Expr::Text { span, .. }
            | Expr::Truth { span, .. }
            | Expr::Empty { span }
            | Expr::List { span, .. }
            | Expr::Map { span, .. }
            | Expr::Var { span, .. }
            | Expr::Call { span, .. }
            | Expr::Of { span, .. }
            | Expr::Environment { span, .. }
            | Expr::Address { span }
            | Expr::Build { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Field { span, .. }
            | Expr::Index { span, .. } => *span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_expr_span_covers_both_operands() {
        let lhs = Expr::Number {
            value: 1.0,
            span: Span::new(0, 1),
        };
        let rhs = Expr::Number {
            value: 2.0,
            span: Span::new(4, 5),
        };
        let sum = Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span: Span::new(0, 5),
        };
        assert_eq!(sum.span(), Span::new(0, 5));
    }

    #[test]
    fn span_is_available_for_every_expression_kind() {
        let s = Span::new(2, 6);
        assert_eq!(Expr::Empty { span: s }.span(), s);
        assert_eq!(
            Expr::Truth {
                value: true,
                span: s
            }
            .span(),
            s
        );
        assert_eq!(
            Expr::Text {
                value: "x".into(),
                span: s
            }
            .span(),
            s
        );
    }
}
