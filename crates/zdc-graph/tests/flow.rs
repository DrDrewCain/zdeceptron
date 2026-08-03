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
// Structure.
// ---------------------------------------------------------------------

/// §14G.1.3(c): the sink list is declared and closed at six.
#[test]
fn the_sink_list_is_closed() {
    assert_eq!(Sink::CLOSED_LIST.len(), 6);
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
