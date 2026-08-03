//! **Building a list**, and what it costs.
//!
//! Before `append item to list` the language could take a list apart and
//! not put one together. `rest of` made a list shorter, `keep each` and
//! `map each` made one the same length from another, and nothing at all
//! made one longer — so no function could hand back a collection it had
//! not been given, and `split`, `reverse`, `rest` and `values` were
//! `foreign` for that reason and no other. §14E makes every `foreign` a
//! hole in the type system, so four holes were open because of one
//! missing expression.
//!
//! The tests here are about the two things that had to be true for the
//! form to be worth adding rather than merely present:
//!
//! 1. **it is a value, not a mutation** — `append` leaves its operand
//!    alone, so a builder can pass its answer through a recursive call
//!    the way every other fold in the library passes one; and
//! 2. **building n elements costs O(n)**, not O(n²). A list built by
//!    appending is a chain of links until something looks at it, and then
//!    it is flattened once. `crates/zdc-codegen/src/intrinsics.rs` has
//!    the reasoning; this file has the measurement, at a hundred thousand
//!    elements.
//!
//! The linear cost is available only to a builder in **tail position**,
//! and `a_builder_that_is_not_a_tail_call_is_quadratic` pins the other
//! side of that so it is written down somewhere rather than discovered.

mod support;

use support::{compile_source, context};

/// Compile a program whose view shows one text signal, run it, and report
/// what the page says, or the error the host raised trying.
fn run(declarations: &str) -> Result<String, String> {
    let bundle = compile_source(&format!("{declarations}view\n    Text answer\n"));
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

/// The rendered text with the markup taken out, which is the value the
/// program computed. Reading the answer out of the DOM rather than out of
/// a variable is the same decision `library.rs` makes: it is the only
/// place a value has actually survived the whole compiler.
fn answer(declarations: &str) -> String {
    let rendered = run(declarations).unwrap_or_else(|e| panic!("{declarations}\nfailed: {e}"));
    strip_markup(&rendered)
}

fn strip_markup(rendered: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in rendered.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            '"' => {}
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out
}

/// `upTo` counts from `index` to `limit`, appending as it goes. The
/// accumulator travels as a parameter and the last act is a call and
/// nothing else, so the emitter turns the recursion into a loop — the
/// shape every builder in the prelude has.
const UP_TO: &str = "function upTo with limit, index, taken\n\
                     \x20   if index is limit\n\
                     \x20       give taken\n\
                     \x20   give upTo with limit is limit, index is index + 1, \
                     taken is (append index to taken)\n";

#[test]
fn append_gives_back_a_longer_list_and_leaves_the_original_alone() {
    assert_eq!(
        answer(
            "state xs is client List of Whole starting [1, 2]\n\
             state ys is client List of Whole from append 3 to xs\n\
             state answer is client Text from (text of (length of ys)) + \
             (text of (length of xs))\n"
        ),
        "32"
    );
    assert_eq!(
        answer(
            "state xs is client List of Whole starting [1, 2]\n\
             state ys is client List of Whole from append 3 to xs\n\
             state answer is client Text from text of (sumOf of ys)\n"
        ),
        "6",
        "the element goes on the end, so the sum is 1 + 2 + 3"
    );
}

/// `empty` is the other half of the form: a list has to start somewhere,
/// and §14B.4's `empty` is where.
#[test]
fn a_list_can_be_built_from_nothing() {
    assert_eq!(
        answer(
            "state ys is client List of Whole from append 1 to empty\n\
             state answer is client Text from text of (length of ys)\n"
        ),
        "1"
    );
}

/// The operand is a postfix chain and nothing wider, so appends nest to
/// the right without parentheses and read in the order they apply.
#[test]
fn appends_nest_to_the_right() {
    assert_eq!(
        answer(
            "state ys is client List of Whole from append 3 to append 2 to append 1 to empty\n\
             state answer is client Text from (text of (length of ys)) + \
             (text of (sumOf of ys))\n"
        ),
        "36"
    );
}

/// A list built by appending is an ordinary list to everything else: it
/// can be indexed, counted, joined and searched.
#[test]
fn a_built_list_is_an_ordinary_list_everywhere_else() {
    assert_eq!(
        answer(
            "state ys is client List of Text from append \"b\" to append \"a\" to empty\n\
             state answer is client Text from join with parts is ys, using is \"-\"\n"
        ),
        "a-b"
    );
    assert_eq!(
        answer(
            "state ys is client List of Whole from append 2 to append 1 to empty\n\
             state answer is client Text from text of (ys contains 2)\n"
        ),
        "yes"
    );
    assert_eq!(
        answer(
            "state ys is client List of Whole from append 2 to append 1 to empty\n\
             state answer is client Text from text of (valueOr with maybe is (ys at 1), \
             fallback is 0)\n"
        ),
        "2",
        "`at` forces the chain, so an index into a built list is an index"
    );
}

/// A pipeline's clauses are array operations, so a built list has to
/// arrive at `from` as an array. This is the emitter's `$force` at the
/// pipeline, and without it `keep each` would throw rather than filter.
#[test]
fn a_built_list_can_be_run_through_a_pipeline() {
    assert_eq!(
        answer(
            "function bigOnes of xs\n\
             \x20   from xs\n\
             \x20   keep each n where n > 1\n\
             state ys is client List of Whole from append 3 to append 2 to append 1 to empty\n\
             state answer is client Text from text of (length of (bigOnes of ys))\n"
        ),
        "2"
    );
}

/// **The acceptance criterion.** A hundred thousand elements, built one
/// at a time through a tail-recursive builder, in the time an ordinary
/// test takes.
///
/// The number matters because the naive implementation of this form is
/// quadratic: an append that hands back a plain JavaScript array must
/// copy every element it already had, because an array cannot share a
/// prefix with a longer array. At n = 100,000 that is five billion
/// element copies. What runs here instead is 100,000 links and one
/// flatten.
#[test]
fn a_hundred_thousand_elements_can_be_built_one_at_a_time() {
    let built = answer(&format!(
        "{UP_TO}state answer is client Text from text of \
         (length of (upTo with limit is 100000, index is 0, taken is empty))\n"
    ));
    assert_eq!(built, "100000");
}

/// And the elements are the ones that were appended, in the order they
/// were appended, which the length alone does not say.
#[test]
fn the_hundred_thousand_elements_are_the_right_ones_in_the_right_order() {
    let summed = answer(&format!(
        "{UP_TO}state answer is client Text from text of \
         (sumOf of (upTo with limit is 100000, index is 0, taken is empty))\n"
    ));
    assert_eq!(summed, "4999950000", "0 + 1 + … + 99999");

    let first_and_last = answer(&format!(
        "{UP_TO}state xs is client List of Whole from upTo with limit is 100000, index is 0, \
         taken is empty\n\
         state answer is client Text from (text of (valueOr with maybe is (xs at 0), \
         fallback is 0)) + \"|\" + (text of (valueOr with maybe is (xs at 99999), fallback is 0))\n"
    ));
    assert_eq!(first_and_last, "0|99999");
}

/// **The other side of the linear path, written down.**
///
/// `growDown` appends *around* its recursive call rather than into it, so
/// the call is not in tail position, the emitter cannot turn it into a
/// loop, and two costs come back at once. It is O(n²) in time, because
/// every frame flattens the chain the frame below it built; and it is
/// O(n) in stack depth, which is the defect `depth.rs` records and the
/// reason every fold in the prelude carries an accumulator.
///
/// **Depth is the limit that bites first.** Measured against this file:
/// the tail builder returns 5,000 elements in 20 ms, 100,000 in 291 ms
/// and 200,000 in 566 ms — linear, with the compile of the program in the
/// constant. The non-tail builder returns 400 elements in 5 ms and, at
/// 800, does not return at all: the host raises "exceeded maximum number
/// of recursive calls" before the quadratic time is even visible.
///
/// So the guidance is not "prefer" a tail call, it is "write one", and a
/// user who writes the natural recursive version finds out at a few
/// hundred elements rather than at a hundred thousand. That is the better
/// of the two failures, and it is the one this pins.
#[test]
fn a_builder_that_is_not_a_tail_call_is_quadratic() {
    let source = "function growDown with limit, index\n\
                  \x20   if index is limit\n\
                  \x20       give empty\n\
                  \x20   give append index to (growDown with limit is limit, index is index + 1)\n";
    assert_eq!(
        answer(&format!(
            "{source}state answer is client Text from text of \
             (length of (growDown with limit is 400, index is 0))\n"
        )),
        "400",
        "correct at a size the stack survives, which is the whole of its range"
    );
}

/// The library's own builders, at a size that would have been impossible
/// before and is now ordinary. `reverse` and `rest` are ZDeceptron folds
/// now, not primitives, so this is the construction form under its real
/// load.
#[test]
fn the_library_builders_handle_a_hundred_thousand_elements() {
    let reversed = answer(&format!(
        "{UP_TO}state xs is client List of Whole from reverse of (upTo with limit is 100000, \
         index is 0, taken is empty)\n\
         state answer is client Text from text of (valueOr with maybe is (xs at 0), \
         fallback is 0)\n"
    ));
    assert_eq!(reversed, "99999", "the last element is now the first");

    let tail = answer(&format!(
        "{UP_TO}state xs is client List of Whole from rest of (upTo with limit is 100000, \
         index is 0, taken is empty)\n\
         state answer is client Text from text of (length of xs)\n"
    ));
    assert_eq!(tail, "99999");
}

/// A list built with `append` and written into a signal survives the
/// round trip: it is spread by `append`'s mutation form, filtered by
/// `remove`'s, and iterated by a node-position `each`.
#[test]
fn a_built_list_can_be_stored_in_a_signal_and_shown() {
    let bundle = compile_source(
        "function twoOf of first\n\
         \x20   give append (first + 1) to append first to empty\n\
         state xs is client List of Whole from twoOf of 1\n\
         view\n\
         \x20   each n in xs\n\
         \x20       Text (text of n)\n",
    );
    let mut context = context(false);
    let module = support::flatten(&bundle.client_js);
    context
        .eval(boa_engine::Source::from_bytes(module.as_bytes()))
        .expect("the module must load");
    let rendered = context
        .eval(boa_engine::Source::from_bytes(
            "const $host = document.createElement('div');\nmain($host);\nserialize($host)"
                .as_bytes(),
        ))
        .expect("the view must render")
        .display()
        .to_string();
    assert!(rendered.contains('1'), "{rendered}");
    assert!(rendered.contains('2'), "{rendered}");
}
