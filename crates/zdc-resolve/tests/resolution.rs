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

// --- `Code`'s arms are names, so a misspelling has nothing to resolve ---

fn resolution_errors(source: &str) -> Vec<String> {
    let program = zdc_parser::parse(source).expect("source parses");
    Resolver::new(&program)
        .resolve()
        .expect_err("source must not resolve")
        .into_iter()
        .map(|error| error.message)
        .collect()
}

/// A view whose `Failed` arm mentions `code`, with the fragment under
/// test spliced in.
fn failure_arm(body: &str) -> String {
    format!(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       when visits\n\
         \x20           Loading show Spinner\n\
         \x20           Failed with error\n\
         {body}\
         \x20           Ready with total show Text total\n"
    )
}

/// **The acceptance criterion.** `error.code is Timout` names no variant
/// and no definition, so it does not resolve — and the diagnostic names
/// the variant that was meant.
///
/// This is what `code` became a choice in order to buy. As `Text`, the
/// same line compared two strings that are never equal: it compiled, it
/// ran, and the arm it guarded never fired.
#[test]
fn a_misspelled_failure_code_is_a_resolution_error_that_names_the_variant() {
    let messages = resolution_errors(&failure_arm(
        "\x20               if error.code is Timout\n\
         \x20                   Text \"slow\"\n",
    ));
    let named = messages
        .iter()
        .find(|message| message.contains("`Timout`"))
        .unwrap_or_else(|| panic!("nothing reported the misspelling: {messages:?}"));
    assert!(
        named.contains("`Timeout`"),
        "the diagnostic must name the variant that was meant: {named}"
    );
}

/// The same misspelling in *pattern* position, where a `when` arm is
/// written. Both positions suggest, because a variant name is a value and
/// a pattern alike.
#[test]
fn a_misspelled_arm_suggests_the_variant_it_is_one_edit_from() {
    let messages = resolution_errors(&failure_arm(
        "\x20               when error.code\n\
         \x20                   Unreachable show ErrorBar message is \"a\"\n\
         \x20                   Timout      show ErrorBar message is \"b\"\n\
         \x20                   Rejected    show ErrorBar message is \"c\"\n",
    ));
    let named = messages
        .iter()
        .find(|message| message.contains("`Timout`"))
        .unwrap_or_else(|| panic!("nothing reported the misspelled arm: {messages:?}"));
    assert!(named.contains("is not a variant name"), "{named}");
    assert!(
        named.contains("Did you mean `Timeout`?"),
        "the arm diagnostic must name the variant that was meant: {named}"
    );
}

/// A name nothing is close to gets no suggestion: naming a variant at
/// random is worse than naming none.
#[test]
fn a_name_far_from_every_variant_is_not_guessed_at() {
    let messages = resolution_errors(&failure_arm(
        "\x20               when error.code\n\
         \x20                   Unreachable show ErrorBar message is \"a\"\n\
         \x20                   Catastrophe show ErrorBar message is \"b\"\n\
         \x20                   Rejected    show ErrorBar message is \"c\"\n",
    ));
    let named = messages
        .iter()
        .find(|message| message.contains("`Catastrophe`"))
        .unwrap_or_else(|| panic!("nothing reported it: {messages:?}"));
    assert!(!named.contains("Did you mean"), "{named}");
}

/// A program cannot add a fourth outcome: the three arms are the
/// language's, and a `choice` that redeclares one is refused by name.
#[test]
fn a_program_cannot_declare_a_variant_of_the_builtin_code_choice() {
    for name in ["Unreachable", "Timeout", "Rejected"] {
        let messages = resolution_errors(&format!(
            "choice Outcome\n\
             \x20   Fine\n\
             \x20   {name}\n\
             state visits is durable Whole starting 0\n"
        ));
        let named = messages
            .iter()
            .find(|message| message.contains(&format!("`{name}`")))
            .unwrap_or_else(|| panic!("`{name}` was redeclarable: {messages:?}"));
        assert!(named.contains("the language provides"), "{named}");
        assert!(named.contains("`Code`"), "{named}");
    }
}

/// And the set the resolver knows is the set the compiler knows: read off
/// `FailureCode` rather than restated, so a fourth code is matchable and
/// unredeclarable in the same edit.
#[test]
fn the_patterns_the_resolver_offers_include_every_failure_code() {
    let offered = zdc_resolve::builtin_patterns();
    let mut checked = 0;
    for code in zdc_types::FailureCode::CLOSED_SET {
        assert!(
            offered.contains(&code.spelling()),
            "`{}` is a failure code the resolver would refuse as an arm",
            code.spelling()
        );
        checked += 1;
    }
    assert_eq!(checked, zdc_types::FailureCode::CLOSED_SET.len());
    assert_eq!(
        offered.len(),
        5 + checked,
        "the built-in arm list changed size"
    );
}

// --- a type name is a name, so an undeclared one has nothing to resolve ---

/// Every error, paired with the source text its span covers.
fn errors_with_spans(source: &str) -> Vec<(String, String)> {
    let program = zdc_parser::parse(source).expect("source parses");
    Resolver::new(&program)
        .resolve()
        .expect_err("source must not resolve")
        .into_iter()
        .map(|error| {
            let covered = source[error.span.start as usize..error.span.end as usize].to_string();
            (error.message, covered)
        })
        .collect()
}

/// Whether anything reported `name`, with its span under `name` itself.
fn refused_at(reports: &[(String, String)], name: &str) -> bool {
    reports
        .iter()
        .any(|(message, covered)| covered == name && message.contains(&format!("`{name}`")))
}

/// **The acceptance criterion for #28.** A type nothing declares is
/// refused, and the caret is under the type name rather than under the
/// literal that happened to reach it.
///
/// `Map of Id to Int` and `List of Zork` used to check *and build* with
/// exit 0, because type positions were not resolved at all. `Zork` alone
/// did error, but it blamed `empty`.
#[test]
fn an_undeclared_type_name_is_refused_at_its_own_span() {
    let reports = errors_with_spans("state votes is client Map of Id to Int starting empty\n");
    assert!(refused_at(&reports, "Id"), "{reports:#?}");
    assert!(refused_at(&reports, "Int"), "{reports:#?}");

    let reports = errors_with_spans("state v is client List of Zork starting empty\n");
    assert!(refused_at(&reports, "Zork"), "{reports:#?}");

    let reports = errors_with_spans("state v is client Zork starting empty\n");
    assert!(refused_at(&reports, "Zork"), "{reports:#?}");
}

/// A record, a choice and a route each declare a type, and every built-in
/// name is one. None of them may become an error in the course of
/// refusing the names that are not.
#[test]
fn declared_and_builtin_type_names_still_resolve() {
    resolve(concat!(
        "record Todo\n",
        "    label is Text\n",
        "    done is Truth\n",
        "choice Status\n",
        "    Draft\n",
        "route Site\n",
        "    Home is \"/\"\n",
        "state todos is client List of Todo starting empty\n",
        "state seen is client Map of Text to Whole starting empty\n",
        "state status is client Option of Status starting None\n",
        "state page is client Option of Site starting address\n",
        "state rate is client Decimal starting 0.5\n",
        "record Post\n",
        "    body is Markup\n",
    ));
}

/// The near miss everyone arriving from another language types. Naming
/// the type that exists is the difference between a diagnostic that ends
/// the search and one that starts it (§7.3).
#[test]
fn a_type_name_from_another_language_names_the_one_that_exists() {
    for (written, expected) in [
        ("Int", "`Whole`"),
        ("Integer", "`Whole`"),
        ("Number", "`Whole`"),
        ("String", "`Text`"),
        ("Bool", "`Truth`"),
        ("Boolean", "`Truth`"),
    ] {
        let reports = errors_with_spans(&format!("state v is client {written} starting empty\n"));
        let named = reports
            .iter()
            .find(|(message, _)| message.contains(&format!("`{written}`")))
            .unwrap_or_else(|| panic!("`{written}` was not reported: {reports:#?}"));
        assert!(
            named.0.contains(expected),
            "`{written}` must name {expected}: {}",
            named.0
        );
    }
}

/// A misspelled record is one edit from the record, and a name nothing is
/// close to gets no guess.
#[test]
fn a_misspelled_declared_type_suggests_the_declaration_and_a_far_one_does_not() {
    let source = concat!(
        "record Todo\n",
        "    label is Text\n",
        "state todos is client List of Todu starting empty\n",
    );
    let reports = errors_with_spans(source);
    let named = reports
        .iter()
        .find(|(message, _)| message.contains("`Todu`"))
        .unwrap_or_else(|| panic!("`Todu` was not reported: {reports:#?}"));
    assert!(named.0.contains("`Todo`"), "{}", named.0);

    let reports = errors_with_spans("state v is client Zork starting empty\n");
    let named = reports
        .iter()
        .find(|(message, _)| message.contains("`Zork`"))
        .unwrap_or_else(|| panic!("`Zork` was not reported: {reports:#?}"));
    assert!(!named.0.contains("Did you mean"), "{}", named.0);
}

/// A name that *is* declared, but by something that declares no type. The
/// fix is a different one, so the message is too.
#[test]
fn a_declaration_that_is_not_a_type_is_refused_as_a_type() {
    let source = concat!(
        "function label with value\n",
        "    give value\n",
        "state v is client label starting \"\"\n",
    );
    let reports = errors_with_spans(source);
    let named = reports
        .iter()
        .find(|(message, _)| message.contains("`label`"))
        .unwrap_or_else(|| panic!("`label` was not reported: {reports:#?}"));
    assert!(named.0.contains("not a type"), "{}", named.0);
}

/// A `foreign` writes types it has no body to infer, and an undeclared
/// name there is a type *parameter* rather than a mistake (§14E.1). The
/// rule above must not reach into one.
#[test]
fn a_foreign_declarations_type_parameters_are_not_undeclared_names() {
    resolve(concat!(
        "foreign firstOf is anywhere\n",
        "    from \"zd:list\" as \"first\"\n",
        "    takes of value is List of T\n",
        "    gives pure Option of T\n",
    ));
}

// --- a name in scope that cannot be the thing called (#103, #104) ---

/// **A parameter used as a callee says so, rather than claiming the
/// parameter does not exist.**
///
/// Locals are deliberately skipped in callee position — `callee_name` and
/// `of_name` both document why — but the message that followed described a
/// different program than the one written. `f` is declared, it is spelled
/// correctly, and "Declare it with `function f of …`, or check the
/// spelling" is wrong on both counts; the nearest-global suggestion then
/// pointed at an unrelated signal.
///
/// This is the message a reader gets when they try to pass a function as
/// an argument, which is what #103 and #104 ask for and what §5.4 rules
/// out, so it is the one place that should say why.
#[test]
fn a_parameter_in_callee_position_is_told_it_is_not_a_function() {
    for source in [
        concat!(
            "function apply of f\n",
            "    give f of 1\n",
            "state n is client Whole from apply of 2\n",
        ),
        concat!(
            "function apply of f\n",
            "    give f with all is 1\n",
            "state n is client Whole from apply of 2\n",
        ),
    ] {
        let reports = errors_with_spans(source);
        let named = reports
            .iter()
            .find(|(message, covered)| covered == "f" && message.contains("`f`"))
            .unwrap_or_else(|| panic!("`f` was not reported: {reports:#?}"));
        assert!(
            named.0.contains("is in scope here"),
            "the message must not claim the parameter is missing: {}",
            named.0
        );
        assert!(
            named.0.contains("no first-class functions"),
            "the message must say why a name in scope cannot be called: {}",
            named.0
        );
        assert!(
            !named.0.contains("check the spelling"),
            "the spelling is not the problem: {}",
            named.0
        );
    }
}

/// The other half: a name that genuinely is not declared must keep the
/// message that names the fix, and must not be told it is in scope.
#[test]
fn a_name_that_is_not_declared_at_all_still_says_so() {
    let reports = errors_with_spans("state n is client Whole from missingThing of 1\n");
    let named = reports
        .iter()
        .find(|(_, covered)| covered == "missingThing")
        .unwrap_or_else(|| panic!("`missingThing` was not reported: {reports:#?}"));
    assert!(
        named.0.contains("check the spelling"),
        "an undeclared name keeps its own message: {}",
        named.0
    );
    assert!(
        !named.0.contains("is in scope"),
        "and must not claim to be in scope: {}",
        named.0
    );
}

/// A `foreign` that touches a `Handle` is `client` or it is nothing.
///
/// **This is the load-bearing half of the handle's information-flow
/// argument.** §14E.3 row 1 lets a secret cross into a foreign only where
/// the call sits in server context, and `zdc-graph` implements that by
/// obliging every argument of a `foreign … is client` to be Public
/// (`E-IFC-13`). Pinning every handle-touching foreign to `is client` is
/// therefore what makes "no secret ever reaches a host object" true, and
/// with nothing secret in one there is nothing to read back out.
#[test]
fn a_foreign_touching_a_handle_must_be_client() {
    let mut checked = 0;
    for site in ["server", "anywhere"] {
        for clause in [
            "    takes v is Handle\n    gives Whole\n",
            "    takes x is Whole\n    gives new Handle\n",
        ] {
            let source = format!(
                "foreign f is {site}\n    from \"./m.js\" as \"F\"\n{clause}view\n    Column\n        Text \"hi\"\n"
            );
            let program = zdc_parser::parse(&source).expect("source parses");
            let errors = Resolver::new(&program)
                .resolve()
                .expect_err("a handle outside the browser is refused");
            checked += 1;
            assert!(
                errors.iter().any(|error| {
                    error.message.contains("mentions `Handle`")
                        && error
                            .message
                            .contains("only be linked into the client bundle")
                }),
                "`is {site}` was allowed to hold a handle: {:?}",
                errors.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }
    }
    assert_eq!(
        checked, 4,
        "both sites, in both positions a handle is written"
    );
}

/// The same declaration written `is client` resolves, so the rule is
/// about the site and not about the word.
#[test]
fn a_client_foreign_may_take_and_give_a_handle() {
    resolve(concat!(
        "foreign make is client\n",
        "    from \"./m.js\" as \"F\"\n",
        "    takes x is Whole\n",
        "    gives new Handle\n",
        "foreign useIt is client\n",
        "    from \"./m.js\" as \"F\"\n",
        "    takes v is Handle\n",
        "    gives Whole\n",
        "state n is client Whole from useIt with v is (make with x is 1)\n",
        "view\n",
        "    Column\n",
        "        Text n\n",
    ));
}
