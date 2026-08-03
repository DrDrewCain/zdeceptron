//! Is the ZDeceptron arm actually ZDeceptron?
//!
//! A benchmark whose "compiled" arm is a hand-written approximation of what
//! the compiler might emit measures the author's optimism. These tests make
//! the arm answerable to `zdc build`: the row's template, the walk to its
//! holes, and the bindings attached there are recompiled from
//! `bench/row.zd` and compared against what `js/benchmark.js` renders.
//!
//! They also pin the *gap*. The workload's list cannot be written in
//! ZDeceptron today, and the reasons are refusals the compiler states in
//! its own words. Each one is asserted here, so that the day it stops being
//! refused this test fails and says which paragraph of `BENCHMARKS.md` is
//! now out of date.

use zdc_bench::{benchmark_row, emitted_row, try_compile, BENCHMARK_JS, ROW_ZD};

/// The row in the benchmark is the row the compiler emits.
#[test]
fn the_benchmark_row_is_the_compilers_own_emission() {
    let emitted = emitted_row(&zdc_bench::compile("crates/zdc-bench/bench/row.zd").client_js);
    let benchmark = benchmark_row(BENCHMARK_JS);

    assert_eq!(
        benchmark.template, emitted.template,
        "the markup the benchmark clones per row is no longer the markup the compiler emits \
         for bench/row.zd. Update `ROW_HTML` in js/benchmark.js — and check whether the \
         numbers in BENCHMARKS.md still describe the same row."
    );
    assert_eq!(
        benchmark.walk, emitted.walk,
        "the walk to the row's holes no longer matches the emitter's"
    );
    assert_eq!(
        benchmark.bindings, emitted.bindings,
        "the bindings attached to the row no longer match the emitter's, so the benchmark is \
         measuring a different number of effects than the compiler would create"
    );

    // A comparison that found nothing on both sides would also be equal.
    assert_eq!(emitted.walk.len(), 4, "{:?}", emitted.walk);
    assert_eq!(emitted.bindings.len(), 5, "{:?}", emitted.bindings);
    assert!(emitted.template.starts_with("<div class=\"zd-row\">"));
}

/// The row source is a program this compiler accepts, not a sketch.
#[test]
fn the_row_source_compiles_without_unchecked() {
    let bundle = try_compile(ROW_ZD, "bench/row.zd").expect("bench/row.zd compiles");
    assert!(bundle.client_js.contains("template("));
    assert!(bundle.styles_css.contains(".zd-row"));
}

/// The gap, in the compiler's own words.
///
/// This is the whole reason the benchmark's list is hand-written. Two
/// independent refusals stand between `bench/row.zd` and a `.zd` file that
/// renders the js-framework-benchmark workload.
#[test]
fn the_workloads_list_is_still_inexpressible() {
    const LIST: &str = "state rows is client List of Text starting empty\n\
                        \n\
                        view\n    \
                        Column\n        \
                        each row in rows\n            \
                        Text row\n";

    let Err(errors) = try_compile(LIST, "list.zd") else {
        panic!(
            "a list in the view now compiles. The ZDeceptron arm of this benchmark should \
             become a real .zd program, and BENCHMARKS.md's gap section is stale."
        );
    };
    let joined = errors.join("\n");
    assert!(
        joined.contains("`each` in the view cannot be compiled yet"),
        "expected the M5b refusal for `each`, got:\n{joined}"
    );
    assert!(
        joined.contains("`empty` cannot be compiled yet"),
        "expected the type-checker refusal for `empty`, got:\n{joined}"
    );
}

/// A list literal — §4.4's `listLiteral`, which §14B.4 records as a closed
/// design gap — is not yet accepted by the lexer, so `starting ["a"]` is not
/// a way around the refusal above.
///
/// Asserted rather than described, because it is the third reason the
/// workload's data cannot be written in the language and it is the one that
/// is least visible: no example in the repository uses a bracket.
#[test]
fn a_list_literal_is_not_yet_lexed() {
    let source = "state rows is client List of Text starting [\"a\", \"b\"]\n";
    let Err(error) = zdc_parser::parse(source) else {
        panic!(
            "a list literal now parses. §4.4's `listLiteral` has landed, so the workload's \
             data may now be expressible — BENCHMARKS.md's gap section is stale."
        );
    };
    assert!(
        error.message.contains('['),
        "expected the parser to name `[`, got: {}",
        error.message
    );
}
