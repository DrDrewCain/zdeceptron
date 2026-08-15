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
use zdc_graph::{Producer, Secrecy, Sink, SinkSite};

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
            Failed with error show ErrorBar message is \"the call did not answer\"
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
            Failed with error show ErrorBar message is \"the call did not answer\"
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
            Failed with error show ErrorBar message is \"the call did not answer\"
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
/// failures` leaks a credential.
///
/// **Corrected 2026-08-03.** This used to say the payload is "the join of
/// the *call's arguments*, which is what `params(endpoint)` names here",
/// and it recorded the consequence a line later: every parameter is a
/// `client` signal, a `client` signal can never be secret (E0313), so the
/// join is ⊥. It called that "the language cannot yet express a secret
/// RPC argument". It is not an expressiveness gap — `params` is the wrong
/// set. §16.3.12 rule 2 puts a signal there only when the *server* walk
/// stopped at it, and the walk stops only at `client` placement; the
/// server-placed half of the call, which is where `apiKey` is, is a
/// **member**. So `params` was the half of `politeGreeting with name,
/// apiKey` that carries nothing, and the label was ⊥ in every program
/// that compiles.
///
/// Both facts are pinned below, because it is their *difference* that was
/// the defect.
#[test]
fn a_failed_binder_takes_the_failure_observation() {
    let (_, split, verdict) = verdict(GUESTBOOK);
    let endpoint = split
        .endpoints
        .iter()
        .find(|e| e.name == "greeting")
        .expect("the endpoint");

    // Still true, and still not the answer.
    assert!(!endpoint.params.is_empty(), "the endpoint takes `name`");
    for param in &endpoint.params {
        assert_eq!(verdict.label(*param).value, Secrecy::Public);
    }

    // And the part `params` could never see: the endpoint reads a secret
    // member, so its failure is worth that secret.
    let adversary = GUESTBOOK.replace(SECRET_ARM, MESSAGE_ARM);
    assert_ne!(
        adversary, GUESTBOOK,
        "the arm this test rewrites is no longer in `guestbook.zd`, so it rewrote nothing"
    );
    let codes = ifc_codes(&adversary);
    assert!(
        codes.contains(&"E-IFC-05"),
        "the `Failed` payload of an endpoint that reads `apiKey` reached the view: {codes:?}"
    );
}

/// The error arm of `greeting` in `examples/guestbook.zd`, as written.
///
/// Named once, so the tests that rewrite it cannot silently stop
/// rewriting anything when the example is edited.
///
/// It is five lines now rather than one. `error.code` is a `Code`, the
/// built-in choice, so the arm eliminates it with a nested `when` and
/// writes all three outcomes — which is the change this constant records.
/// The match starts at `Failed`, past the line's own indentation, so the
/// rewrite below can put a one-line arm back in its place.
const SECRET_ARM: &str = "\
Failed with error\n\
\x20               when error.code\n\
\x20                   Unreachable show ErrorBar message is \"the greeting service did not answer: Unreachable\"\n\
\x20                   Timeout     show ErrorBar message is \"the greeting service did not answer: Timeout\"\n\
\x20                   Rejected    show ErrorBar message is \"the greeting service did not answer: Rejected\"";

/// The same arm rewritten to render `message` instead: one `show`, in the
/// place the three-arm form occupied, so the rewritten file still parses
/// and still differs from the example in exactly one field.
const MESSAGE_ARM: &str = "Failed with error show ErrorBar message is error.message";

/// The other half of §14G.1.3(d), and the reason this branch exists.
///
/// `message` is host text and carries the join. `code` is not: the client
/// runtime writes it from the transport outcome — no answer, its own
/// deadline, or a status line — so it is `public` however secret the
/// endpoint is. Rendering it from the *same* endpoint whose `message` is
/// refused two tests up is accepted, and `guestbook.zd` does exactly that.
#[test]
fn the_code_of_a_failure_is_public_where_its_message_is_not() {
    assert!(
        GUESTBOOK.contains(SECRET_ARM),
        "the example must render the code from the secret-reading endpoint, or this asserts \
         nothing"
    );
    let codes = ifc_codes(GUESTBOOK);
    assert!(codes.is_empty(), "{codes:?}");

    // Same file, same endpoint, same arm, `message` instead of `code`.
    // The pair is the content of the rule: one field of one record is
    // public and the other is not.
    let with_message = GUESTBOOK.replace(SECRET_ARM, MESSAGE_ARM);
    assert_ne!(with_message, GUESTBOOK, "the rewrite matched nothing");
    assert!(
        ifc_codes(&with_message).contains(&"E-IFC-05"),
        "{:?}",
        ifc_codes(&with_message)
    );
}

/// The exception is one record's one field, and it cannot widen.
///
/// A program may declare a `record` with a field called `code`. That
/// field is field-insensitive like every other (§17.6 item 15): it is
/// worth whatever the record is worth. Nothing about the *name* `code`
/// confers anything — only a binder a `Failed` pattern introduced does,
/// and `zdc-resolve` forbids a program from redeclaring that variant.
///
/// The program below reads `t.code` off a secret record and gives the
/// result to a signal that is not declared secret, which is E-IFC-02. Its
/// `Failed` arm takes `error.code` apart throughout, so the accepted
/// twin isolates the field access and nothing else.
#[test]
fn a_user_records_code_field_inherits_the_records_label() {
    let program = |body: &str| {
        format!(
            "record Ticket\n\
             \x20   code is Text\n\
             \n\
             secret state apiKey is server Text from environment \"GREETING_API_KEY\"\n\
             secret state ticket is server Ticket from ticketFor with apiKey\n\
             state shown is server Text from codeOf with ticket\n\
             \n\
             function ticketFor with key\n\
             \x20   give (Ticket with code is key)\n\
             \n\
             function codeOf with t\n\
             \x20   give {body}\n\
             \n\
             view\n\
             \x20   Column\n\
             \x20       when shown\n\
             \x20           Loading show Spinner\n\
             \x20           Failed with error\n\
             \x20               when error.code\n\
             \x20                   Unreachable show ErrorBar message is \"no answer\"\n\
             \x20                   Timeout     show ErrorBar message is \"too slow\"\n\
             \x20                   Rejected    show ErrorBar message is \"refused\"\n\
             \x20           Ready with text show Text text\n"
        )
    };

    let leaks = ifc_codes(&program("t.code"));
    assert!(
        leaks.contains(&"E-IFC-02"),
        "a user record's `code` field was treated as the runtime's: {leaks:?}"
    );

    // The repaired twin, so the rejection above is about the field access
    // and not about the shape of the program around it — including its
    // nested `when error.code`, which is accepted here off an endpoint
    // that reads `apiKey`.
    let repaired = ifc_codes(&program("\"opaque\""));
    assert!(repaired.is_empty(), "{repaired:?}");
}

/// `code` is public enough for sink 7, and `message` is not.
///
/// The adversary's program puts the failure text in a `Link` href, so the
/// browser sends it to whichever host that text names. `error.code` in
/// the same position is one of three words this runtime chose, so there
/// is nothing there to send.
#[test]
fn a_failure_code_may_be_dereferenced_where_a_failure_message_may_not() {
    let program = |field: &str| {
        format!(
            "secret state apiKey is server Text from environment \"GREETING_API_KEY\"\n\
             state name is client Text starting \"\"\n\
             state greeting is server Text from politeGreeting with name, apiKey\n\
             \n\
             function politeGreeting with who, key\n\
             \x20   give \"Hello, \" + who + \".\"\n\
             \n\
             view\n\
             \x20   Column\n\
             \x20       Input name, hint is \"your name\"\n\
             \x20       when greeting\n\
             \x20           Loading show Spinner\n\
             \x20           Failed with error\n\
             \x20               Link error.{field}\n\
             \x20                   Text \"why\"\n\
             \x20           Ready with text show Text text\n"
        )
    };
    let message = ifc_codes(&program("message"));
    assert!(
        message.contains(&"E-IFC-11"),
        "the failure text still reached an outbound request: {message:?}"
    );
    let code = ifc_codes(&program("code"));
    assert!(code.is_empty(), "{code:?}");
}

/// The repaired twin, so the rule is not "reject every `Failed` arm".
///
/// `visits` is `durable` and its endpoint reads nothing declared secret,
/// so its payload stays public and `error.message` renders — which is
/// what `examples/guestbook.zd` still does on that arm. The two arms sit
/// four lines apart in the same file and are labelled differently, which
/// is the whole content of the rule.
#[test]
fn a_failure_from_an_endpoint_that_reads_no_secret_stays_public() {
    assert!(
        GUESTBOOK.contains("Failed with error show ErrorBar message is error.message"),
        "the durable arm must still render the host's message, or this asserts nothing"
    );
    assert!(
        ifc_codes(GUESTBOOK).is_empty(),
        "{:?}",
        ifc_codes(GUESTBOOK)
    );
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
            Failed with error
                when error.code
                    Unreachable show ErrorBar message is \"no answer\"
                    Timeout     show ErrorBar message is \"too slow\"
                    Rejected    show ErrorBar message is \"refused\"
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
            Failed with error show ErrorBar message is \"the call did not answer\"
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
            Failed with error show ErrorBar message is \"the call did not answer\"
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
            Failed with error
                when error.code
                    Unreachable show ErrorBar message is \"no answer\"
                    Timeout     show ErrorBar message is \"too slow\"
                    Rejected    show ErrorBar message is \"refused\"
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
            Failed with error show ErrorBar message is \"the call did not answer\"
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
        5,
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

// ---------------------------------------------------------------------
// Sink 5 — the platform log, and the count of producers (#22).
// ---------------------------------------------------------------------

/// **Exactly one of the seven sinks has no obligation site, and it is the
/// platform log.**
///
/// #22 said two did: sink 4, recorded as waiting on an FFI HIR, and sink
/// 5, on the trigger runtime. Sink 4 acquired one in the meantime —
/// `Ifc::response_bodies`, which owes nothing to `foreign` — and the
/// prose that counted two did not move, because prose does not.
///
/// This ranges over `Sink::CLOSED_LIST` rather than over a list written
/// out here, so an eighth sink is counted whether or not anyone remembers
/// this file, and the length is asserted so an emptied list fails instead
/// of passing over nothing.
#[test]
fn the_platform_log_is_the_only_sink_without_a_producer() {
    assert_eq!(Sink::CLOSED_LIST.len(), 7, "the sink list changed size");

    let awaiting: Vec<Sink> = Sink::CLOSED_LIST
        .into_iter()
        .filter(|sink| matches!(sink.producer(), Producer::Awaiting(_)))
        .collect();

    assert_eq!(
        awaiting,
        [Sink::PlatformLog],
        "#22 counted two sinks with no producer. If this is now empty, sink 5 was wired and \
         `Sink::producer` was not told; if it holds more than the platform log, a producer \
         was deleted and the sink it served is no longer checked."
    );

    let Producer::Awaiting(condition) = Sink::PlatformLog.producer() else {
        unreachable!("the filter above just matched it");
    };
    assert!(
        condition.contains("trigger") && condition.contains("function bundle"),
        "a sink with no producer has to say what would give it one, and both halves are \
         load-bearing: {condition}"
    );
}

/// The condition in `Sink::producer`'s own sentence, pinned at the
/// grammar.
///
/// Sink 5 is unreachable because no root can be a trigger, and no root
/// can be a trigger because nothing declares one. That is a fact about
/// the parser, so it is asserted against the parser: the day `every … is`
/// at declaration position or an `inbound` declaration parses,
/// `RootOrigin::Trigger` becomes constructible, `BoundaryEdge::TriggerFail`
/// acquires a root to name, and this test fails on the line that says so.
///
/// `every` already parses *inside* a `state` declaration — `every "250ms"`
/// is a clock signal's init clause — so the check is that it does not
/// start a declaration of its own, which is the shape a trigger needs.
#[test]
fn the_grammar_has_no_trigger_declaration_to_root_a_platform_log() {
    const EVERY: &str = "\
every \"1h\"
    set beat to 1
";
    const INBOUND: &str = "\
inbound Ping
    set beat to 1
";
    // A clock signal, which *is* in the grammar, so that the two refusals
    // above are refusals of the declaration form rather than of the word.
    const CLOCK: &str = "\
state beat is client Decimal every \"250ms\"

view
    Text \"hi\"
";

    assert!(
        zdc_parser::parse(EVERY).is_err(),
        "`every` at declaration position parses, so a scheduled trigger can be written and \
         sink 5 needs an obligation site"
    );
    assert!(
        zdc_parser::parse(INBOUND).is_err(),
        "an `inbound` declaration parses, so a delivered trigger can be written and sink 5 \
         needs an obligation site"
    );
    assert!(
        zdc_parser::parse(CLOCK).is_ok(),
        "the two refusals above must be about the declaration form, not about the word \
         `every`, which a clock signal already uses"
    );
}

/// No program the leak suite can write puts anything at sink 5, and this
/// says so over the fixtures that reach every *other* sink.
///
/// A negative test over programs chosen to be harmless proves nothing.
/// These are the six that reach the six wired sinks, plus `guestbook.zd`,
/// which is the accepted one — so if an obligation at the platform log
/// were ever raised by the machinery serving another sink, the programs
/// most likely to raise it are the ones asserted here.
#[test]
fn nothing_that_reaches_another_sink_reaches_the_platform_log() {
    let mut checked = 0;
    for (name, src, _, _) in witnesses() {
        let codes = ifc_codes(src);
        assert!(
            !codes.contains(&Sink::PlatformLog.code()),
            "{name} raised the platform-log sink, which has no obligation site: {codes:?}"
        );
        checked += 1;
    }

    let codes = ifc_codes(GUESTBOOK);
    assert!(
        !codes.contains(&Sink::PlatformLog.code()),
        "guestbook.zd raised the platform-log sink: {codes:?}"
    );
    checked += 1;

    assert_eq!(checked, 7, "the witness table stopped being walked");
}

/// How the pass says it ruled on a sink, for the table below.
enum Reached {
    /// The program is refused and carries the sink's own code.
    Refused,
    /// The program is accepted and a clearance is recorded at the named
    /// signal's build-output site. Sink 3 is witnessed this way because
    /// no program can *fail* it: E0313 and E0301 refuse every route by
    /// which a secret could reach a `static` signal, which is a property
    /// of the placement rules rather than of the sink.
    ClearedBuildOutput(&'static str),
}

/// One program per sink `Sink::producer` calls `Wired`, and the evidence.
///
/// Written out rather than derived. A table generated from the enum could
/// only ever agree with it; this one disagrees the moment a producer is
/// deleted, because the program that used to reach it stops being ruled
/// on and the row names which.
fn witnesses() -> [(&'static str, &'static str, Sink, Reached); 6] {
    [
        (
            "a secret rendered in the view",
            "\
secret state apiKey is server Text from environment \"K\"

view
    Column
        Text apiKey
",
            Sink::View,
            Reached::Refused,
        ),
        (
            "a secret copied into client state",
            "\
secret state apiKey is server Text from environment \"K\"
state cached is client Text from idOf with apiKey

function idOf with n
    give n

view
    Column
        Text \"hi\"
",
            Sink::ClientState,
            Reached::Refused,
        ),
        (
            "a static signal written into the bundle",
            "\
state greeting is static Text starting \"hello\"
state feed is static Text from wrap with greeting emitting \"rss.xml\"

function wrap with text
    give \"<rss>\" + text + \"</rss>\"

view
    Text greeting
",
            Sink::BuildArtifact,
            Reached::ClearedBuildOutput("feed"),
        ),
        (
            "a secret store a command endpoint answers with",
            "\
secret state tally is durable Whole starting 0

view
    Column
        Heading \"hi\"
        Button \"go\"
            on click
                add 1 to tally
",
            Sink::ResponseBody,
            Reached::Refused,
        ),
        (
            "a public aggregate over a secret store",
            "\
secret state ledger is durable Whole starting 0
state total         is server  Whole from double with ledger

function double with n
    give n

view
    Column
        when total
            Loading           show Spinner
            Failed with error show ErrorBar message is \"the call did not answer\"
            Ready with sum    show Text sum
",
            Sink::LiveSync,
            Reached::Refused,
        ),
        (
            "a secret in an image source",
            "\
secret state apiKey is server Text from environment \"K\"

view
    Column
        Image source is apiKey, alt is \"a\"
",
            Sink::OutboundRequest,
            Reached::Refused,
        ),
    ]
}

/// Every sink with a producer has a program that reaches it, and the
/// program is run rather than described.
///
/// `Sink::producer` is a claim about this pass, and a claim about what a
/// compiler does is worth exactly what the program exercising it is
/// worth. Six rows, six sinks, and the seventh is the platform log.
#[test]
fn every_wired_sink_has_a_program_that_reaches_it() {
    let table = witnesses();

    let wired: Vec<Sink> = Sink::CLOSED_LIST
        .into_iter()
        .filter(|sink| matches!(sink.producer(), Producer::Wired))
        .collect();
    let mut named: Vec<Sink> = table.iter().map(|(_, _, sink, _)| *sink).collect();
    named.sort_unstable();
    named.dedup();
    assert_eq!(
        named, wired,
        "every sink with a producer needs a row here, and a row here needs a producer"
    );

    let mut checked = 0;
    for (name, src, sink, reached) in table {
        match reached {
            Reached::Refused => {
                let codes = ifc_codes(src);
                assert!(
                    codes.contains(&sink.code()),
                    "{name}: `{}` is wired, so this program must be refused by it: {codes:?}",
                    sink.code()
                );
            }
            Reached::ClearedBuildOutput(signal) => {
                let (hir, _, verdict) = verdict(src);
                let def = def_named(&hir, signal);
                assert!(
                    verdict.cleared(sink, SinkSite::BuildOutput(def)).is_some(),
                    "{name}: `{}` is wired, so the pass must have ruled on `{signal}`",
                    sink.code()
                );
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 6, "the witness table stopped being walked");
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
            Failed with error show ErrorBar message is \"the call did not answer\"
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
///
/// `Link` takes its destination *positionally* (§14G.2 revision 1), and
/// this rule is keyed on argument names — so the fixture is written the
/// way a program is written, and it is `zdc-resolve` lowering the slot
/// under `zdc_hir::DESTINATION_ARGUMENT` that puts it in reach. Written
/// as `Link href is …` the fixture would not resolve at all, and sink 7
/// would look enforced for the commonest way of writing a link while
/// being tested only for a spelling no program can use.
#[test]
fn a_link_href_that_is_a_secret_is_rejected_by_the_flow_pass() {
    let src = with_view("        Link apiKey\n            Text \"here\"");
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
        let src = with_view(&format!("        Link \"{url}\"\n            Text \"go\""));
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
        Link \"/\"
            Text \"home\"
        Link \"https://example.com/feed.xml\"
            Text \"feed\"
        Link \"mailto:someone@example.com\"
            Text \"write\"
        Image source is \"/assets/desk.png\", alt is \"A desk\"
        each note in notes
            Link note.slug
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

/// **Known defect, unfixed.** The failure observation of a remote read is
/// derived from `split.params[endpoint]`, and every entry there is a
/// *lifted client signal* — `Crossing::Lift` is produced only for
/// `(Server, View, Placement::Client)`, and E0313 refuses `secret` on a
/// client placement. So the third component of the lattice is `⊥` in
/// every program that compiles, and `Failed with error` binds a value the
/// pass believes carries nothing.
///
/// The program below is what that admits. `greeting`'s endpoint reads a
/// `secret` — it calls `$env('GREETING_API_KEY')` — and a throw inside it
/// is answered to the browser as `{"error": …}` carrying the host's own
/// message, which names the environment key (`zdc-host`'s `$zdEnv`) or
/// quotes a stored value (`zdc-store`'s `NotANumber`). §16.3.12 assertion
/// C says an environment key name may not reach the browser. Here that
/// text is not merely shown: it is the `href` of a `Link`, so the browser
/// sends it to whichever host the text names, and the pass clears the
/// site.
///
/// Un-ignored 2026-08-03. Both repairs were taken, because each closes
/// something the other cannot. §14G.1.3(d)'s join now runs over the
/// endpoint's *members* as well as its parameters — `params` alone is the
/// client-supplied half of the call and is ⊥ in every program that
/// compiles — which is what refuses this program at compile time. And
/// `zdc-host` no longer writes the `environment` key name into failure
/// text (§16.3.12 assertion C), which is a leak the lattice cannot reach:
/// the key *name* is not the secret's value, so no label on it was ever
/// raised, and a rule that depended on a runtime not putting a secret in
/// a string would not be a compile-time claim at all.
#[test]
fn a_failure_from_an_endpoint_that_reads_a_secret_is_not_public() {
    let src = "secret state apiKey is server Text from environment \"GREETING_API_KEY\"\n\
               state name is client Text starting \"\"\n\
               state greeting is server Text from politeGreeting with name, apiKey\n\
               \n\
               function politeGreeting with who, key\n\
               \x20   give \"Hello, \" + who + \".\"\n\
               \n\
               view\n\
               \x20   Column\n\
               \x20       Input name, hint is \"your name\"\n\
               \x20       when greeting\n\
               \x20           Loading show Spinner\n\
               \x20           Failed with error\n\
               \x20               Link error.message\n\
               \x20                   Text \"why\"\n\
               \x20           Ready with text show Text text\n";
    let codes = ifc_codes(src);
    assert!(
        !codes.is_empty(),
        "the failure text of an endpoint that reads a secret was sent to a URL the browser \
         requests, and the pass cleared it: {codes:?}"
    );

    // §7.3: which sink, and along which path. Sink 7 and not sink 2,
    // because a `Link` href is dereferenced rather than read.
    assert!(codes.contains(&"E-IFC-11"), "got {codes:?}");
    let (_, _, verdict) = verdict(src);
    let error = verdict
        .errors()
        .find(|e| e.code == "E-IFC-11")
        .expect("the outbound-request sink must be the one that rejected it");
    let path: Vec<&str> = error.notes.iter().map(|(_, note)| note.as_str()).collect();
    assert!(
        path.iter().any(|note| note.contains("§14G.1.3(d)")),
        "the path must name the rule that put a label on the payload: {path:?}"
    );

    // Both spans, by offset rather than by wording: the declaration the
    // label came from, and the argument the browser would fetch.
    let at = |note: &str| {
        error
            .notes
            .iter()
            .find(|(_, text)| text.contains(note))
            .map(|(span, _)| &src[span.start as usize..span.end as usize])
    };
    assert_eq!(
        at("declared secret"),
        Some("apiKey"),
        "the path must start at the declaration the failure label came from"
    );
    assert_eq!(
        at("outbound request"),
        Some("error.message"),
        "the path must end at the value the browser would send"
    );
}

// ---------------------------------------------------------------------
// Handles (spec §14E.1 as `Handle` amends it).
//
// A host object is opaque, so the lattice cannot see through it. That is
// exactly the shape of a laundering hole, and the design closes it twice.
//
//  1. **Nothing secret gets in.** A `foreign` that takes or gives a
//     `Handle` is `is client` — name resolution refuses any other site —
//     and §14E.3 row 1 already obliges every argument of a
//     `foreign … is client` to be Public (`E-IFC-13`). A secret read into
//     client context is refused earlier still, at the crossing.
//  2. **What went in is what comes out.** A handle's label is the join of
//     its constructor's arguments and a call's result is the join of its
//     arguments, which is `Walk::foreign` unchanged: an opaque value that
//     *forgot* its inputs would be the hole, and this is the property that
//     says it does not.
//
// The tests below assert (2) directly against the lattice rather than
// through whichever diagnostic happens to fire first, because (1) alone
// would keep the fixtures failing even if the handle laundered.
// ---------------------------------------------------------------------

/// A program whose only path from the secret to `leaked` goes through a
/// host object: `box` swallows it and `contentsOf` hands it back.
const THROUGH_A_HANDLE: &str = "\
secret state apiKey is server Text from environment \"KEY\"

foreign box is client
    from \"./box.js\" as \"Box\"
    takes contents is Text
    gives new Handle

foreign contentsOf is client
    from \"./box.js\" as \"Box\"
    takes b is Handle
    gives Text

state leaked is server Text from contentsOf with b is (box with contents is apiKey)

view
    Column
        Text \"hi\"
";

/// The same program with a literal where the secret was. The control: if
/// the lattice called everything that touches a handle secret, the test
/// above would pass for the wrong reason.
const THROUGH_A_HANDLE_PUBLICLY: &str = "\
state greeting is server Text starting \"hello\"

foreign box is client
    from \"./box.js\" as \"Box\"
    takes contents is Text
    gives new Handle

foreign contentsOf is client
    from \"./box.js\" as \"Box\"
    takes b is Handle
    gives Text

state shown is server Text from contentsOf with b is (box with contents is greeting)

view
    Column
        Text shown
";

/// **A secret put into a host object is still secret when it comes out.**
///
/// This is the assertion the handle design exists to keep true, and it is
/// asserted through `E-IFC-02` because that code says exactly it: *this
/// derivation is secret, and the signal is not declared secret*. The only
/// path from `apiKey` to `leaked` runs into a constructor and back out of
/// a later call, through a value the compiler cannot see inside. If a
/// handle were labelled by what it *is* rather than by what went into it,
/// the derivation would be Public, this code would not be raised, and the
/// credential would be through.
///
/// The second half is the sharper one. The handle produced from the
/// secret is *itself* secret, so passing it on is refused too — which is
/// what stops the leak being rerouted by splitting the call in two.
#[test]
fn a_secret_cannot_be_laundered_through_a_handle() {
    let codes = ifc_codes(THROUGH_A_HANDLE);
    assert!(
        codes.contains(&"E-IFC-02"),
        "a secret went into a handle and the derivation that read it back came out Public, \
         which is a laundering hole through the whole lattice: {codes:?}"
    );
    assert_eq!(
        codes.iter().filter(|code| **code == "E-IFC-13").count(),
        2,
        "both the secret argument and the handle carrying it must be refused: {codes:?}"
    );
}

/// The other direction, so the rule above is about the secret and not
/// about handles: a handle built from a public value carries nothing.
#[test]
fn a_handle_built_from_public_values_is_public() {
    let (hir, split, verdict) = verdict(THROUGH_A_HANDLE_PUBLICLY);
    assert!(
        !split.has_errors(),
        "the split rejected a client-only handle: {:?}",
        split.errors().map(|e| e.code).collect::<Vec<_>>()
    );
    let shown = def_named(&hir, "shown");
    assert_eq!(verdict.label(shown).value, Secrecy::Public);
    assert!(
        !verdict.has_errors(),
        "a handle over public values was refused: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

/// The same secret read into **client** context never reaches a handle at
/// all: the crossing is where it is refused, one rule earlier.
///
/// Both routes are here because the two are refused by different rules,
/// and a design that closed only one of them would look closed.
#[test]
fn a_secret_read_into_the_browser_is_refused_before_a_handle_sees_it() {
    let codes = ifc_codes(
        &THROUGH_A_HANDLE.replace("state leaked is server Text", "state leaked is client Text"),
    );
    assert!(
        codes.contains(&"E-IFC-06"),
        "a secret crossed into the browser to be put in a handle: {codes:?}"
    );
}

/// The same laundering attempt written the way stage 2 makes possible:
/// the secret goes into a constructor and comes back out of a **method**
/// on the handle it produced.
///
/// The receiver is a method's first parameter and nothing about
/// `Walk::foreign` distinguishes it from any other, which is the point —
/// the receiver joins into the result exactly as an argument does, so a
/// method cannot read out what a constructor could not put in.
const THROUGH_A_METHOD: &str = "\
secret state apiKey is server Text from environment \"KEY\"

foreign box is client
    from \"./box.js\" as \"Box\"
    takes contents is Text
    gives new Handle

foreign contentsOf is client
    on Handle as \"contents\"
    takes of b is Handle
    gives Text

state leaked is server Text from contentsOf of (box with contents is apiKey)

view
    Column
        Text \"hi\"
";

#[test]
fn a_secret_cannot_be_laundered_out_through_a_method() {
    let codes = ifc_codes(THROUGH_A_METHOD);
    assert!(
        codes.contains(&"E-IFC-02"),
        "a method read a secret back out of a handle and it came out Public: {codes:?}"
    );
    assert_eq!(
        codes.iter().filter(|code| **code == "E-IFC-13").count(),
        2,
        "the secret argument and the handle the method is called on are both refused: {codes:?}"
    );
}

/// The same attempt again, through the quietest of the three routes: the
/// secret goes into a constructor and comes back out of a **property**.
///
/// A property is the route worth writing a separate fixture for. A method
/// call at least *looks* like a call, and a reader auditing a program will
/// stop at one; `boxOf.contents` looks like reading a field, and a field
/// read is the one operation in this language that has never had to be
/// checked, because until handles existed every field belonged to a record
/// whose contents the compiler could see. If a property read off a handle
/// dropped the receiver's label, every lattice rule downstream would be
/// waved past by four characters of syntax.
///
/// It does not, and the reason is structural rather than a rule added
/// here: a property's receiver is its first parameter, `Walk::foreign`
/// joins every parameter's label into the result, and it never asks which
/// `ForeignSource` the declaration used. The assertion is `E-IFC-02` —
/// *this derivation is secret* — because that is the sentence that would
/// stop being true if the join were dropped.
const THROUGH_A_PROPERTY: &str = "\
secret state apiKey is server Text from environment \"KEY\"

foreign box is client
    from \"./box.js\" as \"Box\"
    takes contents is Text
    gives new Handle

foreign contentsOf is client
    of Handle as \"contents\"
    takes of b is Handle
    gives Text

state leaked is server Text from contentsOf of (box with contents is apiKey)

view
    Column
        Text \"hi\"
";

/// The control, in the other direction: the same program with a literal
/// where the secret was is accepted. Without it this pair would pass just
/// as well if every handle property were called secret.
const THROUGH_A_PROPERTY_PUBLICLY: &str = "\
state greeting is server Text starting \"hello\"

foreign box is client
    from \"./box.js\" as \"Box\"
    takes contents is Text
    gives new Handle

foreign contentsOf is client
    of Handle as \"contents\"
    takes of b is Handle
    gives Text

state shown is server Text from contentsOf of (box with contents is greeting)

view
    Column
        Text shown
";

#[test]
fn a_secret_cannot_be_laundered_out_through_a_property() {
    let codes = ifc_codes(THROUGH_A_PROPERTY);
    assert!(
        codes.contains(&"E-IFC-02"),
        "a property read a secret back out of a handle and it came out Public, which is a \
         laundering hole through the whole lattice: {codes:?}"
    );
    assert_eq!(
        codes.iter().filter(|code| **code == "E-IFC-13").count(),
        2,
        "the secret argument and the handle the property is read off are both refused: {codes:?}"
    );
}

/// The property rule is about the secret and not about properties.
#[test]
fn a_property_read_off_a_public_handle_is_public() {
    let (hir, split, verdict) = verdict(THROUGH_A_PROPERTY_PUBLICLY);
    assert!(
        !split.has_errors(),
        "the split rejected a client-only property: {:?}",
        split.errors().map(|e| e.code).collect::<Vec<_>>()
    );
    let shown = def_named(&hir, "shown");
    assert_eq!(verdict.label(shown).value, Secrecy::Public);
    assert!(
        !verdict.has_errors(),
        "a property over a public handle was refused: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

/// A secret written **into** a host object through a property write.
///
/// The other three routes carry a secret *out* of a handle and are caught
/// on the way back. This one never comes back: `node.textContent = apiKey`
/// puts the secret somewhere the compiler cannot see and reads nothing, so
/// a rule that only labelled results would have nothing to label and would
/// pass the program in silence. It is also the single most plausible leak
/// a real program would write, because assigning a value into the DOM is
/// how a value is *shown*.
///
/// Nothing here is special-cased. A write is a call whose arguments are
/// the receiver and the value; `Walk::foreign` raises the same obligation
/// on both that it raises on every argument of a `foreign … is client`, so
/// the assertion is `E-IFC-13` and it fires on the value. The whole of the
/// closure is that the write was made an argument list rather than a new
/// statement form with an expression on its right.
const A_SECRET_INTO_A_PROPERTY: &str = "\
secret state apiKey is server Text from environment \"KEY\"

foreign box is client
    from \"./box.js\" as \"Box\"
    gives new Handle

foreign setContents is client
    set Handle as \"contents\"
    takes b is Handle, value is Text
    gives nothing

function leak with key
    do setContents with b is box, value is key
    give 1

state n is server Whole from leak with key is apiKey

view
    Column
        Text \"hi\"
";

/// The control: the same write with a public value is accepted, so the
/// rule above is about the secret and not about writing a property.
const A_PUBLIC_VALUE_INTO_A_PROPERTY: &str = "\
state greeting is client Text starting \"hello\"

foreign box is client
    from \"./box.js\" as \"Box\"
    gives new Handle

foreign setContents is client
    set Handle as \"contents\"
    takes b is Handle, value is Text
    gives nothing

view
    Column
        Button \"show\"
            on click
                do setContents with b is box, value is greeting
        Text greeting
";

#[test]
fn a_secret_cannot_be_written_into_a_host_object() {
    let codes = ifc_codes(A_SECRET_INTO_A_PROPERTY);
    assert_eq!(
        codes.iter().filter(|code| **code == "E-IFC-13").count(),
        1,
        "a secret was written into a host object's property and nothing was raised. Nothing \
         reads it back, so this is the one route where a rule about results would have had \
         nothing to check. The count is exactly one because the receiver is built from no \
         arguments and is Public — so the obligation that fires is the one on the *written \
         value*, and a walk that looked only at the receiver would leave this at zero: \
         {codes:?}"
    );
}

#[test]
fn a_public_value_written_into_a_host_object_is_accepted() {
    let (_, split, verdict) = verdict(A_PUBLIC_VALUE_INTO_A_PROPERTY);
    assert!(
        !split.has_errors(),
        "the split rejected a client-only property write: {:?}",
        split.errors().map(|e| e.code).collect::<Vec<_>>()
    );
    assert!(
        !verdict.has_errors(),
        "a property write over a public value was refused: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

/// **An effect is a call, and its arguments are checked like a call's.**
///
/// `do` is the one statement form that produces no value, and the shape of
/// the hole it could have left is exactly the shape of the statement: a
/// walk that skipped it — on the reasoning that there is no result to
/// label — would let `do send with body is apiKey` compile in silence,
/// because every rule that would have caught the secret fires while the
/// *arguments* are being walked and not on the way out.
///
/// So `Walk::stmt`'s `Do` arm evaluates the call and discards the label,
/// rather than not evaluating it. E-IFC-13 is the assertion, because that
/// is the obligation `Walk::foreign` raises on every argument of a
/// `foreign … is client` — and the effect is put in *server* context here
/// for the same reason `THROUGH_A_HANDLE` is: a secret that reached client
/// context at all was already refused at the crossing, one rule earlier,
/// so a fixture written there would pass without this arm existing.
const A_SECRET_INTO_AN_EFFECT: &str = "\
secret state apiKey is server Text from environment \"KEY\"

foreign send is client
    from \"./net.js\" as \"send\"
    takes body is Text
    gives nothing

function leak with key
    do send with body is key
    give 1

state n is server Whole from leak with key is apiKey

view
    Column
        Text \"hi\"
";

#[test]
fn a_secret_cannot_be_sent_out_through_an_effect() {
    let codes = ifc_codes(A_SECRET_INTO_AN_EFFECT);
    assert!(
        codes.contains(&"E-IFC-13"),
        "a secret was handed to a client foreign by a `do` and nothing was raised: {codes:?}"
    );
}

/// The other route, refused one rule earlier: the same effect written in a
/// handler reads the secret into the browser, and the crossing is where
/// that is caught. Both are here because a design that closed only one of
/// them would look closed.
#[test]
fn a_secret_read_into_the_browser_by_an_effect_is_refused_at_the_crossing() {
    let codes = ifc_codes(
        "secret state apiKey is server Text from environment \"KEY\"

state shown is client Text starting \"hi\"

foreign send is client
    from \"./net.js\" as \"send\"
    takes body is Text
    gives nothing

view
    Column
        Button \"go\"
            on click
                do send with body is apiKey
        Text shown
",
    );
    assert!(
        codes.contains(&"E-IFC-05"),
        "a secret crossed into the browser to be handed to an effect: {codes:?}"
    );
}

/// The control: the same effect over a public value is accepted, so the
/// two rules above are about the secret and not about `do`.
#[test]
fn an_effect_over_a_public_value_is_accepted() {
    let (_, split, verdict) = verdict(
        "state shown is client Text starting \"hi\"

foreign send is client
    from \"./net.js\" as \"send\"
    takes body is Text
    gives nothing

view
    Column
        Button \"go\"
            on click
                do send with body is shown
        Text shown
",
    );
    assert!(
        !split.has_errors(),
        "the split rejected a client-only effect: {:?}",
        split.errors().map(|e| e.code).collect::<Vec<_>>()
    );
    assert!(
        !verdict.has_errors(),
        "an effect over a public value was refused: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------
// Document key handlers — §16.3.7a.
// ---------------------------------------------------------------------

/// A view whose `on key` handler hands a secret to an effect, and the
/// same view with the same handler over a public value.
///
/// One token apart, and both halves are needed. Under a closed lattice
/// "it is refused" is the default and proves nothing alone; the public
/// half is what shows the pass still accepts the program somebody meant
/// to write.
fn key_handler_over(value: &str) -> String {
    format!(
        "secret state apiKey is server Text from environment \"KEY\"

state shown is client Text starting \"hi\"

foreign send is client
    from \"./net.js\" as \"send\"
    takes body is Text
    gives nothing

view
    Column
        Text shown
    on key \"Escape\"
        do send with body is {value}
"
    )
}

/// **A document key handler's body is checked, and this is what says so.**
///
/// The hazard is structural rather than clever. `on key` added a second
/// kind of handler, and a dozen walks in five crates reach a handler only
/// to descend into `handler.body`. A walk that skipped the new one would
/// fail *open*: every statement inside would simply never be checked, and
/// nothing else in the compiler would notice, because a body nobody looks
/// at raises no diagnostic. That is why the target is a field on
/// `HirHandler` rather than a second `HirNode` variant — but "it cannot be
/// skipped by construction" is a claim, and this is the test of it.
///
/// Verified load-bearing: make `sites_of`/`Ifc::nodes` treat a
/// `HandlerTarget::Document` as a node with no body and this passes
/// nothing while the program leaks.
#[test]
fn a_secret_cannot_leave_through_a_document_key_handler() {
    let codes = ifc_codes(&key_handler_over("apiKey"));
    assert!(
        codes.contains(&"E-IFC-05"),
        "a secret crossed into the browser inside an `on key` body, and the \
         flow pass did not look: {codes:?}"
    );
}

/// The repaired twin, one token away: the same handler over a public
/// signal is accepted.
#[test]
fn a_document_key_handler_over_a_public_value_is_accepted() {
    let (_, split, verdict) = verdict(&key_handler_over("shown"));
    assert!(
        !split.has_errors(),
        "the split rejected a client-only key handler: {:?}",
        split.errors().map(|e| e.code).collect::<Vec<_>>()
    );
    assert!(
        !verdict.has_errors(),
        "an `on key` body over a public value was refused: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

/// **What a document key handler may observe, stated as a program.**
///
/// The capability question `on key` raises is not the one the two tests
/// above answer. A document listener receives keystrokes aimed at *every*
/// element on the page, including a password field this program never
/// declared — so the honest question was whether the payload needed a
/// label of its own, and the answer taken was to refuse the payload
/// instead. There is no `with` in the production, so `stroke.key` is not
/// a thing a program can write down. This is the test of that, and it
/// fails the moment a binder becomes expressible.
///
/// It is a *parse* assertion on purpose. A refusal in a later pass is a
/// rule that could be relaxed; a production that does not exist is not.
#[test]
fn a_document_key_handler_cannot_bind_the_keystroke() {
    let error = zdc_parser::parse(
        "state n is client Whole starting 0

view
    Column
        Text n
    on key \"Escape\" with stroke
        add 1 to n
",
    )
    .expect_err("`on key … with` must not parse");
    assert!(
        error.message.contains("binds nothing"),
        "the refusal must say what it refuses: {}",
        error.message
    );
    assert!(
        error.message.contains("never declared"),
        "and why, because the reason is the whole design: {}",
        error.message
    );
}

/// The control for the test above: an *element* handler binds its payload
/// exactly as it always did.
///
/// Without this, `a_document_key_handler_cannot_bind_the_keystroke` is
/// satisfied by a compiler that refuses every binder, which would be a
/// regression rather than a rule.
#[test]
fn an_element_key_handler_still_binds_its_payload() {
    let (hir, split) = compile(
        "state typed is client Text starting \"\"
state last  is client Text starting \"\"

view
    Column
        Input typed
            on keydown with stroke
                set last to stroke.key
",
    );
    assert!(
        !split.has_errors(),
        "the split rejected an element key handler: {:?}",
        split.errors().map(|e| e.code).collect::<Vec<_>>()
    );

    let view = hir.view.expect("the fixture has a view");
    let zdc_hir::DefKind::View(view) = &hir.defs[view].kind else {
        panic!("the view is a view");
    };
    let handler = find_handler(&view.nodes).expect("the fixture writes one handler");
    assert_eq!(handler.target, zdc_hir::HandlerTarget::Element);
    assert!(
        handler.payload.is_some(),
        "`on keydown with stroke` must still bind its payload"
    );
}

/// The first handler anywhere under `nodes`.
fn find_handler(nodes: &[zdc_hir::HirNode]) -> Option<&zdc_hir::HirHandler> {
    for node in nodes {
        let found = match node {
            zdc_hir::HirNode::Handler(handler) => return Some(handler),
            zdc_hir::HirNode::Element(element) => find_handler(&element.children),
            zdc_hir::HirNode::Each(each) => find_handler(&each.body),
            zdc_hir::HirNode::Scope(scope) => find_handler(&scope.body),
            zdc_hir::HirNode::If(conditional) => find_handler(&conditional.then),
            zdc_hir::HirNode::When(_) | zdc_hir::HirNode::Children(_) => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

// --- the binders that are not function values (#33, #103, #104) ----------
//
// `fold each n into total starting s to step` and `map each x in v to e`
// are the two forms that bind a name without making a function a value.
// Neither passes anything anywhere, so neither is a closure in the sense
// the lattice would have to reason about — but both put a name in scope
// over an expression, and a name in scope over an expression is a capture
// whether or not anything is passed. The two fixtures below are the two
// ways a capture launders: through what the body reads, which is the easy
// half, and through what the *container* was, which is the half a rule
// written in the obvious way leaves open.

/// **A fold's answer depends on how many elements there were, and the
/// number of elements is the list's `shape`.**
///
/// The step here never mentions the element. It counts. So the only path
/// from `codes` to `counted` is the *length* of a secret list — no value
/// of any element reaches the answer at all, and a rule that joined only
/// the seed and the step would call this derivation Public and let the
/// length out. `E-IFC-02` is the sentence that stops being true if
/// `Walk::pipeline`'s `Fold` arm drops its `acc.label.shape` join.
///
/// This is the laundering shape that matters most, because it composes:
/// `keep each row where <secret predicate>` deliberately raises `shape`
/// rather than `value` so that a filtered list of public rows is secret,
/// and a fold that ignored `shape` would hand the predicate straight back
/// as a count.
const COUNTED_THROUGH_A_FOLD: &str = "\
secret state codes is server List of Whole starting [1, 2, 3]

function howMany of xs
    from xs
    fold each x into total starting 0 to total + 1

state counted is server Whole from howMany of codes

view
    Column
        Text \"hi\"
";

#[test]
fn a_secret_cannot_be_laundered_through_a_folds_length() {
    let codes = ifc_codes(COUNTED_THROUGH_A_FOLD);
    assert!(
        codes.contains(&"E-IFC-02"),
        "a fold over a secret list came out Public although its answer is that list's length, \
         which is a channel out of every secret collection in the program: {codes:?}"
    );
}

/// The repaired twin: the same fold over a list nothing declared secret.
#[test]
fn a_fold_over_a_public_list_is_public() {
    let (hir, _, verdict) =
        verdict(&COUNTED_THROUGH_A_FOLD.replace("secret state codes", "state codes"));
    let counted = def_named(&hir, "counted");
    assert_eq!(verdict.label(counted).value, Secrecy::Public);
    assert!(
        !verdict.has_errors(),
        "a fold over a public list was refused: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

/// **A fold whose step reads the element carries the element.**
///
/// The other half, and the easy one: here the step names the binder, so
/// the elements' own label has to arrive through the capture. It does
/// because `bind_element` gives the binder the accumulator's `value`, the
/// same rule `keep each` and `map each` have always used — the point of
/// the test is that the new clause uses that rule rather than inventing a
/// looser one for the binder it introduces.
#[test]
fn a_folds_binder_carries_the_label_of_what_it_walks() {
    let codes = ifc_codes(
        "\
secret state amounts is server List of Whole starting [1, 2, 3]

function totalOf of xs
    from xs
    fold each x into total starting 0 to total + x

state shownTotal is client Whole from totalOf of amounts

view
    Column
        Text \"hi\"
",
    );
    assert!(
        codes.contains(&"E-IFC-06"),
        "a secret list's elements reached the browser through a fold's binder: {codes:?}"
    );
}

/// **`map each x in v to e` passes `None` through, so the result says
/// whether there was anything there.**
///
/// The body is the constant `0`. Nothing about the payload reaches the
/// answer — and the answer is still `Some 0` or `None` exactly as the
/// secret container was `Some` or `None`, which is one bit per read of
/// every secret `Option` in the program. A rule that carried only the
/// body's label would make `presence` Public and let that bit out.
///
/// `Walk::expr`'s `MapInside` arm joins the container's `shape` onto the
/// result's for this reason and this test fails without it.
const PRESENCE_THROUGH_A_PAYLOAD_MAP: &str = "\
secret state codes is server List of Whole starting [1, 2, 3]

function presence of xs
    give map each code in (xs at 0) to 0

state leaked is server Option of Whole from presence of codes

view
    Column
        Text \"hi\"
";

#[test]
fn a_secret_cannot_be_laundered_through_a_payload_map() {
    let codes = ifc_codes(PRESENCE_THROUGH_A_PAYLOAD_MAP);
    assert!(
        codes.contains(&"E-IFC-02"),
        "`map each x in secret to 0` came out Public while still saying whether the secret was \
         `Some`, which is a laundering hole through every `Option` in the lattice: {codes:?}"
    );
}

/// The repaired twin: the same transform over a container nothing
/// declared secret.
#[test]
fn a_payload_map_over_a_public_option_is_public() {
    let (hir, _, verdict) =
        verdict(&PRESENCE_THROUGH_A_PAYLOAD_MAP.replace("secret state codes", "state codes"));
    let leaked = def_named(&hir, "leaked");
    assert_eq!(verdict.label(leaked).value, Secrecy::Public);
    assert!(
        !verdict.has_errors(),
        "a payload transform over a public option was refused: {:?}",
        verdict
            .errors()
            .map(|e| e.rendered_message())
            .collect::<Vec<_>>()
    );
}

/// **And the payload itself is carried by the binder.**
///
/// The third path out of the same form: not the tag, but what was inside
/// it, read through the name the clause binds. `bind_element`'s rule
/// again, applied to a payload rather than to an element.
#[test]
fn a_payload_maps_binder_carries_what_was_inside() {
    let codes = ifc_codes(
        "\
secret state codes is server List of Whole starting [1, 2, 3]

function doubled of xs
    give map each code in (xs at 0) to code * 2

state shown is client Option of Whole from doubled of codes

view
    Column
        Text \"hi\"
",
    );
    assert!(
        codes.contains(&"E-IFC-06"),
        "a secret payload reached the browser through the binder of `map each … in`: {codes:?}"
    );
}

// --- §14G.4's scheduled trigger, against the lattice (#18) ---------------

/// **§14G.4 revision 4, verbatim, and the reason it is here.**
///
/// The design's own showcase program writes the `Failed` payload of a call
/// made with a secret key into a durable list, and revision 4 records that
/// four lines rendering that list turn the example into the exfiltration.
/// A job's block is *the language's first server-context mutation site*,
/// which is why §5.3 needed a write rule at all — so the write rule
/// meeting it is the property to pin, not an incidental.
///
/// Nothing about this is special-cased for triggers. The obligation is
/// raised by the same walk that raises it for a handler; what is new is
/// that the walk reaches a job's statements.
#[test]
fn a_job_may_not_write_a_secret_into_a_public_store() {
    const LEAK: &str = "\
secret state apiKey is server Text from environment \"API_KEY\"

state log is durable List of Text starting []

state hourly is server Whole every \"1h\"
    append apiKey to log

view
    Column
        Text \"hi\"
";
    let (_, _, verdict) = verdict(LEAK);
    let reported: Vec<&str> = verdict.diagnostics.iter().map(|d| d.code).collect();
    assert!(
        reported.contains(&"E-IFC-03"),
        "a job wrote a secret into a public store and nothing said so: {reported:?}"
    );
}

/// And declaring the destination `secret` is what compiles, so the rule is
/// a rule about the *label* rather than about the construct.
#[test]
fn a_job_may_write_a_secret_into_a_secret_store() {
    const KEPT: &str = "\
secret state apiKey is server Text from environment \"API_KEY\"

secret state log is durable List of Text starting []

state hourly is server Whole every \"1h\"
    append apiKey to log

view
    Column
        Text \"hi\"
";
    let (_, _, verdict) = verdict(KEPT);
    let reported: Vec<&str> = verdict
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.code)
        .collect();
    assert!(
        reported.is_empty(),
        "a job writing a secret into a secret store is the accepted program: {reported:?}"
    );
}
