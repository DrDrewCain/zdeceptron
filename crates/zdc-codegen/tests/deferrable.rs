//! Which definitions sit behind a `view`-position `if`, and nowhere else.
//!
//! `~/zdc-portfolio` ships 33 route bundles of about 543 KB, and the same
//! site with the terminal removed from its shell builds to 102 KB — so 81%
//! of every bundle is code behind one `if` that almost no page render ever
//! takes (#401). Every byte of it is genuinely *reachable*, which is why
//! the emitter is right to include it; reachable and needed-at-first-paint
//! are different questions and only the first has an answer today.
//!
//! This is the first half of the second answer: name the definitions a
//! deferred chunk could hold. It emits nothing.

mod support;

use zdc_codegen::deferrable_regions;

/// The document's root nodes.
fn view_nodes(hir: &zdc_hir::Hir) -> &[zdc_hir::HirNode] {
    nodes_of(hir).expect("this program has a view")
}

/// The same, for a program that may be a module and have none.
fn nodes_of(hir: &zdc_hir::Hir) -> Option<&[zdc_hir::HirNode]> {
    match &hir.defs[hir.view?].kind {
        zdc_hir::DefKind::View(view) => Some(&view.nodes),
        // `hir.view` names a view, so this is unreachable — written out
        // rather than waved past, because a new `DefKind` should be a
        // compile error here and not a silent `None`.
        zdc_hir::DefKind::Signal(_)
        | zdc_hir::DefKind::Function(_)
        | zdc_hir::DefKind::Record(_)
        | zdc_hir::DefKind::Choice(_)
        | zdc_hir::DefKind::Component(_)
        | zdc_hir::DefKind::Foreign(_)
        | zdc_hir::DefKind::Release(_) => None,
    }
}

/// Parse and resolve far enough to have HIR, which is all this needs.
fn hir_of(source: &str) -> zdc_hir::Hir {
    resolved(source).expect("the test program resolves")
}

/// The same, for a source that may legitimately not resolve alone.
fn resolved(source: &str) -> Result<zdc_hir::Hir, ()> {
    let program = zdc_parser::parse(source).map_err(|_| ())?;
    zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .map_err(|_| ())
}

const BEHIND_AN_IF: &str = r#"
state open is client Truth starting no

function onlyInside of value
    give value + 1

function usedByBoth of value
    give value * 2

state count is client Whole starting 0

view
    Column
        Text (text of (usedByBoth of count))
        if open
            Text (text of (onlyInside of count))
            Text (text of (usedByBoth of count))
"#;

#[test]
fn a_function_used_only_inside_a_branch_is_exclusive_to_it() {
    let hir = hir_of(BEHIND_AN_IF);
    let regions = deferrable_regions(&hir, view_nodes(&hir));

    assert_eq!(regions.len(), 1, "one `if`, so one region");
    let names: Vec<&str> = regions[0]
        .exclusive
        .iter()
        .map(|id| hir.defs[*id].name.as_str())
        .collect();
    assert!(
        names.contains(&"onlyInside"),
        "`onlyInside` is called only from inside the branch: {names:?}"
    );
}

#[test]
fn a_function_used_on_both_sides_is_not() {
    let hir = hir_of(BEHIND_AN_IF);
    let regions = deferrable_regions(&hir, view_nodes(&hir));

    let names: Vec<&str> = regions[0]
        .exclusive
        .iter()
        .map(|id| hir.defs[*id].name.as_str())
        .collect();
    // The whole point of the subtraction. A definition the eager part also
    // reaches must stay eager, and it cannot be told apart from a deferred
    // one by looking at the branch alone.
    assert!(
        !names.contains(&"usedByBoth"),
        "`usedByBoth` is called outside the branch too, so deferring it \
         would take code the first paint needs: {names:?}"
    );
    assert!(
        regions[0].reached.len() > regions[0].exclusive.len(),
        "the branch reaches more than it exclusively holds, or the \
         subtraction is doing nothing"
    );
}

#[test]
fn a_program_with_no_branch_offers_nothing() {
    let hir = hir_of(
        r#"
state count is client Whole starting 0

view
    Column
        Text (text of count)
"#,
    );
    assert!(
        deferrable_regions(&hir, view_nodes(&hir)).is_empty(),
        "nothing is behind an `if`, so nothing can be deferred"
    );
}

/// **Run against the corpus**, rather than only against programs written
/// here to make the walk look good.
///
/// The list is every example whose *view* holds an `if`. It is a list
/// rather than one file because the first version of this test named
/// `disclosure.zd` — the repository's disclosure example — which turns out
/// to contain no `if` at all. A test that asserts a property of one file
/// is a test about that file.
#[test]
fn it_runs_on_every_example_whose_view_holds_a_branch() {
    let examples = ["components.zd", "events.zd", "gauge.zd", "content.zd"];
    let mut found = 0;
    for name in examples {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name);
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        // A program that imports another file cannot resolve on its own
        // here — the importing file is outside this compilation unit — so
        // it is skipped rather than failed. `found` is what keeps the skip
        // honest: if every example skipped, this test asserts nothing and
        // says so.
        let Ok(hir) = resolved(&source) else {
            continue;
        };
        let Some(nodes) = nodes_of(&hir) else {
            continue;
        };
        let regions = deferrable_regions(&hir, nodes);
        for region in &regions {
            found += 1;
            assert!(
                region.exclusive.len() <= region.reached.len(),
                "{name}: a branch cannot exclusively hold more than it reaches"
            );
            assert!(
                region.exclusive.is_subset(&region.reached),
                "{name}: an exclusive definition the branch does not reach \
                 means the subtraction is reading the wrong set"
            );
        }
    }
    assert!(
        found > 0,
        "no example offered a region, so this asserted nothing — the walk \
         is not finding `if` nodes at all"
    );
}
