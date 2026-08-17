//! The tier split, against the programs that specify it.
//!
//! Every fixture here is either a spec §17.2 worked example or a program a
//! reviewer exhibited. The names say which.

mod support;

use support::*;
use zdc_graph::{
    split, CommandKey, Crossing, Ctx, EndpointKind, MemberForm, Region, RootOrigin, CLIENT,
};
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
    let roots: Vec<_> = split.emitted_roots().map(|(root, _)| root).collect();
    // Both arms of the assertion below have to be reachable, or the test
    // proves only half of what it is named for.
    assert!(roots.contains(&CLIENT), "no client root: {roots:?}");
    assert!(roots.len() > 1, "no root to be excluded from: {roots:?}");

    for root in roots {
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

/// **E0322 — a clock is the browser's, and four placements have none.**
/// #19.
///
/// Each refusal says its own reason, because they are four different
/// facts: a build has no later, a request does not outlive itself, a
/// store does not run, and `remembered` — the one that *is* on the
/// browser, so the clock could run — would persist a reading taken during
/// a visit that has ended. The `server` and `durable` messages
/// additionally name the construct the program was reaching for — a
/// *scheduled* state — rather than claiming timers are client-only, which
/// is true of the browser's clock and false of the thing they asked for.
#[test]
fn a_clock_outside_the_browser_is_rejected_for_its_own_reason() {
    for (placement, expected) in [
        ("static", "build time"),
        ("server", "scheduled"),
        ("durable", "scheduled"),
        ("remembered", "store"),
    ] {
        let source = format!(
            "state t is {placement} Decimal every \"1s\"\n\nview\n    Column\n        Text \"hi\"\n"
        );
        let (_, split) = compile(&source);
        assert!(
            codes(&split.diagnostics).contains(&"E0322"),
            "`{placement}` should be refused: {:?}",
            codes(&split.diagnostics)
        );
        let error = split
            .errors()
            .find(|e| e.code == "E0322")
            .expect("the refusal");
        assert!(
            error.message.contains(expected),
            "the `{placement}` message must say why, and said: {}",
            error.message
        );
        // Not also reported as "durable and derived": a clock is neither
        // `starting` nor `from`, and E0321's sentence would answer a
        // question this program did not ask.
        assert!(
            !codes(&split.diagnostics).contains(&"E0321"),
            "`{placement}` picked up E0321 as well"
        );
    }

    // And the one placement that has a clock is left alone.
    let (_, split) =
        compile("state t is client Decimal every \"1s\"\n\nview\n    Column\n        Text t\n");
    assert!(
        !codes(&split.diagnostics).contains(&"E0322"),
        "a `client` clock is the whole point: {:?}",
        codes(&split.diagnostics)
    );
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

    assert_table_is_total(&columns, &expected, cell);
}

/// Check a placement table cell by cell, and check that it is a *table*.
///
/// Both callers walked their hand-written rows with `zip`, which truncates
/// in silence: a missing context row or a missing placement column simply
/// went unchecked, and both tests claim in their own names to be the
/// spec's whole table. The write table was in fact missing
/// `Ctx::CLIENT_TRIGGER` entirely — every write from a client handler.
/// Row and column counts are pinned against `Ctx::ALL` here, so an
/// unlisted context fails rather than disappearing.
fn assert_table_is_total(
    columns: &[zdc_graph::SignalPlacement; 5],
    expected: &[(Ctx, [&str; 5])],
    cell: impl Fn(Ctx, zdc_graph::SignalPlacement) -> &'static str,
) {
    use zdc_graph::SignalPlacement as P;

    // Written out rather than derived from `columns`, so the header cannot
    // agree with itself.
    assert_eq!(
        columns,
        &[
            P::Client,
            P::Static,
            P::Server,
            P::Durable,
            P::DurablePerVisitor
        ],
        "the columns are the five placements, in the spec's order"
    );
    assert_eq!(
        expected.len(),
        Ctx::ALL.len(),
        "the table must have one row per context; `Ctx::ALL` has {}",
        Ctx::ALL.len()
    );

    // With as many rows as there are contexts, and every context present,
    // no context can be listed twice.
    let rows: Vec<Ctx> = expected.iter().map(|(ctx, _)| *ctx).collect();
    for ctx in Ctx::ALL {
        assert!(
            rows.contains(&ctx),
            "{ctx:?} has no row, so its whole line of the table is unchecked"
        );
    }

    let mut checked = 0;
    for (ctx, row) in expected {
        for (target, want) in columns.iter().zip(row.iter()) {
            assert_eq!(&cell(*ctx, *target), want, "{ctx:?} × {target:?}");
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        Ctx::ALL.len() * columns.len(),
        "every cell is checked, or the table is not a total function"
    );
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
        // This row was missing. A `set` inside a client handler — the most
        // ordinary write in the language — had no cell in the table this
        // test says is the spec's.
        (
            Ctx::CLIENT_TRIGGER,
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

    assert_table_is_total(&columns, &expected, cell);
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

/// §14C.3b's sub-requirement: `static` emits files as well as reading
/// them. What a build writes is a property of the state whose value it is,
/// so the three preconditions are checked at the declaration.
#[test]
fn only_a_static_text_value_may_be_emitted_to_a_usable_path() {
    let refused = |source: &str| -> Vec<String> {
        let (_, split) = compile(source);
        codes(&split.diagnostics)
            .into_iter()
            .map(str::to_string)
            .collect()
    };

    // The placement: only `static` has a value at build time to write.
    assert_eq!(
        refused("state a is client Text starting \"x\" emitting \"a.txt\"\n\nview\n    Text a\n"),
        vec!["E0314"]
    );
    // The type: a file's contents are text.
    assert_eq!(
        refused("state a is static Whole starting 1 emitting \"a.txt\"\n\nview\n    Text a\n"),
        vec!["E0315"]
    );
    // The path: a generated file goes in the bundle, so it cannot climb
    // out of it or name somewhere else entirely.
    for path in ["../out.txt", "/etc/passwd", "", "sub/", "C:/x"] {
        let source = format!(
            "state a is static Text starting \"x\" emitting \"{path}\"\n\nview\n    Text a\n"
        );
        assert_eq!(refused(&source), vec!["E0316"], "path was {path:?}");
    }

    // And the shape that is right is accepted.
    let source =
        "state a is static Text starting \"x\" emitting \"feeds/rss.xml\"\n\nview\n    Text a\n";
    assert!(refused(source).is_empty());
}

/// **A build capability is legal in build-time evaluation and nowhere
/// else, and that is not a permission — it is who is available to answer.**
///
/// `environment` is confined to server context because a *credential* must
/// not reach a browser (§5.6). This confinement is a different kind: the
/// compiler answers `build read` while it is compiling, so outside the
/// build there is no answerer at all. The two therefore get different
/// codes, E0360 and E0361, rather than one message that would be right
/// about the placement and wrong about the reason.
#[test]
fn a_build_capability_is_refused_everywhere_but_build_time_evaluation() {
    let refused = |source: &str| -> Vec<String> {
        let (_, split) = compile(source);
        codes(&split.diagnostics)
            .into_iter()
            .map(str::to_string)
            .collect()
    };

    // Read from the view: the browser has no filesystem and no compiler.
    assert_eq!(
        refused(concat!(
            "state a is client Text starting \"\"\n",
            "\n",
            "view\n",
            "    Text (build read \"x.md\")\n",
        )),
        vec!["E0361"]
    );

    // Read from a server signal: a `server` invocation happens after the
    // build has finished, so there is nothing left to ask either.
    assert_eq!(
        refused(concat!(
            "state seed is server Text starting \"\"\n",
            "state a is server Text from load with seed\n",
            "\n",
            "function load with source\n",
            "    give build read source\n",
            "\n",
            "view\n",
            "    when a\n",
            "        Loading           show Spinner\n",
            "        Failed with error show Text error.message\n",
            "        Ready with value  show Text value\n",
        )),
        vec!["E0361"]
    );

    // And in the one context that has an answerer, it is accepted.
    assert!(refused(concat!(
        "state a is static Text starting \"\"\n",
        "state b is static Text from load with a\n",
        "\n",
        "function load with source\n",
        "    give build markdown source\n",
        "\n",
        "view\n",
        "    Text b\n",
    ))
    .is_empty());
}

/// The path check is total over what a program can write, and it is done
/// on the *written* path rather than a resolved one — so no build ever
/// gets the chance to write outside the directory it was given.
#[test]
fn a_bundle_relative_path_is_the_only_kind_that_is_usable() {
    assert_eq!(zdc_graph::unusable_path("rss.xml"), None);
    assert_eq!(zdc_graph::unusable_path("feeds/posts.xml"), None);
    assert!(zdc_graph::unusable_path("../rss.xml").is_some());
    assert!(zdc_graph::unusable_path("a/../b").is_some());
    assert!(zdc_graph::unusable_path("./rss.xml").is_some());
    assert!(zdc_graph::unusable_path("/rss.xml").is_some());
    assert!(zdc_graph::unusable_path("\\rss.xml").is_some());
    assert!(zdc_graph::unusable_path("https:rss.xml").is_some());
    assert!(zdc_graph::unusable_path("").is_some());
}

// --- local bindings and the instantiation key (§17.4.10, §17.7) ------------

/// The same function twice: once with its intermediate named by
/// `with ... is ...`, once with the same expression written where it is
/// used.
const INLINED: &str = "\
state hits is server Whole starting 0
state seed is client Whole starting 1

function scaled with n
    give (hits * 2) + n

state shown is client Whole from scaled with n is seed

view
    Column
        Text shown
";

const BOUND: &str = "\
state hits is server Whole starting 0
state seed is client Whole starting 1

function scaled with n
    with doubled is hits * 2
    give doubled + n

state shown is client Whole from scaled with n is seed

view
    Column
        Text shown
";

/// Every `(DefId, RootId)` the fixpoint reached, named, sorted.
///
/// This *is* §17.7's instantiation key. Comparing the key sets of two
/// programs compares the monomorphisation the split performed.
fn instantiations(hir: &zdc_hir::Hir, split: &zdc_graph::TierSplit) -> Vec<String> {
    let mut out: Vec<String> = split
        .reached_by
        .keys()
        .map(|(def, root)| format!("{}@{:?}", hir.defs[*def].name, split.root(*root).ctx.kind))
        .collect();
    out.sort();
    out
}

/// Every crossing the split classified, as a sorted multiset.
///
/// Keyed on the `Crossing` alone rather than on the `ExprId`: the two
/// programs are different sources, so their expression arenas differ by
/// construction and only the verdicts are comparable.
fn crossing_kinds(split: &zdc_graph::TierSplit) -> Vec<String> {
    let mut out: Vec<String> = split
        .crossings
        .values()
        .map(|crossing| format!("{crossing:?}"))
        .collect();
    out.sort();
    out
}

/// Every endpoint a program derives, named, with its parameters.
fn endpoints(hir: &zdc_hir::Hir, split: &zdc_graph::TierSplit) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = split
        .endpoints
        .iter()
        .map(|endpoint| (endpoint.name.clone(), names(hir, endpoint.params.clone())))
        .collect();
    out.sort();
    out
}

/// **The question §17.7 leaves open, answered: a local binding creates no
/// new region-crossing site.**
///
/// §17.7 records that `Res::Local` never crosses a region, which is why
/// the split ignores locals entirely, and it names the consequence if that
/// ever stopped being true: the instantiation key would have to widen from
/// `(DefId, RootId)` to carry the placement vector of the arguments
/// (issue #21).
///
/// It does not stop being true here. A binding names a value; it declares
/// no placement, and `zdc-graph::sites` walks the bound expression in
/// exactly the position it would have occupied written out. So the
/// crossings a body performs are a function of the expressions in it and
/// not of whether one of them was given a name, which is what this
/// asserts: the same body is compiled both ways and what the fixpoint
/// produced is compared.
///
/// Neither side is empty. The read of `hits` from a client root is a real
/// `Remote` crossing, checked before the two are compared, so this is not
/// two absences agreeing.
#[test]
fn naming_an_intermediate_adds_no_region_crossing_site() {
    let (inlined_hir, inlined) = compile(INLINED);
    let (bound_hir, bound) = compile(BOUND);

    assert!(!inlined.has_errors(), "the inlined fixture must compile");
    assert!(!bound.has_errors(), "the bound fixture must compile");

    // The crossing under test exists at all. Without this the comparison
    // below would hold over two programs that crossed nothing.
    let remotes = bound
        .crossings
        .values()
        .filter(|crossing| matches!(crossing, Crossing::Remote { .. }))
        .count();
    assert_eq!(
        remotes,
        1,
        "the binding's value reads a `server` signal from a client root, \
         which is one `Remote` crossing: {:?}",
        crossing_kinds(&bound)
    );

    // The key set is the same set. A binding that carried a region of its
    // own would show up here as an instantiation the inlined program does
    // not have.
    assert_eq!(
        instantiations(&bound_hir, &bound),
        instantiations(&inlined_hir, &inlined),
        "naming an intermediate changed which `(DefId, RootId)` pairs the \
         fixpoint reached"
    );

    // The same verdicts, so the binding did not turn a `Direct` read into
    // a crossing of some other kind either.
    assert_eq!(crossing_kinds(&bound), crossing_kinds(&inlined));

    // And the network the two derive is one network: same roots, same
    // endpoints, same lifted parameters.
    assert_eq!(
        bound.emitted_roots().count(),
        inlined.emitted_roots().count()
    );
    assert_eq!(
        endpoints(&bound_hir, &bound),
        endpoints(&inlined_hir, &inlined)
    );
    assert!(
        !bound.endpoints.is_empty(),
        "a remote read generates an endpoint; comparing two empty lists proves nothing"
    );
}

/// The other half of the same claim: a local is not a *member* of a root.
///
/// Membership is what the split hands to codegen, and it is keyed by
/// `DefId`. A binding allocates a `LocalId` and no `DefId` at all, so
/// there is nothing for it to be a member of. That is the mechanical
/// reason the key never needed to widen, stated as a test rather than as
/// a claim about a data structure.
#[test]
fn a_local_binding_declares_nothing_a_root_can_hold() {
    let (hir, split) = compile(BOUND);
    let mut everything: Vec<String> = split
        .emitted_roots()
        .flat_map(|(root, _)| names(&hir, split.members_of(root).map(|(def, _)| def)))
        .collect();
    everything.sort();
    everything.dedup();

    assert!(
        !everything.is_empty(),
        "the fixture emits members; an empty list would pass the next assertion for free"
    );
    assert!(
        !everything.iter().any(|name| name == "doubled"),
        "`doubled` is a local, and a local has no `DefId` to be a member with: {everything:?}"
    );
    assert!(
        everything.iter().any(|name| name == "scaled"),
        "the function that holds the binding is a member, so the search above looked \
         somewhere real: {everything:?}"
    );
}

/// #13. Two instances of one component write the same signal at the same
/// span, and the two writes must stay distinct.
///
/// Instantiation copies a component's body per call site and **keeps the
/// spans**, while allocating fresh ids for everything else. `mutations_at`
/// used to be keyed on `(Span, Ctx, DefId)`: the span is shared by
/// construction, the context is the same for two siblings in one view, and
/// the signal is the same whenever both instances write the same top-level
/// state — so all three components of the key collided at once and
/// whichever the fixpoint recorded last answered for both.
///
/// This is the last of the span-aliasing family. The span-keyed map in
/// `ifc.rs` is deliberate and documented there: it de-duplicates
/// diagnostics rather than claiming identity.
#[test]
fn two_instances_of_one_component_keep_their_writes_apart() {
    let (_, split) = compile(
        "state votes is durable Whole starting 0\n\
         \n\
         component Bump\n\
         \x20   Button \"up\"\n\
         \x20       on click\n\
         \x20           add 1 to votes\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       Bump\n\
         \x20       Bump\n",
    );

    // Both instances are the same `add 1 to votes` line, so before the fix
    // they shared a key and the map held one entry for two writes.
    assert_eq!(
        split.mutations_at.len(),
        2,
        "one entry per instantiated write, not one per source line: {:?}",
        split.mutations_at
    );

    // And they are genuinely two keys rather than one key seen twice.
    let keys: std::collections::BTreeSet<_> = split.mutations_at.keys().collect();
    assert_eq!(keys.len(), 2, "the two writes share a key");

    // The asymmetry that made the old key wrong, asserted rather than
    // argued: `mutations` is keyed on `MutSite`, whose `owner` and
    // `ordinal` instantiation allocates fresh, and it saw two writes all
    // along. `mutations_at` disagreed because every field of its key —
    // span, context, signal — is shared by these two instances. The two
    // maps describe the same writes and must agree on how many there are.
    assert_eq!(
        split.mutations.len(),
        split.mutations_at.len(),
        "the sound map and the place-keyed map disagree about how many \
         writes exist, which is exactly the aliasing this guards"
    );
}

// ---------------------------------------------------------------------
// E0317 — where a `Handle` may be written.
//
// A handle refers to an object in one JavaScript heap. There is no wire
// form to decline to emit: what would be sent is an identity inside a
// running process. So the rule is a transcription of that fact — a handle
// is a `foreign`'s parameter or result, bare, and nothing else — and the
// tests below are one per position a value crosses or persists.
// ---------------------------------------------------------------------

/// One declaration per fixture, each putting a handle somewhere it would
/// have to travel, and the accepted shape at the end.
fn handle_codes(declaration: &str) -> Vec<&'static str> {
    let src = format!(
        "foreign vector is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes x is Decimal\n\
         \x20   gives new Handle\n\
         {declaration}\n\
         view\n\
         \x20   Column\n\
         \x20       Text \"hi\"\n"
    );
    let program =
        zdc_parser::parse(&src).unwrap_or_else(|e| panic!("fixture does not parse: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| {
            let joined: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            panic!("fixture does not resolve: {}", joined.join("; "))
        });
    split(&hir)
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.code)
        .collect()
}

/// The case §17 names outright: `Remote of Handle` asks for a host object
/// over the network.
#[test]
fn a_handle_may_not_be_written_under_remote() {
    assert!(handle_codes(
        "foreign later is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes x is Decimal\n\
         \x20   gives Remote of Handle"
    )
    .contains(&"E0317"));
}

/// Every container, for the same reason: an array of live objects has no
/// encoding either, and admitting one would mean writing a marshalling
/// rule for a value that has none.
#[test]
fn a_handle_may_not_be_written_inside_any_container() {
    for written in [
        "List of Handle",
        "Option of Handle",
        "Map of Text to Handle",
        "Pair of Handle to Whole",
    ] {
        let codes = handle_codes(&format!(
            "foreign many is client\n\
             \x20   from \"./three.module.js\" as \"Vector3\"\n\
             \x20   takes x is Decimal\n\
             \x20   gives {written}"
        ));
        assert!(codes.contains(&"E0317"), "`{written}` was accepted");
    }
}

/// **Derived state, at every placement.** A derived signal recomputes and
/// there is no `destroy` to run on the value it replaces, so a derived
/// handle signal drops a live WebGL context on every recomputation. That
/// is the whole of the reason `state` was refused outright, and it is a
/// reason about `from` rather than about `state`.
#[test]
fn a_derived_handle_signal_is_refused_at_any_placement() {
    for placement in ["client", "server", "durable", "static"] {
        let codes = handle_codes(&format!(
            "state kept is {placement} Handle from vector with x is 1"
        ));
        assert!(
            codes.contains(&"E0317"),
            "`{placement}` derived state was allowed to hold a handle: {codes:?}"
        );
    }
}

/// **A source signal at every placement but `client`.** These are refused
/// for the other reason: a `server`, `durable` or `static` signal is read
/// across a boundary by definition, and there is nothing to send.
#[test]
fn a_handle_may_not_be_state_anywhere_but_the_browser() {
    for placement in ["server", "durable", "static"] {
        let codes = handle_codes(&format!("state kept is {placement} Handle starting vector"));
        assert!(
            codes.contains(&"E0317"),
            "`{placement}` state was allowed to hold a handle: {codes:?}"
        );
    }
}

/// **The position this branch opens**, and the answer to #276's third
/// blocker: a renderer is acquired once and lives as long as the page.
///
/// A `client` signal declared `starting` is evaluated once, when the
/// bundle loads, and never recomputed — so the recompute argument above
/// does not apply to it, and refusing it was refusing more than the reason
/// supported.
#[test]
fn a_client_source_signal_may_hold_a_handle() {
    let codes = handle_codes("state gl is client Handle starting vector");
    assert!(
        !codes.contains(&"E0317"),
        "a handle acquired once, in the browser, was refused: {codes:?}"
    );
}

/// Acquiring once is only half of *never replaced*. A write puts a second
/// host object where the first was, with nothing having released the
/// first — the same leak a derived signal would have, written by hand.
#[test]
fn nothing_may_write_a_handle_signal() {
    for verb in ["set gl to vector", "append vector to gl"] {
        let codes = handle_codes(&format!(
            "state gl is client Handle starting vector\n\
             \x20   \n\
             function replace\n\
             \x20   {verb}\n\
             \x20   give 1"
        ));
        assert!(
            codes.contains(&"E0317"),
            "`{verb}` replaced a live handle: {codes:?}"
        );
    }
}

/// Reading one is not writing one, and this is the shape the feature
/// exists for: two handles acquired once and used together.
#[test]
fn a_handle_signal_may_be_read() {
    let codes = handle_codes(
        "foreign scene is client\n\
         \x20   from \"./three.module.js\" as \"Scene\"\n\
         \x20   gives new Handle\n\
         foreign addTo is client\n\
         \x20   on Handle as \"add\"\n\
         \x20   takes parent is Handle, child is Handle\n\
         \x20   gives nothing\n\
         state world is client Handle starting scene\n\
         state part is client Handle starting vector\n\
         function grow\n\
         \x20   do addTo with parent is world, child is part\n\
         \x20   give 1",
    );
    assert!(
        !codes.contains(&"E0317"),
        "reading a handle signal was refused: {codes:?}"
    );
}

/// A record is what crosses an endpoint, so a field cannot hold one.
#[test]
fn a_handle_may_not_be_a_record_field() {
    assert!(handle_codes("record Held\n\x20   what is Handle").contains(&"E0317"));
}

/// A `release` exists to move a value across the secrecy boundary, and an
/// opaque one cannot be looked at to decide whether that is safe.
#[test]
fn a_handle_may_not_be_what_a_release_gives() {
    assert!(handle_codes(
        "release leak\n\
         \x20   gives Handle\n\
         \x20   give vector with x is 1"
    )
    .contains(&"E0317"));
}

/// The positions that are admitted on a `foreign`'s own lines, so the rule
/// is a line and not a ban. Nothing here is refused.
#[test]
fn a_bare_handle_is_a_foreigns_parameter_and_result() {
    let codes = handle_codes(
        "foreign lengthOf is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes v is Handle\n\
         \x20   gives Decimal\n\
         state size is client Decimal from lengthOf with v is (vector with x is 1)",
    );
    assert!(
        !codes.contains(&"E0317"),
        "the one shape a handle is for was refused: {codes:?}"
    );
}

// ---------------------------------------------------------------------
// `on key "…"` and the region it needs — E0364, §16.3.7a.
// ---------------------------------------------------------------------

/// **The placement rule, stated over the whole region set.**
///
/// `on key` registers a listener on the browser's document and keeps it
/// until the region that wrote it is discarded. A build host has no
/// browser. A server has no browser *of its own*: it renders for one, so
/// a listener registered there would either not exist or belong to
/// whichever visitor's request happened to be in flight, which is worse
/// than not existing because it looks like it works.
///
/// Asserted over `Region::ALL` rather than over the two regions written
/// out by hand — the same discipline `static_is_the_one_placement_that_
/// reaches_the_build_artefact_sink` applies to `Placement::ALL`. A fourth
/// region added later has to answer this question rather than inherit an
/// answer, and `has_a_document`'s total match is what makes it a compile
/// error to add one without answering.
///
/// **The diagnostic site is defence in depth today, and saying so is the
/// point.** The splitter walks the view from `Ctx::CLIENT_VIEW` and from
/// nowhere else, and `on key` is a view node, so nothing a program can
/// write reaches E0364. The rule is load-bearing here, at the predicate;
/// `inline_budget.rs`'s `UNREACHABLE` table carries the same sentence,
/// which is the alternative to a fixture that only pretends to reach it.
#[test]
fn a_region_without_a_browser_may_not_hold_a_document_listener() {
    let allowed: Vec<Region> = Region::ALL
        .into_iter()
        .filter(|region| region.has_a_document())
        .collect();
    assert_eq!(
        allowed,
        vec![Region::Client],
        "exactly one region has a document of its own"
    );
}

/// The three regions a program has, so the test above is over the set and
/// not over a set that quietly shrank.
#[test]
fn every_region_is_in_the_region_list() {
    assert_eq!(Region::ALL.len(), 3);
    for region in Region::ALL {
        // A total match: adding a region without extending `ALL` leaves
        // this arm uncovered and the crate does not compile.
        match region {
            Region::Static | Region::Client | Region::Server => {}
        }
    }
}

/// A document key handler in a view is accepted, and it is one site the
/// split now records.
#[test]
fn a_document_key_handler_in_a_view_is_a_recorded_site() {
    let (hir, split) = compile(
        "state open is client Truth starting yes

view
    Column
        Text open
    on key \"Escape\"
        set open to no
",
    );
    assert!(
        !split.has_errors(),
        "a client view's key handler was refused: {:?}",
        split.errors().map(|e| e.code).collect::<Vec<_>>()
    );

    let view = hir.view.expect("the fixture has a view");
    let recorded: Vec<String> = zdc_graph::sites_of(&hir, view)
        .into_iter()
        .filter_map(|site| match site {
            zdc_graph::Site::DocumentKey { key, .. } => Some(key),
            zdc_graph::Site::Call { .. }
            | zdc_graph::Site::Read { .. }
            | zdc_graph::Site::Write { .. }
            | zdc_graph::Site::Bind { .. }
            | zdc_graph::Site::NotAPlace { .. }
            | zdc_graph::Site::Environment { .. }
            | zdc_graph::Site::ForeignCall { .. }
            | zdc_graph::Site::Media { .. }
            | zdc_graph::Site::Scroll { .. }
            | zdc_graph::Site::Outbound { .. }
            | zdc_graph::Site::Build { .. } => None,
        })
        .collect();
    assert_eq!(
        recorded,
        vec!["Escape".to_string()],
        "the key must reach the split, or the region rule has nothing to rule on"
    );
}
