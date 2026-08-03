//! §21.7.2's cost claim, tested end to end rather than argued.
//!
//! > *"a program with no `trusted` and no `release` has no site that
//! > requires a value to be Trusted, its integrity labels are never read by
//! > any rule, and the default is unobservable"* — §21.4, N10.
//!
//! That claim is what makes wiring the integrity pass into `zdc check` and
//! `zdc build` affordable, and until now it had never been checked against
//! the compiler. It is checked here, against the whole population §21.7.2
//! measures — the checked-in examples, 0 `trusted`, 0 `release`.
//!
//! **What is asserted, and why it is the right assertion.** Not "the
//! examples still compile" — they would still compile if the pass emitted
//! warnings — but that the pass contributes **no diagnostic of any
//! severity**. `zdc-codegen` reads `Verdict`, and `Verdict.diagnostics` is
//! the only channel the integrity direction has into it (§17.1.2 gives the
//! `ifc` stage one verdict; §17.1.3's last row is codegen's refusal over
//! it). A pass that adds nothing to that vector cannot change a byte of
//! `dist/`, so this assertion is the byte-identity claim stated as
//! something a test can fail on.
//!
//! Verified against the emitted bytes once, by hand, at the commit that
//! wired the pass: `zdc build` over every example that produces a bundle
//! emitted 43 files whose SHA-256 sums were identical before and after.
//! This test is what keeps that true.

mod support;

use support::repository_path;

/// Every checked-in example that parses on this branch.
///
/// Two of the ten are absent, and both are named rather than dropped
/// silently.
///
/// * **`blog.zd`** does not parse on this branch at all: line 38 declares
///   `state posts is static …` and the parser has no `static` placement
///   yet, so it fails before any pass runs. It is worth naming because
///   §21.8.4 makes `blog.zd:48`'s `query` the checked-in exemplar of
///   residual risk R2, and a reader who comes looking for it should find
///   out why it is not here.
/// * **`components.zd`** is a multi-file program — it opens with `use`, and
///   resolving it needs the module linker `zdc build` runs, not the
///   single-program resolver. Its integrity cost is covered by
///   `zdc build`'s own output instead, which was checked byte for byte.
const EXAMPLES: [&str; 8] = [
    "examples/counter.zd",
    "examples/disclosure.zd",
    "examples/guestbook.zd",
    "examples/hello.zd",
    "examples/leaderboard.zd",
    "examples/model.zd",
    "examples/todo.zd",
    "examples/voting-board.zd",
];

/// **§21.7.2, measured on the compiler rather than on the source text.**
///
/// §21.7.2 counts annotations by reading the files. This counts what the
/// pass *says* when it runs on them, which is the stronger question: a
/// program can have 0 `trusted` and 0 `release` and still pay, if some rule
/// reads an integrity label at a site the program did not ask for. None
/// does, and that is the whole of why the default may be inverted.
#[test]
fn the_integrity_pass_says_nothing_about_a_program_that_opts_into_nothing() {
    for relative in EXAMPLES {
        let source = std::fs::read_to_string(repository_path(relative))
            .unwrap_or_else(|e| panic!("reading {relative}: {e}"));
        let program = zdc_parser::parse(&source)
            .unwrap_or_else(|e| panic!("{relative} failed to parse: {}", e.message));
        let hir = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
            .resolve()
            .unwrap_or_else(|errors| panic!("{relative}: {}", errors[0].message));
        let split = zdc_graph::split(&hir);

        let reported: Vec<String> = zdc_graph::authority(&hir, &split)
            .diagnostics()
            .iter()
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect();

        assert!(
            reported.is_empty(),
            "{relative} opts into neither `trusted` nor `release`, so the \
             integrity pass must cost it nothing — not even a warning, \
             because a warning is still a byte of output that was not there \
             before (§21.7.2): {reported:?}"
        );
    }
}

/// The same claim from the other side: the examples still compile.
///
/// Weaker than the assertion above and kept anyway, because it is the thing
/// a user would notice. §18.1.3's row *"All eight checked-in examples
/// compile unchanged"* is called *"the most important row in that table"*,
/// and wiring a pass is exactly the change that would break it.
#[test]
fn wiring_the_integrity_pass_leaves_the_examples_compiling() {
    for relative in EXAMPLES {
        let source = std::fs::read_to_string(repository_path(relative))
            .unwrap_or_else(|e| panic!("reading {relative}: {e}"));
        let program = zdc_parser::parse(&source)
            .unwrap_or_else(|e| panic!("{relative} failed to parse: {}", e.message));
        let hir = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
            .resolve()
            .unwrap_or_else(|errors| panic!("{relative}: {}", errors[0].message));
        let split = zdc_graph::split(&hir);
        let verdict = zdc_graph::ifc(&hir, &split);

        let errors: Vec<&str> = verdict.errors().map(|e| e.code).collect();
        assert!(
            errors.is_empty(),
            "{relative} must still pass the flow pass with the integrity \
             direction wired into it: {errors:?}"
        );
    }
}
