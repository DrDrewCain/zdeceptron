//! The number of examples, as `README.md` and `STATUS.md` state it.
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
