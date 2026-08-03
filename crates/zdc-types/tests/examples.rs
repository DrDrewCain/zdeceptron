//! The checked-in example programs, held to what they actually are.
//!
//! Five typecheck. One does not, and it is pinned to the errors it has, so
//! a change that silently starts accepting it fails here rather than later.

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
    let found = errors(include_str!("../../../examples/hello.zd"));
    assert!(found.is_empty(), "{found:?}");
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

/// `voting-board.zd` is §4.3's complete example: every construct in the
/// language, including a `when` over `Remote of List of Item` and a durable
/// write from a click.
#[test]
fn voting_board_typechecks() {
    let found = errors(include_str!("../../../examples/voting-board.zd"));
    assert!(found.is_empty(), "{found:?}");
}

/// `todo.zd` is the acceptance test for §14B.1's type declarations,
/// §14B.2's membership verbs and §14B.4's literals: a `record`, a `choice`,
/// a list literal of record literals, `append`, `remove`, and a `when` over
/// a user-declared choice, all in one file.
#[test]
fn todo_typechecks() {
    let found = errors(include_str!("../../../examples/todo.zd"));
    assert!(found.is_empty(), "{found:?}");
}

/// `leaderboard.zd` reads a map through `at` and compares the result
/// without eliminating the `Option` §5.4 says indexing returns, and keys a
/// `Map of Text to …` with a whole `Player`. Both are the gap its own
/// header comment documents: `Option` can only be eliminated by `when`,
/// which is a statement, so there is no way to unwrap one inside a sort key
/// (spec §14F).
#[test]
fn leaderboard_does_not_typecheck_and_the_reasons_are_real() {
    let found = errors(include_str!("../../../examples/leaderboard.zd"));
    assert!(
        found.iter().any(|m| m.contains("Option of Whole")),
        "the un-eliminated Option must be reported: {found:?}"
    );
    assert!(
        found.iter().any(|m| m.contains("no `name` to read")),
        "reading a field off a `Text` must be reported: {found:?}"
    );
}
