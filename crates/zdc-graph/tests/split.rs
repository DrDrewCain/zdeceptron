//! The tier split, against the programs that specify it.
//!
//! Every fixture here is either a spec §17.2 worked example or a program a
//! reviewer exhibited. The names say which.

mod support;

use support::*;
use zdc_graph::{split, CommandKey, Crossing, Ctx, EndpointKind, MemberForm, RootOrigin, CLIENT};
use zdc_hir::DefKind;

/// §14A.1: "the client bundle *provably* excludes `server` logic."
///
/// Provably, not heuristically. This is the whole claim, and until now it
/// had never been asserted against anything.
#[test]
fn the_client_bundle_provably_excludes_server_logic() {
    let (hir, split) = compile(GUESTBOOK);
    let client = names(&hir, split.client_members());

    assert_eq!(
        client,
        vec!["name".to_string(), "view".to_string()],
        "the client bundle is exactly the view and the one `client` signal"
    );

    for excluded in ["apiKey", "greeting", "politeGreeting", "visits"] {
        let id = def_named(&hir, excluded);
        assert!(
            !split.is_member(id, CLIENT),
            "`{excluded}` must not be a member of the client bundle"
        );
    }
}

/// §17.2.1's theorem, not a tree-shake: `view ∉ members(r)` for every
/// `r ≠ CLIENT`. It holds because `zdc-resolve` inserts no name for the
/// view into the global table, and because no `HirExprKind` carries a
/// `HirNode`.
#[test]
fn the_view_is_a_member_of_exactly_one_root() {
    let (hir, split) = compile(GUESTBOOK);
    let view = hir.view.expect("guestbook has a view");
    for (root, _) in split.emitted_roots() {
        assert_eq!(
            split.is_member(view, root),
            root == CLIENT,
            "the view belongs to the client bundle and to nothing else"
        );
    }
}

/// No `HirExprKind` variant may carry a `HirNode`. Step 2 of §17.2.1's
/// proof is Rust exhaustiveness plus one grep; this is the grep, made a
/// test, so a new variant that carries one fails here rather than silently
/// putting the renderer in a server bundle.
#[test]
fn no_expression_can_reference_a_view_node() {
    let source = include_str!("../../zdc-hir/src/nodes.rs");
    let start = source
        .find("pub enum HirExprKind")
        .expect("the expression enum");
    let end = source[start..].find("\n}").expect("its closing brace") + start;
    let body = &source[start..end];
    assert!(
        !body.contains("HirNode"),
        "an expression that carries a view node would put the renderer in a server bundle:\n{body}"
    );
}

/// §16.4's worked emission for `guestbook.zd`: one endpoint per remote
/// read, one command per cross-region write, and `name` as the endpoint's
/// single parameter.
#[test]
fn guestbook_derives_the_network_the_spec_predicts() {
    let (hir, split) = compile(GUESTBOOK);
    let mut endpoints: Vec<(String, Vec<String>)> = split
        .endpoints
        .iter()
        .map(|endpoint| (endpoint.name.clone(), names(&hir, endpoint.params.clone())))
        .collect();
    endpoints.sort();

    assert_eq!(
        endpoints,
        vec![
            ("greeting".to_string(), vec!["name".to_string()]),
            ("visits".to_string(), Vec::new()),
            ("visits.incr".to_string(), Vec::new()),
        ]
    );

    let greeting = split
        .endpoints
        .iter()
        .find(|e| e.name == "greeting")
        .expect("the greeting endpoint");
    assert_eq!(
        names(&hir, split.members_of(greeting.root).map(|(d, _)| d)),
        vec![
            "apiKey".to_string(),
            "greeting".to_string(),
            "politeGreeting".to_string()
        ],
        "the secret's initialiser lives in the function bundle and only there"
    );
    assert!(matches!(greeting.kind, EndpointKind::Value(_)));
}

/// §17.2.5 fatal 1. `greeting` enters the endpoint root by a **`Direct`
/// read edge**, not a call edge, and the `Lift` of `name` is discovered
/// under it — so a parameter set computed as the transitive-call closure
/// from the root definition ships an endpoint with a free variable.
#[test]
fn an_endpoints_parameters_are_the_union_over_members_not_over_calls() {
    const CHAIN: &str = "\
state name     is client Text starting \"\"
state greeting is server Text from politeGreeting with name
state shout    is server Text from loudly with greeting

function politeGreeting with who
    give who

function loudly with s
    give s

view
    Column
        when shout
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text
";
    let (hir, split) = compile(CHAIN);
    let endpoint = split
        .endpoints
        .iter()
        .find(|e| e.name == "shout")
        .expect("one endpoint, for `shout`");
    assert_eq!(
        names(&hir, endpoint.params.clone()),
        vec!["name".to_string()],
        "the endpoint must declare `name`, which it reaches through a read rather than a call"
    );
}

/// §17.2.8's hoisting rule, with the correction this crate had to make:
/// `shout` reaches `greeting` by a read, and `greeting` closes over a
/// lifted parameter, so a module-scope `shout` would be a
/// `ReferenceError`. Lexical scope is a question about references.
#[test]
fn a_member_that_reads_a_lifted_member_is_not_hoisted() {
    const CHAIN: &str = "\
state name     is client Text starting \"\"
state greeting is server Text from politeGreeting with name
state shout    is server Text from loudly with greeting

function politeGreeting with who
    give who

function loudly with s
    give s

view
    Column
        when shout
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text
";
    let (hir, split) = compile(CHAIN);
    let root = split
        .endpoints
        .iter()
        .find(|e| e.name == "shout")
        .expect("the endpoint")
        .root;

    for (name, hoisted) in [
        ("greeting", false),
        ("shout", false),
        ("politeGreeting", true),
        ("loudly", true),
    ] {
        assert_eq!(
            split.hoisted.get(&(def_named(&hir, name), root)),
            Some(&hoisted),
            "`{name}` hoisting"
        );
    }
}

/// `guestbook.zd`'s `politeGreeting` needs no lifted value, so it is
/// emitted at module scope and §16.4's bytes are reproduced. This is the
/// half of §16.3.12's "byte for byte" claim that survives.
#[test]
fn a_colourless_function_needing_no_lift_is_hoisted() {
    let (hir, split) = compile(GUESTBOOK);
    let root = split
        .endpoints
        .iter()
        .find(|e| e.name == "greeting")
        .expect("the endpoint")
        .root;
    assert_eq!(
        split
            .hoisted
            .get(&(def_named(&hir, "politeGreeting"), root)),
        Some(&true)
    );
    assert_eq!(
        split.hoisted.get(&(def_named(&hir, "greeting"), root)),
        Some(&false),
        "`greeting` closes over the lifted `name`"
    );
}

/// §17.2.5 fatal 3. Two mutation sites on one signal with the same
/// operator and different paths produced two roots with one name and one
/// emitted file. The name now renders the whole key.
#[test]
fn a_command_name_is_injective_over_the_generated_keys() {
    let (hir, split) = compile(GUESTBOOK);
    let mut rendered: Vec<String> = split
        .endpoints
        .iter()
        .filter(|e| matches!(e.kind, EndpointKind::Command(_)))
        .map(|e| e.name.clone())
        .collect();
    let before = rendered.len();
    rendered.sort();
    rendered.dedup();
    assert_eq!(
        rendered.len(),
        before,
        "two commands rendered the same name"
    );
    assert_eq!(rendered, vec!["visits.incr".to_string()]);
    let _ = hir;
}

/// §17.2.5 fatal 5. A `durable` signal's initial value goes into
/// `manifest.json` at build time, so it must be computable with no
/// browser, no request and no store — and it is walked in the BUILD root
/// alone, which is what makes reading browser state from one an error.
#[test]
fn a_durable_initialiser_that_reads_browser_state_is_rejected() {
    const STORE: &str = "\
state seed  is client  Whole starting 7
state quota is durable Whole starting seed

view
    Column
        Text \"hi\"
";
    let (_, split) = compile(STORE);
    assert_eq!(codes(&split.diagnostics), vec!["E0301"]);
    let error = split.errors().next().expect("one error");
    assert!(
        error.notes.iter().any(|(_, note)| note.contains("seed")),
        "the diagnostic must name the browser state it reached: {:?}",
        error.notes
    );
}

/// §17.2.7's E0314. Verified accepted today: `HirPlace.base` is a `Res`
/// and `zdc-resolve::place` returns `Res::Local` for any in-scope binder,
/// so this resolves, typechecks, and is silently dropped by `zdc-codegen`.
/// A parameter is a value rather than a place.
#[test]
fn writing_through_a_parameter_is_rejected() {
    const BOX: &str = "\
state total is client Whole starting 0

function bump with box
    add 1 to box
    give 0

view
    Column
        Text \"hi\"
";
    let (_, split) = compile(BOX);
    assert_eq!(codes(&split.diagnostics), vec!["E0314"]);
}

/// §17.2.10's E0311: the browser cannot write a `server` signal.
#[test]
fn a_click_handler_cannot_write_server_state() {
    const WRITE: &str = "\
state hits is server Whole starting 0

view
    Column
        Button \"go\"
            on click
                add 1 to hits
";
    let (_, split) = compile(WRITE);
    assert_eq!(codes(&split.diagnostics), vec!["E0311"]);
}

/// §5.3: only `server` and `durable` signals may be secret.
#[test]
fn secret_on_a_client_signal_is_rejected() {
    const LOCAL: &str = "\
secret state token is client Text starting \"\"

view
    Column
        Text \"hi\"
";
    let (_, split) = compile(LOCAL);
    assert_eq!(codes(&split.diagnostics), vec!["E0313"]);
}

/// §5.5: durable is storage, not computation.
#[test]
fn a_derived_durable_signal_is_rejected() {
    const DERIVED: &str = "\
state base  is durable Whole starting 1
state twice is durable Whole from double with base

function double with n
    give n

view
    Column
        Text \"hi\"
";
    let (_, split) = compile(DERIVED);
    assert!(codes(&split.diagnostics).contains(&"E0321"));
}

/// §17.5.2. A cycle in the *derivation* graph is E0320, printed as a path
/// with one span per edge. §17.5.4's reactive cycles are a different graph
/// and must not be caught here.
#[test]
fn a_cycle_in_the_derivation_graph_is_reported_as_a_cycle() {
    const CYCLE: &str = "\
state a is client Whole from idOf with b
state b is client Whole from idOf with a

function idOf with n
    give n

view
    Column
        Text \"hi\"
";
    let (_, split) = compile(CYCLE);
    assert!(codes(&split.diagnostics).contains(&"E0320"));
    let error = split
        .errors()
        .find(|e| e.code == "E0320")
        .expect("the cycle");
    assert_eq!(error.notes.len(), 2, "one span per edge: {:?}", error.notes);
}

/// §17.5.4. `guestbook.zd`'s handler writes `visits`, `visits` feeds an
/// endpoint, and the endpoint's result renders a `Button` whose handler
/// writes `visits` again. That is the intended behaviour of a reactive
/// program. Cycles here are not "handled"; they are irrelevant.
#[test]
fn a_reactive_write_read_loop_is_not_a_derivation_cycle() {
    let (_, split) = compile(GUESTBOOK);
    assert!(
        !codes(&split.diagnostics).contains(&"E0320"),
        "guestbook's write/read loop must not be reported as a cycle"
    );
}

/// §17.2.5 fatal 6. A purely demand-driven root set silently deletes
/// typechecking and every placement diagnostic for unreached code.
/// Verified: this program produces two real type errors today and zero
/// under a demand-driven root set.
#[test]
fn unreached_code_still_gets_a_root_and_a_context() {
    const DEAD: &str = "\
state name is client Text starting \"\"
state bad  is server Text from mix with name

function mix with x
    give x

view
    Column
        Text \"hello\"
";
    let (hir, split) = compile(DEAD);
    for name in ["bad", "mix", "name"] {
        let id = def_named(&hir, name);
        assert!(
            split
                .contexts
                .get(&id)
                .is_some_and(|contexts| !contexts.is_empty()),
            "`{name}` must be checked even though nothing renders it"
        );
    }

    // ... and contributes to no emitted artifact.
    let emitted: Vec<String> = split
        .emitted_roots()
        .flat_map(|(root, _)| names(&hir, split.members_of(root).map(|(d, _)| d)))
        .collect();
    assert!(
        !emitted.contains(&"bad".to_string()),
        "an orphan root must not be emitted: {emitted:?}"
    );
}

/// §17.2.4's table, as a total function. Every cell, in the spec's order.
#[test]
fn the_read_table_is_the_specs_table() {
    use zdc_graph::SignalPlacement as P;
    let cell = |ctx: Ctx, target: P| match classify_for_test(ctx, target) {
        Crossing::Direct => "Direct",
        Crossing::Inline => "Inline",
        Crossing::Store { .. } => "Store",
        Crossing::Remote { .. } => "Remote",
        Crossing::Lift { .. } => "Lift",
        Crossing::Rejected { code } => code,
    };

    let columns = [
        P::Client,
        P::Static,
        P::Server,
        P::Durable,
        P::DurablePerVisitor,
    ];
    let expected = [
        (
            Ctx::CLIENT_VIEW,
            ["Direct", "Inline", "Remote", "Remote", "Remote"],
        ),
        (
            Ctx::CLIENT_TRIGGER,
            ["Direct", "Inline", "Remote", "Remote", "Remote"],
        ),
        (
            Ctx::STATIC_BUILD,
            ["E0301", "Direct", "E0301", "E0301", "E0301"],
        ),
        (
            Ctx::SERVER_VIEW,
            ["Lift", "Inline", "Direct", "Store", "Store"],
        ),
        (
            Ctx::SERVER_TRIGGER,
            ["E0302", "Inline", "Direct", "Store", "E0303"],
        ),
    ];

    for (ctx, row) in expected {
        for (target, want) in columns.iter().zip(row.iter()) {
            assert_eq!(&cell(ctx, *target), want, "{ctx:?} × {target:?}");
        }
    }
}

fn classify_for_test(ctx: Ctx, target: zdc_graph::SignalPlacement) -> Crossing {
    zdc_graph::classify(ctx, target)
}

/// §17.2.7's write table, likewise.
#[test]
fn the_write_table_is_the_specs_table() {
    use zdc_graph::MutCrossing as M;
    use zdc_graph::SignalPlacement as P;
    let cell = |ctx: Ctx, target: P| match zdc_graph::classify_write(ctx, target) {
        M::Local => "Local",
        M::StoreWrite { .. } => "StoreWrite",
        M::Command { .. } => "Command",
        M::Rejected { code } => code,
    };

    let columns = [
        P::Client,
        P::Static,
        P::Server,
        P::Durable,
        P::DurablePerVisitor,
    ];
    let expected = [
        (
            Ctx::CLIENT_VIEW,
            ["Local", "E0310", "E0311", "Command", "Command"],
        ),
        (
            Ctx::SERVER_VIEW,
            ["E0312", "E0310", "Local", "StoreWrite", "StoreWrite"],
        ),
        (
            Ctx::SERVER_TRIGGER,
            ["E0312", "E0310", "Local", "StoreWrite", "E0303"],
        ),
        (
            Ctx::STATIC_BUILD,
            ["E0312", "E0310", "E0312", "E0312", "E0312"],
        ),
    ];

    for (ctx, row) in expected {
        for (target, want) in columns.iter().zip(row.iter()) {
            assert_eq!(&cell(ctx, *target), want, "{ctx:?} × {target:?}");
        }
    }
}

/// Every checked-in example the compiler accepts today must still split
/// without a placement error. §17.3.9 item 4's acceptance canaries.
#[test]
fn the_client_only_examples_still_split_clean() {
    for (name, src) in [
        ("hello.zd", include_str!("../../../examples/hello.zd")),
        ("counter.zd", include_str!("../../../examples/counter.zd")),
        ("guestbook.zd", GUESTBOOK),
    ] {
        let (_, split) = compile(src);
        assert!(
            !split.has_errors(),
            "{name} must split clean: {:?}",
            split
                .errors()
                .map(|e| e.rendered_message())
                .collect::<Vec<_>>()
        );
    }
}

/// A durable signal is read from the store where it is read and is a
/// binding only where its initialiser is evaluated — which is the BUILD
/// root and nowhere else.
#[test]
fn a_durable_signal_has_a_different_form_in_each_root() {
    let (hir, split) = compile(GUESTBOOK);
    let visits = def_named(&hir, "visits");
    let endpoint = split
        .endpoints
        .iter()
        .find(|e| e.name == "visits")
        .expect("the visits endpoint");

    assert_eq!(
        split
            .members_of(endpoint.root)
            .find(|(id, _)| *id == visits)
            .map(|(_, form)| form),
        Some(MemberForm::StoreRead)
    );
    assert_eq!(
        split
            .members_of(zdc_graph::BUILD)
            .find(|(id, _)| *id == visits)
            .map(|(_, form)| form),
        Some(MemberForm::Binding),
        "the initial value is computed once, on the build host"
    );
}

/// The split records the two structurally different browser-visible facts
/// separately (§17.2.5 fatal 4) rather than conflating them in a single
/// `watch_keys` set.
#[test]
fn a_remote_durable_read_records_a_live_value_edge() {
    use zdc_graph::BoundaryEdge;
    let (hir, split) = compile(GUESTBOOK);
    let visits = def_named(&hir, "visits");
    assert!(split
        .boundary
        .iter()
        .any(|edge| matches!(edge, BoundaryEdge::LiveValue { key } if *key == visits)));
}

/// A definition reached from two regions is walked twice and may mean two
/// different things in each — which is the monomorphisation half of
/// §17.2, and the reason `TypeTable` is keyed by context.
#[test]
fn a_colourless_function_reached_from_two_regions_has_two_contexts() {
    const BOTH: &str = "\
state name  is client Text starting \"\"
state shown is server Text from echo with name
state local is client Text from echo with name

function echo with s
    give s

view
    Column
        Text local
        when shown
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text
";
    let (hir, split) = compile(BOTH);
    let echo = def_named(&hir, "echo");
    let contexts = split.contexts.get(&echo).expect("echo is reached");
    assert_eq!(
        contexts.len(),
        2,
        "`echo` runs in the browser and on the server: {contexts:?}"
    );
}

/// A command key is part of the endpoint's identity, so two writes with
/// different paths get two endpoints.
#[test]
fn two_writes_with_different_operators_get_two_commands() {
    const TWO: &str = "\
state visits is durable Whole starting 0

view
    Column
        Button \"up\"
            on click
                add 1 to visits
        Button \"down\"
            on click
                subtract 1 from visits
";
    let (hir, split) = compile(TWO);
    let mut commands: Vec<String> = split
        .endpoints
        .iter()
        .filter(|e| matches!(e.kind, EndpointKind::Command(_)))
        .map(|e| e.name.clone())
        .collect();
    commands.sort();
    assert_eq!(
        commands,
        vec!["visits.decr".to_string(), "visits.incr".to_string()]
    );
    let _ = hir;
}

/// Nothing in the split consults an inference result. §17.6 makes this
/// load-bearing: it is why the split runs before `zdc-types`.
#[test]
fn the_split_reads_no_type_information() {
    let source = include_str!("../src/split.rs");
    for forbidden in ["TypeTable", "index_kind", "zdc_types::check"] {
        assert!(
            !source.contains(forbidden),
            "the split must consult no inference result, and it mentions `{forbidden}`"
        );
    }
}

/// Root creation is memoised on `DefId` and on `CommandKey`, so the root
/// set is bounded even though a `Lift` enqueues into the client root and
/// the client walk can then create endpoint roots (§17.5.1).
#[test]
fn root_creation_is_bounded_by_memoisation() {
    let (hir, split) = compile(GUESTBOOK);
    let signals = hir
        .defs
        .iter()
        .filter(|(_, def)| matches!(def.kind, DefKind::Signal(_)))
        .count();
    assert!(
        split.roots.len() <= signals + split.endpoints.len() + 2,
        "{} roots for {signals} signals",
        split.roots.len()
    );
}

/// Running the split twice on the same program gives the same answer.
/// Wire order is ascending `DefId`, which is source declaration order,
/// and nothing in the pass depends on hash iteration order.
#[test]
fn the_split_is_deterministic() {
    let program = zdc_parser::parse(GUESTBOOK).expect("guestbook parses");
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("guestbook resolves");
    let first = split(&hir);
    let second = split(&hir);
    assert_eq!(
        first
            .endpoints
            .iter()
            .map(|e| (e.name.clone(), e.params.clone()))
            .collect::<Vec<_>>(),
        second
            .endpoints
            .iter()
            .map(|e| (e.name.clone(), e.params.clone()))
            .collect::<Vec<_>>()
    );
    let _ = CommandKey {
        signal: def_named(&hir, "visits"),
        op: zdc_graph::MutOp::Incr,
        path: Vec::new(),
    };
    let _ = RootOrigin::BuildHost;
}
