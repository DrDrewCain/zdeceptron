//! No shipped sentence tells a user their program is robust.
//!
//! §21.8.8's decision was taken on **option 2** (ratified 2026-08-06): the
//! declaration shape, the report, `limit`, REL-PLACE′, REL-CLOSED,
//! REL-PURE and REL-ARG all stay built, as **review aids rather than
//! guarantees**, and §21.7.10's claim stays withdrawn. Everything in §21.7
//! stays; only the claim goes.
//!
//! That decision is currently held in prose — `zdc-graph`'s `integrity.rs`
//! module doc says callers "must not turn any of this into a promise", and
//! `zdc-cli`'s `build` doc says "do not emit a field that answers 'is this
//! program safe', because nothing here answers that". Prose is what the
//! rule was already stated in when §21.6 item 18 forbade REL-ARG the first
//! time, and it was restated because the prose did not stop it.
//!
//! So this is the mechanism. §21.8.8 names three surfaces the claim must
//! not reach — `report.json`'s framing, the diagnostics' help text, and
//! §21.7.10's sentence — and two of the three exist today and are scanned
//! here. The third does not exist yet: `--report` is unimplemented, and
//! `zdc-cli` carries the standing instruction that when it lands it must
//! not carry `attacker_reachable`. When it lands, its strings belong in
//! [`shipped_text`] on the same day.
//!
//! # Why the affirmative form, and not the word
//!
//! Banning the word `robust` outright would forbid the honest sentences
//! too — a diagnostic that says a rule *does not* make a program robust is
//! exactly what option 2 wants written. What is banned is the **claim**,
//! so the patterns below are affirmative shapes that no correct sentence
//! in this codebase has a reason to contain. A negation like "is not
//! robust" does not contain "is robust", which is what makes the scan
//! usable rather than a source of false positives to be silenced.
//!
//! The list is deliberately short. A long banned-phrase list is a list
//! someone eventually adds an exception to, and an exception is how the
//! claim gets back in.

use zdc_diagnostics::explain;

/// The claim shapes that must not ship, and why each one is a claim.
///
/// Each is affirmative: it asserts a property of the user's program rather
/// than describing what a rule checked. §21.8.8's own framing is the test
/// to apply when adding one — does the sentence tell a user their program
/// is safe, when three independent adversarial passes broke the argument
/// that it is?
const FORBIDDEN: &[&str] = &[
    // §21.7.10's withdrawn sentence, and the vocabulary around it.
    "is robust",
    "are robust",
    "program is safe",
    "programs are safe",
    // The report's withdrawn verdict. §21.8.3: for `launder3.zd` the
    // `attacker_reachable` list is empty and a visitor steers the
    // declassification with a query string, so this reads as a verdict and
    // would be a false one.
    "no visitor can",
    "cannot be steered",
    // §21.8.7: `limit` is per declaration, per anonymous session, and
    // unenforced, so nothing here bounds cumulative disclosure.
    "free of laundering",
    "bounds cumulative disclosure",
    // A guarantee by any other name.
    "guarantees that",
    "we guarantee",
    "proves that no",
];

/// Every string that reaches a user, from the surfaces that exist today.
///
/// The long form behind `zdc explain` is the whole of it for now: it is
/// where §21.8.8's "diagnostics' help text" lives, and it is the surface
/// with room enough to editorialise, which is what makes it the one most
/// likely to acquire a claim.
fn shipped_text() -> Vec<(String, String)> {
    explain::EXPLANATIONS
        .iter()
        .map(|entry| (entry.code.to_string(), entry.render()))
        .collect()
}

#[test]
fn no_shipped_sentence_claims_a_program_is_robust() {
    let shipped = shipped_text();

    // Non-vacuity, for the same reason `explanations.rs` carries one: a
    // scanner that found no text would report that nothing claims
    // robustness, which is the failure mode this test exists to prevent
    // rather than to have.
    assert!(
        shipped.len() >= 20,
        "the scan found only {} explanations, which means it stopped working \
         rather than that the compiler lost its diagnostics",
        shipped.len()
    );

    let mut violations = Vec::new();
    for (code, text) in &shipped {
        let folded = text.to_lowercase();
        for pattern in FORBIDDEN {
            if folded.contains(pattern) {
                violations.push(format!("{code} contains {pattern:?}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "§21.8.8 option 2 withdrew the robustness claim, and these sentences \
         restate it. Say what the rule checked, not what the program is: {violations:#?}"
    );
}

/// The scanner fires on a claim, so a passing run above means the corpus is
/// clean rather than that the scan is broken.
///
/// §14G.8 item 3's own instruction — check that the tests can fail — is
/// what this is, applied to a test whose healthy state is silence. Without
/// it, deleting [`FORBIDDEN`]'s contents would turn the gate green.
#[test]
fn the_scan_detects_a_claim_when_one_is_present() {
    let claim = "This analysis is complete, so your program is robust.";
    let folded = claim.to_lowercase();
    let caught: Vec<&&str> = FORBIDDEN
        .iter()
        .filter(|pattern| folded.contains(**pattern))
        .collect();
    assert!(
        !caught.is_empty(),
        "the scanner missed an explicit robustness claim, so the gate above proves nothing"
    );

    // And the honest negation is not caught, which is the property that
    // makes the gate usable: option 2 wants these sentences written.
    let honest = "These rules are review aids. Nothing here establishes that a program \
                  is free of leaks, and a clean run is not robust in any sense.";
    let honest_folded = honest.to_lowercase();
    let false_positives: Vec<&&str> = FORBIDDEN
        .iter()
        .filter(|pattern| honest_folded.contains(**pattern))
        .collect();
    assert!(
        false_positives.is_empty(),
        "the scanner flagged an honest disclaimer, which would push authors \
         toward silence instead of accuracy: {false_positives:?}"
    );
}
