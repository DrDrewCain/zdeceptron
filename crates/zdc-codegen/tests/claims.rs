//! What a `test` declaration comes to when it is run — issue #169.
//!
//! `zdc-cli/tests/expectations.rs` covers the command and its report.
//! This file covers the two things the command cannot reach from outside:
//! the claims a runner must *not* let stop it, and the guarantee that a
//! claim is never evaluated by anything except the runner.
//!
//! The second is the one worth stating twice. A claim is emitted into the
//! same module `zdc build` evaluates, so the only thing standing between a
//! false or non-terminating expectation and a broken build is that claims
//! are printed as thunks. If that ever stops being true, a program with a
//! failing test stops *building*, and the failure would look like a
//! compiler bug rather than a test result.

mod support;

use support::build_module_of;

fn run(source: &str) -> Vec<zdc_codegen::Outcome> {
    let module = build_module_of(source, "test.zd").expect("a program with a claim has a root");
    zdc_codegen::run_tests(&module, std::path::Path::new("."))
        .unwrap_or_else(|error| panic!("the claims did not run: {}", error.report()))
}

fn verdicts(source: &str) -> Vec<(String, &'static str)> {
    run(source)
        .into_iter()
        .map(|outcome| {
            let name = match outcome.verdict {
                zdc_codegen::ClaimVerdict::Held => "held",
                zdc_codegen::ClaimVerdict::Broken(_) => "broken",
                zdc_codegen::ClaimVerdict::Unevaluable(_) => "undecided",
            };
            (outcome.claim, name)
        })
        .collect()
}

/// An expectation that never terminates is undecided, not false.
///
/// §17.4.8's bound is on work done rather than on time taken, so this
/// stops on every machine and not only on the slow ones — which is the
/// only reason a runner may report it at all instead of hanging.
#[test]
fn an_expectation_that_never_terminates_is_undecided_rather_than_broken() {
    let outcomes = run("function forever of n\n    give forever of n\n\ntest \"this claim never gets an answer\"\n    expect (forever of 1) is 1\n");

    let [outcome] = &outcomes[..] else {
        panic!("one claim, one outcome: {outcomes:?}");
    };
    let zdc_codegen::ClaimVerdict::Unevaluable(error) = &outcome.verdict else {
        panic!("a claim that never answers is undecided, not false: {outcome:?}");
    };
    assert_eq!(error.code, "E-TEST-02");
    // The claim is named. An undecided verdict with no sentence in it
    // would leave the reader with a budget message and no way to tell
    // which of their claims produced it.
    assert!(
        error.message.contains("this claim never gets an answer"),
        "the message names the claim: {}",
        error.message
    );
    assert!(
        error.message.contains("more work than"),
        "the message says the budget ran out rather than that the claim is false: {}",
        error.message
    );
}

/// One claim that cannot be decided does not abandon the ones after it.
///
/// A runner that stopped at the first problem would report one of a
/// reader's four mistakes and hide three, and the reader would fix one and
/// run it again — which is the slowest possible way to find out.
#[test]
fn a_claim_that_cannot_be_decided_does_not_abandon_the_rest_of_the_suite() {
    let verdicts = verdicts(concat!(
        "function double of n\n",
        "    give n * 2\n",
        "\n",
        "function forever of n\n",
        "    give forever of n\n",
        "\n",
        "test \"this one is asked first and holds\"\n",
        "    expect (double of 4) is 8\n",
        "\n",
        "test \"this one never answers\"\n",
        "    expect (forever of 1) is 1\n",
        "\n",
        "test \"this one is asked after the undecidable one and is false\"\n",
        "    expect (double of 4) is 9\n",
    ));

    assert_eq!(
        verdicts,
        vec![
            ("this one is asked first and holds".to_string(), "held"),
            ("this one never answers".to_string(), "undecided"),
            (
                "this one is asked after the undecidable one and is false".to_string(),
                "broken"
            ),
        ],
        "every claim is asked, in the order it was written"
    );
}

/// The two sides of a broken comparison are rendered for the report.
#[test]
fn a_broken_comparison_reports_what_each_side_came_to() {
    let outcomes = run(
        "function double of n\n    give n * 2\n\ntest \"doubling four gives nine\"\n    expect (double of 4) is 9\n",
    );

    let [outcome] = &outcomes[..] else {
        panic!("one claim, one outcome: {outcomes:?}");
    };
    let zdc_codegen::ClaimVerdict::Broken(broken) = &outcome.verdict else {
        panic!("`8 is 9` is a false claim: {outcome:?}");
    };
    assert_eq!(broken.code, "E-TEST-01");
    assert_eq!(
        broken.sides,
        Some(("8".to_string(), "9".to_string())),
        "both sides are shown, so the reader does not run the computation in their head"
    );
}

/// An expectation whose outermost operator is not a comparison has no two
/// sides, and the report says so rather than inventing a pair.
#[test]
fn an_expectation_that_is_not_a_comparison_reports_no_sides() {
    let outcomes = run(concat!(
        "function double of n\n",
        "    give n * 2\n",
        "\n",
        // An `and` at the top. Digging inside it for the `is` would report
        // one of the two pairs as though it were the claim, and the reader
        // would go and look at the half that was fine.
        "test \"both halves hold\"\n",
        "    expect ((double of 4) is 8) and ((double of 5) is 11)\n",
    ));

    let [outcome] = &outcomes[..] else {
        panic!("one claim, one outcome: {outcomes:?}");
    };
    let zdc_codegen::ClaimVerdict::Broken(broken) = &outcome.verdict else {
        panic!("the right half is false, so the claim is: {outcome:?}");
    };
    assert_eq!(
        broken.sides, None,
        "an `and` has no two values to show, and showing one pair would point at the wrong half"
    );
}

/// **The guarantee `zdc build` depends on.** A claim is a thunk, so
/// loading the build root defines it and runs nothing.
///
/// Without this, a program with one false test would fail to *build*, and
/// the thing shipping to production would be gated on an assertion that
/// has nothing to do with whether the program compiles.
#[test]
fn a_false_claim_costs_the_build_nothing() {
    let module = build_module_of(
        concat!(
            "function double of n\n",
            "    give n * 2\n",
            "\n",
            "state heading is static Text from \"Writing\"\n",
            "\n",
            "test \"doubling four gives nine\"\n",
            "    expect (double of 4) is 9\n",
            "\n",
            "view\n",
            "    Text heading\n",
        ),
        "test.zd",
    )
    .expect("a program with a `static` and a claim has a build root");

    // The claim is in the module the build evaluates…
    assert_eq!(module.tests.len(), 1);
    // …and evaluating it is untroubled by the claim being false.
    let evaluated = zdc_codegen::evaluate(&module, std::path::Path::new("."))
        .unwrap_or_else(|error| panic!("a false claim broke the build: {}", error.report()));
    assert_eq!(
        evaluated.values.get("heading").map(String::as_str),
        Some("\"Writing\""),
        "the `static` still has its value"
    );
}

/// A claim that does not terminate costs the build nothing either.
///
/// The stronger half of the test above: `false` only proves the value was
/// not *inspected*, and this proves it was not *computed*. A build root
/// that evaluated its claims eagerly would exhaust §17.4.8's budget here
/// and refuse a program that is perfectly well-formed.
#[test]
fn a_claim_that_never_terminates_costs_the_build_nothing() {
    let module = build_module_of(
        concat!(
            "function forever of n\n",
            "    give forever of n\n",
            "\n",
            "state heading is static Text from \"Writing\"\n",
            "\n",
            "test \"this claim never gets an answer\"\n",
            "    expect (forever of 1) is 1\n",
            "\n",
            "view\n",
            "    Text heading\n",
        ),
        "test.zd",
    )
    .expect("a program with a `static` and a claim has a build root");

    let evaluated =
        zdc_codegen::evaluate(&module, std::path::Path::new(".")).unwrap_or_else(|error| {
            panic!("an unterminating claim broke the build: {}", error.report())
        });
    assert_eq!(
        evaluated.values.get("heading").map(String::as_str),
        Some("\"Writing\"")
    );
}

/// A program with no claims has an empty `$tests`, and asking it produces
/// no outcomes rather than an error.
#[test]
fn a_program_with_no_claims_has_no_outcomes() {
    let module = build_module_of(
        "state heading is static Text from \"Writing\"\n\nview\n    Text heading\n",
        "test.zd",
    )
    .expect("a program with `static` has a build root");

    assert!(module.tests.is_empty());
    assert!(zdc_codegen::run_tests(&module, std::path::Path::new("."))
        .expect("asking nothing is not an error")
        .is_empty());
}
