use zdc_ast::{Decl, Node, Stmt};
use zdc_lexer::Span;

fn covered(source: &str, span: Span) -> &str {
    &source[span.start as usize..span.end as usize]
}

#[test]
fn statement_patterns_expose_all_named_field_bindings() {
    let source = concat!(
        "function render with status\n",
        "    when status\n",
        "        Archived with why, moment show why\n",
        "        Loading show empty\n",
    );
    let program = zdc_parser::parse(source).expect("source parses");
    let Decl::Function(function) = &program.decls[0] else {
        panic!("expected a function")
    };
    let Stmt::When(branch) = &function.body.stmts[0] else {
        panic!("expected a when statement")
    };

    let archived = &branch.arms[0].pattern;
    let binding_names = archived
        .bindings
        .iter()
        .map(|binding| binding.text.as_str())
        .collect::<Vec<_>>();

    assert_eq!(archived.name.text, "Archived");
    assert_eq!(binding_names, ["why", "moment"]);
    assert_eq!(covered(source, archived.span), "Archived with why, moment");
    assert!(branch.arms[1].pattern.bindings.is_empty());
}

#[test]
fn view_patterns_use_the_same_multi_binding_shape() {
    let source = concat!(
        "view\n",
        "    when entry\n",
        "        Archived with why, moment show Text why\n",
    );
    let program = zdc_parser::parse(source).expect("source parses");
    let Decl::View(view) = &program.decls[0] else {
        panic!("expected a view")
    };
    let Node::When(branch) = &view.nodes[0] else {
        panic!("expected a when node")
    };

    let pattern = &branch.arms[0].pattern;
    let binding_names = pattern
        .bindings
        .iter()
        .map(|binding| binding.text.as_str())
        .collect::<Vec<_>>();

    assert_eq!(binding_names, ["why", "moment"]);
    assert_eq!(covered(source, pattern.span), "Archived with why, moment");
}

#[test]
fn a_trailing_pattern_comma_reports_the_missing_name() {
    let source = concat!(
        "function render with status\n",
        "    when status\n",
        "        Archived with why,\n",
    );

    let error = zdc_parser::parse(source).unwrap_err();

    assert!(error.message.contains("a name"), "got: {}", error.message);
    assert_eq!(error.span.start, source.len() as u32);
}

#[test]
fn adjacent_pattern_bindings_require_a_comma() {
    let source = concat!(
        "function render with status\n",
        "    when status\n",
        "        Archived with why moment show why\n",
    );

    let error = zdc_parser::parse(source).unwrap_err();

    assert!(
        error.message.contains("line break"),
        "expected the stray second name to be rejected: {}",
        error.message
    );
}
