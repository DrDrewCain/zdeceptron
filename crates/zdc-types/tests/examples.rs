//! The checked-in example programs, held to what they actually are.
//!
//! Three typecheck. Three do not, and each of those is pinned to the
//! errors it has, so a change that silently starts accepting one of them
//! fails here rather than later.

fn errors(src: &str) -> Vec<String> {
    let program = zdc_parser::parse(src).expect("the example must parse");
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("the example must resolve");
    match zdc_types::check(&hir) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|error| error.message).collect(),
    }
}

#[test]
fn hello_typechecks() {
    assert!(
        errors(include_str!("../../../examples/hello.zd")).is_empty(),
        "{:?}",
        errors(include_str!("../../../examples/hello.zd"))
    );
}

#[test]
fn counter_typechecks() {
    let found = errors(include_str!("../../../examples/counter.zd"));
    assert!(found.is_empty(), "{found:?}");
}

/// Three placements, a cross-boundary read eliminated by a `when`, and a
/// durable write from a click. The whole point of the language, and it
/// typechecks end to end.
#[test]
fn guestbook_typechecks() {
    let found = errors(include_str!("../../../examples/guestbook.zd"));
    assert!(found.is_empty(), "{found:?}");
}

/// `voting-board.zd` is §4.3's complete example. Its client half — the
/// `Text` and `Truth` signals, the `Input`, and the `when` over
/// `Remote of List of Item` — is clean. Its one error is real: `Int` is
/// not a ZDeceptron type, and §5.4 calls the whole number `Whole`.
#[test]
fn voting_boards_only_error_is_the_undefined_number_type() {
    let found = errors(include_str!("../../../examples/voting-board.zd"));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`Int`"), "{found:?}");
    assert!(found[0].contains("add"), "{found:?}");
}

/// `leaderboard.zd` reads a map through `at` and compares the result
/// without eliminating the `Option` §5.4 says indexing returns, shows a
/// whole record as text, and keys a `Map of Text to …` with a record.
#[test]
fn leaderboard_does_not_typecheck_and_the_reasons_are_real() {
    let found = errors(include_str!("../../../examples/leaderboard.zd"));
    assert!(
        found.iter().any(|m| m.contains("Option of Whole")),
        "the un-eliminated Option must be reported: {found:?}"
    );
    assert!(
        found.iter().any(|m| m.contains("`Row` shows text")),
        "showing a whole record as text must be reported: {found:?}"
    );
    assert!(
        found.iter().any(|m| m.contains("map key")),
        "keying a `Map of Text` with a record must be reported: {found:?}"
    );
}

/// `todo.zd` says in its own header that `add` is overloaded between
/// numeric increment and list append, that `Checkbox` two-way binding is
/// unspecified, and that the nested place expression is unreadable. The
/// checker finds exactly those.
#[test]
fn todo_does_not_typecheck_and_reports_the_gaps_its_header_names() {
    let found = errors(include_str!("../../../examples/todo.zd"));
    assert!(
        found
            .iter()
            .filter(|m| m.contains("`append` and `remove`"))
            .count()
            >= 2,
        "`add draft to todos` and `subtract todo from todos`: {found:?}"
    );
    assert!(
        found.iter().any(|m| m.contains("`Checkbox` writes back")),
        "binding a checkbox to a field is not a signal: {found:?}"
    );
}
