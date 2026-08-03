//! **The library, executed.** What §17.4.7 calls the parity test.
//!
//! §14F.2's argument is that a language whose `List.length` is a
//! JavaScript call has borrowed its semantics rather than defined them.
//! The answer is that the prelude is written in ZDeceptron above a named
//! primitive layer — but "written in ZDeceptron" is only worth anything if
//! the code that comes out the far end *computes the right answers*.
//!
//! So these run a program end to end: parse, resolve against the prelude,
//! typecheck, emit, and evaluate the emitted JavaScript against the same
//! DOM shim the runtime's own tests use. Nothing here inspects the
//! generated source; every assertion is about a value the library
//! produced.

mod support;

use support::{compile_source, context, run};

/// Compile a program whose view shows one text signal, run it, and return
/// what the page says.
///
/// Reading the answer out of the rendered DOM rather than out of a
/// variable is deliberate: it is the only place a value has actually
/// survived the whole compiler.
fn shown(declarations: &str) -> String {
    let source = format!("{declarations}view\n    Text answer\n");
    let bundle = compile_source(&source);
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    )
}

/// The rendered text with the markup taken out, which is the value the
/// program computed.
fn text(declarations: &str) -> String {
    let rendered = shown(declarations);
    let mut out = String::new();
    let mut inside = false;
    for ch in rendered.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out
}

// --- Text ----------------------------------------------------------------

#[test]
fn text_length_counts_code_points_not_utf16_units() {
    assert_eq!(
        text("state answer is client Text from text of (length of \"abc\")\n"),
        "3"
    );
    // `'🎉'.length` is 2 in JavaScript. §5.4 says a `Text` is text, and
    // this is where the language stops leaking the encoding.
    assert_eq!(
        text("state answer is client Text from text of (length of \"a🎉b\")\n"),
        "3"
    );
}

#[test]
fn slice_is_written_in_zdeceptron_and_computes_the_right_answer() {
    assert_eq!(
        text("state answer is client Text from slice with value is \"abcdef\", start is 1, stop is 4\n"),
        "bcd"
    );
    assert_eq!(
        text(
            "state answer is client Text from slice with value is \"abc\", start is 2, stop is 2\n"
        ),
        ""
    );
}

#[test]
fn text_contains_is_written_in_zdeceptron_and_agrees_with_the_platform() {
    for (haystack, needle, expected) in [
        ("hello world", "world", "yes"),
        ("hello world", "hello", "yes"),
        ("hello world", "lo wo", "yes"),
        ("hello world", "xyz", "no"),
        ("hello", "", "yes"),
        ("hello", "hello!", "no"),
        ("", "a", "no"),
    ] {
        assert_eq!(
            text(&format!(
                "state answer is client Text from text of (\"{haystack}\" contains \"{needle}\")\n"
            )),
            expected,
            "`{haystack}` contains `{needle}`"
        );
    }
}

#[test]
fn starts_with_and_ends_with_agree_with_the_platform() {
    assert_eq!(
        text("state answer is client Text from text of (startsWith with value is \"abcdef\", prefix is \"abc\")\n"),
        "yes"
    );
    assert_eq!(
        text("state answer is client Text from text of (startsWith with value is \"abcdef\", prefix is \"bcd\")\n"),
        "no"
    );
    assert_eq!(
        text("state answer is client Text from text of (endsWith with value is \"abcdef\", suffix is \"def\")\n"),
        "yes"
    );
    assert_eq!(
        text("state answer is client Text from text of (endsWith with value is \"abcdef\", suffix is \"abc\")\n"),
        "no"
    );
}

#[test]
fn the_case_and_trim_primitives_do_what_they_say() {
    assert_eq!(
        text("state answer is client Text from uppercase of \"abc\"\n"),
        "ABC"
    );
    assert_eq!(
        text("state answer is client Text from lowercase of \"ABC\"\n"),
        "abc"
    );
    assert_eq!(
        text("state answer is client Text from text of (isBlank of \"   \")\n"),
        "yes"
    );
    assert_eq!(
        text("state answer is client Text from text of (isBlank of \" a \")\n"),
        "no"
    );
}

// --- List ----------------------------------------------------------------

#[test]
fn join_is_written_in_zdeceptron_and_puts_the_separator_between() {
    assert_eq!(
        text(
            "state names is client List of Text starting [\"a\", \"b\", \"c\"]\n\
             state answer is client Text from join with parts is names, using is \", \"\n"
        ),
        "a, b, c"
    );
    assert_eq!(
        text(
            "state names is client List of Text starting [\"only\"]\n\
             state answer is client Text from join with parts is names, using is \", \"\n"
        ),
        "only"
    );
    assert_eq!(
        text(
            "state names is client List of Text starting []\n\
             state answer is client Text from join with parts is names, using is \", \"\n"
        ),
        ""
    );
}

#[test]
fn list_contains_walks_the_whole_list() {
    assert_eq!(
        text(
            "state xs is client List of Whole starting [1, 2, 3]\n\
             state answer is client Text from text of (xs contains 3)\n"
        ),
        "yes"
    );
    assert_eq!(
        text(
            "state xs is client List of Whole starting [1, 2, 3]\n\
             state answer is client Text from text of (xs contains 4)\n"
        ),
        "no"
    );
}

#[test]
fn sum_of_folds_by_index_recursion() {
    assert_eq!(
        text(
            "state xs is client List of Whole starting [1, 2, 3, 4]\n\
             state answer is client Text from text of (sumOf of xs)\n"
        ),
        "10"
    );
}

#[test]
fn reverse_leaves_the_original_alone() {
    assert_eq!(
        text(
            "state xs is client List of Text starting [\"a\", \"b\"]\n\
             state back is client List of Text from reverse of xs\n\
             state answer is client Text from (join with parts is back, using is \"\") + \
             (join with parts is xs, using is \"\")\n"
        ),
        "baab"
    );
}

/// `rest of` is the primitive a fold consumes its input with, and the
/// empty case is the base case: dropping the first of nothing is nothing,
/// so no length check stands in front of it.
#[test]
fn rest_drops_the_first_element_and_bottoms_out_at_empty() {
    assert_eq!(
        text(
            "state xs is client List of Text starting [\"a\", \"b\", \"c\"]\n\
             state answer is client Text from join with parts is (rest of xs), using is \"\"\n"
        ),
        "bc"
    );
    assert_eq!(
        text(
            "state xs is client List of Text starting []\n\
             state answer is client Text from text of (length of (rest of xs))\n"
        ),
        "0"
    );
}

/// And it leaves its operand alone, as every ZDeceptron value is
/// unaliased.
#[test]
fn rest_leaves_the_original_alone() {
    assert_eq!(
        text(
            "state xs is client List of Text starting [\"a\", \"b\"]\n\
             state tail is client List of Text from rest of xs\n\
             state answer is client Text from (join with parts is tail, using is \"\") + \
             (join with parts is xs, using is \"\")\n"
        ),
        "bab"
    );
}

// --- Option, Map, and the thing §14F.2a said no program could do ---------

#[test]
fn reading_a_map_and_using_the_result_works() {
    assert_eq!(
        text(
            "state scores is client Map of Text to Whole starting [\"ada\" to 7]\n\
             state answer is client Text from text of (atOr with table is scores, key is \"ada\", fallback is 0)\n"
        ),
        "7"
    );
    assert_eq!(
        text(
            "state scores is client Map of Text to Whole starting [\"ada\" to 7]\n\
             state answer is client Text from text of (atOr with table is scores, key is \"bob\", fallback is 0)\n"
        ),
        "0",
        "a missing key is the fallback, which is the decision §5.4 forces"
    );
}

#[test]
fn indexing_out_of_bounds_gives_none_rather_than_undefined() {
    assert_eq!(
        text(
            "state xs is client List of Text starting [\"a\"]\n\
             state answer is client Text from valueOr with maybe is (xs at 5), fallback is \"none\"\n"
        ),
        "none"
    );
    assert_eq!(
        text(
            "state xs is client List of Text starting [\"a\"]\n\
             state answer is client Text from valueOr with maybe is (xs at (0 - 1)), fallback is \"none\"\n"
        ),
        "none",
        "a negative index is out of bounds too, which `xs[-1]` would not have caught"
    );
}

#[test]
fn map_membership_and_keys_agree_with_each_other() {
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting [\"a\" to 1, \"b\" to 2]\n\
             state answer is client Text from text of (m contains \"b\")\n"
        ),
        "yes"
    );
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting [\"a\" to 1, \"b\" to 2]\n\
             state ks is client List of Text from keys of m\n\
             state answer is client Text from join with parts is ks, using is \"\"\n"
        ),
        "ab"
    );
}

// --- numbers -------------------------------------------------------------

#[test]
fn the_numeric_helpers_are_polymorphic_at_runtime_too() {
    assert_eq!(
        text(
            "state a is client Whole from min with first is 3, second is 9\n\
             state b is client Decimal from max with first is 1.5, second is 2.5\n\
             state answer is client Text from (text of a) + (text of b)\n"
        ),
        "32.5"
    );
}

#[test]
fn abs_and_clamp_do_what_they_say() {
    assert_eq!(
        text("state answer is client Text from text of (abs of (0 - 4))\n"),
        "4"
    );
    assert_eq!(
        text("state answer is client Text from text of (clamp with value is 12, low is 0, high is 10)\n"),
        "10"
    );
    assert_eq!(
        text("state answer is client Text from text of (clamp with value is (0 - 3), low is 0, high is 10)\n"),
        "0"
    );
}

#[test]
fn text_of_renders_each_base_type_the_way_the_language_writes_it() {
    assert_eq!(text("state answer is client Text from text of 42\n"), "42");
    assert_eq!(
        text("state answer is client Text from text of 1.5\n"),
        "1.5"
    );
    // `yes` and `no`, not `true` and `false`: §17.4.9 gives the ZDeceptron
    // definition and the emission has to agree with it.
    assert_eq!(
        text("state answer is client Text from text of yes\n"),
        "yes"
    );
    assert_eq!(text("state answer is client Text from text of no\n"), "no");
    assert_eq!(
        text("state answer is client Text from text of \"x\"\n"),
        "x"
    );
}
