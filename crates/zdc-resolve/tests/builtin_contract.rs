use std::collections::BTreeSet;

use zdc_ast::Decl;
use zdc_resolve::{builtin_patterns, Resolver};

#[test]
fn builtin_pattern_vocabulary_matches_the_hir_closed_set() {
    let expected: Vec<_> = zdc_hir::BuiltinVariant::ALL
        .iter()
        .map(|variant| variant.name())
        .collect();

    assert_eq!(builtin_patterns(), expected);
}

#[test]
fn builtin_pattern_names_are_unique() {
    let variants = builtin_patterns();
    let unique: BTreeSet<_> = variants.iter().copied().collect();

    assert_eq!(unique.len(), variants.len());
}

#[test]
fn builtin_pattern_membership_is_exact_and_case_sensitive() {
    let patterns = builtin_patterns();
    for ordinary in ["", "ready", "READY", "Somewhere", "Failure", "Unknown"] {
        assert!(!patterns.contains(&ordinary), "accepted `{ordinary}`");
    }
}

#[test]
fn programs_cannot_redeclare_any_builtin_variant() {
    for variant in builtin_patterns() {
        let source = format!("choice Mine\n    {variant}\n");
        let program = zdc_parser::parse(&source).expect("fixture parses");
        let errors = Resolver::new(&program)
            .resolve()
            .expect_err("builtin variant must be reserved");

        assert!(
            errors.iter().any(|error| {
                error.message.contains(variant) && error.message.contains("language provides")
            }),
            "{variant}: {errors:#?}"
        );
    }
}

#[test]
fn user_variant_positions_are_recorded_in_declaration_order() {
    let source = concat!(
        "choice Status\n",
        "    Draft\n",
        "    Published with date is Text\n",
        "    Archived\n",
    );
    let program = zdc_parser::parse(source).unwrap();
    let decls: Vec<&Decl> = program.decls.iter().collect();
    let table = zdc_resolve::collect(&decls, 0).unwrap();

    assert_eq!(table.variant("Draft"), Some((0, 0)));
    assert_eq!(table.variant("Published"), Some((0, 1)));
    assert_eq!(table.variant("Archived"), Some((0, 2)));
    assert!(table.declares_variant("Published"));
    assert!(!table.declares_variant("Missing"));
}

#[test]
fn single_module_visibility_is_exact_and_rejects_unknown_module_indices() {
    let source = concat!(
        "state count is client Whole starting 0\n",
        "function next with value\n",
        "    give value + 1\n",
    );
    let program = zdc_parser::parse(source).unwrap();
    let decls: Vec<&Decl> = program.decls.iter().collect();
    let table = zdc_resolve::collect(&decls, 0).unwrap();

    assert_eq!(table.lookup_in(0, "count"), Some(0));
    assert_eq!(table.lookup_in(0, "next"), Some(1));
    assert_eq!(table.lookup_in(0, "missing"), None);
    assert_eq!(table.lookup_in(99, "count"), None);
    assert!(!table.is_declared_elsewhere(0, "count"));
}
