use std::collections::HashSet;

use zdc_types::{
    code_choice, error_fields, event_names, Choice, EventPayload, Type, Variant, ERROR_CODE_FIELD,
    EVENTS,
};

#[test]
fn event_names_preserve_the_public_table_order() {
    assert_eq!(
        event_names(),
        vec!["click", "input", "change", "submit", "keydown", "keyup", "focus", "blur",]
    );
    assert_eq!(
        event_names(),
        EVENTS.iter().map(|(name, _)| *name).collect::<Vec<_>>()
    );
}

#[test]
fn public_event_names_are_unique() {
    let names = event_names();
    let unique: HashSet<_> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len());
}

#[test]
fn event_lookup_is_closed_and_case_sensitive() {
    // Sized, so that an emptied table cannot satisfy "every event resolves"
    // while leaving the rejection cases below to carry the whole test.
    assert_eq!(EVENTS.len(), 8);
    for (name, payload) in EVENTS {
        assert_eq!(zdc_types::payload_of(name), Some(*payload));
    }
    for unknown in ["", "Click", "keypress", "gamepadconnected", " click"] {
        assert_eq!(zdc_types::payload_of(unknown), None, "accepted `{unknown}`");
    }
}

#[test]
fn every_payload_has_a_stable_diagnostic_name() {
    assert_eq!(EventPayload::Pointer.describe(), "PointerEvent");
    assert_eq!(EventPayload::Edit.describe(), "EditEvent");
    assert_eq!(EventPayload::Key.describe(), "KeyEvent");
    assert_eq!(EventPayload::Focus.describe(), "FocusEvent");
    assert_eq!(EventPayload::Submit.describe(), "SubmitEvent");
}

#[test]
fn event_field_tables_are_ordered_unique_and_self_consistent() {
    for payload in [
        EventPayload::Pointer,
        EventPayload::Edit,
        EventPayload::Key,
        EventPayload::Focus,
        EventPayload::Submit,
    ] {
        let fields = payload.fields();
        let unique: HashSet<_> = fields.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            unique.len(),
            fields.len(),
            "duplicate in {}",
            payload.describe()
        );
        for (name, ty) in fields {
            assert_eq!(payload.field(name), Some(ty.clone()));
        }
        assert_eq!(payload.field("not-a-field"), None);
    }
}

#[test]
fn modifier_payloads_share_the_same_truth_fields() {
    let pointer_modifiers = &EventPayload::Pointer.fields()[2..];
    let key_modifiers = &EventPayload::Key.fields()[1..];
    assert_eq!(pointer_modifiers, key_modifiers);
    assert!(pointer_modifiers.iter().all(|(_, ty)| *ty == Type::Truth));
}

#[test]
fn code_choice_tracks_the_closed_failure_code_set() {
    let choice = code_choice();
    assert_eq!(choice.described, "Code");
    assert_eq!(
        choice
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        ["Unreachable", "Timeout", "Rejected"]
    );
    assert!(choice
        .variants
        .iter()
        .all(|variant| variant.fields.is_empty() && variant.field_names.is_empty()));
}

#[test]
fn choice_lookup_and_name_formatting_cover_empty_single_and_many() {
    let variant = |name: &str| Variant {
        name: name.to_string(),
        field_names: Vec::new(),
        fields: Vec::new(),
    };
    let choice = |variants| Choice {
        described: "Example".to_string(),
        variants,
    };

    assert_eq!(choice(Vec::new()).variant_names(), "");
    assert_eq!(choice(vec![variant("Only")]).variant_names(), "`Only`");

    let many = choice(vec![variant("First"), variant("Second"), variant("Third")]);
    assert_eq!(many.variant_names(), "`First`, `Second`, and `Third`");
    assert_eq!(
        many.variant("Second").map(|item| item.name.as_str()),
        Some("Second")
    );
    assert!(many.variant("second").is_none());
}

#[test]
fn error_fields_have_the_stable_public_order_and_types() {
    assert_eq!(ERROR_CODE_FIELD, "code");
    assert_eq!(
        error_fields(),
        [("message", Type::Text), (ERROR_CODE_FIELD, Type::Code)]
    );
}
