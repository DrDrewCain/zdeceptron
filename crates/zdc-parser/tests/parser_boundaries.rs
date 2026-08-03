use zdc_ast::{Decl, Node};

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
