//! `zdc test`, end to end — issue #169.
//!
//! Written before the command existed, and failing for the right reason
//! while it did not: the point of the feature is the *report*, so the test
//! is over what the report says and not over an internal data structure.
//! A runner that ran every expectation and printed nothing would satisfy
//! any test written against the runner's return value, and would be
//! useless.
//!
//! The two cases that matter are one file with a claim that holds and one
//! file with a claim that does not, because a runner that reports every
//! test as passing is the failure mode that costs the most: it is silent,
//! and it is indistinguishable from a suite that works.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args(args)
        .output()
        .expect("failed to run the zdc binary")
}

/// A file under the system temporary directory, removed when the test
/// ends whether it passed or not.
struct TempSource {
    path: PathBuf,
}

impl TempSource {
    fn new(name: &str, contents: &str) -> TempSource {
        let path = std::env::temp_dir().join(format!("zdc-{}-{name}.zd", std::process::id()));
        std::fs::write(&path, contents).expect("failed to write the temporary source file");
        TempSource { path }
    }

    fn arg(&self) -> &str {
        self.path.to_str().expect("a UTF-8 temporary path")
    }
}

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// Two functions and two claims about them, one of each verdict.
const MIXED: &str = "\
function double of n
    give n * 2

test \"doubling four gives eight\"
    expect (double of 4) is 8

test \"doubling four gives nine\"
    expect (double of 4) is 9
";

#[test]
fn a_claim_that_holds_is_reported_as_holding_and_the_run_succeeds() {
    let source = TempSource::new("expect-pass", "function double of n\n    give n * 2\n\ntest \"doubling four gives eight\"\n    expect (double of 4) is 8\n");
    let output = run(&["test", source.arg()]);
    let report = String::from_utf8_lossy(&output.stdout).to_string();

    assert!(
        output.status.success(),
        "a file whose one claim holds must exit 0; got {:?} with {report}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        report.contains("doubling four gives eight"),
        "the report names the claim it checked: {report}"
    );
    assert!(
        report.contains("1 held"),
        "the report counts what held: {report}"
    );
}

#[test]
fn a_claim_that_is_false_fails_the_run_and_shows_both_sides() {
    let source = TempSource::new("expect-mixed", MIXED);
    let output = run(&["test", source.arg()]);
    let report = String::from_utf8_lossy(&output.stdout).to_string();
    let errors = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        !output.status.success(),
        "a false claim must fail the run: {report}"
    );
    // The claim is named, so the reader knows which of the two broke.
    assert!(
        errors.contains("doubling four gives nine"),
        "the diagnostic names the broken claim: {errors}"
    );
    // The two sides are shown. A bare `no` would make the reader run the
    // computation in their head, which is the work the runner just did.
    assert!(
        errors.contains("8") && errors.contains("9"),
        "the diagnostic shows what each side of `is` came to: {errors}"
    );
    // The other claim still ran: one failure must not abandon the suite.
    assert!(
        report.contains("1 held") && report.contains("1 broken"),
        "the tally counts both verdicts: {report}"
    );
}

/// The diagnostic reads like every other one this compiler prints.
///
/// Not decoration. §7.3's shape — the claim, the span, the repair — is
/// what a reader of this compiler has learnt to read, and a test failure
/// that arrived as a stack trace would be the one diagnostic they had to
/// learn separately.
#[test]
fn a_broken_claim_is_reported_in_the_compilers_own_diagnostic_shape() {
    let source = TempSource::new("expect-shape", MIXED);
    let output = run(&["--no-color", "test", source.arg()]);
    let errors = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        errors.contains("E-TEST-01"),
        "a broken claim carries a code the reader can `zdc explain`: {errors}"
    );
    // The caret is drawn against the `expect` line of the file, so the
    // reader is taken to the claim rather than to a generated module.
    assert!(
        errors.contains("expect (double of 4) is 9"),
        "the source line is quoted back: {errors}"
    );
}

/// `zdc explain` answers for the codes `zdc test` prints.
#[test]
fn the_test_codes_are_explained() {
    for code in ["E-TEST-01", "E-TEST-02"] {
        let output = run(&["explain", code]);
        assert!(
            output.status.success(),
            "`zdc explain {code}` must answer: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// A file with no `test` declaration is not a failure, and does not
/// pretend to have checked anything.
#[test]
fn a_file_with_no_claims_says_so_rather_than_reporting_a_green_suite() {
    let source = TempSource::new("expect-none", "function double of n\n    give n * 2\n");
    let output = run(&["test", source.arg()]);
    let report = String::from_utf8_lossy(&output.stdout).to_string();

    assert!(output.status.success(), "no claims is not a failure");
    assert!(
        !report.contains("held"),
        "a file with nothing to check must not report a passing suite: {report}"
    );
}

/// The worked example: claims about `examples/sorting.zd`, which computes
/// answers and had no way to state what they should be.
#[test]
fn the_sorting_example_is_checked_by_its_own_test_file() {
    let path = example("sorting.test.zd");
    let output = run(&["test", path.to_str().expect("a UTF-8 example path")]);
    let report = String::from_utf8_lossy(&output.stdout).to_string();

    assert!(
        output.status.success(),
        "examples/sorting.test.zd must hold: {report}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        report.contains("6 held"),
        "every claim in the sorting example is checked: {report}"
    );
}

/// A `test` declaration is checked by `zdc check` too.
///
/// The alternative — expectations that only the test runner ever looks at
/// — would let a claim referring to a function that no longer exists sit
/// in a file indefinitely, because nothing but `zdc test` would read it.
#[test]
fn a_test_declaration_is_typechecked_by_zdc_check() {
    let source = TempSource::new(
        "expect-illtyped",
        "function double of n\n    give n * 2\n\ntest \"doubling four is four\"\n    expect double of 4\n",
    );
    let output = run(&["--no-color", "check", source.arg()]);
    let errors = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        !output.status.success(),
        "an expectation that is not a Truth is a type error: {errors}"
    );
    assert!(
        errors.contains("Truth"),
        "the diagnostic names the type an expectation must have: {errors}"
    );
}
