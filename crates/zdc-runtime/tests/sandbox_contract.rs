use std::path::Path;

use zdc_runtime::{Ask, Capability, Provided, ProvidedPart, Sandbox};

fn echo_with_root(ask: Ask<'_>) -> Result<Provided, String> {
    Ok(Provided::Text(format!(
        "{}::{}",
        ask.root.display(),
        ask.argument
    )))
}

fn markup(ask: Ask<'_>) -> Result<Provided, String> {
    Ok(Provided::Markup(format!("<strong>{}</strong>", ask.argument)))
}

fn words(ask: Ask<'_>) -> Result<Provided, String> {
    Ok(Provided::List(
        ask.argument.split_whitespace().map(str::to_owned).collect(),
    ))
}

/// One prose part and one widget part, which is the shape `build parts`
/// gives back and the only shape this variant can carry.
fn halves(ask: Ask<'_>) -> Result<Provided, String> {
    Ok(Provided::Parts(vec![
        ProvidedPart {
            markup: format!("<p>{}</p>", ask.argument),
            widget: String::new(),
            argument: String::new(),
        },
        ProvidedPart {
            markup: String::new(),
            widget: ask.widgets.join("+"),
            argument: ask.argument.to_string(),
        },
    ]))
}

fn refuse(ask: Ask<'_>) -> Result<Provided, String> {
    Err(format!("refused `{}`", ask.argument))
}

#[test]
fn loaded_modules_keep_their_bindings_for_later_questions() {
    let mut sandbox = Sandbox::new();
    sandbox
        .load("export const base = 40;\nexport function answer() { return base + 2; }")
        .expect("module loads");

    assert_eq!(sandbox.text("answer()").expect("binding remains"), "42");
}

#[test]
fn default_constructs_the_same_empty_sandbox_as_new() {
    let mut from_new = Sandbox::new();
    let mut from_default = Sandbox::default();

    assert_eq!(from_new.text("typeof $build").unwrap(), "undefined");
    assert_eq!(from_default.text("typeof $build").unwrap(), "undefined");
}

#[test]
fn text_returns_javascript_string_conversion_not_debug_rendering() {
    let mut sandbox = Sandbox::new();

    assert_eq!(sandbox.text("'plain text'").unwrap(), "plain text");
    assert_eq!(sandbox.text("[1, 2, 3]").unwrap(), "1,2,3");
    assert_eq!(sandbox.text("undefined").unwrap(), "undefined");
}

#[test]
fn capabilities_receive_the_exact_root_and_stringified_argument() {
    let mut sandbox = Sandbox::new();
    sandbox
        .provide(
            Path::new("/project/root"),
            &[],
            &[Capability {
                name: "echo",
                answer: echo_with_root,
            }],
        )
        .expect("capability installs");

    assert_eq!(
        sandbox
            .text("$build.echo(21 * 2)")
            .expect("capability answers"),
        "/project/root::42"
    );
}

#[test]
fn every_provided_shape_crosses_the_javascript_boundary() {
    let mut sandbox = Sandbox::new();
    sandbox
        .provide(
            Path::new("/project"),
            &["Ring".to_string(), "Bars".to_string()],
            &[
                Capability {
                    name: "markup",
                    answer: markup,
                },
                Capability {
                    name: "words",
                    answer: words,
                },
                Capability {
                    name: "halves",
                    answer: halves,
                },
            ],
        )
        .expect("capabilities install");

    assert_eq!(
        sandbox.text("$build.markup('safe')").unwrap(),
        "<strong>safe</strong>"
    );
    assert_eq!(
        sandbox
            .text("JSON.stringify($build.words('one two three'))")
            .unwrap(),
        r#"["one","two","three"]"#
    );
    // The fourth shape, and the one thing it says that the others do not:
    // a capability sees the widget names the program declares.
    assert_eq!(
        sandbox
            .text("JSON.stringify($build.halves('x'))")
            .unwrap(),
        r#"[{"markup":"<p>x</p>","widget":"","argument":""},{"markup":"","widget":"Ring+Bars","argument":"x"}]"#
    );
}

#[test]
fn a_capability_called_without_an_argument_fails_actionably() {
    let mut sandbox = Sandbox::new();
    sandbox
        .provide(
            Path::new("/project"),
            &[],
            &[Capability {
                name: "echo",
                answer: echo_with_root,
            }],
        )
        .expect("capability installs");

    let error = sandbox
        .text("$build.echo()")
        .expect_err("argument is required");

    assert!(
        error.message.contains("takes one argument"),
        "unexpected error: {error}"
    );
    assert!(!error.budget_exceeded);
}

#[test]
fn capability_refusals_are_errors_and_not_return_values() {
    let mut sandbox = Sandbox::new();
    sandbox
        .provide(
            Path::new("/project"),
            &[],
            &[Capability {
                name: "refuse",
                answer: refuse,
            }],
        )
        .expect("capability installs");

    let error = sandbox
        .text("$build.refuse('outside')")
        .expect_err("refusal must stop evaluation");

    assert!(error.message.contains("refused `outside`"), "{error}");
    assert!(!error.budget_exceeded);
}

#[test]
fn separate_sandboxes_do_not_share_loaded_bindings() {
    let mut first = Sandbox::new();
    first.load("const privateValue = 7;").unwrap();
    assert_eq!(first.text("privateValue").unwrap(), "7");

    let mut second = Sandbox::new();
    assert_eq!(second.text("typeof privateValue").unwrap(), "undefined");
}

#[test]
fn non_terminating_evaluation_is_stopped_by_a_deterministic_budget() {
    let mut sandbox = Sandbox::new();
    let error = sandbox
        .text("while (true) {}")
        .expect_err("the loop must be interrupted");

    assert!(error.budget_exceeded, "unexpected error: {error}");
    assert!(!error.message.is_empty());
}

#[test]
fn ordinary_javascript_failures_are_not_reported_as_budget_exhaustion() {
    let mut sandbox = Sandbox::new();
    for expression in [
        "throw new Error('ordinary failure')",
        "missingFunction()",
        "(()",
    ] {
        let error = sandbox.text(expression).expect_err(expression);
        assert!(!error.budget_exceeded, "{expression}: {error}");
        assert!(!error.message.is_empty(), "{expression}");
    }
}

#[test]
fn a_failed_question_does_not_discard_previously_loaded_bindings() {
    let mut sandbox = Sandbox::new();
    sandbox.load("const stable = 42;").unwrap();
    sandbox
        .text("throw new Error('one bad question')")
        .expect_err("the question fails");

    assert_eq!(sandbox.text("stable").unwrap(), "42");
}
