//! §19.5's audit trail, and the half of residual risk **R6** it closes.
//!
//! R6 is *"a purity grant has no argument chain for an
//! attacker-reachability walk to follow"*, and its consequence — stated by
//! §21.8.3 and by the issue that asked for this — is that the grants
//! §21.7's soundness leans on are the ones no review artifact reaches. This
//! file pins the artifact that reaches them.
//!
//! What is **not** here, deliberately: any assertion that the report says
//! whether an attacker can steer a grant. It does not, it cannot, and
//! `a_purity_grants_argument_chain_answers_the_wrong_question` is the test
//! that records why — the same program `authority.rs` uses to record that
//! an asserted marker still launders (R5).

mod support;

use support::*;
use zdc_graph::integrity::Grant;
use zdc_graph::report::{report, LIBRARY_NOTE, NOT_CLAIMED};
use zdc_hir::{HirExprKind, Res};

/// A `gives pure` foreign that a release reaches, which is the shape R6 is
/// about: the assertion is what lets the declassification compile.
const ORACLE: &str = r#"
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

/// **The R6 reproduction, inverted into a gate.**
///
/// Before this landed there was no artifact at all — `--report` was
/// unimplemented, so `dist/` held no file naming `queryParam`'s purity
/// claim and nothing else did either. A reader of a built bundle had one
/// route to the assertion the program's integrity rests on, which was to
/// read the source and know what to look for.
#[test]
fn a_purity_grant_is_in_the_audit_trail_with_its_declaration_and_its_calls() {
    let (hir, _) = compile(ORACLE);
    let trail = report(&hir);

    assert_eq!(
        trail.asserted.len(),
        1,
        "the program declares one asserted grant: {:?}",
        trail.asserted.iter().map(|g| &g.name).collect::<Vec<_>>()
    );
    let entry = &trail.asserted[0];
    assert_eq!(entry.grant, Grant::ForeignPure);
    assert_eq!(entry.grant.code(), "G-FGN-P");
    assert!(entry.grant.is_asserted());
    assert_eq!(entry.name, "queryParam");
    assert_eq!(entry.marker, "pure");
    assert_eq!(entry.module.as_deref(), Some("./request"));
    assert_eq!(entry.export, "queryParam");
    assert!(!entry.primitive);

    // The declaration's span, so a reader is sent to the line carrying the
    // claim rather than to the file.
    assert_eq!(
        entry.declared_at,
        hir.defs[def_named(&hir, "queryParam")].span
    );

    // Both calls: the derived signal's initialiser and the release body.
    assert_eq!(
        entry.calls.len(),
        2,
        "`queryParam` is called twice in this program: {:?}",
        entry.calls
    );
}

/// **What the walk that exists can answer, and R6's question restated as
/// one it can.**
///
/// R6 wants to know whether an unchecked assertion is load-bearing. The
/// argument chain cannot say, but REL-PURE's own reachability walk can say
/// something stronger and cheaper: this grant is what lets *that* release
/// past the rule. A release is the program's declassification boundary, so
/// an entry here is the report telling a reviewer which four lines to read
/// first.
#[test]
fn the_release_that_depends_on_a_purity_grant_is_named_beside_it() {
    let (hir, _) = compile(ORACLE);
    let trail = report(&hir);

    let reached: Vec<&str> = trail.asserted[0]
        .releases
        .iter()
        .map(|release| release.name.as_str())
        .collect();
    assert_eq!(
        reached,
        ["digitOracle"],
        "`digitOracle`'s body calls `queryParam`, so the grant is load-bearing for it"
    );

    let release = &trail.asserted[0].releases[0];
    assert_eq!(
        release.declared_at,
        hir.defs[def_named(&hir, "digitOracle")].span
    );
    assert!(
        trail.asserted[0].calls.contains(&release.reached_at),
        "the span a release reaches the grant at is one of the grant's call sites"
    );
}

/// The same program with the marker removed. There is nothing asserted, so
/// there is nothing to review — and the compiler says so itself, with
/// E-REL-10, which is the case R6 was never about.
#[test]
fn an_unmarked_foreign_is_not_an_asserted_grant() {
    let opaque = ORACLE.replace("gives pure Text", "gives Text");
    let (hir, _) = compile(&opaque);

    assert!(
        report(&hir).asserted.is_empty(),
        "`gives Text` claims nothing, so it is not an entry a reviewer must read"
    );
}

/// `gives trusted T` is the stronger assertion and is listed the same way.
#[test]
fn a_trusted_grant_is_listed_beside_the_purity_grants() {
    let signed = ORACLE.replace("gives pure Text", "gives trusted Text");
    let (hir, _) = compile(&signed);
    let trail = report(&hir);

    assert_eq!(trail.asserted.len(), 1);
    assert_eq!(trail.asserted[0].grant, Grant::ForeignTrusted);
    assert_eq!(trail.asserted[0].marker, "trusted");
}

/// **Why there is no `attackerReachable` field, as a program rather than
/// as a paragraph.**
///
/// `queryParam` reads the request URL and takes one string literal. Give
/// its grant the argument chain the issue asks for and walk it: every
/// argument at every call site is a literal, so the walk reports that no
/// attacker-controlled value reaches the grant. A visitor steers the
/// declassification with a query string. The answer the field would carry
/// is available, cheap, and false, which is why the report carries the
/// assertion and not a verdict about it.
#[test]
fn a_purity_grants_argument_chain_answers_the_wrong_question() {
    let (hir, _) = compile(ORACLE);
    let trail = report(&hir);
    let query_param = def_named(&hir, "queryParam");
    assert_eq!(
        trail.asserted[0].def, query_param,
        "the grant under test is the one the trail reports"
    );

    // The chain the issue asks for, walked: every argument at every call
    // site of the grant.
    let mut arguments = Vec::new();
    for (_, expr) in hir.exprs.iter() {
        let HirExprKind::Call { callee, args } = &expr.kind else {
            continue;
        };
        if *callee != Res::Def(query_param) {
            continue;
        }
        for arg in args {
            arguments.push(hir.exprs[zdc_graph::sites::arg_expr(arg)].kind.clone());
        }
    }

    assert_eq!(
        arguments.len(),
        2,
        "one argument at each of `queryParam`'s two call sites: {arguments:?}"
    );
    let literals = arguments
        .iter()
        .filter(|kind| matches!(kind, HirExprKind::Text(_)))
        .count();
    assert_eq!(
        literals, 2,
        "both are string literals, so an argument walk answers `no attacker-controlled \
         value reaches this grant` — while the JavaScript reads `location.search`"
    );

    // And the sentence that says so ships in the file itself.
    let stated = NOT_CLAIMED
        .iter()
        .filter(|line| line.contains("attackerReachable"))
        .count();
    assert_eq!(
        stated, 1,
        "the report has to state the field's absence where a reader of it will look"
    );
}

/// **A sentence that shipped before the flag did, made true.**
///
/// E-REL-08's help text tells an author that writing `trusted p` *"records
/// that this makes the release a function of a value the browser chose,
/// and it will appear in `zdc build --report`"*. It is the other human
/// signature — obligation site A5, whose whole reason for existing is that
/// *"every signature is enumerable"* — so an audit trail that listed the
/// foreign grants and not this one would leave the same gap R6 names, one
/// grant over.
#[test]
fn an_endorsement_is_the_other_signature_and_it_is_enumerated_too() {
    let (hir, _) = compile(ORACLE);
    let trail = report(&hir);

    assert_eq!(trail.endorsed.len(), 1, "`trusted all`, and nothing else");
    assert_eq!(trail.endorsed[0].release, "digitOracle");
    assert_eq!(trail.endorsed[0].parameter, "all");

    let unsigned = ORACLE.replace("    trusted all\n", "");
    let (hir, _) = compile(&unsigned);
    assert!(
        report(&hir).endorsed.is_empty(),
        "the clause is what puts the signature in the trail"
    );
}

/// The prelude's purity grants are named, not located.
///
/// They are assertions too — the same `gives pure T` marker, about the
/// modules this compiler emits — so leaving them out entirely would make
/// the trail say the program rests on one assertion when it rests on
/// twenty-eight. They carry no spans because a prelude span indexes the
/// library's own file rather than anything the linked program holds; see
/// [`LIBRARY_NOTE`].
#[test]
fn the_preludes_purity_grants_are_named_and_the_reason_ships_with_them() {
    let program = zdc_parser::parse("view\n    Column\n        Text \"hello\"\n")
        .expect("the program parses");
    let hir = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .expect("the program resolves against the prelude");
    let trail = report(&hir);

    assert!(
        trail.asserted.is_empty(),
        "the program declares no `foreign` of its own"
    );
    assert_eq!(
        trail.library.pure.len(),
        41,
        "every prelude primitive but the clock is `gives pure`; the count moves \
         whenever the library gains one, and it has moved by fourteen since \
         this was written"
    );
    assert!(trail.library.pure.contains(&"textLength".to_string()));
    assert!(
        !trail.library.pure.contains(&"clock".to_string()),
        "`clock` carries no marker, which is what §21.9 settled"
    );
    assert!(
        trail.library.trusted.is_empty(),
        "the prelude signs for nothing unconditionally"
    );
    assert!(
        NOT_CLAIMED.contains(&LIBRARY_NOTE),
        "the reason the library is named rather than located ships in the file"
    );
}

/// **R5's third assertion, which had no entry here at all.**
///
/// §21.8 counts three words asserted about third-party JavaScript and
/// checked by nobody, and `report()` used to collect two of them: its
/// `Opaque => continue` dropped a `foreign` that claimed nothing about
/// its result, and `is anywhere` went down with it. So the audit trail
/// was complete over grants and silent about placements, and a reviewer
/// reading it could not see that this program asks for somebody's
/// JavaScript in both bundles.
#[test]
fn an_unmarked_foreign_still_asserts_where_it_may_be_linked() {
    let opaque = ORACLE.replace("gives pure Text", "gives Text");
    let (hir, _) = compile(&opaque);
    let trail = report(&hir);

    assert!(
        trail.asserted.is_empty(),
        "`gives Text` claims nothing about the result"
    );
    assert_eq!(
        trail.anywhere.len(),
        1,
        "`is anywhere` is still a claim about somebody's JavaScript"
    );
    assert_eq!(trail.anywhere[0].name, "queryParam");
    assert_eq!(trail.anywhere[0].export, "queryParam");
    assert!(
        !trail.anywhere[0].primitive,
        "`./request` is somebody's file, not this compiler's primitive layer"
    );
    assert!(
        !trail.anywhere[0].calls.is_empty(),
        "a reviewer wants where it is used as well as where it is declared"
    );
}

/// The two assertions are independent, so a `foreign` making both is in
/// both lists. Reporting a placement only when nothing else was asserted
/// would make the list's meaning depend on the other line.
#[test]
fn a_foreign_that_asserts_both_appears_under_both() {
    let (hir, _) = compile(ORACLE);
    let trail = report(&hir);

    assert_eq!(trail.asserted.len(), 1);
    assert_eq!(trail.anywhere.len(), 1);
    assert_eq!(trail.asserted[0].name, trail.anywhere[0].name);
}

/// **Why the list is `anywhere` and not every placement.**
///
/// `is client` is as unverified as `is anywhere` — both are claims about
/// JavaScript this compiler cannot read. It is *confined*, though: the
/// split refuses it a place in the server bundle, so the compiler acts on
/// the claim it cannot check. `anywhere` asks for both bundles and is
/// confined by nothing, which is what leaves it with no reader but the
/// report.
#[test]
fn a_confined_placement_is_not_in_the_list() {
    let confined = ORACLE.replace("is anywhere", "is client");
    let (hir, _) = compile(&confined);

    assert!(
        report(&hir).anywhere.is_empty(),
        "`is client` has an enforcement site, so the report is not its only reader"
    );
}

/// The prelude's are named rather than located, for [`LIBRARY_NOTE`]'s
/// reason, and held apart because they are identical in every program.
///
/// `clock` is the case worth pinning. §21.9 took its purity marker away
/// and left its placement alone, so it is exactly a declaration that
/// asserts where it may be linked and nothing about its result — absent
/// from `pure`, present here. That is the entry the old report lost.
#[test]
fn the_preludes_placements_are_named_beside_its_purity_grants() {
    let program = zdc_parser::parse("view\n    Column\n        Text \"hello\"\n")
        .expect("the program parses");
    let hir = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .expect("the program resolves against the prelude");
    let trail = report(&hir);

    assert!(
        trail.library.anywhere.contains(&"clock".to_string()),
        "`clock` asserts where it may be linked and nothing about its result, \
         so the placement list is the only one it is in"
    );
    assert!(
        !trail.library.pure.contains(&"clock".to_string()),
        "and §21.9 is what took it out of the other one"
    );
    assert!(
        trail.library.anywhere.len() > trail.library.pure.len(),
        "a language's primitives are the things that have to work in both \
         bundles, so this list is the longer of the two"
    );
    let mut sorted = trail.library.anywhere.clone();
    sorted.sort();
    assert_eq!(
        trail.library.anywhere, sorted,
        "sorted, so a reviewer diffing two reports sees only what changed"
    );
}
