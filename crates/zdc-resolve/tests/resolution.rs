use zdc_hir::{Def, DefId, DefKind, Hir, HirArg, HirExprKind, HirNode, HirStmt, Res};
use zdc_resolve::Resolver;

fn resolve(source: &str) -> Hir {
    let program = zdc_parser::parse(source).expect("source parses");
    Resolver::new(&program).resolve().expect("source resolves")
}

fn definition_named<'a>(hir: &'a Hir, name: &str) -> (DefId, &'a Def) {
    hir.defs
        .iter()
        .find(|(_, definition)| definition.name == name)
        .unwrap_or_else(|| panic!("missing definition `{name}`"))
}

#[test]
fn forward_signal_references_become_definition_ids() {
    let hir = resolve(concat!(
        "state doubled is client Whole from count + count\n",
        "state count is client Whole starting 1\n",
    ));
    let (count_id, count) = definition_named(&hir, "count");
    let (_, doubled) = definition_named(&hir, "doubled");
    let DefKind::Signal(count) = &count.kind else {
        panic!("expected count to be a signal")
    };
    let DefKind::Signal(doubled) = &doubled.kind else {
        panic!("expected doubled to be a signal")
    };
    let HirExprKind::Binary { lhs, rhs, .. } = hir.exprs[doubled.init].kind else {
        panic!("expected a binary initializer")
    };

    assert!(count.is_source);
    assert!(!doubled.is_source);
    assert_eq!(hir.exprs[lhs].kind, HirExprKind::Ref(Res::Def(count_id)));
    assert_eq!(hir.exprs[rhs].kind, HirExprKind::Ref(Res::Def(count_id)));
}

#[test]
fn a_parameter_shadows_a_top_level_definition() {
    let hir = resolve(concat!(
        "state value is client Whole starting 1\n",
        "function identity with value\n",
        "    give value\n",
    ));
    let (_, identity) = definition_named(&hir, "identity");
    let DefKind::Function(identity) = &identity.kind else {
        panic!("expected a function")
    };
    let parameter = identity.params[0];
    let HirStmt::Give(result) = hir.blocks[identity.body].stmts[0] else {
        panic!("expected a give statement")
    };

    assert_eq!(hir.locals[parameter].name, "value");
    assert_eq!(
        hir.exprs[result].kind,
        HirExprKind::Ref(Res::Local(parameter))
    );
}

#[test]
fn a_view_loop_binding_reaches_nested_event_handlers() {
    let hir = resolve(concat!(
        "state items is client Whole starting 0\n",
        "view\n",
        "    each item in items\n",
        "        Row item\n",
        "            on click\n",
        "                give item\n",
    ));
    let view_id = hir.view.expect("view definition");
    let DefKind::View(view) = &hir.defs[view_id].kind else {
        panic!("expected a view")
    };
    let HirNode::Each(each) = &view.nodes[0] else {
        panic!("expected an each node")
    };
    let HirNode::Element(row) = &each.body[0] else {
        panic!("expected a row")
    };
    let HirArg::Positional(row_argument) = row.args[0] else {
        panic!("expected a positional row argument")
    };
    let HirNode::Handler(handler) = &row.children[0] else {
        panic!("expected an event handler")
    };
    let HirStmt::Give(handler_result) = hir.blocks[handler.body].stmts[0] else {
        panic!("expected the handler to give a value")
    };

    assert_eq!(
        hir.exprs[row_argument].kind,
        HirExprKind::Ref(Res::Local(each.var))
    );
    assert_eq!(
        hir.exprs[handler_result].kind,
        HirExprKind::Ref(Res::Local(each.var))
    );
}

#[test]
fn resolution_reports_every_bad_public_name_with_its_source_span() {
    let source = concat!(
        "function f\n",
        "    give missing + other\n",
        "view\n",
        "    Bogus\n",
    );
    let program = zdc_parser::parse(source).expect("source parses");
    let errors = Resolver::new(&program).resolve().unwrap_err();
    let mut covered = errors
        .iter()
        .map(|error| &source[error.span.start as usize..error.span.end as usize])
        .collect::<Vec<_>>();
    covered.sort_unstable();

    assert_eq!(covered, ["Bogus", "missing", "other"]);
    assert!(errors.iter().all(|error| {
        error.message.contains("not defined") || error.message.contains("not a view element")
    }));
}
