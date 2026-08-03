use zdc_types::{code_choice, error_fields, FailureCode, Type, ERROR_CODE_FIELD};

#[test]
fn the_surface_code_choice_is_derived_from_the_runtime_failure_set() {
    let choice = code_choice();
    let expected: Vec<&str> = FailureCode::CLOSED_SET
        .iter()
        .map(|code| code.spelling())
        .collect();
    let actual: Vec<&str> = choice
        .variants
        .iter()
        .map(|variant| variant.name.as_str())
        .collect();

    assert_eq!(choice.described, Type::Code.to_string());
    assert_eq!(actual, expected);
    assert!(choice
        .variants
        .iter()
        .all(|variant| variant.fields.is_empty() && variant.field_names.is_empty()));
}

#[test]
fn every_failure_code_has_one_distinct_position_spelling_and_explanation() {
    let mut spellings = Vec::new();
    let mut explanations = Vec::new();

    for code in FailureCode::CLOSED_SET {
        assert_eq!(FailureCode::CLOSED_SET[code.position()], code);
        assert_eq!(FailureCode::from_spelling(code.spelling()), Some(code));
        assert!(!code.observed().is_empty());
        spellings.push(code.spelling());
        explanations.push(code.observed());
    }

    spellings.sort_unstable();
    spellings.dedup();
    explanations.sort_unstable();
    explanations.dedup();
    assert_eq!(spellings.len(), FailureCode::CLOSED_SET.len());
    assert_eq!(explanations.len(), FailureCode::CLOSED_SET.len());
}

#[test]
fn error_code_is_the_closed_choice_while_error_message_is_text() {
    assert_eq!(ERROR_CODE_FIELD, "code");
    assert_eq!(
        error_fields(),
        [("message", Type::Text), (ERROR_CODE_FIELD, Type::Code)]
    );
}

#[test]
fn failure_code_spellings_are_exact_and_case_sensitive() {
    for invalid in ["", "timeout", "TIMEOUT", "Timout", "Malformed", "Rejected "] {
        assert_eq!(
            FailureCode::from_spelling(invalid),
            None,
            "{invalid:?} unexpectedly names a failure code"
        );
    }
}

#[test]
fn the_choice_diagnostic_lists_exactly_the_closed_set() {
    assert_eq!(
        code_choice().variant_names(),
        "`Unreachable`, `Timeout`, and `Rejected`"
    );
}
