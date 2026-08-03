//! **A bundle ships nothing it does not use** — §16.3.1 and §14A.1, as
//! assertions over the emitted bytes rather than over the walk.
//!
//! §17.4.5's prelude closure is the one part of the member set the split
//! cannot compute, because which library function `contains` names is the
//! checker's verdict and the checker runs after the split (§17.1.1). It is
//! completed in codegen instead — and it has to be completed from *this
//! bundle's* members, not from every operator in the compilation unit.
//!
//! The difference is not academic. Seeded from the compilation unit, a
//! single `contains` written inside a prelude body put `textContains` and
//! `$split` into `hello.zd` — a six-line program that names no library
//! function at all — and cost every bundle in the repository 181 bytes.
//! The library was written around the compiler to avoid it. These tests
//! are what let it be written the natural way again.
//!
//! Every assertion here is over `client_js` or over a function bundle's
//! own source, because §16.3.1 is a claim about what ships.

mod support;

use support::{compile_example, compile_source, try_compile};

/// Names in the prelude that a program has to *reach* to pay for.
///
/// Deliberately spread across four of the seven prelude files: an
/// over-approximating seed shows up as some arbitrary subset of the
/// library, so a test that watched one function would pass by luck.
const LIBRARY_SYMBOLS: &[&str] = &[
    "textContains",
    "listContains",
    "mapContains",
    "indexOf",
    "before",
    "after",
    "joinFrom",
    "dropFirst",
    "valueOr",
    "$split",
    "$textAt",
    "$listAt",
    "$mapAt",
    "reverseFrom",
    "$uppercase",
];

fn assert_absent(js: &str, symbols: &[&str], why: &str) {
    let present: Vec<&str> = symbols
        .iter()
        .copied()
        .filter(|symbol| js.contains(symbol))
        .collect();
    assert!(
        present.is_empty(),
        "{why}\nbut the emission carries {present:?}\n\n{js}"
    );
}

// --- the reported defect, asserted on the bytes ---------------------------

/// The defect verbatim: one `contains` in a prelude body put `textContains`
/// and `$split` into `hello.zd`'s bundle.
///
/// `hello.zd` reads a signal and shows it. It calls nothing, dispatches
/// nothing, and under §16.3.1 must therefore carry none of the library.
#[test]
fn hello_carries_neither_text_contains_nor_the_split_helper() {
    let bundle = compile_example("examples/hello.zd");
    assert_absent(
        &bundle.client_js,
        &["textContains", "$split"],
        "`hello.zd` reaches no library function, so §16.3.1 says its bundle carries none",
    );
}

/// The general form of the same claim, over a wider set of operators.
///
/// A program that uses none of the standard library emits none of it. This
/// is the assertion that would have caught the defect no matter which
/// prelude body happened to write the stray operator.
#[test]
fn a_program_that_uses_no_library_function_emits_none_of_it() {
    let bundle = compile_source(
        "state n is client Whole starting 1\n\
         state greeting is client Text starting \"hi\"\n\
         view\n\
         \x20   Text greeting\n\
         \x20   Text n\n",
    );
    assert_absent(
        &bundle.client_js,
        LIBRARY_SYMBOLS,
        "this program names no library function and dispatches no operator",
    );
}

/// The same for every example that compiles today, so a future prelude
/// edit cannot quietly re-seed every bundle in the repository.
///
/// The examples that use the library are exempted by name rather than by a
/// predicate over the emission, so an example that *starts* reaching the
/// library fails here and has to be added deliberately.
#[test]
fn no_example_carries_a_library_function_it_never_reaches() {
    for example in ["examples/hello.zd", "examples/counter.zd"] {
        let bundle = compile_example(example);
        assert_absent(
            &bundle.client_js,
            LIBRARY_SYMBOLS,
            &format!("{example} reaches no library function"),
        );
    }
}

// --- reachability, in both directions ------------------------------------

/// Reaching a library function through a *chain* still pays for the chain.
///
/// `indexOf` writes `value contains needle`, so a program that calls
/// `indexOf` must carry `textContains` — and `textContains` calls `split`,
/// so it must carry `$split` too. This is the transitive half of §17.4.5:
/// a definition the closure adds can dispatch in turn, and stopping after
/// one step would emit a call to a name the module never declares.
#[test]
fn dispatch_reached_through_a_library_function_is_still_emitted() {
    let bundle = compile_source(
        "state spot is client Option of Whole from indexOf with value is \"a,b\", needle is \",\"\n\
         state shown is client Text from text of (valueOr with maybe is spot, fallback is 0)\n\
         view\n\
         \x20   Text shown\n",
    );
    assert!(
        bundle.client_js.contains("function indexOf("),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("function textContains("),
        "`indexOf` dispatches `contains`, so its target ships with it:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("$split"),
        "`textContains` calls `split`, so the helper ships too:\n{}",
        bundle.client_js
    );
}

/// And the converse, which is the whole point: the same `contains` inside
/// `indexOf` costs a program that never calls `indexOf` nothing at all.
#[test]
fn a_dispatch_inside_an_unreached_library_function_costs_nothing() {
    let bundle = compile_source(
        "state s is client Text from uppercase of \"ab\"\n\
         view\n\
         \x20   Text s\n",
    );
    assert!(
        bundle.client_js.contains("$uppercase"),
        "what it does reach is emitted:\n{}",
        bundle.client_js
    );
    assert_absent(
        &bundle.client_js,
        &["indexOf", "textContains", "$split"],
        "`uppercase` reaches neither `indexOf` nor anything `indexOf` dispatches",
    );
}

/// A dispatch the program itself writes is still emitted — the fix narrows
/// the seed, and this is the check that it did not narrow it to nothing.
#[test]
fn a_dispatch_the_program_writes_is_emitted() {
    let bundle = compile_source(
        "state found is client Truth from \"abc\" contains \"b\"\n\
         view\n\
         \x20   Text found\n",
    );
    assert!(
        bundle.client_js.contains("function textContains("),
        "{}",
        bundle.client_js
    );
    assert_absent(
        &bundle.client_js,
        &["listContains", "mapContains"],
        "the two it did not dispatch to stay out",
    );
}

// --- per root, not per compilation unit ----------------------------------

/// The closure is computed per root, so a dispatch that only a `server`
/// derivation reaches is emitted into that function bundle and **not** into
/// `client.js`.
///
/// This is the assertion that a merely per-compilation-unit fix fails, and
/// it is the shape routing will multiply: one bundle per page means the
/// answer has to be a function of the root's own members.
#[test]
fn a_dispatch_only_the_server_reaches_stays_out_of_the_client_bundle() {
    let bundle = compile_source(
        "state needle is client Text starting \"a\"\n\
         state corpus is server Text starting \"alpha\"\n\
         state found is server Truth from corpus contains needle\n\
         view\n\
         \x20   when found\n\
         \x20       Loading           show Spinner\n\
         \x20       Failed with error show ErrorBar message is error.message\n\
         \x20       Ready with hit    show Text hit\n",
    );
    assert_absent(
        &bundle.client_js,
        &["function textContains(", "$split"],
        "the client walk stops at the `server` read (§16.3.12 rule 1), so the dispatch \
         behind it is not the client's to carry",
    );
    let server: String = bundle
        .functions
        .iter()
        .map(|function| function.source.clone())
        .collect();
    assert!(
        server.contains("function textContains("),
        "the root that does reach it must emit it, or the endpoint calls a name it never \
         declares:\n{server}"
    );
}

/// Two roots, two answers. The client reaches `contains` over a `Text` and
/// the server reaches it over a `List`, and neither bundle carries the
/// other's target.
#[test]
fn two_roots_carry_only_their_own_dispatch_targets() {
    let bundle = compile_source(
        "state needle is client Text starting \"a\"\n\
         state here is client Truth from \"alpha\" contains needle\n\
         state words is server List of Text starting [\"alpha\", \"beta\"]\n\
         state there is server Truth from words contains needle\n\
         view\n\
         \x20   Text here\n\
         \x20   when there\n\
         \x20       Loading           show Spinner\n\
         \x20       Failed with error show ErrorBar message is error.message\n\
         \x20       Ready with hit    show Text hit\n",
    );
    assert!(
        bundle.client_js.contains("function textContains("),
        "{}",
        bundle.client_js
    );
    assert_absent(
        &bundle.client_js,
        &["function listContains("],
        "the client dispatched over a `Text`",
    );
    let server: String = bundle
        .functions
        .iter()
        .map(|function| function.source.clone())
        .collect();
    assert!(
        server.contains("function listContains("),
        "the server dispatched over a `List`:\n{server}"
    );
    assert!(
        !server.contains("function textContains("),
        "and did not dispatch over a `Text`:\n{server}"
    );
}

// --- the library, written the natural way --------------------------------

/// `indexOf` is written with the infix operator, which is what the source
/// language offers and what every other function in `text.zd` uses.
///
/// It was written `textContains with value is …` instead, purely to keep
/// its `contains` from seeding every bundle in the repository. That is a
/// library bending around a compiler defect, and this pins the shape the
/// fix restored: `indexOf` still works, and a program that does not call it
/// still pays nothing (asserted above).
#[test]
fn index_of_is_written_with_the_operator_the_language_offers() {
    let source = include_str!("../../zdc-lib/prelude/text.zd");
    assert!(
        source.contains("if value contains needle"),
        "`indexOf` should use the infix form; the call-form workaround is what this \
         change removed"
    );
    assert!(
        !source.contains("operator_closure"),
        "the workaround's note names `operator_closure`; if the note is still there the \
         workaround probably is too"
    );
}

/// And it still computes the right answers, which is the only reason the
/// rewrite is safe. §17.4.7's parity argument in one assertion.
#[test]
fn index_of_still_answers_correctly_through_the_operator() {
    let bundle = try_compile(
        "state a is client Text from text of (valueOr with maybe is (indexOf with value is \
         \"hello\", needle is \"ll\"), fallback is 99)\n\
         state b is client Text from text of (valueOr with maybe is (indexOf with value is \
         \"hello\", needle is \"zz\"), fallback is 99)\n\
         view\n\
         \x20   Text a\n\
         \x20   Text b\n",
        "index-of.zd",
    );
    let bundle = bundle.unwrap_or_else(|errors| panic!("{}", errors[0].message));
    let mut context = support::context(false);
    let rendered = support::run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    assert!(
        rendered.contains('2') && rendered.contains("99"),
        "`indexOf` should find \"ll\" at 2 and not find \"zz\":\n{rendered}"
    );
}
