//! The checked-in example programs, held to what they actually are.
//!
//! Four typecheck. Two do not, and each of those is pinned to the errors
//! it has, so a change that silently starts accepting one of them fails
//! here rather than later.

fn errors(src: &str) -> Vec<String> {
    let program = zdc_parser::parse(src).expect("the example must parse");
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("the example must resolve");
    let split = zdc_graph::split(&hir);
    match zdc_types::check(&hir, &split) {
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

/// `voting-board.zd` is §4.3's complete example: three placements, a
/// pipeline, a `when` over `Remote of List of Item`, and a durable write
/// from a click. It typechecks.
///
/// It did not until `Map of Id to Int` became `Map of Id to Whole`. `Int`
/// is not a ZDeceptron type — §5.4 calls the whole number `Whole` — and
/// the checker found it in the spec's own reference program.
#[test]
fn voting_board_typechecks() {
    let found = errors(include_str!("../../../examples/voting-board.zd"));
    assert!(found.is_empty(), "{found:?}");
}

/// `leaderboard.zd` reads a map through `at` and compares the result
/// without eliminating the `Option` §5.4 says indexing returns, then maps
/// its pipeline to names and reads a field off one.
///
/// The first is the language gap §14F names: `when` is a statement, so an
/// `Option` cannot be unwrapped inside an expression. The file documents
/// it in its own header.
#[test]
fn leaderboard_does_not_typecheck_and_the_reasons_are_real() {
    let found = errors(include_str!("../../../examples/leaderboard.zd"));
    assert!(
        found.iter().any(|m| m.contains("Option of Whole")),
        "the un-eliminated Option must be reported: {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|m| m.contains("`Text` has no fields") && m.contains("name")),
        "reading a field off a name must be reported: {found:?}"
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
