use zdc_diagnostics::{explain, render, Diagnostic, Level};
use zdc_graph::GraphError;
use zdc_lexer::Span;

#[test]
fn resolve_error_conversion_preserves_location_and_claim_only() {
    let error = zdc_resolve::ResolveError {
        message: "`missing` is not defined".into(),
        span: Span::new(6, 13),
        label: None,
        suggestion: None,
        code: None,
    };

    let diagnostic = Diagnostic::from(error);

    assert_eq!(diagnostic.message, "`missing` is not defined");
    assert_eq!(diagnostic.span, Some(Span::new(6, 13)));
    assert!(diagnostic.notes.is_empty());
    assert_eq!(diagnostic.help, None);
    assert_eq!(diagnostic.code, None);
}

#[test]
fn type_error_conversion_preserves_its_actionable_help() {
    let error = zdc_types::TypeError {
        message: "expected Text, found Whole".into(),
        span: Span::new(12, 17),
        help: Some("Convert the number with `text of`.".into()),
        code: None,
    };

    let diagnostic = Diagnostic::from(error);

    assert_eq!(diagnostic.message, "expected Text, found Whole");
    assert_eq!(diagnostic.span, Some(Span::new(12, 17)));
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("Convert the number with `text of`.")
    );
    assert!(diagnostic.notes.is_empty());
    assert_eq!(diagnostic.code, None);
}

/// A coded type error reaches a reader the way a coded graph finding does:
/// the code ahead of the claim, the caret label read from the rule, and the
/// pointer to `zdc explain` as the help line.
///
/// Asserted over **every** code rather than over one, because the property
/// is of the conversion and not of any family: a code that reached a
/// `TypeError` without an explanation behind it would produce a diagnostic
/// pointing at a page that does not exist.
#[test]
fn a_coded_type_error_carries_its_code_its_caret_and_the_pointer() {
    for code in explain::codes() {
        let error = zdc_types::TypeError {
            message: "the claim, in one sentence.".into(),
            span: Span::new(12, 17),
            help: None,
            code: Some(code),
        };

        let diagnostic = Diagnostic::from(error);

        assert_eq!(
            diagnostic.message,
            format!("[{code}] the claim, in one sentence.")
        );
        assert_eq!(diagnostic.code, Some(code));
        assert_eq!(
            diagnostic.help,
            Some(format!("run 'zdc explain {code}' for the rule")),
            "{code}"
        );
        assert_eq!(diagnostic.label.as_deref(), explain::caret(code), "{code}");
        assert!(diagnostic.label.is_some(), "{code}'s caret says nothing");
    }
}

/// A site whose repair names something in *this* file keeps its own help.
/// The code still reaches the reader, because it is on the message.
#[test]
fn a_coded_type_error_with_a_local_repair_keeps_it() {
    let error = zdc_types::TypeError {
        message: "`clik` is not an event the language knows.".into(),
        span: Span::new(0, 4),
        help: Some("Did you mean `click`?".into()),
        code: Some("E0103"),
    };

    let diagnostic = Diagnostic::from(error);

    assert_eq!(diagnostic.help.as_deref(), Some("Did you mean `click`?"));
    assert!(
        diagnostic.message.starts_with("[E0103] "),
        "{}",
        diagnostic.message
    );
}

#[test]
fn codegen_error_conversion_preserves_span_without_inventing_help() {
    let error = zdc_codegen::CodegenError {
        message: "the element cannot contain children".into(),
        span: Span::new(3, 10),
    };

    let diagnostic = Diagnostic::from(error);

    assert_eq!(diagnostic.message, "the element cannot contain children");
    assert_eq!(diagnostic.span, Some(Span::new(3, 10)));
    assert_eq!(diagnostic.help, None);
    assert_eq!(diagnostic.code, None);
}

#[test]
fn graph_error_conversion_preserves_code_and_ordered_path() {
    let notes = vec![
        (Span::new(0, 5), "secret begins here".into()),
        (Span::new(20, 25), "then reaches the view".into()),
    ];
    let finding = GraphError::new(
        "E-IFC-05",
        "a secret reaches browser-visible text",
        Span::new(30, 35),
    )
    .with_notes(notes.clone())
    .with_help("this phase-local help is replaced by progressive disclosure");

    let diagnostic = Diagnostic::from(finding);

    assert_eq!(
        diagnostic.message,
        "[E-IFC-05] a secret reaches browser-visible text"
    );
    assert_eq!(diagnostic.span, Some(Span::new(30, 35)));
    assert_eq!(diagnostic.notes, notes);
    assert_eq!(diagnostic.code, Some("E-IFC-05"));
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("run 'zdc explain E-IFC-05' for the rule")
    );
}

#[test]
fn every_explanation_code_has_the_same_generated_inline_help() {
    // Counted, because "every code" over an empty list is every code.
    let codes = explain::codes();
    // 50 before the type family. The twenty-one `E02xx` codes are the new
    // ones (#148): the type errors could not be looked up at all until
    // `TypeError` carried a code.
    assert_eq!(codes.len(), 71, "the explanation table changed size");
    for code in codes {
        let diagnostic =
            Diagnostic::from(GraphError::new(code, "generated finding", Span::new(0, 1)));

        assert_eq!(
            diagnostic.help,
            Some(format!("run 'zdc explain {code}' for the rule")),
            "{code}"
        );
    }
}

#[test]
fn rendered_secondary_labels_retain_reading_order_in_their_messages() {
    let source = "first middle final";
    let diagnostic = Diagnostic {
        message: "flow crossed two boundaries".into(),
        span: Some(Span::new(13, 18)),
        label: None,
        notes: vec![
            (Span::new(6, 12), "second boundary".into()),
            (Span::new(0, 5), "first boundary".into()),
        ],
        help: None,
        suggestion: None,
        code: None,
        level: Level::Error,
    };

    let output = render(source, "flow.zd", &diagnostic);

    assert!(output.contains("1. second boundary"), "{output}");
    assert!(output.contains("2. first boundary"), "{output}");
}

#[test]
fn file_errors_do_not_echo_control_sequences_from_paths_or_messages() {
    let diagnostic = Diagnostic::file_error("bad\u{1b}[2Jmessage");
    let output = render("", "bad\u{1b}]0;title\u{7}.zd", &diagnostic);

    assert!(!output.contains("\u{1b}[2J"), "{output:?}");
    assert!(!output.contains("\u{1b}]0;"), "{output:?}");
    assert!(output.contains("bad?[2Jmessage"));
    assert!(output.contains("bad?]0;title?.zd"));
}
