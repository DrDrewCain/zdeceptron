//! Every example that parses must also resolve.
//!
//! Excluded, deliberately, because they use syntax that is designed but
//! not implemented — both files say so at the top of themselves:
//!   - `components.zd`: `component`, `use`, `children` (spec §14D)
//!   - `blog.zd`: `static`, `record`, `foreign` (spec §14C.3b, §14B.1, §14E)
//!
//! Resolution is against the prelude, exactly as `zdc check` does it
//! (§17.4.1): an example that calls `atOr` resolves only if the library
//! is beneath it, and testing without one would be testing a pipeline
//! nothing runs.
//!
//! Keeping the rest under test stops the examples rotting as the compiler
//! grows: resolution is the first pass that checks names, and adding it
//! found two examples whose pipelines read a signal nobody declared.
const EXCLUDED: &[&str] = &["components.zd", "blog.zd"];

/// The examples that must resolve, named so that deleting or renaming one
/// is a test failure rather than a silently smaller run.
const EXPECTED: &[&str] = &[
    "counter.zd",
    "guestbook.zd",
    "hello.zd",
    "leaderboard.zd",
    "todo.zd",
    "voting-board.zd",
];

#[test]
fn every_parseable_example_also_resolves() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
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

        let src = std::fs::read_to_string(&path).expect("read");
        let program = zdc_parser::parse(&src)
            .unwrap_or_else(|e| panic!("{name} failed to parse: {}", e.message));

        let prelude = zdc_lib::load();
        match zdc_resolve::Resolver::with_prelude(prelude.program(), &program).resolve() {
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

/// The excluded two are excluded for the reason stated, not because they
/// happen to resolve anyway. If either starts parsing, this test fails and
/// the exclusion list is revisited rather than quietly outliving its cause.
#[test]
fn the_excluded_examples_are_still_beyond_the_grammar() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");

    for name in EXCLUDED {
        let src = std::fs::read_to_string(dir.join(name)).expect("read");
        assert!(
            zdc_parser::parse(&src).is_err(),
            "{name} now parses, so it no longer needs excluding"
        );
    }
}
