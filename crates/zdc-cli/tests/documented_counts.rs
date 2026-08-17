//! Counts this repository states in prose, checked against the tree.
//!
//! Both documents say how many examples there are, in words. Both have
//! been wrong, more than once, and each time they were right when written:
//! a count is a fact about the tree restated in prose, and nothing failed
//! when the two diverged. `tree/` and `tree-webgl/` went uncounted across
//! several merges for exactly that reason, and the number was corrected
//! twice in one afternoon while six branches were landing.
//!
//! `resolve_examples.rs` already pins the *set* of top-level examples, so
//! a new file is a test failure until it is named there. This pins the
//! *sentences that count them*, which is the other half and the half that
//! kept rotting.
//!
//! It asserts the exact sentence rather than merely the presence of a
//! number-word, because a document that says "thirty-five" somewhere and
//! "thirty-two files" in the sentence that matters would otherwise pass.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.zd` file under `examples/`, including the two that live in a
/// directory of their own because they have assets beside them.
fn example_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("a readable directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().and_then(|e| e.to_str()) == Some("zd") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&repository().join("examples"), &mut found);
    found.sort();
    found
}

/// English for the counts these documents can plausibly reach.
///
/// Words rather than digits because that is how both documents are
/// written, and rewriting them to use digits to make a test easier would
/// be the test dictating the prose.
fn spelled(n: usize) -> String {
    const UNITS: [&str; 10] = [
        "", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    assert!((20..100).contains(&n), "no spelling for {n}");
    let (ten, unit) = (n / 10, n % 10);
    if unit == 0 {
        TENS[ten].to_string()
    } else {
        format!("{}-{}", TENS[ten], UNITS[unit])
    }
}

/// A `*.test.zd` states claims about another file and declares no view of
/// its own, so it is a file in `examples/` and not a program in it. That
/// distinction is what both documents draw, and it is drawn here the same
/// way rather than by listing names.
fn is_a_program(path: &Path) -> bool {
    !path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".test.zd"))
}

#[test]
fn the_readme_counts_the_examples_that_are_actually_there() {
    let programs = example_files().iter().filter(|p| is_a_program(p)).count();
    let readme = std::fs::read_to_string(repository().join("README.md")).expect("README.md");

    let sentence = format!(
        "All {} programs in [`examples/`](examples/)",
        spelled(programs)
    );
    assert!(
        readme.contains(&sentence),
        "`examples/` holds {programs} programs, so README.md should say:\n  {sentence}\n\
         It does not. The count is a fact about the tree and this is the only thing \
         that checks it."
    );
}

#[test]
fn status_counts_the_example_files_that_are_actually_there() {
    let files = example_files();
    let status = std::fs::read_to_string(repository().join("STATUS.md")).expect("STATUS.md");

    let sentence = format!("holds {} files", spelled(files.len()));
    assert!(
        status.contains(&sentence),
        "`examples/` holds {} files, so STATUS.md should say:\n  {sentence}\n\
         It does not.",
        files.len()
    );
}

/// The two documents count different things — files and programs — and a
/// reader comparing them should find the difference explained rather than
/// apparently contradictory. There is exactly one file that is not a
/// program today; if that ever changes, the sentences saying so have to
/// change with it.
#[test]
fn the_two_counts_differ_by_the_files_that_are_not_programs() {
    let files = example_files();
    let not_programs: Vec<_> = files.iter().filter(|p| !is_a_program(p)).collect();

    assert_eq!(
        not_programs.len(),
        1,
        "README.md and STATUS.md both name `sorting.test.zd` as the one file that is \
         not a program. That is now {} files: {:?}",
        not_programs.len(),
        not_programs
    );
}

// --- the per-crate test table (#259) -------------------------------------

/// How many tests a crate declares.
///
/// **Counted statically, and that is the definition.** `STATUS.md` used to
/// quote what a `cargo test` run printed, and #259 records why that number
/// kept rotting: it depends on which flags the run used — a bare run stops
/// at the first failing target and reports about an eighth of the suite —
/// so "the number of tests" had no definition anybody could reproduce.
///
/// A count of `#[test]` and `#[tokio::test]` attributes has one. It is the
/// number of test *functions written*, which is what the table is cited as
/// evidence of, and it does not move when a run is truncated, when a
/// machine is slow, or when an `#[ignore]` is added.
fn declared_tests(crate_name: &str) -> usize {
    fn walk(dir: &Path, found: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let source = std::fs::read_to_string(&path).unwrap_or_default();
                *found += source
                    .lines()
                    .filter(|line| {
                        let line = line.trim();
                        line == "#[test]" || line == "#[tokio::test]"
                    })
                    .count();
            }
        }
    }
    let root = repository().join("crates").join(crate_name);
    let mut found = 0;
    walk(&root.join("src"), &mut found);
    walk(&root.join("tests"), &mut found);
    found
}

/// Every `| `crate` | N |` row of STATUS.md's per-crate table.
fn documented_test_counts() -> Vec<(String, usize)> {
    let status = std::fs::read_to_string(repository().join("STATUS.md")).expect("STATUS.md");
    let mut rows = Vec::new();
    for line in status.lines() {
        let line = line.trim();
        if !line.starts_with("| `zdc-") {
            continue;
        }
        let mut cells = line.split('|').map(str::trim);
        cells.next();
        let Some(name) = cells.next() else { continue };
        let Some(count) = cells.next() else { continue };
        let name = name.trim_matches('`');
        if let Ok(count) = count.parse::<usize>() {
            rows.push((name.to_string(), count));
        }
    }
    rows
}

/// **The table says what the tree contains.**
///
/// #259 found six rows off by more than twenty and did not fix them,
/// because §2's milestone table quotes the same figures and correcting one
/// leaves the file disagreeing with itself. This is the fix it asked for
/// instead: a measurement rather than a recount, so the class closes and
/// not the instance.
#[test]
fn the_per_crate_test_table_matches_the_crates() {
    let documented = documented_test_counts();

    // Non-vacuity. A parser that matched no rows would report every count
    // correct, which is the failure mode a table-scraping test has.
    assert!(
        documented.len() >= 15,
        "found only {} rows in STATUS.md's per-crate table, so the scan stopped working \
         rather than the table losing its crates: {documented:?}",
        documented.len()
    );

    let wrong: Vec<String> = documented
        .iter()
        .filter_map(|(name, said)| {
            let measured = declared_tests(name);
            (measured != *said).then(|| format!("{name}: table says {said}, tree has {measured}"))
        })
        .collect();

    assert!(
        wrong.is_empty(),
        "STATUS.md's per-crate test counts have drifted from the crates they describe. \
         These counts are cited as evidence of coverage, and a number nobody checks is a \
         claim nobody checks:\n  {}",
        wrong.join("\n  ")
    );
}

/// Every crate appears, so a crate added later cannot be quietly absent
/// from the table that is offered as the coverage story.
#[test]
fn every_crate_has_a_row() {
    let documented: BTreeSet<String> = documented_test_counts()
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    let missing: Vec<String> = std::fs::read_dir(repository().join("crates"))
        .expect("the crates directory")
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| !documented.contains(name))
        .collect();

    assert!(
        missing.is_empty(),
        "these crates have no row in STATUS.md's per-crate test table: {missing:?}"
    );
}

/// **README's headline test count, held to the tree.**
///
/// This file already checks the *example* counts both documents state, and
/// its opening explains why: a count is a fact about the tree restated in
/// prose, and nothing fails when the two diverge. The headline in README's
/// Status section was the one such number nothing checked, and it had
/// rotted the furthest — **2358 tests across 20 crates against a tree of
/// 2710 across 21**, wrong on `main` for months, in the sentence a reader
/// meets first.
///
/// # Why a floor and not an equality
///
/// The per-crate table below is exact, and that is right for a table whose
/// whole purpose is per-crate coverage. Applying the same rule to a
/// headline would tax every branch that adds a test with an edit to a
/// second document — and on a night when a dozen branches were open, the
/// per-crate gate alone reddened nine of them, each for a number that was
/// correct when written.
///
/// A floor cannot rot in the direction that misleads. The suite growing
/// past it leaves the claim true; only *removing* tests below it fails,
/// which is exactly when someone should be made to look.
#[test]
fn the_readme_does_not_overstate_its_own_test_count() {
    let readme = std::fs::read_to_string(repository().join("README.md")).expect("README.md");
    let claimed: usize = readme
        .split("Over ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|word| word.parse().ok())
        .expect(
            "README's Status section states a test floor as `Over <n> tests pass across …`. \
             If the wording changed, change this test with it rather than deleting it — the \
             number is the claim a reader meets first.",
        );

    // Walked rather than listed, the same way `every_crate_has_a_row` walks
    // it: a hand-kept list of crates is the very thing this file exists to
    // stop trusting.
    let measured: usize = std::fs::read_dir(repository().join("crates"))
        .expect("the crates directory")
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .map(|name| declared_tests(&name))
        .sum();
    assert!(
        measured >= claimed,
        "README claims over {claimed} tests and the tree has {measured}. A floor only fails \
         when tests are removed below it, so this is either a real loss of coverage or a \
         claim that was never true."
    );

    // Non-vacuity, and the reason it is here: a parser that matched
    // nothing, or a crate list that stopped being walked, would satisfy the
    // inequality above by measuring zero against a claim of zero.
    assert!(
        claimed > 1_000 && measured > 1_000,
        "claimed {claimed} and measured {measured} — one of the two is not being read"
    );
}

/// **The crate count and the gate list are facts about the tree.**
///
/// STATUS.md said a "20-crate Cargo workspace" with `zdc-fmt` the
/// twentieth, and there are 21 — `zdc-wasm` arrived after the sentence
/// was written. It said "eight scripted gates" and named seven of the
/// nine in `scripts/`; the two it omitted were `check-message-budget.py`
/// and `check-installer.sh`, both of which CI runs and one of which fails
/// builds.
///
/// The same argument as the per-crate table above: these are counts of
/// things on disk, restated in prose, with nothing to notice when they
/// diverge. Counted here instead.
#[test]
fn the_workspace_description_matches_the_workspace() {
    let root = repository();
    let status = std::fs::read_to_string(root.join("STATUS.md")).expect("STATUS.md");

    let crates = std::fs::read_dir(root.join("crates"))
        .expect("a crates directory")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().join("Cargo.toml").is_file())
        .count();
    assert!(
        crates >= 15,
        "found only {crates} crates, so the scan stopped working"
    );
    assert!(
        status.contains(&format!("**{crates}-crate** Cargo workspace")),
        "STATUS.md should say `**{crates}-crate** Cargo workspace`; the tree has {crates}"
    );

    let gates: Vec<String> = std::fs::read_dir(root.join("scripts"))
        .expect("a scripts directory")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("check-"))
        .collect();
    assert!(
        gates.len() >= 5,
        "found only {} gates, so the scan stopped working",
        gates.len()
    );

    // Every gate is named, so one that is added and not described is a
    // failure here rather than a line nobody reads.
    let missing: Vec<&String> = gates
        .iter()
        .filter(|name| !status.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "STATUS.md does not mention {missing:?}, and CI runs them"
    );
}
