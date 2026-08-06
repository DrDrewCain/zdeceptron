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
use zdc_graph::authority::Solution;
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

/// `⨆ ∅ = Trusted`, and what now stands between that identity and R1.
///
/// The identity is correct lattice algebra and it was the visible edge of
/// residual risk R1: a no-argument `is anywhere` foreign — the prelude's
/// own `clock` — joined the empty set and came out Trusted forever.
///
/// **§21.9 did not change the fold, and did not need to.** A genuinely
/// pure function of no arguments is a constant, so Trusted is the right
/// answer for one. What changed is which declarations reach the fold: only
/// `gives pure T` does. So this test now pins both halves — the identity,
/// and the fact that `clock` does not carry the marker that would let it
/// use the identity.
#[test]
fn the_empty_join_is_trusted_and_only_a_pure_foreign_may_use_it() {
    assert_eq!(Authority::join_all([]), Authority::Trusted);

    let clock = zdc_lib::load()
        .program()
        .decls
        .iter()
        .find_map(|decl| match decl {
            zdc_ast::Decl::Foreign(foreign) if foreign.name.text == "clock" => Some(foreign),
            _ => None,
        })
        .expect("the prelude declares `clock`")
        .result_grant;

    assert_eq!(
        clock,
        zdc_ast::ForeignGrant::Opaque,
        "`clock` takes no arguments and returns a different value every call. If it ever \
         carries the purity marker, the empty join makes it Trusted forever and R1 is back"
    );
}

/// **The other half of the prelude's classification**, counted rather than
/// described.
///
/// §21.7.5 assumption 4 said the primitive layer was *"pure by construction"*
/// and §21.8.0 answered that a primitive reading the environment *"was never
/// added, because one was there from the start"*. Twenty-seven of the
/// twenty-eight are functions of their arguments; the last is the clock. A
/// primitive wrongly marked pure is a fresh instance of R1, so the split is
/// asserted here rather than left to a reader of five files.
///
/// It was sixteen of seventeen, then twenty of twenty-one. The layer
/// shrank and grew at once — `reverse`, `keys` and `values` became
/// ordinary ZDeceptron over `mapKeyAt` and `append`, and six bitwise and
/// wrapping operations arrived to write a generator in the language rather
/// than to acquire one from the platform. Seven more arrived after that:
/// `parseDecimal`, `sqrt`, `power` and `fixedText`, each a statement about
/// the f64 representation the language cannot observe, and `urlEncoded`,
/// `jsonEncoded` and `base64Encoded`, each a statement about the bytes of
/// a `Text` it cannot observe either. Every one of the seven is a function
/// of its arguments, so the impure list is still the clock alone. The
/// count is asserted as well as the split, because
/// a primitive that appears without anybody ruling on its grant defaults
/// to `Opaque`, and an `Opaque` primitive silently makes every value
/// computed through it Untrusted.
#[test]
fn all_but_one_of_the_primitives_are_pure_and_the_clock_is_not() {
    let mut impure: Vec<&str> = Vec::new();
    let mut pure = 0;
    for decl in &zdc_lib::load().program().decls {
        let zdc_ast::Decl::Foreign(foreign) = decl else {
            continue;
        };
        match foreign.result_grant {
            zdc_ast::ForeignGrant::Pure => pure += 1,
            zdc_ast::ForeignGrant::Opaque => impure.push(&foreign.name.text),
            // The prelude signs for nothing unconditionally. A primitive
            // declared `gives trusted T` would be the compiler asserting
            // that a result is not attacker-chosen whatever went in, which
            // is a claim no primitive needs and none should make.
            zdc_ast::ForeignGrant::Trusted => {
                panic!("`{}` declares `gives trusted T`", foreign.name.text)
            }
        }
    }
    assert_eq!(pure, 27);
    assert_eq!(impure, ["clock"]);
    assert_eq!(
        pure + impure.len(),
        28,
        "the primitive layer is twenty-eight"
    );
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
        ["G-BLD", "G-ENV", "G-FGN-P", "G-FGN-T", "G-LIT", "G-REL", "G-SIG", "G-VIS"]
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
    assert_eq!(asserted, ["G-FGN-T", "G-FGN-P"]);
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

/// **Residual risk R2, repaired.** `examples/blog.zd`'s `query` has no
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
    let (hir, split) = compile(TWO_WAY);
    let writers = Writers::of(&hir, &split);
    let query = def_named(&hir, "query");

    assert!(
        writers.is_written(query),
        "a two-way `Input` binding must count as a write site (§21.8.4, R2)"
    );

    let solution = Solution::solve(&hir, &writers);
    let integrity = Integrity::new(&hir, &solution);
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
    let (hir, split) = compile(UNWRITTEN);
    let writers = Writers::of(&hir, &split);
    let greeting = def_named(&hir, "greeting");
    assert!(!writers.is_written(greeting));
    assert_eq!(read_of(&hir, &writers, greeting), Authority::Trusted);
}

const UNWRITTEN_DURABLE: &str = r#"
secret state cards is durable List of Text starting empty

function countOf with rows
    give rows

state hits is server List of Text from countOf with rows is cards

view
    Column
        Text "x"
"#;

/// **The contradiction inside §21.7.3, decided.**
///
/// §21.7.3's verdict table says §19.9.1's `cards` is *"a `durable` signal
/// with write sites → Untrusted (G-SIG)"*. `launder.zd` contains no
/// `set cards`, so under G-SIG clause 2 as written — no write site among
/// **statement forms**, initialiser `empty` Trusted by G-LIT — a read of
/// `cards` is **Trusted**, and the table's own premise is false about the
/// program it is ruling on.
///
/// The table is the side that is right, and §21.8.4 says why in its own
/// words: *"the document holds both readings and the exploitable one is the
/// one written as the rule."* Clause 2's reachability query answers a
/// question about the **program text**; a durable store outlives the build,
/// and a previous deployment, a migration or a database client is not a
/// statement form. §21.8.4's stated one-clause fix names `Crossing::Store`
/// for exactly this, and its status line reads *"BREAK, one-clause fix, not
/// applied"*. It is applied.
///
/// Without it, `launder.zd` raises **E-REL-08 ×2** where §21.7.3 asserts
/// **×3**, and the third endorsement — the one naming the card table — is
/// the one a reviewer most needs to see.
#[test]
fn an_unwritten_durable_signal_is_untrusted() {
    let (hir, split) = compile(UNWRITTEN_DURABLE);
    let writers = Writers::of(&hir, &split);
    let cards = def_named(&hir, "cards");

    assert!(
        writers.is_written(cards),
        "a durable cell has a writer outside the program's statement forms (§21.8.4, R2)"
    );
    assert_eq!(read_of(&hir, &writers, cards), Authority::Untrusted);
}

const UNWRITTEN_LIFTED: &str = r#"
state probePrefix is client Text starting ""

function echo with value
    give value

state hits is server Text from echo with value is probePrefix

view
    Column
        Text "x"
"#;

/// The other half of §21.8.4's conjunct: `Crossing::Lift`.
///
/// `probePrefix` has no `set` and no `Input` binding, so neither the
/// statement-form query nor the [`Writers`] bind arm sees a writer — and
/// §21.7.3's table still rules it Untrusted, because *"client signals
/// reaching `(Server, View)` by `Lift`"* are values **the browser sends**.
/// The cell is the browser's; what arrives at the server is whatever the
/// browser chose to put in the request, bound or not.
///
/// Decided over the lifted set rather than over the placement, so that a
/// client signal nothing lifts keeps the grant — which is what
/// `an_unwritten_signal_with_a_literal_initialiser_is_trusted` pins, and
/// what keeps `launder3_compiles_clean_and_that_is_r1` observing R1 through
/// G-FGN-A rather than through this rule.
#[test]
fn an_unwritten_lifted_client_signal_is_untrusted() {
    let (hir, split) = compile(UNWRITTEN_LIFTED);
    let writers = Writers::of(&hir, &split);
    let prefix = def_named(&hir, "probePrefix");

    assert!(writers.is_written(prefix));
    assert_eq!(read_of(&hir, &writers, prefix), Authority::Untrusted);
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
    let (hir, split) = compile(DECLARED_TRUSTED);
    let writers = Writers::of(&hir, &split);
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
    let (hir, _split) = compile(READS_A_SIGNAL);
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
/// `queryParam` carries no marker on its `gives` line, so the release is
/// rejected **at the declaration**. §19.11.2's point was that the attack
/// had no term in the program; this is the term. It is not repairable by
/// an endorsement, because the attacker's values are not arguments.
///
/// The placement moved from the message to the help, and the move is the
/// point: `is server` is why this foreign cannot be linked into a browser
/// bundle, and it was never why the release is refused.
#[test]
fn a_release_reaching_an_ungranted_foreign_is_rejected() {
    let (hir, _split) = compile(IMPURE_FOREIGN);
    let errors = rel_pure(&hir, def_named(&hir, "digitOracle"));
    assert_eq!(codes(&errors), ["E-REL-10"]);
    assert!(errors[0].message.contains("queryParam"));
    assert!(errors[0]
        .help
        .as_deref()
        .expect("E-REL-10 carries help")
        .contains("is server"));
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

const PURE_FOREIGN: &str = r#"
foreign renderMarkdown is anywhere
    from  "marked" as "parse"
    takes source is Text
    gives pure Text

release digitOracle with guess
    gives Text
    limit 10 per visitor
    give renderMarkdown with source is guess

view
    Column
        Text "x"
"#;

/// **R1, closed: REL-PURE demands the purity marker, not `is anywhere`.**
///
/// This test used to assert the opposite, and the reason it did is the
/// finding. REL-PURE was stated over `is anywhere`, which answers "which
/// bundles may this be linked into?" (§14E.2's own heading) and not "is
/// this pure" — so a query-string reader passed it with an honest, and in
/// fact forced, declaration.
///
/// The two programs below differ by one word on the `gives` line and by
/// nothing else. `is anywhere` is identical in both, so the placement can
/// no longer be what decides — which is exactly what §21.9 separated.
#[test]
fn rel_pure_demands_the_purity_marker_not_is_anywhere() {
    let (hir, _split) = compile(ANYWHERE_FOREIGN);
    let errors = rel_pure(&hir, def_named(&hir, "digitOracle"));
    assert_eq!(codes(&errors), ["E-REL-10"]);
    assert!(errors[0].message.contains("renderMarkdown"));
    assert!(
        errors[0].message.contains("REL-PURE"),
        "the diagnostic must name the rule: {}",
        errors[0].message
    );

    let (hir, _split) = compile(PURE_FOREIGN);
    assert!(
        rel_pure(&hir, def_named(&hir, "digitOracle")).is_empty(),
        "the same declaration with `gives pure Text` is accepted, and the marker is the only \
         difference between the two programs"
    );
}

const RELEASE_READS_THE_CLOCK: &str = r#"
release stamp with guess
    gives Whole
    limit 10 per visitor
    give clock

view
    Column
        Text "x"
"#;

/// **The prelude's own counterexample, refused.**
///
/// §21.8.6 attack 5 is *"the clock as an in-body channel"*, and §21.8's
/// answer was that E0361 blocked it — a rule *"quantifying over a category
/// the grammar cannot express"*, which is what turned into the refutation.
/// The grammar can express it now, so the block is REL-PURE rather than a
/// hard-coded list of prelude names.
#[test]
fn a_release_body_that_reaches_the_clock_is_refused() {
    let program = zdc_parser::parse(RELEASE_READS_THE_CLOCK).expect("parses");
    let hir = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{}", errors[0].message));
    let errors = rel_pure(&hir, def_named(&hir, "stamp"));
    assert_eq!(codes(&errors), ["E-REL-10"]);
    assert!(errors[0].message.contains("clock"));
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
    let (hir, _split) = compile(UNBOUNDED);
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
    let (hir, _split) = compile(UNBOUNDED);
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
    let (hir, _split) = compile(READS_A_SIGNAL);
    assert!(w_rel_01(&hir, def_named(&hir, "digitOracle")).is_none());
}

// ---------------------------------------------------------------------
// G-BLD, which had no expression form to award until §4.4's capabilities
// landed.
// ---------------------------------------------------------------------

const LITERAL_PATH_BUILD_READ: &str = r#"
state about is static Text from build read "content/about.md"

view
    Column
        Text about
"#;

/// **G-BLD.** A path the author wrote names a file the author committed,
/// which is §21.7.3's whole argument for the grant and the same bargain
/// G-ENV makes about a variable the operator set.
#[test]
fn a_build_read_at_a_literal_path_is_granted() {
    let (hir, split) = compile(LITERAL_PATH_BUILD_READ);
    let writers = Writers::of(&hir, &split);
    let solution = Solution::solve(&hir, &writers);
    let integrity = Integrity::new(&hir, &solution);
    let (authority, grant) = integrity.of(init_of(&hir, def_named(&hir, "about")));
    assert_eq!(authority, Authority::Trusted);
    assert_eq!(grant, Some(Grant::Build));
}

const COMPUTED_PATH_BUILD_READ: &str = r#"
state bodies is static List of Text from readAll with directory is "content"

function readAll with directory
    from build list directory
    map each path to build read path

view
    Column
        each body in bodies
            Text body
"#;

/// The other half of the grant, and the half that makes it narrow: `build
/// read path` reads whatever chose `path`. Nothing here is a literal, so
/// the closed lattice answers Untrusted and the capability launders
/// nothing.
#[test]
fn a_build_read_at_a_computed_path_is_not_granted() {
    let (hir, split) = compile(COMPUTED_PATH_BUILD_READ);
    let writers = Writers::of(&hir, &split);
    let solution = Solution::solve(&hir, &writers);
    let integrity = Integrity::new(&hir, &solution);
    let read = build_read_expr(&hir);
    let (authority, grant) = integrity.of(read);
    assert_eq!(authority, Authority::Untrusted);
    assert_eq!(grant, None);
}

const MARKDOWN_OF_A_COMPUTED_READ: &str = r#"
state bodies is static List of Markup from render with directory is "content"

function render with directory
    from build list directory
    map each path to build markdown (build read path)

view
    Column
        each body in bodies
            Prose body
"#;

/// `build markdown` propagates rather than granting: the compiler is its
/// implementation, so its result is a function of its argument — and an
/// argument nothing granted stays ungranted through it.
#[test]
fn build_markdown_carries_its_arguments_authority() {
    let (hir, split) = compile(MARKDOWN_OF_A_COMPUTED_READ);
    let writers = Writers::of(&hir, &split);
    let solution = Solution::solve(&hir, &writers);
    let integrity = Integrity::new(&hir, &solution);
    let markdown = build_markdown_expr(&hir);
    let (authority, grant) = integrity.of(markdown);
    assert_eq!(authority, Authority::Untrusted);
    assert_eq!(grant, None);
}

// ---------------------------------------------------------------------
// Fixture plumbing.
// ---------------------------------------------------------------------

/// The one `build read` expression in a fixture.
fn build_read_expr(hir: &zdc_hir::Hir) -> zdc_hir::ExprId {
    capability_expr(hir, zdc_hir::BuildCapability::Read)
}

/// The one `build markdown` expression in a fixture.
fn build_markdown_expr(hir: &zdc_hir::Hir) -> zdc_hir::ExprId {
    capability_expr(hir, zdc_hir::BuildCapability::Markdown)
}

fn capability_expr(hir: &zdc_hir::Hir, wanted: zdc_hir::BuildCapability) -> zdc_hir::ExprId {
    hir.exprs
        .iter()
        .find(|(_, expr)| {
            matches!(
                &expr.kind,
                zdc_hir::HirExprKind::Build { capability, .. } if *capability == wanted
            )
        })
        .map(|(id, _)| id)
        .expect("the fixture writes one")
}

fn init_of(hir: &zdc_hir::Hir, signal: zdc_hir::DefId) -> zdc_hir::ExprId {
    match &hir.defs[signal].kind {
        DefKind::Signal(s) => s.init,
        // Written out rather than wildcarded, for the reason every match
        // over `DefKind` in this workspace is: a new kind that *does* have
        // an initialiser must be a compile error here, not a panic in a
        // test nobody reads until it fires.
        DefKind::Function(_)
        | DefKind::View(_)
        | DefKind::Record(_)
        | DefKind::Choice(_)
        | DefKind::Component(_)
        | DefKind::Foreign(_)
        | DefKind::Release(_) => panic!("not a signal"),
    }
}

/// What a *read* of a signal is worth, which is the question G-SIG asks.
///
/// It goes through [`Solution`] rather than through the expression walk,
/// because G-SIG clause 2 reads the initialiser and an initialiser may call
/// a function whose body reads another signal — so the answer is a
/// fixpoint's, not a walk's.
fn read_of(hir: &zdc_hir::Hir, writers: &Writers, signal: zdc_hir::DefId) -> Authority {
    let solution = Solution::solve(hir, writers);
    let integrity = Integrity::new(hir, &solution);
    integrity.of_signal_read(signal)
}
