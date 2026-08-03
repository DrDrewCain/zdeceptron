//! The integrity direction, against the programs that specify it.
//!
//! Two things are being tested, and they are different in kind:
//!
//! * that the **lattice is closed** — a value is Untrusted unless one of
//!   the eight grants applies — which is a property of a total function
//!   and is checked here directly;
//! * that the **release rules fire**, which is a property of review aids
//!   and is checked against the counterexamples that refuted them.
//!
//! Nothing here tests that a program is free of laundering, because
//! nothing establishes it. §21.8.8 option 2 is the footing: the rules
//! ship, the claim does not.

mod support;

use support::*;
use zdc_graph::integrity::{rel_closed, rel_pure, w_rel_01, Authority, Grant, Integrity, Writers};
use zdc_hir::DefKind;

// ---------------------------------------------------------------------
// The lattice itself.
// ---------------------------------------------------------------------

/// Default-closed, stated as a property of the type rather than of a walk.
///
/// This is §21.7.0 in one assertion. If it ever flips, every other test in
/// this file could still pass while the analysis silently trusted the
/// world.
#[test]
fn the_default_is_untrusted() {
    assert_eq!(Authority::default(), Authority::Untrusted);
}

#[test]
fn join_is_untrusted_absorbing() {
    assert_eq!(
        Authority::Trusted.join(Authority::Untrusted),
        Authority::Untrusted
    );
    assert_eq!(
        Authority::Untrusted.join(Authority::Trusted),
        Authority::Untrusted
    );
    assert_eq!(
        Authority::Trusted.join(Authority::Trusted),
        Authority::Trusted
    );
}

/// `⨆ ∅ = Trusted`, and this test exists to keep that visible.
///
/// It is correct lattice algebra and it is exactly the shape of residual
/// risk R1: a no-argument `is anywhere` foreign — the prelude's own
/// `clock` — joins the empty set and comes out Trusted forever. The
/// assertion is not a claim that this is safe. It is a marker on the spot
/// where the unsoundness lives, so that a later `pure` modifier has a
/// failing test to aim at.
#[test]
fn the_empty_join_is_trusted_which_is_r1() {
    assert_eq!(Authority::join_all([]), Authority::Trusted);
}

/// The grant set is closed at eight. §19.5's completeness argument is a
/// claim about the grammar, and it is only as good as this list.
#[test]
fn the_grant_set_is_closed_at_eight() {
    assert_eq!(Grant::CLOSED_LIST.len(), 8);
    let mut codes: Vec<&str> = Grant::CLOSED_LIST.iter().map(|g| g.code()).collect();
    codes.sort();
    assert_eq!(
        codes,
        ["G-BLD", "G-ENV", "G-FGN-A", "G-FGN-T", "G-LIT", "G-REL", "G-SIG", "G-VIS"]
    );
}

/// Only the two human-asserted grants are marked as such. A reviewer
/// reading the report needs the list of things nobody checked.
#[test]
fn only_the_foreign_grants_are_asserted() {
    let asserted: Vec<&str> = Grant::CLOSED_LIST
        .iter()
        .filter(|g| g.is_asserted())
        .map(|g| g.code())
        .collect();
    assert_eq!(asserted, ["G-FGN-T", "G-FGN-A"]);
}

// ---------------------------------------------------------------------
// G-SIG, and the §21.8.4 repair.
// ---------------------------------------------------------------------

const TWO_WAY: &str = r#"
state query is client Text starting ""

view
    Column
        Input query, hint is "filter posts"
"#;

/// **Residual risk R2, repaired.** `examples/blog.zd:48`'s `query` has no
/// `set` anywhere, and its initialiser is the literal `""`. G-SIG as
/// §21.7.3 wrote it therefore made a read of it **Trusted** — and it is a
/// text box, which is how §21.8.4 restored §19.9's counterexample with the
/// attacker-side endorsements removed.
///
/// The decision §21.8.4 left open is taken: a two-way binding is a write
/// site. `Site::Bind` already recorded one, so the repair is to ask the
/// site walk instead of the statement forms.
#[test]
fn a_two_way_bound_signal_is_untrusted() {
    let (hir, _) = compile(TWO_WAY);
    let writers = Writers::of(&hir);
    let query = def_named(&hir, "query");

    assert!(
        writers.is_written(query),
        "a two-way `Input` binding must count as a write site (§21.8.4, R2)"
    );

    let integrity = Integrity::new(&hir, &writers);
    let (authority, grant) = integrity.of(init_of(&hir, query));
    // Its *initialiser* is a literal and so is Trusted by G-LIT …
    assert_eq!(authority, Authority::Trusted);
    assert_eq!(grant, Some(Grant::Literal));

    // … but a *read* of the signal is not, because the browser writes it.
    assert_eq!(read_of(&hir, &writers, query), Authority::Untrusted);
}

const UNWRITTEN: &str = r#"
state greeting is client Text starting "hello"

view
    Column
        Text greeting
"#;

/// G-SIG clause 2, the case it was written for: no writer anywhere, and a
/// Trusted initialiser.
#[test]
fn an_unwritten_signal_with_a_literal_initialiser_is_trusted() {
    let (hir, _) = compile(UNWRITTEN);
    let writers = Writers::of(&hir);
    let greeting = def_named(&hir, "greeting");
    assert!(!writers.is_written(greeting));
    assert_eq!(read_of(&hir, &writers, greeting), Authority::Trusted);
}

const DECLARED_TRUSTED: &str = r#"
trusted state orders is server Text starting ""

view
    Column
        Text orders
"#;

/// G-SIG clause 1. The declaration is the grant, and it holds even though
/// the signal is a source a program writes.
#[test]
fn a_declared_trusted_signal_is_trusted() {
    let (hir, _) = compile(DECLARED_TRUSTED);
    let writers = Writers::of(&hir);
    let orders = def_named(&hir, "orders");
    assert_eq!(read_of(&hir, &writers, orders), Authority::Trusted);
}

// ---------------------------------------------------------------------
// The release rules, against the programs that refuted them.
// ---------------------------------------------------------------------

const READS_A_SIGNAL: &str = r#"
state cards is server Text starting ""

release digitOracle with guess
    gives Whole
    limit 10 per visitor
    give cards

view
    Column
        Text "x"
"#;

/// **REL-CLOSED / E-REL-04.** A release's inputs are its parameters and
/// nothing else. This is the premise REL-ARG never had in §19.10, and it
/// is what makes the parameter list the whole of the release's input.
#[test]
fn a_release_may_not_read_a_signal() {
    let (hir, _) = compile(READS_A_SIGNAL);
    let errors = rel_closed(&hir, def_named(&hir, "digitOracle"));
    assert_eq!(codes(&errors), ["E-REL-04"]);
    assert!(errors[0].message.contains("cards"));
}

const IMPURE_FOREIGN: &str = r#"
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

/// **REL-PURE / E-REL-10**, on §19.11.1's counterexample.
///
/// `queryParam` is `is server` with no `gives trusted T`, so the release
/// is rejected **at the declaration**. §19.11.2's point was that the
/// attack had no term in the program; this is the term. It is not
/// repairable by an endorsement, because the attacker's values are not
/// arguments.
#[test]
fn a_release_reaching_an_ungranted_foreign_is_rejected() {
    let (hir, _) = compile(IMPURE_FOREIGN);
    let errors = rel_pure(&hir, def_named(&hir, "digitOracle"));
    assert_eq!(codes(&errors), ["E-REL-10"]);
    assert!(errors[0].message.contains("queryParam"));
    assert!(errors[0].message.contains("is server"));
}

const ANYWHERE_FOREIGN: &str = r#"
foreign renderMarkdown is anywhere
    from  "marked" as "parse"
    takes source is Text
    gives Text

release digitOracle with guess
    gives Text
    limit 10 per visitor
    give renderMarkdown with source is guess

view
    Column
        Text "x"
"#;

/// **The R1 hole, asserted rather than described.**
///
/// REL-PURE accepts this program because `renderMarkdown` is
/// `is anywhere`. That classification answers "which bundles may this be
/// linked into?" (§14E.2's own heading), not "is this pure" — so an
/// `is anywhere` foreign that reads the environment passes, and the
/// prelude's `clock` is exactly such a foreign.
///
/// The test asserts the **current, unsound** behaviour on purpose. When
/// §21.8.8 option 1's `pure` modifier is added, this test must fail and be
/// rewritten; that is what it is for.
#[test]
fn rel_pure_accepts_is_anywhere_and_that_is_the_break() {
    let (hir, _) = compile(ANYWHERE_FOREIGN);
    let errors = rel_pure(&hir, def_named(&hir, "digitOracle"));
    assert!(
        errors.is_empty(),
        "REL-PURE is stated over `is anywhere`, which is a linkability \
         classification; this is residual risk R1, not a passing program"
    );
}

const UNBOUNDED: &str = r#"
release judge with guess
    gives Text
    give guess

view
    Column
        Text "x"
"#;

/// **W-REL-01.** An unbounded release warns.
#[test]
fn an_unbounded_release_warns() {
    let (hir, _) = compile(UNBOUNDED);
    let warning = w_rel_01(&hir, def_named(&hir, "judge")).expect("expected W-REL-01");
    assert_eq!(warning.code, "W-REL-01");
    assert!(!warning.is_error());
}

/// The warning must not sell `limit` as something it is not.
///
/// §21.8.7: `limit` bounds evaluations per (declaration, anonymous
/// session). It does not bound them per program, per person, or per
/// secret, and it is unenforced entirely until durable storage exists. A
/// diagnostic that told a user otherwise would be the failure §21.6 item
/// 18 named when it forbade REL-ARG the first time.
#[test]
fn the_unbounded_warning_does_not_promise_a_disclosure_bound() {
    let (hir, _) = compile(UNBOUNDED);
    let warning = w_rel_01(&hir, def_named(&hir, "judge")).expect("expected W-REL-01");
    let help = warning.help.unwrap_or_default().to_lowercase();
    assert!(
        help.contains("not a cumulative disclosure bound"),
        "the help text must say what `limit` does not do: {help}"
    );
    for promise in ["guarantee", "ensures", "prevents", "safe", "robust"] {
        assert!(
            !help.contains(promise),
            "W-REL-01's help must not promise `{promise}`: {help}"
        );
    }
}

/// A budgeted release does not warn.
#[test]
fn a_budgeted_release_does_not_warn() {
    let (hir, _) = compile(READS_A_SIGNAL);
    assert!(w_rel_01(&hir, def_named(&hir, "digitOracle")).is_none());
}

// ---------------------------------------------------------------------
// Fixture plumbing.
// ---------------------------------------------------------------------

fn init_of(hir: &zdc_hir::Hir, signal: zdc_hir::DefId) -> zdc_hir::ExprId {
    match &hir.defs[signal].kind {
        DefKind::Signal(s) => s.init,
        _ => panic!("not a signal"),
    }
}

/// What a *read* of a signal is worth, which is the question G-SIG asks.
fn read_of(hir: &zdc_hir::Hir, writers: &Writers, signal: zdc_hir::DefId) -> Authority {
    let integrity = Integrity::new(hir, writers);
    integrity.of_signal_read(signal)
}
