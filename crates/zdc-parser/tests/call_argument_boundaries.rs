use zdc_ast::{Arg, Decl, Expr, Stmt};

fn returned_expression(source: &str) -> Expr {
    let program = zdc_parser::parse(source).expect("source parses");
    let Decl::Function(function) = &program.decls[0] else {
        panic!("expected a function")
    };
    let Stmt::Give(expression) = &function.body.stmts[0] else {
        panic!("expected a give statement")
    };
    expression.clone()
}

#[test]
fn identifier_is_expression_is_always_a_named_argument() {
    let expression = returned_expression("function f\n    give choose with item is value\n");
    let Expr::Call { args, .. } = &expression else {
        panic!("expected a call")
    };

    let Arg::Named { name, value } = &args[0] else {
        panic!("expected a named argument")
    };
    assert_eq!(name.text, "item");
    assert!(matches!(value, Expr::Var { name, .. } if name.text == "value"));
}

#[test]
fn parenthesised_nested_calls_are_allowed_as_positional_arguments() {
    let expression =
        returned_expression("function f\n    give outer with (inner with value), second\n");
    let Expr::Call { args, .. } = &expression else {
        panic!("expected an outer call")
    };

    assert_eq!(args.len(), 2);
    assert!(matches!(
        &args[0],
        Arg::Positional(Expr::Call { name, .. }) if name.text == "inner"
    ));
}

#[test]
fn parenthesised_nested_calls_are_allowed_as_named_argument_values() {
    let expression = returned_expression(
        "function f\n    give outer with result is (inner with value), second\n",
    );
    let Expr::Call { args, .. } = &expression else {
        panic!("expected an outer call")
    };

    assert_eq!(args.len(), 2);
    assert!(matches!(
        &args[0],
        Arg::Named {
            name,
            value: Expr::Call { .. },
        } if name.text == "result"
    ));
}

#[test]
fn an_unparenthesised_nested_positional_call_is_rejected() {
    let error = zdc_parser::parse("function f\n    give outer with inner with value, second\n")
        .unwrap_err();

    assert!(
        error.message.contains("parenthes"),
        "expected guidance to parenthesize the nested call: {}",
        error.message
    );
}

#[test]
fn an_unparenthesised_nested_named_value_is_rejected() {
    let error =
        zdc_parser::parse("function f\n    give outer with result is inner with value, second\n")
            .unwrap_err();

    assert!(
        error.message.contains("parenthes"),
        "expected guidance to parenthesize the nested call: {}",
        error.message
    );
}
