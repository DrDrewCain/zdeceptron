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
    // 27 before `prelude/math.zd`, which added fourteen pure ones: the
    // circular family and its inverses, `exp` and three logarithms, `cbrt`,
    // `hypotenuse` and `hyperbolicTangent`. None of them is impure — a
    // transcendental function of its argument is the definition of pure,
    // and `clock` remains the only primitive that is not.
    assert_eq!(pure, 41);
    assert_eq!(impure, ["clock"]);
    assert_eq!(pure + impure.len(), 42, "the primitive layer is forty-two");
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

const CHOSEN_FILE: &str = r#"
state chosen is client Option of Text starting None

view
    Column
        FileInput chosen
"#;

/// A file a reader chose is untrusted input, and the lattice already knows
/// (#47).
///
/// **This is the assertion that says the new element needed no new rule.**
/// The name a `FileInput` yields is chosen by whoever made the file —
/// `../../etc/passwd` is a legal filename on several systems, and a name
/// is exactly the kind of value §18.1 keeps out of a path. Nothing was
/// added to [`Grant`] or to the walk to say so: `Site::Bind` records a
/// two-way binding whatever element made it, so a picker's signal has a
/// writer and fails G-SIG's second clause for `Input`'s reason.
///
/// Written down rather than assumed, because the enumeration being closed
/// is the property, and an element added without a `Site::Bind` would be
/// a new source of attacker-chosen text that read as Trusted.
#[test]
fn the_name_of_a_chosen_file_is_untrusted() {
    let (hir, split) = compile(CHOSEN_FILE);
    let writers = Writers::of(&hir, &split);
    let chosen = def_named(&hir, "chosen");

    assert!(
        writers.is_written(chosen),
        "a `FileInput` binding must count as a write site: the browser writes it"
    );
    assert_eq!(read_of(&hir, &writers, chosen), Authority::Untrusted);
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

const CLOCK: &str = r#"
state elapsed is client Decimal every "250ms"
state motion is client Decimal every frame
state ready is client Truth after "2s"

view
    Column
        Text elapsed
        Text motion
        Text ready
"#;

/// **The clock conjunct** — the argument `a_two_way_bound_signal_is_
/// untrusted` makes, one step further out.
///
/// A clock signal has no `set` anywhere and a literal resting value, so
/// G-SIG clause 2 as §21.7.3 writes it would call a read of one Trusted.
/// That is wrong twice over. The browser's scheduler writes the cell, so
/// "no write site in this program" does not entail "holds its
/// initialiser"; and the value is *environmental*, because a visitor
/// controls their own clock. §21.9 already reached that verdict for the
/// prelude's `clock`, and the two spellings of "what time is it" must not
/// disagree about who may trust the answer.
#[test]
fn a_clock_signal_is_untrusted() {
    let (hir, split) = compile(CLOCK);
    let writers = Writers::of(&hir, &split);
    for name in ["elapsed", "motion", "ready"] {
        let id = def_named(&hir, name);
        assert!(
            writers.is_written(id),
            "`{name}` is written by the browser's scheduler, so it has a writer"
        );
        assert_eq!(
            read_of(&hir, &writers, id),
            Authority::Untrusted,
            "a read of `{name}`"
        );
    }
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

// The two fixtures below name a URL rather than the bare `marked` they used
// to, and the change is deliberately not a change of subject: a bare
// specifier now resolves only through a project's `[packages]` table
// (#238), and these compile a source string with no project beside it. A
// URL resolves on its own, so what these tests are about — whether `gives
// pure` is the marker `rel_pure` demands — is all that is left varying.

const ANYWHERE_FOREIGN: &str = r#"
foreign renderMarkdown is anywhere
    from  "https://esm.sh/marked@15.0.7" as "parse"
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
    from  "https://esm.sh/marked@15.0.7" as "parse"
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

// ---------------------------------------------------------------------
// `remembered` — the browser's own store, and the round trip through it.
// ---------------------------------------------------------------------

/// The laundering program and its control, differing in **one word**.
///
/// Both declare a signal with a literal initialiser that nothing in the
/// program ever writes, derive from it directly, and derive from it again
/// through a call. Under G-SIG clause 2 — *no write site anywhere, so it
/// holds its initialiser* — every one of those reads is Trusted, and for
/// the `client` version that is the right answer: the cell is one tab's
/// memory, it is created at load with `"root"` in it, and nothing else on
/// the machine can reach it.
///
/// It is the wrong answer for the `remembered` version, and that is the
/// whole attack. `starting "root"` describes the value on a browser that
/// has never run this program. On every later visit the value is whatever
/// is in the browser's store — put there by an earlier session of this
/// program, by another tab, or by any other script on the origin. None of
/// those is among this program's statement forms, so the whole-program
/// query G-SIG asks cannot see any of them: it looks for a `set`, there
/// is no `set`, and it concludes the cell still holds the literal. A
/// literal goes in on Monday and an attacker's value comes back on
/// Tuesday, with the compiler reporting the same clean build both times.
const ROUND_TRIP: &str = r#"
state stash is PLACEMENT Text starting "root"
state shown is client Text from stash

function tag with t
    give t

state via is client Text from tag with t is stash

view
    Column
        Text shown
        Text via
"#;

/// **A value cannot be laundered to Trusted by round-tripping it through
/// the browser's store.**
///
/// The single most important assertion in this file, and it is written as
/// a *pair* rather than as one verdict. A single "it is Untrusted" proves
/// almost nothing on its own: under a closed lattice Untrusted is the
/// default, so an assertion that some signal is Untrusted would go on
/// passing if the analysis stopped looking at the program altogether. The
/// two fixtures differ in exactly one token — `client` against
/// `remembered` — so the difference in verdict can have exactly one cause,
/// and the `client` half is what proves the pass is still awarding the
/// grant it is supposed to award.
///
/// The verdict is checked at three reads, not one: the signal itself, a
/// signal derived from it, and a signal derived from it through a
/// function call. Laundering is a question about where a value *ends up*,
/// so a rule that held at the declaration and leaked one derivation later
/// would be no rule at all.
///
/// **This test fails if laundering is possible.** Return `false` from
/// `SignalPlacement::is_externally_written`'s `Remembered` arm — or add a
/// sixth placement and classify it wrongly — and `stash` regains G-SIG
/// clause 2, all three reads come back Trusted, and the first three
/// assertions below fail.
#[test]
fn a_value_cannot_be_laundered_to_trusted_through_browser_storage() {
    let stored_src = ROUND_TRIP.replace("PLACEMENT", "remembered");
    let (hir, split) = compile(&stored_src);
    let writers = Writers::of(&hir, &split);

    for name in ["stash", "shown", "via"] {
        let def = def_named(&hir, name);
        assert_eq!(
            read_of(&hir, &writers, def),
            Authority::Untrusted,
            "`{name}` derives from a cell holding whatever a previous session, another tab, or \
             another script on the origin put there, so no initialiser says anything about it"
        );
    }
    assert!(
        writers.is_written(def_named(&hir, "stash")),
        "the store is the writer G-SIG clause 2 has to account for, and the program contains no \
         `set` for a statement-form query to find"
    );

    let memory_src = ROUND_TRIP.replace("PLACEMENT", "client");
    let (hir, split) = compile(&memory_src);
    let writers = Writers::of(&hir, &split);

    for name in ["stash", "shown", "via"] {
        let def = def_named(&hir, name);
        assert_eq!(
            read_of(&hir, &writers, def),
            Authority::Trusted,
            "one word away, a `client` cell nothing writes really does hold its initialiser — if \
             `{name}` ever fails here, the assertions above are passing for the wrong reason"
        );
    }
    assert!(!writers.is_written(def_named(&hir, "stash")));
}

/// The same value at an obligation site, which is where a program is
/// actually refused.
///
/// **Not a difference test, and deliberately not written as one.** The
/// `client` version of this program is refused too, by §21.8.4's `Lift`
/// conjunct: `mine` is a server derivation, so the browser *sends*
/// `stash`, and what arrives is whatever the browser chose to send whether
/// a `set` exists or not. That is the correct answer for both, and it
/// means this site cannot distinguish the two placements — which is worth
/// stating rather than papering over. The placement classification is
/// load-bearing at the *label*, tested above; here it is defence in depth,
/// and defence in depth is only worth having if somebody checks the second
/// layer is actually there.
#[test]
fn indexing_a_trusted_place_with_a_remembered_value_is_refused() {
    let src = r#"
trusted state orders is durable Map of Text to Text starting empty

state stash is remembered Text starting "root"
state mine  is server Text from orders at stash

view
    Column
        Text "x"
"#;
    let (hir, split) = compile(src);
    let analysis = zdc_graph::authority::authority(&hir, &split);
    let raised: Vec<&str> = analysis.errors().map(|e| e.code).collect();
    assert!(
        raised.contains(&"E-INT-02"),
        "the index into a `trusted` collection came out of the browser's store: {raised:?}"
    );
}

/// Writing a value out of the store into a `trusted` place is refused at
/// the write — A3, not A1.
///
/// The read side and the write side are separate obligations and closing
/// one is not closing the other. Like the test above this is defence in
/// depth: a command argument crosses, so the `client` version is refused
/// here too.
#[test]
fn writing_a_remembered_value_into_a_trusted_place_is_refused() {
    let src = r#"
trusted state ledger is durable Text starting ""

state stash is remembered Text starting "root"

view
    Column
        Button "save"
            on click
                set ledger to stash
"#;
    let (hir, split) = compile(src);
    let analysis = zdc_graph::authority::authority(&hir, &split);
    let raised: Vec<&str> = analysis.errors().map(|e| e.code).collect();
    assert!(
        raised.contains(&"E-INT-03"),
        "a value out of the browser's store reached a `trusted` place: {raised:?}"
    );
}

/// `trusted remembered` is not spellable — E-INT-01.
///
/// The declaration is the other way the grant could be obtained, and it is
/// the direct one: G-SIG clause 1 awards Trusted to any signal declared
/// `trusted`, with no round trip to arrange and no derivation to trace. A
/// rule that closed clause 2 and left clause 1 open would have closed
/// nothing.
#[test]
fn a_remembered_signal_cannot_be_declared_trusted() {
    let src = r#"
trusted state stash is remembered Text starting "root"

view
    Column
        Text stash
"#;
    let (hir, _) = compile(src);
    let raised = zdc_graph::integrity::int_01(&hir);
    let found: Vec<&str> = raised.iter().map(|e| e.code).collect();
    assert_eq!(found, ["E-INT-01"]);
    assert!(
        raised[0].message.contains("any other script on the origin"),
        "the diagnostic has to say why, not only that: {}",
        raised[0].message
    );
}

/// A `secret` may not live in the browser's store — E0313, with a reason.
///
/// The other half of the placement's declaration rules, and the one the
/// survey of the target site found already violated in the wild: an OAuth
/// refresh token in `localStorage`, readable by every script on the origin
/// and still there after the visit.
#[test]
fn a_remembered_signal_cannot_be_declared_secret() {
    let src = r#"
secret state token is remembered Text starting ""

view
    Column
        Text "x"
"#;
    let (_, split) = compile(src);
    let raised = codes(&split.diagnostics);
    assert!(
        raised.contains(&"E0313"),
        "a secret in the browser's own store must be refused: {raised:?}"
    );
    assert!(!zdc_types::SignalPlacement::Remembered.may_be_secret());
}

/// Every placement something outside the program can write is classified
/// as one, and the list is asserted rather than assumed.
///
/// `Writers::of` asks `is_externally_written`, and that function is where
/// a sixth placement has to be ruled on. This pins the classification for
/// the reason `only_server_and_durable_placements_may_hold_a_secret` pins
/// the other one: an exhaustive match makes a new variant a compile error,
/// and no test can stand in for that — but a later edit could still
/// quietly flip an existing arm and leave both enforcement sites agreeing
/// with a rule nobody meant.
#[test]
fn the_placements_a_program_does_not_own_are_written_out() {
    use zdc_types::SignalPlacement as P;

    assert!(P::Durable.is_externally_written());
    assert!(P::DurablePerVisitor.is_externally_written());
    assert!(P::Remembered.is_externally_written());

    assert!(!P::Client.is_externally_written());
    assert!(!P::Static.is_externally_written());
    assert!(!P::Server.is_externally_written());
}

// --- §14G.4's scheduled trigger, against the lattice (#18) ---------------

const SCHEDULED_JOB: &str = r#"
state visits is durable Whole starting 0

state hourly is server Whole every "1h"
    add 1 to visits

view
    Column
        Text "hi"
"#;

/// **A beat is Untrusted, and the lattice needed no new rule to say so.**
///
/// This is the question a new *source* of data has to be asked, and the
/// answer is the default-closed one: §21.7.0 makes a value Untrusted
/// unless one of the closed set of grants applies, and no grant covers a
/// timestamp the platform chose. G-ENV is the near miss and it is the
/// instructive one — the operator set an environment variable, and nobody
/// set the beat.
///
/// It would have been easy to reason the other way. The schedule is in a
/// config file this compiler generated from this program's own text, so
/// the *cadence* is as trusted as the source is. The **time** is not the
/// cadence: it is the platform's reading of a clock, and `clock` is the
/// one impure primitive the language has, admitted to the fold by §21.9
/// only behind a `gives pure` marker precisely so that a reading cannot
/// launder itself into evidence. A ninth grant would have to be argued
/// for on that ground, and `the_grant_set_is_closed_at_eight` above is
/// what makes adding one deliberate: the set did not grow for this.
///
/// **This test failed when it was first written, and that is why it is
/// here.** A scheduled cell's declaration carries a resting `0` so that
/// every pass sees an expression rather than a hole — and G-SIG clause 2
/// reads a signal with no write site as holding its initialiser, so the
/// beat came out **Trusted on the strength of a literal nothing ever
/// reads**. `Writers::of` had the conjunct for exactly this and it named
/// only `clock`; the repair is that a schedule joins it.
#[test]
fn a_beat_is_untrusted_and_needs_no_new_grant() {
    let (hir, split) = compile(SCHEDULED_JOB);
    let writers = Writers::of(&hir, &split);
    let hourly = def_named(&hir, "hourly");

    assert!(
        writers.is_written(hourly),
        "the scheduler puts the beat in the cell, and no `set` in the program says so"
    );
    assert_eq!(read_of(&hir, &writers, hourly), Authority::Untrusted);
}
