use std::collections::BTreeSet;

use zdc_ast::Decl;
use zdc_lib::{load, SOURCES};

fn declared_name(decl: &Decl) -> Option<&str> {
    match decl {
        Decl::Function(decl) => Some(&decl.name.text),
        Decl::Foreign(decl) => Some(&decl.name.text),
        Decl::Record(decl) => Some(&decl.name.text),
        Decl::Choice(decl) => Some(&decl.name.text),
        Decl::State(_)
        | Decl::View(_)
        | Decl::Route(_)
        | Decl::Component(_)
        | Decl::Release(_)
        | Decl::Use(_) => None,
    }
}

#[test]
fn every_embedded_source_has_a_unique_stable_prelude_path() {
    let paths: Vec<_> = SOURCES.iter().map(|(path, _)| *path).collect();
    let unique: BTreeSet<_> = paths.iter().copied().collect();

    assert_eq!(unique.len(), paths.len(), "duplicate prelude source path");
    assert!(
        paths
            .iter()
            .all(|path| path.starts_with("prelude/") && path.ends_with(".zd")),
        "unexpected source paths: {paths:?}"
    );
}

#[test]
fn every_embedded_source_parses_as_its_own_compilation_unit() {
    for (path, source) in SOURCES {
        assert!(!source.trim().is_empty(), "{path} is empty");
        zdc_parser::parse(source)
            .unwrap_or_else(|error| panic!("{path} failed at {:?}: {}", error.span, error.message));
    }
}

#[test]
fn loading_combines_every_source_without_losing_a_declaration() {
    let separately_parsed = SOURCES
        .iter()
        .map(|(_, source)| zdc_parser::parse(source).unwrap().decls.len())
        .sum::<usize>();

    assert_eq!(load().program().decls.len(), separately_parsed);
}

#[test]
fn public_names_are_sorted_unique_and_match_the_loaded_program() {
    let names = load().names();
    let expected: BTreeSet<_> = load()
        .program()
        .decls
        .iter()
        .filter_map(declared_name)
        .collect();

    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(names, expected.into_iter().collect::<Vec<_>>());
}

#[test]
fn repeated_loads_return_the_same_parsed_prelude() {
    assert!(std::ptr::eq(load(), load()));
    assert!(std::ptr::eq(load().program(), load().program()));
}

#[test]
fn the_loaded_prelude_contains_only_library_declarations() {
    for decl in &load().program().decls {
        assert!(
            matches!(
                decl,
                Decl::Function(_) | Decl::Foreign(_) | Decl::Record(_) | Decl::Choice(_)
            ),
            "non-library declaration reached the public prelude"
        );
    }
}
