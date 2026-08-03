//! Every code the compiler can produce has an explanation.
//!
//! The code list is **enumerated from the compiler's source**, not from a
//! list maintained beside it. A hand-maintained list would be correct on
//! the day it was written and wrong on the day someone added a code, which
//! is exactly the day the test needed to fail.
//!
//! Two crates produce codes and both are scanned. `zdc-graph` reports
//! placement, secrecy, integrity (§18.1's `E-INT-…`) and declassification
//! (§19's `E-REL-…`); `zdc-types` reports types and routing. Scanning only
//! the first is how four codes once reached a release with no `zdc explain`
//! entry behind them, and scanning for only *some* of the prefixes is how
//! the `E-REL-…` family reached one — the shape test below is the fix, so
//! it must list every prefix the spec uses.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use zdc_diagnostics::explain;

/// Every crate that can print a diagnostic code.
fn code_producing_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![root.join("../zdc-graph/src"), root.join("../zdc-types/src")]
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
        for len in [5, 8] {
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

/// `E0301`, `W0330`, `E-IFC-05`, `E-INT-03`, `E-REL-08`, `W-REL-01` — the
/// shapes the spec uses.
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

/// An explanation with an empty section is an entry that exists to satisfy
/// the test above rather than to help a reader.
#[test]
fn every_explanation_says_something_in_all_three_sections() {
    for entry in explain::EXPLANATIONS {
        let code = entry.code;
        assert!(!entry.name.is_empty(), "{code} has no name");
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
