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
    Server,
    Durable,
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
    pub name: Ident,
    pub placement: Placement,
    pub ty: TypeExpr,
    pub init: Init,
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

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub name: Ident,
    pub params: Vec<Ident>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    pub event: Ident,
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
    Environment {
        key: String,
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
            | Expr::Environment { span, .. }
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
