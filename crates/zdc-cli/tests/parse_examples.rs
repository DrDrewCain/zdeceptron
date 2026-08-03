use std::path::Path;
use zdc_ast::{Arg, Decl, Mutation, Node, NodeArmBody, PipelineClause, Stmt};

#[test]
fn voting_board_exercises_the_front_end() {
    let src = include_str!("../../../examples/voting-board.zd");
    let program = zdc_parser::parse(src).expect("the reference example must parse");
    assert_eq!(program.decls.len(), 8, "6 state, 1 function, 1 view");

    let state_names: Vec<&str> = program.decls[..6]
        .iter()
        .map(|decl| match decl {
            Decl::State(state) => state.name.text.as_str(),
            other => panic!("expected a state declaration, got {other:?}"),
        })
        .collect();
    assert_eq!(
        state_names,
        ["apiKey", "votes", "items", "ranked", "query", "open"]
    );

    let Decl::Function(rank) = &program.decls[6] else {
        panic!("expected the rank function")
    };
    assert_eq!(rank.name.text, "rank");
    assert!(matches!(
        rank.body.stmts[0],
        Stmt::Pipeline(PipelineClause::From(_))
    ));
    assert!(matches!(
        rank.body.stmts[1],
        Stmt::Pipeline(PipelineClause::Keep { .. })
    ));
    assert!(matches!(
        rank.body.stmts[2],
        Stmt::Pipeline(PipelineClause::Sort { .. })
    ));
    assert!(matches!(
        rank.body.stmts[3],
        Stmt::Pipeline(PipelineClause::TakeFirst(_))
    ));

    let Decl::View(view) = &program.decls[7] else {
        panic!("expected the view")
    };
    let Node::Element(column) = &view.nodes[0] else {
        panic!("expected a root Column")
    };
    assert_eq!(column.name.text, "Column");

    let Node::Element(input) = &column.children[0] else {
        panic!("expected an Input")
    };
    assert!(matches!(
        input.args.as_slice(),
        [Arg::Positional(_), Arg::Named { .. }]
    ));

    let Node::When(ranked) = &column.children[1] else {
        panic!("expected a when node")
    };
    assert_eq!(ranked.arms.len(), 3);
    assert!(matches!(ranked.arms[0].body, NodeArmBody::Show(_)));
    assert!(matches!(ranked.arms[1].body, NodeArmBody::Show(_)));

    let NodeArmBody::Nodes(ready_nodes) = &ranked.arms[2].body else {
        panic!("expected the Ready arm to have a node block")
    };
    let Node::Each(items) = &ready_nodes[0] else {
        panic!("expected the Ready arm to iterate over items")
    };
    let Node::Element(row) = &items.body[0] else {
        panic!("expected an item Row")
    };
    let Node::Handler(click) = &row.children[0] else {
        panic!("expected a click handler")
    };
    assert!(matches!(
        click.body.stmts.as_slice(),
        [Stmt::Mutation(Mutation::Add { .. })]
    ));
}

/// **Every** example in `examples/` must parse, with no exclusions.
///
/// There were two, both self-documented as aspirational: `components.zd`
/// for §14D, and `blog.zd` for the `static` placement and the FFI. §14D
/// landed, and `blog.zd`'s remaining aspiration — a call with a bare
/// argument, `readMarkdown "content/blog"` — was respelled by the spec on
/// 2026-08-03 into the `build` capability form the compiler implements.
/// Neither exclusion has a cause left, so neither is kept: an exclusion
/// that outlives its reason is how an example rots unobserved.
///
/// Keeping them under test stops the examples rotting as the grammar
/// evolves; they have already caught two spec defects.
#[test]
fn every_example_parses() {
    const EXCLUDED: &[&str] = &[];

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
