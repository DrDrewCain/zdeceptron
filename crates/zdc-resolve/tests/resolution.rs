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

/// A `Link`'s destination is written first and arrives named `href`.
///
/// This is the whole of what keeps a positional destination visible to a
/// rule keyed on URL-bearing attribute *names* (`href`, `src`, `srcset`,
/// …). A leading argument is otherwise lowered by its position, and a
/// position has no name to test — so a destination left positional would
/// be a URL such a rule never sees, for the commonest way there is to
/// write a link. Lowering it here means the rule needs to know nothing
/// about `Link` at all.
#[test]
fn a_links_destination_is_lowered_to_the_href_it_becomes() {
    for source in [
        "view\n    Link \"https://example.com\"\n        Text \"there\"\n",
        "route Site\n    Home is \"/\"\nview\n    Link Home\n        Text \"home\"\n",
    ] {
        let hir = resolve(source);
        let element = only_element(&hir);
        assert!(
            element
                .args
                .iter()
                .all(|arg| !matches!(arg, HirArg::Positional(_))),
            "the destination must not stay positional: {:?}",
            element.args
        );
        assert!(
            zdc_hir::destination_of(element).is_some(),
            "the destination must be reachable by name: {:?}",
            element.args
        );
        assert!(
            element.args.iter().any(|arg| matches!(
                arg,
                HirArg::Named { name, .. } if name == zdc_hir::DESTINATION_ARGUMENT
            )),
            "the destination must be named `{}`: {:?}",
            zdc_hir::DESTINATION_ARGUMENT,
            element.args
        );
    }
}

/// And the name is not a second phrasing: §4.1 gives the destination one,
/// and it is the leading position.
#[test]
fn a_link_may_not_write_its_destination_as_a_named_argument() {
    let program = zdc_parser::parse("view\n    Link href is \"/x\"\n        Text \"there\"\n")
        .expect("source parses");
    let errors = Resolver::new(&program)
        .resolve()
        .expect_err("`Link href is …` must be refused");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("first argument")),
        "{errors:#?}"
    );
}

fn only_element(hir: &Hir) -> &zdc_hir::HirElement {
    let view = hir.view.expect("the source declares a view");
    let DefKind::View(view) = &hir.defs[view].kind else {
        panic!("`Hir::view` names a view")
    };
    view.nodes
        .iter()
        .find_map(|node| match node {
            HirNode::Element(element) if element.name == "Link" => Some(element),
            HirNode::Element(_)
            | HirNode::Handler(_)
            | HirNode::Each(_)
            | HirNode::When(_)
            | HirNode::If(_)
            | HirNode::Scope(_)
            | HirNode::Children(_) => None,
        })
        .expect("the view holds a `Link`")
}

/// A chain of *distinct* components that each use two of the next is
/// bounded, and says so.
///
/// The cycle check bounds a component that contains itself; nothing
/// bounded one that contains two of the next. Twenty-six of them is a
/// hundred-line file that expands to 2²⁶ nodes, and the compiler used to
/// allocate until the machine stopped it — no diagnostic, no line number,
/// no exit code worth reading. The parser's own nesting guard is charged
/// per declaration and released at its end, so it never sees this: every
/// declaration below is three levels deep.
///
/// Small here on purpose. The point is the message, and the sizes that
/// demonstrate the old behaviour take minutes to not finish.
#[test]
fn a_chain_of_components_that_each_use_two_of_the_next_is_bounded() {
    let mut source = String::new();
    for index in 0..26 {
        source.push_str(&format!("component C{index}\n    Column\n"));
        match index + 1 {
            26 => source.push_str("        Text \"leaf\"\n"),
            next => source.push_str(&format!("        C{next}\n        C{next}\n")),
        }
    }
    source.push_str("\nview\n    Column\n        C0\n");

    let program = zdc_parser::parse(&source).expect("source parses");
    let errors = Resolver::new(&program)
        .resolve()
        .expect_err("this expands to 2^26 nodes and must be refused");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("component instances")),
        "expected the expansion budget to name itself, got: {:?}",
        errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

/// The budget is a ceiling on a pathological program and not a limit on
/// an ordinary one: a component used many times over, and components
/// nested inside each other in a chain, both still resolve.
#[test]
fn the_expansion_budget_leaves_ordinary_component_use_alone() {
    let mut source = String::from("component Leaf with label\n    Text label\n\n");
    for index in 0..20 {
        source.push_str(&format!("component C{index}\n    Column\n"));
        match index + 1 {
            20 => source.push_str("        Leaf \"leaf\"\n"),
            next => source.push_str(&format!("        C{next}\n")),
        }
    }
    source.push_str("\nview\n    Column\n        C0\n");
    for _ in 0..200 {
        source.push_str("        Leaf \"again\"\n");
    }
    resolve(&source);
}
