use std::collections::BTreeSet;

use zdc_hir::{
    destination_as_href, destination_of, is_event_attribute, is_url_attribute, url_is_safe,
    url_scheme, BuildCapability, Builtin, BuiltinElement, BuiltinVariant, Hir, HirArg, HirElement,
    HirExpr, HirExprKind, OperatorName, Res, BUILTIN_OF_OPERATORS, DESTINATION_ARGUMENT,
    DESTINATION_ELEMENT, URL_ATTRIBUTES, URL_SCHEMES,
};
use zdc_lexer::Span;

#[test]
fn builtin_element_names_are_unique_and_round_trip() {
    let names: BTreeSet<_> = BuiltinElement::ALL
        .iter()
        .map(|element| element.name())
        .collect();

    assert_eq!(names.len(), BuiltinElement::ALL.len());
    assert_eq!(BuiltinElement::NAMES.len(), BuiltinElement::ALL.len());
    for element in BuiltinElement::ALL {
        assert_eq!(BuiltinElement::from_name(element.name()), Some(element));
    }
}

/// Written out rather than derived, so that adding an element and marking
/// it two-way by reflex fails here as well as in the `match`. A two-way
/// binding is the one place a keystroke writes a signal, and §14B.5 rules
/// on which signals it may write, so the list is worth pinning.
#[test]
fn exactly_the_input_elements_are_two_way() {
    let two_way: Vec<_> = BuiltinElement::ALL
        .iter()
        .copied()
        .filter(|element| element.is_two_way())
        .map(BuiltinElement::name)
        .collect();

    assert_eq!(
        two_way,
        [
            "Input",
            "TextArea",
            "PasswordInput",
            "Slider",
            "Select",
            "Radio",
            "Checkbox"
        ]
    );
}

#[test]
fn builtin_variants_round_trip_with_their_payload_field_names() {
    let expected = [
        (BuiltinVariant::Loading, "Loading", &[][..]),
        (BuiltinVariant::Ready, "Ready", &["value"][..]),
        (BuiltinVariant::Failed, "Failed", &["error"][..]),
        (BuiltinVariant::Some, "Some", &["value"][..]),
        (BuiltinVariant::None, "None", &[][..]),
        (BuiltinVariant::Unreachable, "Unreachable", &[][..]),
        (BuiltinVariant::Timeout, "Timeout", &[][..]),
        (BuiltinVariant::Rejected, "Rejected", &[][..]),
    ];

    assert_eq!(BuiltinVariant::ALL.len(), expected.len());
    for (variant, name, fields) in expected {
        assert_eq!(variant.name(), name);
        assert_eq!(variant.field_names(), fields);
        assert_eq!(BuiltinVariant::from_name(name), Some(variant));
    }
    assert_eq!(BuiltinVariant::from_name("Unknown"), None);
}

#[test]
fn unary_operator_names_and_descriptions_form_a_closed_set() {
    let expected = [
        (OperatorName::Length, "length", "length of"),
        (OperatorName::TextOf, "text", "text of"),
    ];

    assert_eq!(BUILTIN_OF_OPERATORS.len(), expected.len());
    for ((operator, name, description), listed) in expected.into_iter().zip(BUILTIN_OF_OPERATORS) {
        assert_eq!(name, *listed);
        assert_eq!(OperatorName::from_name(name), Some(operator));
        assert_eq!(operator.describe(), description);
    }
    assert_eq!(OperatorName::from_name("size"), None);
}

#[test]
fn build_capabilities_have_unique_names_and_actionable_descriptions() {
    let names: BTreeSet<_> = BuildCapability::ALL
        .iter()
        .map(|capability| capability.name())
        .collect();

    assert_eq!(names.len(), BuildCapability::ALL.len());
    for capability in BuildCapability::ALL {
        assert_eq!(
            BuildCapability::from_name(capability.name()),
            Some(capability)
        );
        assert!(!capability.describe().trim().is_empty());
    }
    assert_eq!(BuildCapability::from_name("fetch"), None);
}

#[test]
fn url_attribute_and_scheme_tables_are_sorted_unique_allowlists() {
    let attributes: BTreeSet<_> = URL_ATTRIBUTES.iter().copied().collect();
    let schemes: BTreeSet<_> = URL_SCHEMES.iter().copied().collect();

    assert_eq!(attributes.len(), URL_ATTRIBUTES.len());
    assert_eq!(schemes.len(), URL_SCHEMES.len());
    assert!(URL_ATTRIBUTES.windows(2).all(|pair| pair[0] < pair[1]));
    for attribute in URL_ATTRIBUTES {
        assert!(is_url_attribute(attribute));
    }
}

#[test]
fn url_scheme_detection_distinguishes_protocols_from_path_colons() {
    assert_eq!(url_scheme("https://example.com"), Some("https"));
    assert_eq!(url_scheme("  MAILTO:user@example.com"), Some("MAILTO"));
    assert_eq!(url_scheme("/folder:a/file"), None);
    assert_eq!(url_scheme("?redirect=a:b"), None);
    assert_eq!(url_scheme("#section:a"), None);
    assert_eq!(url_scheme("relative/path"), None);
}

#[test]
fn safe_urls_are_relative_or_use_only_the_closed_scheme_allowlist() {
    for safe in [
        "/local/path",
        "relative.html",
        "https://example.com",
        "HTTP://example.com",
        "mailto:user@example.com",
        "tel:+15551234567",
    ] {
        assert!(url_is_safe(safe), "rejected `{safe}`");
    }
    for unsafe_url in [
        "javascript:alert(1)",
        " data:text/html,boom",
        "VBSCRIPT:msgbox(1)",
        "file:///etc/passwd",
        "ftp://example.com/file",
    ] {
        assert!(!url_is_safe(unsafe_url), "accepted `{unsafe_url}`");
    }
}

#[test]
fn inline_event_attributes_require_more_than_the_on_prefix() {
    for event in ["onclick", "onLoad", "ONERROR", "once", "only"] {
        assert!(is_event_attribute(event), "missed `{event}`");
    }
    for ordinary in ["on", "o", "data-onclick", "aria-label"] {
        assert!(!is_event_attribute(ordinary), "misclassified `{ordinary}`");
    }
}

#[test]
fn link_destinations_are_rewritten_once_and_found_by_attribute_name() {
    let mut hir = Hir::new();
    let destination = hir.exprs.alloc(HirExpr {
        kind: HirExprKind::Text("/home".into()),
        span: Span::new(0, 7),
    });
    let extra = hir.exprs.alloc(HirExpr {
        kind: HirExprKind::Text("label".into()),
        span: Span::new(8, 15),
    });
    let args = destination_as_href(
        DESTINATION_ELEMENT,
        vec![HirArg::Positional(destination), HirArg::Positional(extra)],
    );
    let element = HirElement {
        name: DESTINATION_ELEMENT.into(),
        res: Res::Builtin(Builtin::Element(BuiltinElement::Link)),
        args,
        children: Vec::new(),
        span: Span::new(0, 15),
    };

    assert!(matches!(
        &element.args[0],
        HirArg::Named { name, value }
            if name == DESTINATION_ARGUMENT && *value == destination
    ));
    assert_eq!(element.args[1], HirArg::Positional(extra));
    assert_eq!(destination_of(&element), Some(destination));

    let ordinary = destination_as_href("Button", vec![HirArg::Positional(destination)]);
    assert_eq!(ordinary, [HirArg::Positional(destination)]);
}
