use zdc_ast::{Decl, Node};
use zdc_lexer::Span;

/// The source a span actually covers. Every assertion below compares this
/// against the exact text the construct is written with: an inequality
/// between two spans can be satisfied by two wrong spans, but the text a
/// span quotes cannot.
fn covered(src: &str, span: Span) -> &str {
    &src[span.start as usize..span.end as usize]
}

#[test]
fn state_declarations_must_end_at_a_line_break() {
    let src = "state a is client Whole starting 1 state b is client Whole starting 2";
    let err = zdc_parser::parse(src).unwrap_err();
    assert!(err.message.contains("line break"), "got: {}", err.message);
    assert!(
        err.message.contains("declaration goes on its own line"),
        "got: {}",
        err.message
    );
}

#[test]
fn simple_statements_must_end_at_a_line_break() {
    let err = zdc_parser::parse("function f\n    give 1 give 2").unwrap_err();
    assert!(err.message.contains("line break"), "got: {}", err.message);
    assert!(
        err.message.contains("statement goes on its own line"),
        "got: {}",
        err.message
    );
}

#[test]
fn view_nodes_must_end_at_a_line_break() {
    let err = zdc_parser::parse("view\n    Text \"one\" Text \"two\"").unwrap_err();
    assert!(err.message.contains("line break"), "got: {}", err.message);
    assert!(
        err.message.contains("view node goes on its own line"),
        "got: {}",
        err.message
    );
}

#[test]
fn inline_match_arms_must_end_at_a_line_break() {
    let src = "function f\n    when status\n        Loading show 0 Ready show 1\n";
    let err = zdc_parser::parse(src).unwrap_err();
    assert!(err.message.contains("line break"), "got: {}", err.message);
    assert!(
        err.message.contains("match arm goes on its own line"),
        "got: {}",
        err.message
    );
}

#[test]
fn nested_element_span_stops_before_its_next_sibling() {
    let src = "view\n    Row\n        Button \"first\"\n            on click\n                give 1\n        Button \"second\"\n";
    let program = zdc_parser::parse(src).expect("parses");
    let Decl::View(view) = &program.decls[0] else {
        panic!("expected a view")
    };
    let Node::Element(row) = &view.nodes[0] else {
        panic!("expected a row")
    };
    let (Node::Element(first), Node::Element(second)) = (&row.children[0], &row.children[1]) else {
        panic!("expected two buttons")
    };

    assert!(
        first.span.end <= second.name.span.start,
        "first sibling span {:?} overlaps second sibling name {:?}",
        first.span,
        second.name.span
    );
}

#[test]
fn block_view_node_spans_stop_before_their_next_sibling() {
    let src = "view\n    each item in items\n        Text item\n    when status\n        Ready with value\n            Text value\n    Footer\n";
    let program = zdc_parser::parse(src).expect("parses");
    let Decl::View(view) = &program.decls[0] else {
        panic!("expected a view")
    };
    let (Node::Each(each), Node::When(when), Node::Element(footer)) =
        (&view.nodes[0], &view.nodes[1], &view.nodes[2])
    else {
        panic!("expected each, when, and footer nodes")
    };

    assert!(
        each.span.end <= when.span.start,
        "each span {:?} overlaps when span {:?}",
        each.span,
        when.span
    );
    assert!(
        when.span.end <= footer.name.span.start,
        "when span {:?} overlaps footer name {:?}",
        when.span,
        footer.name.span
    );
}

/// The span tree must be a tree: a parent covers its children, and
/// siblings do not overlap. These assert the exact text of each span on a
/// view with *multiple siblings at the same level* — the case the
/// reference-example snapshot cannot catch, because that view is the last
/// declaration in the file and so every over-running end coincidentally
/// lands on the file length and looks right.
#[test]
fn element_spans_cover_exactly_their_own_source() {
    let src = "view\n    Column\n        Row\n    Other\n";
    let program = zdc_parser::parse(src).expect("parses");
    let Decl::View(view) = &program.decls[0] else {
        panic!("expected a view")
    };
    let (Node::Element(column), Node::Element(other)) = (&view.nodes[0], &view.nodes[1]) else {
        panic!("expected two sibling elements")
    };
    let Node::Element(row) = &column.children[0] else {
        panic!("expected a nested child")
    };

    assert_eq!(covered(src, column.span), "Column\n        Row");
    assert_eq!(covered(src, row.span), "Row");
    assert_eq!(covered(src, other.span), "Other");
    assert_eq!(
        covered(src, view.span),
        "view\n    Column\n        Row\n    Other"
    );
    assert!(
        column.span.end <= other.span.start,
        "sibling spans overlap: {:?} then {:?}",
        column.span,
        other.span
    );
}

#[test]
fn block_node_spans_cover_exactly_their_own_source() {
    let src = "view\n    each item in items\n        Text item\n    when status\n        Ready with value\n            Text value\n    Footer\n";
    let program = zdc_parser::parse(src).expect("parses");
    let Decl::View(view) = &program.decls[0] else {
        panic!("expected a view")
    };
    let (Node::Each(each), Node::When(when), Node::Element(footer)) =
        (&view.nodes[0], &view.nodes[1], &view.nodes[2])
    else {
        panic!("expected each, when, and footer nodes")
    };

    assert_eq!(
        covered(src, each.span),
        "each item in items\n        Text item"
    );
    assert_eq!(
        covered(src, when.span),
        "when status\n        Ready with value\n            Text value"
    );
    assert_eq!(
        covered(src, when.arms[0].span),
        "Ready with value\n            Text value"
    );
    assert_eq!(covered(src, footer.span), "Footer");
}

#[test]
fn statement_and_handler_spans_cover_exactly_their_own_source() {
    let src = "function f\n    give 1\n    give 2\nview\n    Row\n        on click\n            set a to 1\n        Next\n";
    let program = zdc_parser::parse(src).expect("parses");
    let Decl::Function(function) = &program.decls[0] else {
        panic!("expected a function")
    };
    assert_eq!(
        covered(src, function.span),
        "function f\n    give 1\n    give 2"
    );
    assert_eq!(covered(src, function.body.span), "\n    give 1\n    give 2");

    let Decl::View(view) = &program.decls[1] else {
        panic!("expected a view")
    };
    let Node::Element(row) = &view.nodes[0] else {
        panic!("expected a row")
    };
    let (Node::Handler(handler), Node::Element(next)) = (&row.children[0], &row.children[1]) else {
        panic!("expected a handler and a sibling element")
    };

    assert_eq!(
        covered(src, handler.span),
        "on click\n            set a to 1"
    );
    assert_eq!(covered(src, next.span), "Next");
    assert!(
        handler.span.end <= next.span.start,
        "handler span {:?} overlaps its next sibling {:?}",
        handler.span,
        next.span
    );
}

#[test]
fn view_declaration_span_stops_before_the_next_declaration() {
    let src = "view\n    Text \"hello\"\nstate x is client Whole starting 1\n";
    let program = zdc_parser::parse(src).expect("parses");
    let (Decl::View(view), Decl::State(state)) = (&program.decls[0], &program.decls[1]) else {
        panic!("expected a view followed by state")
    };

    assert!(
        view.span.end <= state.span.start,
        "view span {:?} overlaps state span {:?}",
        view.span,
        state.span
    );
}

#[test]
fn chained_comparisons_are_rejected_as_ambiguous() {
    let err = zdc_parser::parse("function f\n    give a < b < c\n").unwrap_err();
    assert!(
        err.message.contains("Comparisons cannot be chained"),
        "got: {}",
        err.message
    );
}

#[test]
fn separate_or_parenthesised_comparisons_are_allowed() {
    for expression in ["a < b and b < c", "(a < b) is yes"] {
        let src = format!("function f\n    give {expression}\n");
        zdc_parser::parse(&src)
            .unwrap_or_else(|err| panic!("expected `{expression}` to parse, got: {}", err.message));
    }
}
