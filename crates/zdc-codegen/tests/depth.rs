//! How deep the library's folds go before the host stack gives out.
//!
//! §17.4.10's finding, measured. There are no local bindings in
//! ZDeceptron, so a fold over a collection cannot carry an accumulator
//! through a loop; §17.4.9's technique is index recursion, and the stack
//! depth of `sumOf`, `join`, `listContains` and `slice` is therefore
//! linear in the input rather than constant.
//!
//! This is not a bug in the library — it is the shape of the language, and
//! §14F.2 says a failure to write an operation in ZDeceptron is a finding
//! about the language rather than a reason to reach for the FFI. So the
//! limit is recorded rather than hidden, in the terms a program hits it
//! in: **how many elements**.
//!
//! §17.4.10 already names the fix and calls it the single change with the
//! largest return: local bindings, plus a `rest of`, would turn every one
//! of these into a loop.

mod support;

use support::{compile_source, context};

/// Sum a list of `count` ones and report what came back, or the error the
/// host raised trying.
fn sum_of(count: usize) -> Result<String, String> {
    let items: Vec<String> = (0..count).map(|_| "1".to_string()).collect();
    let source = format!(
        "state xs is client List of Whole starting [{}]\n\
         state answer is client Text from text of (sumOf of xs)\n\
         view\n    Text answer\n",
        items.join(", ")
    );
    let bundle = compile_source(&source);
    let mut context = context(false);
    let module = support::flatten(&bundle.client_js);
    context
        .eval(boa_engine::Source::from_bytes(module.as_bytes()))
        .map_err(|e| e.to_string())?;
    let driver = "const $host = document.createElement('div');\nmain($host);\nserialize($host)";
    context
        .eval(boa_engine::Source::from_bytes(driver.as_bytes()))
        .map(|value| value.display().to_string())
        .map_err(|e| e.to_string())
}

/// A list of a size an ordinary program has works, which is what makes
/// the library usable at all.
#[test]
fn a_fold_over_an_ordinary_list_is_fine() {
    let answer = sum_of(200).expect("200 elements must fold");
    assert!(answer.contains("200"), "{answer}");
}

/// And this is that it stops, and why.
///
/// The *number* here is the embedded interpreter's own recursion budget
/// rather than the language's — a browser's is roughly an order of
/// magnitude larger — so what is pinned is the shape of the failure, not
/// a threshold. What matters is that the depth grows with the input at
/// all, and that when it runs out the reason is named: it is the fold, not
/// the program.
#[test]
fn a_fold_deep_enough_runs_out_of_stack_rather_than_giving_a_wrong_answer() {
    let error = sum_of(4_000).expect_err(
        "if this now succeeds, folds stopped being linear in depth and the finding \
         §17.4.10 records has been fixed — say so there",
    );
    assert!(
        error.contains("recursive"),
        "the failure must name the recursion: {error}"
    );
    // Not a wrong answer, and not a silent one. §5.4's whole argument is
    // that a bounds check beats an `undefined`, and the same reasoning
    // applies here: running out is a stated failure.
    assert!(error.contains("sumFrom"), "{error}");
}
