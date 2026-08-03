//! Is the ZDeceptron arm actually ZDeceptron?
//!
//! A benchmark whose "compiled" arm is a hand-written approximation of what
//! the compiler might emit measures the author's optimism. These tests make
//! the arm answerable to `zdc build`: the row's template, the walk to its
//! holes, and the bindings attached there are recompiled from
//! `bench/row.zd` and compared against what `js/benchmark.js` renders.
//!
//! They also pin what used to be the *gap*. The workload's list could not
//! be written in ZDeceptron while `each` in the view, `empty`, and the list
//! literal were all refused. All three have landed, so the tests below now
//! pin the other direction: the list compiles, and the literal parses. The
//! day either stops being true, this test says so — and `BENCHMARKS.md`'s
//! gap section, which those refusals were the whole subject of, describes a
//! compiler that no longer exists.

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

/// The gap, closed.
///
/// Two independent refusals used to stand between `bench/row.zd` and a `.zd`
/// file that renders the js-framework-benchmark workload: `each` in the view
/// and `empty`. Both are gone, so the same source that was pinned as
/// uncompilable is pinned here as compiling — down to the `eachInto` call
/// the benchmark's JavaScript arm still writes by hand.
#[test]
fn the_workloads_list_is_expressible() {
    const LIST: &str = "state rows is client List of Text starting empty\n\
                        \n\
                        view\n    \
                        Column\n        \
                        each row in rows\n            \
                        Text row\n";

    let bundle = try_compile(LIST, "list.zd").unwrap_or_else(|errors| {
        panic!(
            "a list in the view no longer compiles:\n{}",
            errors.join("\n")
        )
    });
    assert!(
        bundle.client_js.contains("eachInto"),
        "`each` compiled without reaching the runtime's reconciler:\n{}",
        bundle.client_js
    );
}

/// A list literal — §4.4's `listLiteral`, which §14B.4 recorded as a closed
/// design gap — now lexes and parses, so `starting ["a", "b"]` is a way to
/// write the workload's data directly.
///
/// Asserted rather than described, because it was the third reason the
/// workload's data could not be written in the language and it is the one
/// that is least visible: it turns on a single bracket.
#[test]
fn a_list_literal_parses() {
    let source = "state rows is client List of Text starting [\"a\", \"b\"]\n";
    let program = zdc_parser::parse(source).unwrap_or_else(|error| {
        panic!("a list literal no longer parses: {}", error.message);
    });

    let [zdc_ast::Decl::State(state)] = program.decls.as_slice() else {
        panic!("expected one state declaration, got {:?}", program.decls);
    };
    let zdc_ast::Init::Starting(zdc_ast::Expr::List { items, .. }) = &state.init else {
        panic!("expected `starting` a list literal, got {:?}", state.init);
    };
    assert_eq!(items.len(), 2, "{items:?}");
}
