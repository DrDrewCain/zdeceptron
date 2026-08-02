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
    Set { place: Place, value: Expr },
    Add { value: Expr, place: Place },
    Subtract { value: Expr, place: Place },
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

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub name: Ident,
    pub binding: Option<Ident>,
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
    pub nodes: Vec<Node>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Each(EachNode),
    When(WhenNode),
    Handler(Handler),
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
    Show(Expr),
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
