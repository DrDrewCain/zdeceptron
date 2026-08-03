//! Every example that parses must also resolve.
//!
//! Excluded, deliberately, because it uses syntax that is designed but not
//! implemented — the file says so at the top of itself:
//!   - `blog.zd`: `static`, `foreign` (spec §14C.3b, §14E)
//!
//! Resolution is against the prelude, exactly as `zdc check` does it
//! (§17.4.1): an example that calls `atOr` resolves only if the library
//! is beneath it, and testing without one would be testing a pipeline
//! nothing runs.
//!
//! Keeping the rest under test stops the examples rotting as the compiler
//! grows: resolution is the first pass that checks names, and adding it
//! found two examples whose pipelines read a signal nobody declared.
const EXCLUDED: &[&str] = &["blog.zd"];

/// The examples that must resolve, named so that deleting or renaming one
/// is a test failure rather than a silently smaller run.
const EXPECTED: &[&str] = &[
    "components.zd",
    "counter.zd",
    "disclosure.zd",
    "guestbook.zd",
    "hello.zd",
    "leaderboard.zd",
    "model.zd",
    // The only example that stores something other than a number. It was
    // added because there was none: `JSON.stringify(new Map(...))` is
    // `{}`, so every `durable Map` silently stored an empty object, and
    // with no example exercising that path nothing ever noticed.
    "tally.zd",
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

        let linked = zdc_resolve::load(&path)
            .unwrap_or_else(|errors| panic!("{name} failed to load: {}", errors[0].message));

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

/// The excluded one is excluded for the reason stated, not because it
/// happens to resolve anyway. If it starts parsing, this test fails and the
/// exclusion list is revisited rather than quietly outliving its cause.
#[test]
fn the_excluded_example_is_still_beyond_the_grammar() {
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
