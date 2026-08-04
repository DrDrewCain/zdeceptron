//! The checked-in example programs, held to what they actually are.
//!
//! All six typecheck. `leaderboard.zd` was the sixth for a reason worth
//! keeping in view: it reads a map through `at`, §5.4 makes that an
//! `Option of Whole`, and until the prelude landed there was no way to
//! eliminate one inside an expression (§14F.2a). It compiles now because
//! `atOr` exists, not because the rule was relaxed.
//!
//! Checked against the prelude, exactly as `zdc check` does it (§17.4.1).

fn errors(src: &str) -> Vec<String> {
    let program = zdc_parser::parse(src).expect("the example must parse");
    let prelude = zdc_lib::load();
    let hir = zdc_resolve::Resolver::with_prelude(prelude.program(), &program)
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

/// The example §14F.2a named as un-writable. `table at player.name` is an
/// `Option of Whole` and a sort key is an expression, so this file could
/// not be written at all until `atOr` existed — which is an ordinary
/// ZDeceptron function over an ordinary `when`, not a grammar change.
#[test]
fn leaderboard_typechecks_now_that_an_option_can_be_eliminated() {
    let found = errors(include_str!("../../../examples/leaderboard.zd"));
    assert!(found.is_empty(), "{found:?}");
}

/// The soundness property that made it hard is still there: the `Option`
/// has to be eliminated, and using one as a number is still an error.
#[test]
fn reading_a_map_without_eliminating_the_option_is_still_an_error() {
    let found = errors(
        "state table is client Map of Text to Whole starting empty\n\
         state score is client Whole from table at \"a\"\n",
    );
    assert!(
        found.iter().any(|m| m.contains("Option of Whole")),
        "{found:?}"
    );
}
