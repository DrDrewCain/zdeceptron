//! The interprocedural argument-authority fixpoint, and the sites on it.
//!
//! Three things are being tested, and they are different in kind:
//!
//! * that the **fixpoint terminates and is interprocedural** — a function
//!   summary is a property of a body, not of its call sites, and recursion
//!   is not a special case;
//! * that the **obligation sites fire** — A1, A2, A3 and A5 each have a
//!   program here that raises them and would stop raising them if the site
//!   were deleted;
//! * that **REL-ARG fires where §19.10.1 says it does and nowhere else**.
//!
//! Nothing here tests that a program is free of laundering.
//! `launder3_is_rejected_which_closes_r1` records what the repair of §21.9
//! reaches, and `an_asserted_purity_marker_still_launders_and_that_is_r5_not_r1`
//! records what it does not: a human may still assert `gives pure T` about
//! JavaScript that reads the request URL, and the compiler has nothing to
//! say against it.

mod support;

use support::*;
use zdc_graph::authority::{authority, Analysis, ObligationSite};
use zdc_graph::integrity::{Authority, Grant};

fn codes(analysis: &Analysis) -> Vec<&str> {
    analysis.errors().map(|e| e.code).collect()
}

fn sites(analysis: &Analysis, site: ObligationSite) -> Vec<&zdc_graph::authority::Obligation> {
    analysis.at(site).collect()
}

// ---------------------------------------------------------------------
// Termination.
// ---------------------------------------------------------------------

const RECURSIVE: &str = r#"
function countDown with n
    if n is 0
        give 0
    give countDown with n is n - 1

state total is client Whole from countDown with n is 5

view
    Column
        Text total
"#;

/// A recursive function terminates, and lands on the right answer.
///
/// The lattice is two points and every transfer function here is built
/// from `⊔` and substitution alone, both monotone, so each summary moves
/// only upward and can move at most `params + 1` times. A cycle in the
/// call graph is therefore not a special case: `countDown` simply stops
/// changing. If the worklist ever loses that property this test hangs
/// rather than failing, which is the honest failure mode for a fixpoint.
#[test]
fn a_recursive_function_terminates() {
    let (hir, split) = compile(RECURSIVE);
    let analysis = authority(&hir, &split);
    // Its only base case is a literal, and the recursive call carries
    // nothing else in, so the least fixpoint is Trusted — which is true of
    // a function that can only ever return `0`.
    assert_eq!(
        analysis.solution.signal(def_named(&hir, "total")).0,
        Authority::Trusted
    );
}

const MUTUAL: &str = r#"
function isEven with n
    if n is 0
        give yes
    give isOdd with n is n - 1

function isOdd with n
    if n is 0
        give no
    give isEven with n is n - 1

state answer is client Truth from isEven with n is 4

view
    Column
        Text "x"
"#;

/// A mutually recursive pair terminates. Same argument, two definitions.
#[test]
fn mutually_recursive_functions_terminate() {
    let (hir, split) = compile(MUTUAL);
    let analysis = authority(&hir, &split);
    assert_eq!(
        analysis.solution.signal(def_named(&hir, "answer")).0,
        Authority::Trusted
    );
}

// ---------------------------------------------------------------------
// The fixpoint is interprocedural, and that is the point of it.
// ---------------------------------------------------------------------

const LAUNDERS_THROUGH_A_HELPER: &str = r#"
foreign header is server
    from  "./request" as "header"
    takes name is Text
    gives Text

function whoIsIt with key
    give header with name is key

state caller is server Text from whoIsIt with key is "x-user"

view
    Column
        Text "x"
"#;

/// **The hole the fixpoint closes.**
///
/// `whoIsIt` is called with a string literal, so under "a function is
/// transparent — it computes from its arguments" its result was the join
/// of its arguments, which is Trusted. But its body reaches a
/// `foreign … is server` with no `gives trusted T`, which is Untrusted by
/// the replacement to §18.1 semantics 6, and the value that comes back is
/// whatever the request headers held. One indirection was enough to turn
/// an Untrusted value Trusted, and no rule in the grant set was violated
/// to do it — the rule was simply never asked about the callee's body.
#[test]
fn a_helper_does_not_launder_an_ungranted_foreign() {
    let (hir, split) = compile(LAUNDERS_THROUGH_A_HELPER);
    let analysis = authority(&hir, &split);
    assert_eq!(
        analysis.solution.signal(def_named(&hir, "caller")).0,
        Authority::Untrusted,
        "a function's result is what its body computes, not what its arguments were"
    );
}

const TWO_CALL_SITES: &str = r#"
state typed is client Text starting ""

function shout with word
    give word + "!"

state fromBox is server Text from shout with word is typed
state fromLiteral is server Text from shout with word is "hello"

view
    Column
        Input typed, hint is "say something"
"#;

/// **The summary is relational, and that is why there is no second
/// interleaved fixpoint.**
///
/// `shout` is called twice, once with a text box and once with a literal.
/// Had the analysis merged one authority per parameter and fed it back
/// into the result, the text box would have poisoned the literal call and
/// `fromLiteral` would read Untrusted — E-REL-08 would then fire at call
/// sites where nothing attacker-chosen flows. Recording the result as a
/// join over *parameter positions* keeps the two apart, and it is what
/// lets fixpoint 2 sit above fixpoint 1 instead of inside it.
#[test]
fn one_untrusted_call_site_does_not_poison_another() {
    let (hir, split) = compile(TWO_CALL_SITES);
    let analysis = authority(&hir, &split);
    assert_eq!(
        analysis.solution.signal(def_named(&hir, "fromBox")).0,
        Authority::Untrusted
    );
    assert_eq!(
        analysis.solution.signal(def_named(&hir, "fromLiteral")).0,
        Authority::Trusted
    );
}

/// Fixpoint 2 merges the other way, because a body is checked once against
/// the worst of its callers.
///
/// `shout`'s own `word` is Untrusted, because one call site passes a text
/// box. That is the sound merge: there is one body and it must hold for
/// every caller.
#[test]
fn a_parameter_is_the_join_of_every_call_site() {
    let (hir, split) = compile(TWO_CALL_SITES);
    let analysis = authority(&hir, &split);
    assert_eq!(
        analysis.param(def_named(&hir, "shout"), 0),
        Authority::Untrusted
    );
}

// ---------------------------------------------------------------------
// REL-ARG / E-REL-08.
// ---------------------------------------------------------------------

/// §21.8.4's counterexample, with the R2 repair in place.
///
/// The two probes are two-way-bound `Input`s with no `set` anywhere. G-SIG
/// as §21.7.3 wrote it made them Trusted and §19.9's attack ran with one
/// benign self-endorsement; counting a binding as a write site makes them
/// Untrusted, and REL-ARG then asks the author to sign for them.
const PROBES: &str = r#"
record Card
    holder is Text
    number is Text

secret state cards is durable List of Card starting empty

state probeHolder is client Text starting ""
state probePrefix is client Text starting ""

release digitOracle with all, holder, prefix
    gives Whole
    trusted all
    limit 10 per visitor
    give 0

state hits is server Whole from digitOracle with all is cards, holder is probeHolder, prefix is probePrefix

view
    Column
        Input probeHolder, hint is "holder"
        Input probePrefix, hint is "prefix"
        Button "add"
            on click
                append "x" to cards
"#;

/// **E-REL-08.** Two unendorsed arguments the browser chose.
#[test]
fn e_rel_08_fires_on_an_unendorsed_untrusted_argument() {
    let (hir, split) = compile(PROBES);
    let analysis = authority(&hir, &split);
    assert_eq!(codes(&analysis), ["E-REL-08", "E-REL-08"]);
}

/// The diagnostic prints two spans and names the rule, because a reviewer
/// has to be able to find both ends of the flow and know which rule sent
/// them there.
#[test]
fn e_rel_08_prints_two_spans_and_names_the_rule() {
    let (hir, split) = compile(PROBES);
    let analysis = authority(&hir, &split);
    let first = analysis
        .errors()
        .find(|e| e.code == "E-REL-08")
        .expect("expected E-REL-08");

    assert!(first.message.contains("REL-ARG"), "{}", first.message);
    assert_eq!(first.notes.len(), 1, "the argument, and the declaration");
    assert_ne!(
        first.span, first.notes[0].0,
        "the two spans must be different places in the file"
    );
    // falsifiable: neither arm is unconditional. `PROBES` passes both
    // `holder` and `prefix` untrusted to the same release, and which of the
    // two is reported *first* is the order `errors()` yields, which is
    // source order — so a message naming neither parameter fails, and that
    // is the regression this guards: E-REL-08 naming the release rather
    // than the argument that raised it.
    assert!(first.message.contains("holder") || first.message.contains("prefix"));
    assert!(
        first
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("trusted"),
        "the repair is the exact `trusted <param>` line"
    );
}

const ENDORSED: &str = r#"
state typed is client Text starting ""

release judge with guess
    gives Truth
    trusted guess
    limit 10 per visitor
    give yes

state verdict is server Truth from judge with guess is typed

view
    Column
        Input typed, hint is "guess"
"#;

/// An endorsement discharges REL-ARG at this release's sites, and nowhere
/// else.
#[test]
fn an_endorsement_discharges_rel_arg() {
    let (hir, split) = compile(ENDORSED);
    let analysis = authority(&hir, &split);
    assert!(
        codes(&analysis).is_empty(),
        "an endorsed parameter raises no E-REL-08: {:?}",
        codes(&analysis)
    );
}

/// **Obligation site A5.** The endorsement is *counted*, not merely
/// obeyed.
///
/// This is the whole reason A5 exists (§19.10.4): the site is discharged
/// trivially by the declaration, and it is in the enum so that a human's
/// signature is enumerable rather than being an absence. Delete the site
/// and this test fails while every diagnostic still passes, which is the
/// failure mode it is here to catch.
#[test]
fn a5_counts_the_endorsement() {
    let (hir, split) = compile(ENDORSED);
    let analysis = authority(&hir, &split);
    let raised = sites(&analysis, ObligationSite::A5);
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0].found, Authority::Untrusted);
    assert_eq!(raised[0].required, Authority::Trusted);
    assert_eq!(raised[0].discharged_by, Some(Grant::Release));
}

/// §19.10.3(a): an endorsement is result-transparent. It discharges
/// REL-ARG at this release's call sites and raises nothing inside the
/// body, because raising the label inside would make four lines a
/// universal integrity launderer.
#[test]
fn an_endorsement_does_not_raise_the_label_inside_the_body() {
    let (hir, split) = compile(ENDORSED);
    let analysis = authority(&hir, &split);
    assert_eq!(
        analysis.param(def_named(&hir, "judge"), 0),
        Authority::Untrusted,
        "`trusted guess` grants nothing inside `judge`"
    );
}

// ---------------------------------------------------------------------
// §21.8.1's `launder3.zd`, which compiles.
// ---------------------------------------------------------------------

const LAUNDER3: &str = r#"
record Card
    holder is Text
    number is Text

secret state cards is durable List of Card starting empty

foreign queryParam is anywhere
    from  "./request" as "queryParam"
    takes name is Text
    gives Text

state shownHolder is client Text from queryParam with name is "holder"

release digitOracle with all
    gives Whole
    trusted all
    limit 10 per visitor
    give queryParam with name is "prefix"

state hits is server Whole from digitOracle with all is cards

view
    Column
        Text shownHolder
        Button "add"
            on click
                append "x" to cards
"#;

/// **Residual risk R1, closed. This test used to assert the opposite.**
///
/// This is §21.8.1's `launder3.zd`: §19.11.1 with `is server` changed to
/// `is anywhere`. The change makes the declaration *more* truthful, not
/// less — §14E.2's own heading asks *"which output bundles may this
/// library be linked into?"*, and for a query-string read the honest
/// answer is both, which makes `is anywhere` the only legal spelling once
/// the value is also shown on the page.
///
/// It used to type-check: fifteen checks, zero fire. G-FGN-A gave
/// `queryParam` the join of its arguments — a string literal, hence
/// Trusted — and REL-PURE was satisfied by the same word, so a visitor
/// steered the declassification with a query string and the reviewer's
/// short list was empty.
///
/// **Nothing about the declaration changed to close it.** `is anywhere` is
/// still there and is still the only honest spelling; the file below is
/// byte for byte what it was. What changed is that `is anywhere` no longer
/// answers a question it was never asked. `queryParam`'s `gives` line
/// carries no marker, so the foreign is Untrusted, `shownHolder` is
/// Untrusted, and REL-PURE refuses the release at the declaration.
///
/// **What still gets through, stated rather than left to be discovered.**
/// An author who writes `gives pure Text` on `queryParam` compiles this
/// program again. That is a false claim about JavaScript, sitting on a
/// conspicuous declaration, and nothing in the compiler can contradict it
/// — §14E.4's dev-mode check reads the shape of a return value and cannot
/// read purity. The repair moved the claim from *inferred* to *declared*.
/// It did not make it checkable, and no diagnostic here may imply that it
/// did.
#[test]
fn launder3_is_rejected_which_closes_r1() {
    let (hir, split) = compile(LAUNDER3);
    let analysis = authority(&hir, &split);

    assert!(
        codes(&analysis).contains(&"E-REL-10"),
        "REL-PURE must refuse `digitOracle` at the declaration: {:?}",
        codes(&analysis)
    );
    let rel_pure = zdc_graph::integrity::rel_pure(&hir, def_named(&hir, "digitOracle"));
    assert_eq!(rel_pure.len(), 1);
    assert!(rel_pure[0].message.contains("queryParam"));
    assert!(
        rel_pure[0].message.contains("REL-PURE"),
        "the diagnostic must name the rule: {}",
        rel_pure[0].message
    );

    assert_eq!(
        analysis.solution.signal(def_named(&hir, "shownHolder")).0,
        Authority::Untrusted,
        "a query-string read is Untrusted unless a human declares otherwise"
    );
}

const LAUNDER3_ASSERTED_PURE: &str = r#"
record Card
    holder is Text
    number is Text

secret state cards is durable List of Card starting empty

foreign queryParam is anywhere
    from  "./request" as "queryParam"
    takes name is Text
    gives pure Text

state shownHolder is client Text from queryParam with name is "holder"

release digitOracle with all
    gives Whole
    trusted all
    limit 10 per visitor
    give queryParam with name is "prefix"

state hits is server Whole from digitOracle with all is cards

view
    Column
        Text shownHolder
        Button "add"
            on click
                append "x" to cards
"#;

/// **The attempt to break the repair, kept as a test rather than as a
/// paragraph.**
///
/// `launder3.zd` with `gives pure Text` asserted on the query-string
/// reader compiles, and the leak of §21.8.1 runs unchanged. This is the
/// evasion, it is the only one found, and it is not a defect in the rule:
/// it is residual risk **R5** — *"`gives trusted T` and `is anywhere` are
/// asserted about third-party JavaScript and checked by nobody"* — with
/// `gives pure T` added to the list of things nobody checks.
///
/// The difference from R1 is the whole of what §21.9 bought, and it is
/// worth stating exactly: before, the author wrote the *only true* answer
/// to §14E.2's question and the compiler read a purity claim out of it.
/// Now the author has to write a claim that is false, in a slot that
/// exists for nothing else, on a line a reviewer reads.
#[test]
fn an_asserted_purity_marker_still_launders_and_that_is_r5_not_r1() {
    let (hir, split) = compile(LAUNDER3_ASSERTED_PURE);
    let analysis = authority(&hir, &split);
    assert!(
        codes(&analysis).is_empty(),
        "an asserted marker is a human's word and the compiler has nothing to say against it: \
         {:?}",
        codes(&analysis)
    );
    assert_eq!(
        analysis.solution.signal(def_named(&hir, "shownHolder")).0,
        Authority::Trusted
    );
    // The read of `shownHolder` is G-SIG clause 2 — no writer, and an
    // initialiser that joins to Trusted. What made that initialiser Trusted
    // is G-FGN-P, awarded one level down at the `foreign` declaration, and
    // that is the honest limit of what a per-signal grant can attribute
    // (§19.5: the completeness claim is about declarations, not
    // expressions).
    assert_eq!(
        analysis.solution.signal(def_named(&hir, "shownHolder")).1,
        Some(Grant::Signal)
    );
    assert!(Grant::ForeignPure.is_asserted());
}

// ---------------------------------------------------------------------
// A1, A2, A3 — the obligation sites §18.1 semantics 8 closes.
// ---------------------------------------------------------------------

const IDOR: &str = r#"
trusted state orders is durable Map of Text to Text starting empty

state candidate is client Text starting ""

state mine is server Text from pick with key is candidate

function pick with key
    give orders at key

view
    Column
        Input candidate, hint is "order id"
"#;

/// **A1 / E-INT-02.** IDOR, caught.
///
/// The index is a text box and the collection is declared `trusted`, so
/// the browser chooses whose row comes back. The obligation is discharged
/// over the index expressions of a place, which is why `durable per
/// visitor` reaches it and an opaque partition key does not (§20.4 T2).
///
/// Note what makes this test interprocedural: `candidate` reaches `key`
/// through a call, so the site inside `pick` can only be ruled on once
/// fixpoint 2 has merged the argument onto the parameter. Note also what
/// makes it grammatical: A1 is discharged over the index expressions of a
/// **place**, and §4.4 gives `place := IDENT (("at" primary) | ("." IDENT))*`
/// — so the collection has to be named, which is the premise §20.4's T2
/// rests on and the reason an opaque partition key never reaches this site.
#[test]
fn a1_fires_on_an_untrusted_index_into_a_trusted_place() {
    let (hir, split) = compile(IDOR);
    let analysis = authority(&hir, &split);
    let raised = sites(&analysis, ObligationSite::A1);
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0].found, Authority::Untrusted);
    assert!(codes(&analysis).contains(&"E-INT-02"));
}

const IDOR_OK: &str = r#"
trusted state orders is durable Map of Text to Text starting empty

state mine is server Text from orders at "root"

view
    Column
        Text "x"
"#;

/// The same site, discharged, and the grant that discharged it is named.
///
/// A grant is only attributable where it was awarded. Had the key arrived
/// through a parameter — as it does in `IDOR` above — the site would still
/// be raised and still be discharged, but `discharged_by` would be `None`,
/// because the literal was written in another definition. That is a real
/// limit on what the audit trail can say and it is why §19.5's
/// completeness argument is a claim about *declarations* rather than about
/// expressions.
#[test]
fn a1_is_discharged_by_a_literal_key() {
    let (hir, split) = compile(IDOR_OK);
    let analysis = authority(&hir, &split);
    let raised = sites(&analysis, ObligationSite::A1);
    assert_eq!(
        raised.len(),
        1,
        "the site is raised whether or not it fires"
    );
    assert!(raised[0].is_discharged());
    assert_eq!(raised[0].discharged_by, Some(Grant::Literal));
    assert!(codes(&analysis).is_empty());
}

const UNTRUSTED_COLLECTION: &str = r#"
state rows is durable Map of Text to Text starting empty

state candidate is client Text starting ""

state mine is server Text from rows at candidate

view
    Column
        Input candidate, hint is "id"
"#;

/// A1 is an obligation of the **declaration**, not of indexing.
///
/// `rows` is not declared `trusted`, so no obligation exists over it and
/// none is raised. §18.1.6 limit 4 is the honest reading of that: without
/// the word, nothing is checked, and a program can pass every integrity
/// check and still be wide open.
#[test]
fn an_index_into_an_ordinary_collection_raises_nothing() {
    let (hir, split) = compile(UNTRUSTED_COLLECTION);
    let analysis = authority(&hir, &split);
    assert!(sites(&analysis, ObligationSite::A1).is_empty());
    assert!(codes(&analysis).is_empty());
}

const PATH_TRAVERSAL: &str = r#"
foreign putObject is server
    from  "./s3" as "put"
    takes key is trusted Text, body is Text
    gives Text

state typed is client Text starting ""

state receipt is server Text from putObject with key is typed, body is "hello"

view
    Column
        Input typed, hint is "object key"
"#;

/// **A2 / E-INT-05.** The parameter the declaration asked to be Trusted.
#[test]
fn a2_fires_on_an_untrusted_argument_to_a_trusted_foreign_parameter() {
    let (hir, split) = compile(PATH_TRAVERSAL);
    let analysis = authority(&hir, &split);
    let raised = sites(&analysis, ObligationSite::A2);
    assert_eq!(raised.len(), 1, "one `trusted` parameter, one site");
    assert_eq!(raised[0].found, Authority::Untrusted);
    assert!(codes(&analysis).contains(&"E-INT-05"));
}

const CLIENT_FOREIGN: &str = r#"
foreign focusOn is client
    from  "./dom" as "focus"
    takes id is trusted Text
    gives Text

state typed is client Text starting ""

state focused is client Text from focusOn with id is typed

view
    Column
        Input typed, hint is "id"
"#;

/// §18.1 semantics 7: there is no such thing as protecting a browser from
/// itself, so the whole client walk is exempt and it falls out of the
/// declaration rather than needing a rule.
#[test]
fn a_client_foreign_raises_no_a2() {
    let (hir, split) = compile(CLIENT_FOREIGN);
    let analysis = authority(&hir, &split);
    assert!(sites(&analysis, ObligationSite::A2).is_empty());
    assert!(codes(&analysis).is_empty());
}

const MODERATOR: &str = r#"
trusted state moderators is durable Map of Text to Truth starting empty

state typed is client Text starting ""

view
    Column
        Input typed, hint is "who"
        Button "promote"
            on click
                set moderators at typed to yes
"#;

/// **A3 / E-INT-03**, and **A1** on the write side of the same statement.
///
/// A browser must not choose who is a moderator, and here it chooses both
/// halves — the key from a text box, and the value by being the sender.
///
/// **This test used to assert that `yes` discharged A3**, on the reading
/// that a literal derives from G-LIT and so is Trusted wherever it is
/// written. That is right about the expression and wrong about the write:
/// this handler runs in the browser and the write leaves it as a command,
/// so what arrives at the endpoint is whatever a browser posted and not
/// what the source says. §18.1 semantics 4 is the rule, it is not
/// derivable from the grant set, and it survives §21.7.6's deletion of
/// semantics 5. Without it `set role to "admin"` in a click handler is a
/// program that opts into `trusted` and is checked by nobody.
#[test]
fn a3_and_a1_are_raised_on_a_write_to_a_trusted_place() {
    let (hir, split) = compile(MODERATOR);
    let analysis = authority(&hir, &split);
    let written = sites(&analysis, ObligationSite::A3);
    assert_eq!(written.len(), 1);
    assert!(
        !written[0].is_discharged(),
        "a literal is not a grant over the wire: a browser sends this write"
    );

    let indexed = sites(&analysis, ObligationSite::A1);
    assert_eq!(indexed.len(), 1);
    assert_eq!(indexed[0].found, Authority::Untrusted);
    assert_eq!(codes(&analysis), ["E-INT-03", "E-INT-02"]);
}

const MODERATOR_VALUE: &str = r#"
trusted state flags is durable Map of Text to Text starting empty

state typed is client Text starting ""

view
    Column
        Input typed, hint is "value"
        Button "save"
            on click
                set flags at "root" to typed
"#;

/// **A3 alone.** The key is a literal and the value is the text box.
#[test]
fn a3_fires_on_an_untrusted_value_written_to_a_trusted_place() {
    let (hir, split) = compile(MODERATOR_VALUE);
    let analysis = authority(&hir, &split);
    let written = sites(&analysis, ObligationSite::A3);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].found, Authority::Untrusted);
    assert_eq!(codes(&analysis), ["E-INT-03"]);
}

const ORDINARY_WRITE: &str = r#"
state notes is durable Map of Text to Text starting empty

state typed is client Text starting ""

view
    Column
        Input typed, hint is "value"
        Button "save"
            on click
                set notes at "root" to typed
"#;

/// Without the word, there is no obligation. This is what makes the
/// inversion cost 0 on a program that opts into nothing (§21.7.1): the
/// polarity of the lattice and the density of the obligations are
/// independent axes.
#[test]
fn a_write_to_an_ordinary_place_raises_nothing() {
    let (hir, split) = compile(ORDINARY_WRITE);
    let analysis = authority(&hir, &split);
    assert!(sites(&analysis, ObligationSite::A3).is_empty());
    assert!(codes(&analysis).is_empty());
}

// ---------------------------------------------------------------------
// The site set, and what the diagnostics may say.
// ---------------------------------------------------------------------

/// The obligation list is closed at **four** — A1, A2, A3, A5 — and
/// **A4 must never return**.
///
/// A4 was *a selector expression inside a `release` body*, added by §19.2
/// rule 11 to discharge REL-SELECT. §19.9 refuted REL-SELECT by
/// counterexample and §19.10.1 deleted it: the rule asked a syntactic
/// question about a semantic property, and the next spelling of the attack
/// was an index-recursive fold with nothing left to read. A5 replaced it by
/// moving the quantifier to the parameter list, which is finite and named
/// in the source. Anyone reaching for a fifth variant should read §21.7.6
/// before adding it.
#[test]
fn the_obligation_set_is_closed_at_four() {
    assert_eq!(ObligationSite::CLOSED_LIST.len(), 4);
    let codes: Vec<&str> = ObligationSite::CLOSED_LIST
        .iter()
        .map(|site| site.code())
        .collect();
    assert_eq!(codes, ["A1", "A2", "A3", "A5"]);
    assert!(
        !codes.contains(&"A4"),
        "A4 discharged a rule that is known not to hold"
    );
}

/// A5 is the one site with no error code: it is discharged by the
/// declaration that creates it, and the unendorsed case is REL-ARG's
/// E-REL-08 rather than a failure of A5.
#[test]
fn only_a5_has_no_diagnostic() {
    let without: Vec<&str> = ObligationSite::CLOSED_LIST
        .iter()
        .filter(|site| site.error_code().is_none())
        .map(|site| site.code())
        .collect();
    assert_eq!(without, ["A5"]);
}

const REL_CLOSED_FIXTURE: &str = r#"
state cards is server Text starting ""

release digitOracle with guess
    gives Whole
    limit 10 per visitor
    give cards

view
    Column
        Text "x"
"#;

const REL_PURE_FIXTURE: &str = r#"
foreign queryParam is server
    from  "zd:http" as "query"
    takes key is Text
    gives Text

release digitOracle with guess
    gives Whole
    limit 10 per visitor
    give queryParam with key is guess

view
    Column
        Text "x"
"#;

const UNBOUNDED_FIXTURE: &str = r#"
release judge with guess
    gives Text
    give guess

view
    Column
        Text "x"
"#;

/// The declaration-level release rules fire through this entry point.
///
/// REL-ARG is raised by the walk, at call sites. REL-CLOSED, REL-PURE and
/// W-REL-01 are properties of the **declaration**, and they were three free
/// functions no driver called. They are checked here now, so that there is
/// one entry point rather than three a driver could forget one of — which
/// is the shape of the defect this branch exists to remove.
#[test]
fn the_declaration_level_release_rules_fire_through_authority() {
    for (source, expected) in [
        (REL_CLOSED_FIXTURE, "E-REL-04"),
        (REL_PURE_FIXTURE, "E-REL-10"),
    ] {
        let (hir, split) = compile(source);
        let analysis = authority(&hir, &split);
        assert!(
            codes(&analysis).contains(&expected),
            "expected {expected}: {:?}",
            codes(&analysis)
        );
    }

    let (hir, split) = compile(UNBOUNDED_FIXTURE);
    let analysis = authority(&hir, &split);
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|d| d.code == "W-REL-01" && !d.is_error()),
        "an unbounded release must warn, and only warn"
    );
}

/// **No diagnostic added here may promise anything.**
///
/// Three adversarial passes broke the soundness argument and R1 remains
/// open. A rule may say what it requires; it may never say that the
/// program is thereby anything. This is the same grep
/// `the_unbounded_warning_does_not_promise_a_disclosure_bound` runs on
/// W-REL-01, applied to every diagnostic this pass can emit — because a
/// diagnostic that tells a user their program is robust when it is not is
/// the exact failure §21.6 item 18 named when it forbade REL-ARG the first
/// time.
#[test]
fn no_diagnostic_here_promises_anything() {
    let sources = [
        PROBES,
        IDOR,
        PATH_TRAVERSAL,
        MODERATOR,
        MODERATOR_VALUE,
        // The three declaration-level release rules now reach a user
        // through this entry point too, so they are read by the same grep.
        // A diagnostic is only as honest as the last text edit to it.
        REL_CLOSED_FIXTURE,
        REL_PURE_FIXTURE,
        UNBOUNDED_FIXTURE,
    ];
    let mut seen = 0;
    for source in sources {
        let (hir, split) = compile(source);
        let analysis = authority(&hir, &split);
        for diagnostic in analysis.diagnostics() {
            seen += 1;
            let text = format!(
                "{} {}",
                diagnostic.message,
                diagnostic.help.clone().unwrap_or_default()
            )
            .to_lowercase();
            for promise in ["guarantee", "ensures", "prevents", "safe", "robust"] {
                assert!(
                    !text.contains(promise),
                    "`{}` must not promise `{promise}`: {text}",
                    diagnostic.code
                );
            }
        }
    }
    assert!(seen >= 5, "the grep is only worth what it read: {seen}");
}

/// Every checked-in example still analyses, and none of them raises an
/// obligation — which is §21.7.2's measurement restated as a test: the
/// inversion costs nothing on programs that opt into nothing, and the ten
/// examples contain 0 `trusted`, 0 `release` and 1 `foreign`.
///
/// It is **not** a measurement of the machinery, and §21.8.5 says so: the
/// honest count on a program that exercises the feature is 7.6% of lines,
/// 2.7 grants per release, of which 56% is overhead over Jif's condition.
#[test]
fn the_checked_in_examples_opt_into_nothing() {
    let (hir, split) = compile(GUESTBOOK);
    let analysis = authority(&hir, &split);
    assert!(analysis.obligations().is_empty());
    assert!(codes(&analysis).is_empty());
}

// ---------------------------------------------------------------------
// The routes an event payload takes to a `trusted` place.
//
// These three came from the default-open integrity pass in `zdc-types`,
// which this module replaced. They are about *routes* — through a
// component parameter, through a function parameter, and through a
// condition — rather than about the lattice, so they survive the pass
// they were written for and are asserted here against the closed one.
// ---------------------------------------------------------------------

/// A component is written out at each call site before this pass runs, so
/// a payload that reaches a `trusted` place *through a component
/// parameter* is not a route around the pass — it is the same write, in
/// the caller's own place. This pins that, because "the parameter is
/// gone" is an argument about a pass this one does not run.
const PAYLOAD_THROUGH_A_COMPONENT: &str = r#"
trusted state note is durable Text starting ""

component Recorder with sink
    Button "go"
        on keydown with press
            set sink to press.key

view
    Recorder note
"#;

#[test]
fn a_payload_written_through_a_component_parameter_is_still_the_payload() {
    let (hir, split) = compile(PAYLOAD_THROUGH_A_COMPONENT);
    let analysis = authority(&hir, &split);
    assert!(
        codes(&analysis).contains(&"E-INT-03"),
        "expected E-INT-03: {:?}",
        codes(&analysis)
    );
}

/// A payload handed to a function and written there. The label travels on
/// the parameter, which is what fixpoint 2 over `params` is for.
const PAYLOAD_THROUGH_A_FUNCTION: &str = r#"
trusted state note is durable Text starting ""

function stash with v
    set note to v
    give yes

state done is server Truth from stash with v is "seed"

view
    Button "go"
        on keydown with press
            set note to press.key
"#;

#[test]
fn a_payload_written_through_a_function_parameter_is_still_the_payload() {
    let (hir, split) = compile(PAYLOAD_THROUGH_A_FUNCTION);
    let analysis = authority(&hir, &split);
    assert!(
        codes(&analysis).contains(&"E-INT-03"),
        "expected E-INT-03: {:?}",
        codes(&analysis)
    );
}

/// §18.1 semantics 11 — the implicit flow. The value written is a
/// literal; the decision to write it is not.
const WRITE_DECIDED_BY_AN_UNTRUSTED_VALUE: &str = r#"
trusted state moderators is durable Map of Text to Truth starting empty

state wanted is client Truth starting no
state promoted is server Truth from promote with wanted

function promote with asked
    if asked
        set moderators at "root" to yes
    give yes

view
    Checkbox wanted
"#;

#[test]
fn a_write_decided_by_an_untrusted_value_is_rejected() {
    let (hir, split) = compile(WRITE_DECIDED_BY_AN_UNTRUSTED_VALUE);
    let analysis = authority(&hir, &split);
    assert!(
        codes(&analysis).contains(&"E-INT-04"),
        "expected E-INT-04: {:?}",
        codes(&analysis)
    );
}
