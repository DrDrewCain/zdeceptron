use zdc_ast::{Arg, BinOp, Expr, Ident, UnaryOp};
use zdc_lexer::Span;

fn ident(text: &str, span: Span) -> Ident {
    Ident {
        text: text.into(),
        span,
    }
}

fn empty(span: Span) -> Expr {
    Expr::Empty { span }
}

#[test]
fn every_expression_variant_reports_its_outer_span() {
    let outer = Span::new(3, 17);
    let inner = Span::new(5, 8);
    let expressions = vec![
        Expr::Number {
            value: 1.0,
            span: outer,
        },
        Expr::Text {
            value: "hello".into(),
            span: outer,
        },
        Expr::Truth {
            value: true,
            span: outer,
        },
        empty(outer),
        Expr::Var {
            name: ident("value", inner),
            span: outer,
        },
        Expr::Call {
            name: ident("choose", inner),
            args: vec![Arg::Positional(empty(inner))],
            span: outer,
        },
        Expr::Environment {
            key: "API_KEY".into(),
            span: outer,
        },
        Expr::Unary {
            op: UnaryOp::Not,
            operand: Box::new(empty(inner)),
            span: outer,
        },
        Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(empty(inner)),
            rhs: Box::new(empty(inner)),
            span: outer,
        },
        Expr::Field {
            base: Box::new(empty(inner)),
            name: ident("name", inner),
            span: outer,
        },
        Expr::Index {
            base: Box::new(empty(inner)),
            index: Box::new(empty(inner)),
            span: outer,
        },
    ];

    for expression in expressions {
        assert_eq!(expression.span(), outer, "variant: {expression:?}");
    }
}
