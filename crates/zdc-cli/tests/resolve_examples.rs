//! Every example that parses must also resolve, and there is no longer an
//! exception.
//!
//! `blog.zd` was excluded because it used syntax that was designed but not
//! implemented. `static` landed, and its `readMarkdown "content/blog"` —
//! a call with a bare argument, which has no production in §4.4 — was
//! respelled by the spec into the `build` capability form. `EXPECTED` is
//! therefore every `.zd` file in `examples/`, so a new example is a test
//! failure until it is named here.
//!
//! Resolution is against the prelude, exactly as `zdc check` does it
//! (§17.4.1): an example that calls `atOr` resolves only if the library
//! is beneath it, and testing without one would be testing a pipeline
//! nothing runs.
//!
//! Keeping the rest under test stops the examples rotting as the compiler
//! grows: resolution is the first pass that checks names, and adding it
//! found two examples whose pipelines read a signal nobody declared.
const EXCLUDED: &[&str] = &[];

/// The examples that must resolve, named so that deleting or renaming one
/// is a test failure rather than a silently smaller run.
const EXPECTED: &[&str] = &[
    "blog.zd",
    // The typed numeric field (#45). What it demonstrates is the *type*:
    // it binds an `Option`, because a box with nothing usable in it has
    // no number in it.
    "booking.zd",
    "components.zd",
    "content.zd",
    "counter.zd",
    "disclosure.zd",
    "dungeon.zd",
    // Levenshtein over a flat table, because there is no two-dimensional
    // structure to fill (#195).
    "edit-distance.zd",
    "events.zd",
    // The DOM-owning FFI (§14E.1): a foreign written as a view element.
    "gauge.zd",
    // The six algorithm examples. Their answers are pinned by running
    // them, in `zdc-codegen/tests/algorithms.rs`; what is asserted here
    // is only that they resolve against the prelude.
    "graph-traversal.zd",
    "guestbook.zd",
    "hello.zd",
    "knapsack.zd",
    // The two components `blog.zd` composes its pages out of. A module is
    // a unit of naming rather than of deployment (§14D.2).
    "layout.zd",
    "leaderboard.zd",
    "model.zd",
    "page.zd",
    "poker.zd",
    "queens.zd",
    "shortest-path.zd",
    "site.zd",
    "sorting.zd",
    // The only example that stores something other than a number. It was
    // added because there was none: `JSON.stringify(new Map(...))` is
    // `{}`, so every `durable Map` silently stored an empty object, and
    // with no example exercising that path nothing ever noticed.
    "tally.zd",
    "terminal-help.zd",
    "todo.zd",
    "voting-board.zd",
    "writing.zd",
];

fn examples() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Resolution goes through the module loader rather than through `parse`
/// alone.
///
/// A file with a `use` line is not a whole program on its own (§14D.2), and
/// the loader is what turns the entry file and everything it reaches into
/// one. Files with no imports take the same route and link to themselves,
/// so there is one path rather than two that could disagree.
#[test]
fn every_parseable_example_also_resolves() {
    let dir = examples();
    let mut resolved = Vec::new();

    for entry in std::fs::read_dir(&dir).expect("examples directory") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("zd") {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("utf-8 file name")
            .to_string();
        if EXCLUDED.contains(&name.as_str()) {
            continue;
        }

        let linked = zdc_resolve::load(&path).unwrap_or_else(|failure| {
            panic!("{name} failed to load: {}", failure.errors[0].message)
        });

        let prelude = zdc_lib::load();
        match zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked).resolve() {
            Ok(_) => resolved.push(name),
            Err(errors) => panic!(
                "{name} failed to resolve, {} error(s), the first being: {}",
                errors.len(),
                errors[0].message
            ),
        }
    }

    resolved.sort();
    assert_eq!(resolved, EXPECTED);
}

/// Nothing is excluded, and that is asserted rather than assumed.
///
/// This test used to check that the excluded example was still beyond the
/// grammar, so that an exclusion could not outlive its cause. It is kept,
/// inverted: the list is empty, and any name added back to it must first
/// fail to parse.
#[test]
fn no_example_is_excluded_that_the_grammar_can_reach() {
    for name in EXCLUDED {
        let src = std::fs::read_to_string(examples().join(name)).expect("read");
        assert!(
            zdc_parser::parse(&src).is_err(),
            "{name} now parses, so it no longer needs excluding"
        );
    }
}

/// `components.zd` is the acceptance criterion for §14D, so it is named
/// here rather than only counted among the rest: it must typecheck, not
/// merely resolve.
///
/// The split runs first, in §17.1.2's order, because the type of a
/// cross-placement read depends on the crossing. Asserting it produced no
/// error is part of the criterion: a component whose state or arguments the
/// placement pass rejected has not been accepted.
#[test]
fn the_components_example_typechecks() {
    let path = examples().join("components.zd");
    let linked = zdc_resolve::load(&path).expect("components.zd links");
    let hir = zdc_resolve::Resolver::linked(&linked)
        .resolve()
        .unwrap_or_else(|errors| panic!("components.zd failed to resolve: {}", errors[0].message));
    let split = zdc_graph::split(&hir);
    if let Some(error) = split.errors().next() {
        panic!("components.zd was rejected by the split: {}", error.message);
    }
    if let Err(errors) = zdc_types::check(&hir, &split) {
        panic!("components.zd failed to typecheck: {}", errors[0].message);
    }
}
