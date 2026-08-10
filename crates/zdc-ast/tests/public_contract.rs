use std::collections::BTreeSet;

use zdc_ast::{
    is_javascript_identifier, CallForm, ExportName, ForeignDecl, ForeignGrant, ForeignResult,
    ForeignSite, ForeignSource, Ident, Placement, TypeExpr,
};
use zdc_lexer::Span;

fn ident(text: &str) -> Ident {
    Ident {
        text: text.into(),
        span: Span::new(0, u32::try_from(text.len()).unwrap()),
    }
}

fn foreign(result: ForeignResult) -> ForeignDecl {
    ForeignDecl {
        name: ident("render"),
        site: ForeignSite::Anywhere,
        site_span: Span::new(18, 26),
        source: ForeignSource::Import {
            module: "./render.js".into(),
            module_span: Span::new(32, 45),
        },
        export: ExportName::parse("render").unwrap(),
        export_span: Span::new(49, 55),
        form: CallForm::With,
        params: Vec::new(),
        result_grant: ForeignGrant::Opaque,
        result,
        result_span: Span::new(63, 67),
        span: Span::new(0, 67),
    }
}

#[test]
fn placements_have_one_stable_word_and_index_each() {
    let expected = [
        (Placement::Client, 0, "client"),
        (Placement::Static, 1, "static"),
        (Placement::Server, 2, "server"),
        (Placement::Durable, 3, "durable"),
        (Placement::Remembered, 4, "remembered"),
    ];

    assert_eq!(Placement::ALL.len(), expected.len());
    for (position, (placement, index, word)) in expected.into_iter().enumerate() {
        assert_eq!(Placement::ALL[position], placement);
        assert_eq!(placement.index(), index);
        assert_eq!(placement.word(), word);
    }
}

#[test]
fn placement_words_and_indices_are_unique() {
    let words: BTreeSet<_> = Placement::ALL.iter().map(|site| site.word()).collect();
    let indices: BTreeSet<_> = Placement::ALL.iter().map(|site| site.index()).collect();

    assert_eq!(words.len(), Placement::ALL.len());
    assert_eq!(indices, BTreeSet::from([0, 1, 2, 3, 4]));
}

#[test]
fn javascript_identifier_validation_accepts_the_conservative_ascii_grammar() {
    for valid in [
        "a",
        "Z",
        "_private",
        "$runtime",
        "camelCase2",
        "snake_case",
        "class",
    ] {
        assert!(is_javascript_identifier(valid), "rejected `{valid}`");
    }
}

#[test]
fn javascript_identifier_validation_rejects_syntax_and_unicode() {
    for invalid in [
        "",
        "2fast",
        "has-dash",
        "has space",
        "dotted.name",
        "naïve",
        "λ",
        "name;alert(1)",
        "line\nbreak",
    ] {
        assert!(!is_javascript_identifier(invalid), "accepted `{invalid:?}`");
    }
}

#[test]
fn export_names_can_only_be_constructed_from_valid_identifiers() {
    for candidate in ["read", "_read2", "$read", "bad-name", "", "é"] {
        let export = ExportName::parse(candidate);
        assert_eq!(
            export.is_some(),
            is_javascript_identifier(candidate),
            "constructor and validator disagree for `{candidate}`"
        );
        if let Some(export) = export {
            assert_eq!(export.as_str(), candidate);
            assert_eq!(export.to_string(), candidate);
        }
    }
}

#[test]
fn foreign_sites_have_exact_diagnostic_spellings() {
    assert_eq!(ForeignSite::Client.describe(), "client");
    assert_eq!(ForeignSite::Server.describe(), "server");
    assert_eq!(ForeignSite::Anywhere.describe(), "anywhere");
}

#[test]
fn foreign_result_grants_default_to_opaque_and_describe_only_markers() {
    assert_eq!(ForeignGrant::default(), ForeignGrant::Opaque);
    assert_eq!(ForeignGrant::Opaque.describe(), None);
    assert_eq!(ForeignGrant::Pure.describe(), Some("pure"));
    assert_eq!(ForeignGrant::Trusted.describe(), Some("trusted"));
}

#[test]
fn only_view_returning_foreigns_own_a_dom_node() {
    assert!(foreign(ForeignResult::View).owns_view());
    assert!(!foreign(ForeignResult::Value(TypeExpr::Named(ident("Text")))).owns_view());

    // The three answers to "what does this hand back", and the three to
    // "where does the symbol live", are read off the declaration and
    // nowhere else.
    let handle = TypeExpr::Named(ident("Handle"));
    assert!(foreign(ForeignResult::New(handle.clone())).constructs());
    assert!(!foreign(ForeignResult::View).constructs());

    let imported = foreign(ForeignResult::New(handle.clone()));
    assert_eq!(imported.module(), Some("./render.js"));
    assert!(!imported.is_method());
    assert!(!imported.is_property());

    let method = ForeignDecl {
        source: ForeignSource::Receiver {
            span: Span::new(32, 41),
        },
        ..foreign(ForeignResult::Value(TypeExpr::Named(ident("Whole"))))
    };
    assert_eq!(method.module(), None);
    assert!(method.is_method());
    assert!(!method.is_property());

    // A property is the minimal pair with a method: it imports nothing for
    // the same reason, and it is the *other* one.
    let property = ForeignDecl {
        source: ForeignSource::Property {
            span: Span::new(32, 41),
        },
        ..foreign(ForeignResult::Value(TypeExpr::Named(ident("Whole"))))
    };
    assert_eq!(property.module(), None);
    assert!(property.is_property());
    assert!(!property.is_method());
}
