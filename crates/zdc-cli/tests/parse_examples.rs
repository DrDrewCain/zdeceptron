use std::path::Path;

#[test]
fn voting_board_parses_to_a_stable_tree() {
    let src = include_str!("../../../examples/voting-board.zd");
    let program = zdc_parser::parse(src).expect("the reference example must parse");
    insta::assert_debug_snapshot!("voting_board", program);
}

#[test]
fn every_declaration_kind_is_present_in_the_example() {
    let src = include_str!("../../../examples/voting-board.zd");
    let program = zdc_parser::parse(src).expect("parses");
    assert_eq!(program.decls.len(), 7, "5 state, 1 function, 1 view");
}

/// Every example in `examples/` must parse, except the following, which are
/// deliberately ASPIRATIONAL and self-documented as exercising unimplemented
/// spec constructs:
///
/// - `components.zd` — `component`, `use`, and `children` (spec §14D).
/// - `blog.zd` — the `static` placement (§14C.3b), FFI (§14E), and `record`
///   declarations (§14B.1).
///
/// Keeping the rest under test here stops the examples rotting as the
/// grammar evolves; they have already caught two spec defects.
#[test]
fn every_example_except_components_parses() {
    const EXCLUDED: &[&str] = &["components.zd", "blog.zd"];

    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut checked = 0;

    for entry in std::fs::read_dir(&examples_dir).expect("examples/ must exist") {
        let entry = entry.expect("readable directory entry");
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("zd") {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("utf-8 file name")
            .to_string();

        if EXCLUDED.contains(&file_name.as_str()) {
            continue;
        }

        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {file_name}: {e}"));

        zdc_parser::parse(&src).unwrap_or_else(|e| panic!("{file_name} failed to parse: {e:?}"));

        checked += 1;
    }

    assert!(checked > 0, "expected at least one example to be checked");
}
