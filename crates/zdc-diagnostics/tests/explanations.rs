//! Every code the compiler can produce has an explanation.
//!
//! The code list is **enumerated from the compiler's source**, not from a
//! list maintained beside it. A hand-maintained list would be correct on
//! the day it was written and wrong on the day someone added a code, which
//! is exactly the day the test needed to fail.
//!
//! Three crates produce codes and all three are scanned. `zdc-parser`
//! reports syntax (§4.1's `E01…`); `zdc-graph` reports placement, secrecy,
//! integrity (§18.1's `E-INT-…`) and declassification (§19's `E-REL-…`);
//! `zdc-types` reports types and routing. Scanning only the first two is
//! how four codes once reached a release with no `zdc explain` entry behind
//! them, and scanning for only *some* of the prefixes is how the `E-REL-…`
//! family reached one — the shape test below is the fix, so it must list
//! every prefix the spec uses.
//!
//! `zdc-parser` was added when parse errors gained codes. The gate is what
//! makes that stick: the family cannot lose its explanations, and a
//! seventh parse code cannot be added without one.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use zdc_diagnostics::explain;

/// Every crate that can print a diagnostic code.
fn code_producing_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![
        root.join("../zdc-parser/src"),
        root.join("../zdc-graph/src"),
        root.join("../zdc-types/src"),
        // `zdc-codegen` joined the scan with `zdc test` (issue #169). It
        // is the only crate that reports on a program by *running* it, so
        // it is the only one whose codes could not have been found by
        // scanning the analysis passes — which is exactly the reason a
        // scan of "the crates that report" has to name it rather than
        // assume the list is closed.
        root.join("../zdc-codegen/src"),
    ]
}

/// Every string literal in `dir` that has the shape of a diagnostic code.
///
/// Deliberately a scan for the *shape* rather than for `GraphError::new`
/// call sites: a code that is built by a helper, matched on, or passed
/// through a table is still a code the compiler can print, and the point
/// of enumerating from source is that no such route is missed.
fn codes_in_source(dir: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let entries = std::fs::read_dir(dir).expect("the zdc-graph source directory is readable");
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("a readable source file");
        for literal in string_literals(&source) {
            found.extend(codes_in_literal(&literal));
        }
    }
    found
}

/// The contents of every `"…"` literal in a source file. Escapes are not
/// interpreted, which is fine: a diagnostic code contains none.
fn string_literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut literal = String::new();
        let mut escaped = false;
        for (_, c) in chars.by_ref() {
            if escaped {
                escaped = false;
                literal.push(c);
                continue;
            }
            match c {
                '\\' => escaped = true,
                '"' => break,
                _ => literal.push(c),
            }
        }
        out.push(literal);
    }
    out
}

/// Every code inside one string literal.
///
/// A code is not always the whole literal. `zdc-graph` builds a
/// `GraphError` from a bare `"E0311"`; `zdc-types` writes the code into
/// the sentence it belongs to, as `"… (E-INT-03)."`. Both are codes the
/// compiler prints, so both are found here — the point of enumerating
/// from source is that no route to a printed code is missed.
fn codes_in_literal(literal: &str) -> Vec<String> {
    let chars: Vec<char> = literal.chars().collect();
    let mut out = Vec::new();
    for start in 0..chars.len() {
        // A code begins at a word boundary, so `SUFFIXE0311` is not one.
        if start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '-') {
            continue;
        }
        // The lengths a code can have: `E0311` is 5, `E-IFC-05` is 8, and
        // `E-TEST-01` is 9. A family whose length is missing here is a
        // family the scan cannot see, which is the silent failure the
        // comment on `looks_like_a_code` describes.
        for len in [5, 8, 9] {
            if start + len > chars.len() {
                continue;
            }
            let candidate: String = chars[start..start + len].iter().collect();
            if looks_like_a_code(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

/// `E0301`, `W0330`, `E-IFC-05`, `E-INT-03`, `E-REL-08`, `W-REL-01`,
/// `E-URL-01`, `E-TEST-01` — the shapes the spec uses.
///
/// Every family is listed, and adding one here is the price of adding one
/// to the compiler. `E-URL-` was left out when it arrived, and the effect
/// was not a failing test: the scan simply did not see the code, so it was
/// neither reported as unexplained nor reported as stale. A family this
/// function does not know about is a family with no coverage at all.
fn looks_like_a_code(literal: &str) -> bool {
    let numeric = |rest: &str| rest.len() == 4 && rest.chars().all(|c| c.is_ascii_digit());
    if let Some(rest) = literal
        .strip_prefix('E')
        .or_else(|| literal.strip_prefix('W'))
    {
        if numeric(rest) {
            return true;
        }
    }
    let two_digits = |rest: &str| rest.len() == 2 && rest.chars().all(|c| c.is_ascii_digit());
    literal
        .strip_prefix("E-IFC-")
        .or_else(|| literal.strip_prefix("E-INT-"))
        .or_else(|| literal.strip_prefix("E-REL-"))
        .or_else(|| literal.strip_prefix("W-REL-"))
        .or_else(|| literal.strip_prefix("E-URL-"))
        // `E-TEST-` arrived with `zdc test` (issue #169). Listing it here
        // is the price named above: a family this function does not know
        // about is a family with no coverage at all, reported neither as
        // unexplained nor as stale.
        .or_else(|| literal.strip_prefix("E-TEST-"))
        .is_some_and(two_digits)
}

#[test]
fn every_code_in_the_source_has_an_explanation() {
    let in_source: BTreeSet<String> = code_producing_sources()
        .iter()
        .flat_map(|dir| codes_in_source(dir))
        .collect();

    // Non-vacuity. A scanner that matched nothing would otherwise report
    // that every code is explained, which is the failure mode the
    // forbid-unsafe script was once able to have.
    assert!(
        in_source.len() >= 20,
        "the scan found only {} codes, which means it stopped working rather than \
         that the compiler lost twenty diagnostics: {in_source:?}",
        in_source.len()
    );

    let explained: BTreeSet<String> = explain::codes().iter().map(|c| c.to_string()).collect();

    let missing: Vec<&String> = in_source.difference(&explained).collect();
    assert!(
        missing.is_empty(),
        "these codes are produced by the compiler and have no `zdc explain` entry: {missing:?}"
    );

    let stale: Vec<&String> = explained.difference(&in_source).collect();
    assert!(
        stale.is_empty(),
        "these codes have a `zdc explain` entry and are no longer produced: {stale:?}"
    );
}

/// **No two entries may claim the same code**, and the near-miss that
/// motivates this is worth stating.
///
/// Three branches open at once each took `E0362` — `media` on #286, the
/// outbound request on #288, and the document key handler on #290. Each
/// author did the right thing: read the table, found the highest `E03xx`
/// in use, took the next. Nothing records that a number is *spoken for*
/// by a branch that has not merged, so "the next free code" is a question
/// whose answer goes stale the moment somebody else asks it.
///
/// The counted assertion in `conversion_contract.rs` does not catch this.
/// It checks the table's **length**, and two entries sharing a code with
/// different text pass a length check whenever the arithmetic happens to
/// work out. What would have shipped is worse than a duplicate: `explain`
/// returns the *first* match, so the second feature's rule becomes
/// unreachable and `zdc explain E0362` prints one feature's explanation
/// for the other feature's error — with every gate green.
#[test]
fn no_two_explanations_claim_the_same_code() {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut duplicated: Vec<&str> = Vec::new();
    for entry in explain::EXPLANATIONS {
        if !seen.insert(entry.code) {
            duplicated.push(entry.code);
        }
    }
    assert!(
        duplicated.is_empty(),
        "these codes have more than one entry, so `zdc explain` reaches only the first \
         and the others describe an error nobody can look up: {duplicated:?}"
    );
}

/// `codes()` is what the coverage test above compares against, so a
/// duplicate there would hide a missing explanation rather than report
/// one: the set it builds would silently be smaller than the table.
#[test]
fn the_code_list_has_one_entry_per_explanation() {
    let codes = explain::codes();
    let unique: BTreeSet<&str> = codes.iter().copied().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "`codes()` returned {} codes of which {} are distinct; the coverage test compares \
         sets, so a duplicate here is a code that cannot be found missing",
        codes.len(),
        unique.len()
    );
}

/// An explanation with an empty section is an entry that exists to satisfy
/// the test above rather than to help a reader.
#[test]
fn every_explanation_says_something_in_all_three_sections() {
    for entry in explain::EXPLANATIONS {
        let code = entry.code;
        assert!(!entry.name.is_empty(), "{code} has no name");
        // The caret label is what replaced the word `here`. An entry with
        // an empty one puts `here`'s silence back, so it is checked in the
        // same place the other sections are.
        assert!(
            entry.caret.len() > 12,
            "{code}'s caret label is too short to say anything: {:?}",
            entry.caret
        );
        assert_ne!(entry.caret, "here", "{code}'s caret label says nothing");
        assert!(
            entry.meaning.len() > 80,
            "{code}'s `what it means` is too short to mean anything"
        );
        assert!(
            entry.why.len() > 80,
            "{code}'s `why the rule exists` is too short to mean anything"
        );
        assert!(
            entry.example.len() > 60,
            "{code}'s worked example is too short to be worked"
        );
        let rendered = entry.render();
        assert!(rendered.contains(code), "{code} is not in its own output");
        assert!(rendered.contains("How to fix it"), "{code} has no repair");
    }
}

#[test]
fn a_code_can_be_looked_up_and_an_unknown_one_cannot() {
    assert!(explain::explain("E-IFC-05").is_some());
    assert!(explain::explain("E0301").is_some());
    assert!(explain::explain("E-IFC-04").is_none());
    assert!(explain::explain("").is_none());
}
