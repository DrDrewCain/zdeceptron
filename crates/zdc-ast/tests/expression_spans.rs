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
        // `Address` arrived with declarative routing (§14G.2 revision 1),
        // and the count below is what required it to be constructed here
        // rather than merely named in `variant`.
        Expr::Address { span: outer },
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
        // `List` and `Map` were absent, and the exhaustiveness assertion
        // below is what found them: the test claimed to cover every
        // variant while covering eleven of thirteen.
        Expr::List {
            items: vec![empty(inner)],
            span: outer,
        },
        Expr::Map {
            entries: vec![(empty(inner), empty(inner))],
            span: outer,
        },
        // `Of` arrived with the prelude's unary accessors, and the count
        // below is what required it to be constructed here rather than
        // merely named in `variant`.
        Expr::Of {
            name: ident("length", inner),
            operand: Box::new(empty(inner)),
            span: outer,
        },
    ];

    // "Every variant" was the claim, and a hand-written list was the
    // evidence: a new `Expr` variant that reported an inner span would
    // have been added to `zdc-ast` and simply not listed here, leaving
    // this test green and its name wrong. `variant` below is exhaustive
    // and this workspace forbids wildcard arms, so a new variant is a
    // compile error until it is named — and the count then fails until it
    // is also constructed above.
    let mut covered: Vec<&str> = expressions.iter().map(variant).collect();
    covered.sort_unstable();
    covered.dedup();
    assert_eq!(
        covered.len(),
        VARIANTS.len(),
        "every `Expr` variant must be exercised; missing: {:?}",
        VARIANTS
            .iter()
            .filter(|name| !covered.contains(name))
            .collect::<Vec<_>>()
    );

    for expression in expressions {
        assert_eq!(expression.span(), outer, "variant: {expression:?}");
    }
}

/// Written out by hand, so the list cannot agree with the code it checks.
const VARIANTS: [&str; 15] = [
    "Address",
    "Binary",
    "Call",
    "Empty",
    "Environment",
    "Field",
    "Index",
    "List",
    "Map",
    "Number",
    "Of",
    "Text",
    "Truth",
    "Unary",
    "Var",
];

fn variant(expression: &Expr) -> &'static str {
    match expression {
        Expr::Number { .. } => "Number",
        Expr::Text { .. } => "Text",
        Expr::Truth { .. } => "Truth",
        Expr::Empty { .. } => "Empty",
        Expr::Var { .. } => "Var",
        Expr::Call { .. } => "Call",
        Expr::Binary { .. } => "Binary",
        Expr::Unary { .. } => "Unary",
        Expr::Environment { .. } => "Environment",
        Expr::Address { .. } => "Address",
        Expr::List { .. } => "List",
        Expr::Map { .. } => "Map",
        Expr::Field { .. } => "Field",
        Expr::Index { .. } => "Index",
        Expr::Of { .. } => "Of",
    }
}
