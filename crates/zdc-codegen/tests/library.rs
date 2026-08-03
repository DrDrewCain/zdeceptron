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

// --- the delimiter family ------------------------------------------------
//
// Every operation here is linear in the input because it is written over
// `split`, which is the one primitive that walks a whole `Text` in a
// single step. The character walk `slice` uses cannot reach the end of a
// document (see the ten-thousand-character test at the bottom of the file),
// so these are what a content site actually runs.

#[test]
fn before_and_after_cut_at_the_first_delimiter() {
    assert_eq!(
        text(
            "state answer is client Text from before with value is \"a/b/c\", delimiter is \"/\"\n"
        ),
        "a"
    );
    assert_eq!(
        text(
            "state answer is client Text from after with value is \"a/b/c\", delimiter is \"/\"\n"
        ),
        "b/c",
        "everything after the first delimiter, separators and all"
    );
    // The delimiter is absent: `before` keeps the whole text, because the
    // whole text *is* what comes before an occurrence that never happens;
    // `after` has nothing to give.
    assert_eq!(
        text("state answer is client Text from before with value is \"abc\", delimiter is \"/\"\n"),
        "abc"
    );
    assert_eq!(
        text(
            "state answer is client Text from (after with value is \"abc\", delimiter is \"/\") + \"!\"\n"
        ),
        "!"
    );
}

#[test]
fn before_last_and_after_last_cut_at_the_final_delimiter() {
    assert_eq!(
        text(
            "state answer is client Text from beforeLast with value is \"a/b/c\", delimiter is \"/\"\n"
        ),
        "a/b"
    );
    assert_eq!(
        text(
            "state answer is client Text from afterLast with value is \"a/b/c\", delimiter is \"/\"\n"
        ),
        "c"
    );
    assert_eq!(
        text(
            "state answer is client Text from (beforeLast with value is \"abc\", delimiter is \"/\") + \"!\"\n"
        ),
        "!",
        "absent: the mirror of `after`, which is also empty"
    );
    assert_eq!(
        text(
            "state answer is client Text from afterLast with value is \"abc\", delimiter is \"/\"\n"
        ),
        "abc",
        "absent: the mirror of `before`, which is also the whole text"
    );
}

#[test]
fn stripping_an_affix_only_strips_one_that_is_there() {
    assert_eq!(
        text(
            "state answer is client Text from withoutPrefix with value is \"# Title\", prefix is \"# \"\n"
        ),
        "Title"
    );
    assert_eq!(
        text(
            "state answer is client Text from withoutPrefix with value is \"Title\", prefix is \"# \"\n"
        ),
        "Title",
        "a prefix that is not there leaves the value alone"
    );
    assert_eq!(
        text(
            "state answer is client Text from withoutSuffix with value is \"post.md\", suffix is \".md\"\n"
        ),
        "post"
    );
    assert_eq!(
        text(
            "state answer is client Text from withoutSuffix with value is \"post.markdown\", suffix is \".md\"\n"
        ),
        "post.markdown",
        "`.md` is not a suffix of `.markdown`, and `endsWith` is what says so"
    );
    // An empty affix is the identity in both directions. Without the guard
    // it would reach `split` with an empty separator, which is the
    // platform's per-UTF-16-unit split and would break the code-point
    // invariant `$textLength` exists to keep.
    assert_eq!(
        text(
            "state answer is client Text from withoutPrefix with value is \"ab\", prefix is \"\"\n"
        ),
        "ab"
    );
    assert_eq!(
        text(
            "state answer is client Text from withoutSuffix with value is \"ab\", suffix is \"\"\n"
        ),
        "ab"
    );
}

#[test]
fn replace_changes_every_occurrence_and_not_only_the_first() {
    assert_eq!(
        text(
            "state answer is client Text from replace with value is \"a-b-c\", old is \"-\", new is \"+\"\n"
        ),
        "a+b+c"
    );
    assert_eq!(
        text(
            "state answer is client Text from replace with value is \"abc\", old is \"-\", new is \"+\"\n"
        ),
        "abc"
    );
    assert_eq!(
        text(
            "state answer is client Text from replace with value is \"a&b\", old is \"&\", new is \"&amp;\"\n"
        ),
        "a&amp;b",
        "the one every generated feed needs"
    );
}

#[test]
fn index_of_gives_a_position_or_none_rather_than_a_sentinel() {
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is (indexOf with value is \"hello world\", needle is \"world\"), fallback is 0 - 1)\n"
        ),
        "6"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is (indexOf with value is \"hello\", needle is \"zzz\"), fallback is 0 - 1)\n"
        ),
        "-1",
        "absent is `None`, and only the caller's own fallback is a number"
    );
    // Counted in code points, like `length of`, so an emoji before the
    // needle moves the answer by one rather than by two.
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is (indexOf with value is \"a🎉b\", needle is \"b\"), fallback is 0 - 1)\n"
        ),
        "2"
    );
}

#[test]
fn lines_and_unlines_round_trip_a_document() {
    assert_eq!(
        text(
            "state doc is client Text from unlines of [\"one\", \"two\", \"three\"]\n\
             state answer is client Text from text of (listLength of (lines of doc))\n"
        ),
        "3"
    );
    assert_eq!(
        text(
            "state doc is client Text from unlines of [\"one\", \"two\"]\n\
             state answer is client Text from join with parts is (lines of doc), using is \"|\"\n"
        ),
        "one|two",
        "the separator the lexer cannot write survives the round trip"
    );
    // `newline` is a `Text` and composes like one.
    assert_eq!(
        text(
            "state answer is client Text from text of (textLength of (\"a\" + newline + \"b\"))\n"
        ),
        "3"
    );
}

// --- the three things a content site could not be written without -------
//
// Each of these is a case a previous agent reported as blocked on the
// absence of text operations. They are written the way a program would
// write them, out of library calls only, because that is the whole claim
// being tested.

/// **Worked case 1.** Extract a title from markdown.
///
/// The build-capabilities agent's exact blocker: the build host reads a
/// `.md` file and the language cannot get `Some Title` out of
/// `# Some Title`. It is one `before` and one `withoutPrefix`, and it is
/// linear in the length of the document rather than in the length of the
/// title's line, which matters because the document is the whole file.
#[test]
fn a_title_is_extracted_from_a_markdown_document() {
    assert_eq!(
        text(
            "state doc is client Text from unlines of [\"# Some Title\", \"\", \"Body text, and more of it.\"]\n\
             state answer is client Text from withoutPrefix with value is (before with value is doc, delimiter is newline), prefix is \"# \"\n"
        ),
        "Some Title"
    );
    // A document whose first line is not a heading keeps its first line,
    // which is what lets a caller test for the heading rather than having
    // to trust it.
    assert_eq!(
        text(
            "state doc is client Text from unlines of [\"Body first.\", \"# Not a title\"]\n\
             state answer is client Text from withoutPrefix with value is (before with value is doc, delimiter is newline), prefix is \"# \"\n"
        ),
        "Body first."
    );
    // And the body is the other half of the same cut.
    assert_eq!(
        text(
            "state doc is client Text from unlines of [\"# Some Title\", \"Body text.\"]\n\
             state answer is client Text from after with value is doc, delimiter is newline\n"
        ),
        "Body text."
    );
}

/// **Worked case 2.** Derive a slug from a file path.
///
/// The build-capabilities agent reported `Post.slug` as the file path and
/// `Post.title` as absent, because stripping a directory prefix and a
/// `.md` suffix needed operations §14F had no library for. Two calls, and
/// two ways to write the first depending on whether the directory is known
/// in advance.
#[test]
fn a_slug_is_derived_from_a_file_path() {
    assert_eq!(
        text(
            "state path is client Text from \"content/blog/hello-world.md\"\n\
             state answer is client Text from withoutSuffix with value is (withoutPrefix with value is path, prefix is \"content/blog/\"), suffix is \".md\"\n"
        ),
        "hello-world"
    );
    // Without knowing the directory: the last segment, minus the extension.
    assert_eq!(
        text(
            "state path is client Text from \"content/blog/2026/hello-world.md\"\n\
             state answer is client Text from withoutSuffix with value is (afterLast with value is path, delimiter is \"/\"), suffix is \".md\"\n"
        ),
        "hello-world"
    );
    // A dot inside the name is not the extension, which is why this is
    // `withoutSuffix` and not `before … delimiter is \".\"`.
    assert_eq!(
        text(
            "state path is client Text from \"v1.2.release.md\"\n\
             state answer is client Text from withoutSuffix with value is path, suffix is \".md\"\n"
        ),
        "v1.2.release"
    );
}

/// **Worked case 3.** Build an RSS feed by folding a `List of Post` into
/// `Text` — what the `static` placement agent reported §14F as blocking,
/// leaving `rss.xml` derived from `heading` rather than from every post.
///
/// The fold is the program's own index recursion, which is what §17.4.9's
/// technique already was; what did not exist is everything inside it —
/// concatenating the item, and escaping `&` and `<` so the feed is
/// well-formed. `replace` is that.
#[test]
fn an_rss_feed_is_folded_out_of_a_list_of_posts() {
    let feed = shown(
        "record Post\n\
         \x20   slug  is Text\n\
         \x20   title is Text\n\
         \n\
         function escaped of value\n\
         \x20   give replace with value is (replace with value is value, old is \"&\", new is \"&amp;\"), old is \"<\", new is \"&lt;\"\n\
         \n\
         function itemFor of post\n\
         \x20   give \"<item><title>\" + (escaped of post.title) + \"</title><link>https://example.com/\" + post.slug + \"</link></item>\"\n\
         \n\
         function feedFrom with posts, index\n\
         \x20   when listAt with value is posts, index is index\n\
         \x20       None\n\
         \x20           give \"\"\n\
         \x20       Some with post\n\
         \x20           give (itemFor of post) + (feedFrom with posts is posts, index is index + 1)\n\
         \n\
         state posts is client List of Post starting [(Post with slug is \"hello\", title is \"Ada & Bob\"), (Post with slug is \"next\", title is \"Two < Three\")]\n\
         state answer is client Text from feedFrom with posts is posts, index is 0\n",
    );
    assert!(
        feed.contains(
            "<item><title>Ada &amp; Bob</title><link>https://example.com/hello</link></item>"
        ),
        "{feed}"
    );
    assert!(
        feed.contains(
            "<item><title>Two &lt; Three</title><link>https://example.com/next</link></item>"
        ),
        "{feed}"
    );
}

// --- the size a real document is ----------------------------------------

/// A ten-thousand character document, run through every operation a
/// content site puts a whole file through.
///
/// This is the test that stops a quadratic operation getting into the
/// library unnoticed, and it has two teeth rather than one. A builder that
/// recurses once per *character* does not merely get slow: it exceeds the
/// host's stack and the program returns an error instead of an answer, so
/// the assertions below fail outright. And an operation that is quadratic
/// but shallow blows the elapsed budget, which is set an order of
/// magnitude above what the linear versions take.
///
/// The stack depth of the delimiter family is one frame per *piece*, not
/// per character — `join` and `joinFrom` recurse over the list `split`
/// produced — so what bounds these operations is the number of lines, and
/// two hundred is what an ordinary post has.
#[test]
fn the_delimiter_family_survives_a_ten_thousand_character_document() {
    let body: Vec<String> = (0..200)
        .map(|i| format!("line {i} of the document, padded out to a realistic width."))
        .collect();
    let characters: usize = body.iter().map(|line| line.chars().count()).sum::<usize>() + 199;
    assert!(
        characters > 10_000,
        "the document must be the size the test claims: {characters}"
    );
    let literal = body
        .iter()
        .map(|line| format!("\"{line}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let started = std::time::Instant::now();
    let answer = text(&format!(
        "state doc is client Text from unlines of [{literal}]\n\
         state answer is client Text from (text of (length of doc)) + \"|\" \
         + (text of (doc contains \"line 150 of\")) + \"|\" \
         + (text of (doc contains \"line 200 of\")) + \"|\" \
         + (text of (listLength of (lines of doc))) + \"|\" \
         + (before with value is doc, delimiter is newline) + \"|\" \
         + (text of (length of (after with value is doc, delimiter is newline))) + \"|\" \
         + (text of (length of (replace with value is doc, old is \"line \", new is \"row \"))) + \"|\" \
         + (afterLast with value is doc, delimiter is newline)\n"
    ));
    let elapsed = started.elapsed();

    let first = &body[0];
    let last = &body[199];
    let after_first = characters - first.chars().count() - 1;
    // `line ` is five characters and `row ` is four, so 200 replacements
    // take 200 characters off.
    let replaced = characters - 200;
    assert_eq!(
        answer,
        format!("{characters}|yes|no|200|{first}|{after_first}|{replaced}|{last}")
    );
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "the delimiter family went superlinear: {elapsed:?}"
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

/// `split` is written in ZDeceptron now, and the answers it has to agree
/// with are the platform's. Every case here is one JavaScript's own
/// `String.prototype.split` settles, including the two that are easy to
/// get wrong: a separator at the end yields a trailing empty piece, and
/// splitting the empty text yields one empty piece rather than none.
#[test]
fn split_is_written_in_zdeceptron_and_agrees_with_the_platform() {
    for (value, using, expected) in [
        ("a,b,c", ",", "a|b|c"),
        ("a,,b", ",", "a||b"),
        ("a,", ",", "a|"),
        (",a", ",", "|a"),
        ("", ",", ""),
        ("abc", ",", "abc"),
        ("a::b", "::", "a|b"),
        ("aa", "aaa", "aa"),
    ] {
        assert_eq!(
            text(&format!(
                "state parts is client List of Text from split with value is \"{value}\", \
                 using is \"{using}\"\n\
                 state answer is client Text from join with parts is parts, using is \"|\"\n"
            )),
            expected,
            "`{value}` split on `{using}`"
        );
    }
}

/// And where it deliberately does *not* agree. JavaScript's `split("")`
/// divides a Text into UTF-16 units, so it hands back two halves of a
/// `🎉` that are not characters at all. §5.4 says a `Text` is text, and
/// `textAt` and `length of` already index it by code point; `split` now
/// does too, and this is the one behaviour the move changed.
#[test]
fn splitting_on_nothing_gives_characters_not_utf16_units() {
    assert_eq!(
        text(
            "state parts is client List of Text from split with value is \"ab\", using is \"\"\n\
             state answer is client Text from join with parts is parts, using is \"|\"\n"
        ),
        "a|b"
    );
    assert_eq!(
        text(
            "state parts is client List of Text from split with value is \"a🎉b\", \
             using is \"\"\n\
             state answer is client Text from text of (length of parts)\n"
        ),
        "3",
        "three characters, not four UTF-16 units"
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

/// `keys of` is a ZDeceptron fold over `mapKeyAt` now — it was the last
/// primitive that handed back a collection — so it has to enumerate the
/// map itself, in the order the map was built in.
#[test]
fn keys_of_is_written_in_zdeceptron_and_gives_insertion_order() {
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting [\"c\" to 1, \"a\" to 2, \"b\" to 3]\n\
             state answer is client Text from join with parts is (keys of m), using is \"\"\n"
        ),
        "cab",
        "the order the map was written in, not sorted and not the platform's idea of an order"
    );
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting empty\n\
             state answer is client Text from text of (length of (keys of m))\n"
        ),
        "0"
    );
    assert_eq!(
        text(
            "state m is client Map of Whole to Text starting [10 to \"x\", 2 to \"y\", 1 to \"z\"]\n\
             state answer is client Text from join with parts is (values of m), using is \"\"\n"
        ),
        "xyz",
        "integer-like keys keep their insertion order, which a plain object would not have"
    );
}

/// `values of` is a ZDeceptron fold over the entries now, so it has to
/// give the same answers in the same order as the map's own iteration —
/// which is insertion order, and is what `keys of` reports.
#[test]
fn values_of_is_written_in_zdeceptron_and_follows_the_keys() {
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting [\"a\" to 1, \"b\" to 2, \"c\" to 3]\n\
             state answer is client Text from text of (sumOf of (values of m))\n"
        ),
        "6"
    );
    assert_eq!(
        text(
            "state m is client Map of Text to Text starting [\"a\" to \"x\", \"b\" to \"y\"]\n\
             state answer is client Text from join with parts is (values of m), using is \"\"\n"
        ),
        "xy",
        "in the order `keys of` gives, which is the order the map was written in"
    );
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting empty\n\
             state answer is client Text from text of (length of (values of m))\n"
        ),
        "0"
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

/// The fold that reads both halves of an entry: given a value, the key
/// that holds it. Nothing above it needs a key and a value at the same
/// time, so this is what says a map can be *walked* rather than merely
/// projected into two lists.
#[test]
fn a_map_can_be_read_backwards_by_folding_over_its_entries() {
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting [\"ada\" to 7, \"bob\" to 9]\n\
             state answer is client Text from keyOfOr with table is m, value is 9, \
             fallback is \"nobody\"\n"
        ),
        "bob"
    );
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting [\"ada\" to 7, \"bob\" to 9]\n\
             state answer is client Text from keyOfOr with table is m, value is 5, \
             fallback is \"nobody\"\n"
        ),
        "nobody",
        "a value no key holds is the fallback"
    );
    assert_eq!(
        text(
            "state m is client Map of Text to Whole starting [\"ada\" to 7, \"bob\" to 7]\n\
             state answer is client Text from keyOfOr with table is m, value is 7, \
             fallback is \"nobody\"\n"
        ),
        "ada",
        "the first key in insertion order wins, which is only a rule if the order is one"
    );
}

/// **The determinism the wire format depends on.** A `durable Map` is
/// stored as its pairs and rebuilt from them, so an enumeration that
/// disagreed with itself between two reads — or between a map and the map
/// that came back from storage — would make a build unreproducible.
///
/// Two facts, both checked against the emitted helper rather than
/// asserted: the same map enumerates the same way twice, and a map that
/// has been through the pair form enumerates exactly as it did before.
/// The keys are integer-like on purpose: that is where a plain object
/// reorders and a `Map` does not, and §5.4's choice of `Map` is what the
/// promise rests on.
#[test]
fn a_map_enumerates_the_same_way_twice_and_survives_the_wire_form() {
    let bundle = compile_source(
        "state m is client Map of Whole to Text starting [10 to \"x\", 2 to \"y\"]\n\
         state answer is client Text from text of (length of (keys of m))\n\
         view\n    Text answer\n",
    );
    let mut context = context(false);
    let answer = run(
        &mut context,
        &bundle.client_js,
        "const $walk = (table) => {\n  \
         const out = [];\n  \
         for (let i = 0; ; i += 1) {\n    \
         const step = $mapKeyAt(table, i);\n    \
         if (step.tag === 'None') return out.join(',');\n    \
         out.push(step.fields[0]);\n  \
         }\n\
         };\n\
         const $written = new Map([[10, 'x'], [2, 'y'], [1, 'z']]);\n\
         const $restored = new Map(JSON.parse(JSON.stringify([...$written])));\n\
         [$walk($written), $walk($written), $walk($restored)].join(' ')",
    );
    assert_eq!(
        answer, "10,2,1 10,2,1 10,2,1",
        "the same map twice, then the map rebuilt from its pairs"
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

// --- the numeric half of §14F, and the soundness hole it closed ----------

/// **The acceptance test for the division fix.**
///
/// The re-measurement of 2026-08-03 found `set q to a / b` with `a = 7`,
/// `b = 3` putting `2.3333333333333335` into a signal declared `Whole`.
/// `/` now gives a `Decimal` — `zdc-types` refuses the old program, and
/// this is the other half: every route from a `Whole` back through
/// division to a `Whole` lands on an integer, and it does so for the
/// negative cases too, where floor and truncation disagree.
#[test]
fn whole_arithmetic_stays_integral_across_division() {
    // The exact value the report quoted, now unreachable as a `Whole`.
    assert_eq!(
        text("state answer is client Text from text of (quotient with value is 7, divisor is 3)\n"),
        "2"
    );
    assert_eq!(
        text("state answer is client Text from text of (mod with value is 7, divisor is 3)\n"),
        "1"
    );
    // The `Decimal` that `/` gives is the true quotient and says so.
    assert_eq!(
        text("state answer is client Text from text of (7 / 3)\n"),
        "2.3333333333333335"
    );
    // Floored, not truncated, so a remainder is never negative for a
    // positive divisor — this is the property a torus index needs.
    assert_eq!(
        text(
            "state answer is client Text from text of (quotient with value is (0 - 7), divisor is 3)\n"
        ),
        "-3"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (mod with value is (0 - 7), divisor is 3)\n"
        ),
        "2"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (mod with value is (0 - 1), divisor is 8)\n"
        ),
        "7"
    );
}

/// `divisor * quotient + mod` is the value, which is the identity that
/// makes the pair a division rather than two unrelated functions.
#[test]
fn quotient_and_remainder_reconstruct_the_value() {
    for (value, divisor) in [(17, 5), (-17, 5), (17, -5), (-17, -5), (0, 7), (9, 3)] {
        let source = format!(
            "state answer is client Text from text of \
             (({divisor} * (quotient with value is {value}, divisor is {divisor})) \
             + (mod with value is {value}, divisor is {divisor}))\n"
        );
        let source = source.replace('-', "0 - ");
        assert_eq!(
            text(&source),
            value.to_string(),
            "{value} / {divisor} must reconstruct"
        );
    }
}

/// The window the prelude promises: the low 32 bits, unsigned, for all
/// six. The negative operands are the interesting ones — JavaScript's own
/// `&`, `|`, `^` and `<<` give back a *signed* int32, and a `Whole` that
/// came out of a bitwise operation claiming to be in `0 … 4294967295` and
/// holding `-1` would be the division bug again in another place.
#[test]
fn the_bitwise_window_is_thirty_two_bits_and_unsigned() {
    let bits = |expr: &str| {
        text(&format!(
            "state answer is client Text from text of ({expr})\n"
        ))
    };

    assert_eq!(bits("bitAnd with left is 12, right is 10"), "8");
    assert_eq!(bits("bitOr with left is 12, right is 10"), "14");
    assert_eq!(bits("bitXor with left is 12, right is 10"), "6");
    assert_eq!(bits("shiftLeft with value is 1, places is 4"), "16");
    assert_eq!(bits("shiftRight with value is 256, places is 4"), "16");

    // `-1` is all ones, and the window says so as an unsigned number.
    assert_eq!(bits("toUnsigned32 of (0 - 1)"), "4294967295");
    assert_eq!(
        bits("bitXor with left is (0 - 1), right is 0"),
        "4294967295"
    );
    // `1 << 31` is negative in JavaScript and is not here.
    assert_eq!(
        bits("shiftLeft with value is 1, places is 31"),
        "2147483648"
    );
    // Overflow out of the window wraps rather than growing.
    assert_eq!(bits("shiftLeft with value is 1, places is 32"), "1");

    // `*` cannot do this: 65535 * 65535 is 4294836225, which fits, but
    // 2147483647 * 3 does not, and `wrappingProduct` is the one that
    // keeps the low bits.
    assert_eq!(
        bits("wrappingProduct with left is 65535, right is 65535"),
        "4294836225"
    );
    assert_eq!(
        bits("wrappingProduct with left is 2147483647, right is 3"),
        "2147483645"
    );
}

/// mulberry32, written in ZDeceptron over those six, agreeing with the
/// reference implementation bit for bit.
///
/// The expected values are what the published JavaScript produces from
/// seed 0 — if the prelude's arithmetic drifted by one bit, a generator
/// would still *look* random and would no longer be this generator.
#[test]
fn the_seeded_generator_reproduces_mulberry32() {
    let draw = |steps: usize| {
        let mut seed = "12345".to_string();
        for _ in 0..steps {
            seed = format!("nextSeed of ({seed})");
        }
        text(&format!(
            "state answer is client Text from text of (randomBits of ({seed}))\n"
        ))
    };
    // Computed by the reference mulberry32 for seed 12345, in an
    // implementation written outside this compiler, and pinned so that a
    // change to any of the six primitives is a failure here rather than a
    // different game.
    assert_eq!(draw(1), "4207900869");
    assert_eq!(draw(2), "1317490944");
    assert_eq!(draw(3), "2079646450");
}

/// Same seed, same number — which is the whole property that lets a game
/// engine be replayed and a `static` value be computed twice.
#[test]
fn the_generator_is_a_pure_function_of_its_seed() {
    assert_eq!(
        text(
            "state answer is client Text from text of \
             ((randomBits of 7) - (randomBits of 7))\n"
        ),
        "0"
    );
    // And two different seeds do not agree, so it is not a constant.
    assert_eq!(
        text(
            "state answer is client Text from text of \
             ((randomBits of 7) is (randomBits of 8))\n"
        ),
        "no"
    );
}

/// `randomBelow` stays inside its bound, including for the seeds whose
/// raw output is above 2^31 — where a signed shift would have produced a
/// negative index and `at` would have given `None` for ever.
#[test]
fn a_bounded_draw_is_inside_its_bound() {
    for seed in [1, 2, 3, 99, 4294967295u32] {
        let answer = text(&format!(
            "state answer is client Text from text of \
             (randomBelow with seed is {seed}, bound is 6)\n"
        ));
        let roll: i64 = answer.parse().unwrap_or_else(|_| panic!("{answer}"));
        assert!((0..6).contains(&roll), "seed {seed} gave {roll}");
    }
}

/// The limits of the numeric library, pinned so that they are recorded
/// rather than discovered.
///
/// **These are not assertions that the answers are good.** A zero divisor
/// puts `Infinity` or `NaN` into a value whose type says `Whole`, which is
/// the same shape of unsoundness as the `Whole / Whole` defect this branch
/// closed and is *not* closed here: `floor of` and `round of` have
/// declared `gives Whole` and returned `Infinity` for an infinite
/// `Decimal` since they were written, so the hole is theirs, and shutting
/// it means deciding in §14A.3 whether a `Whole` admits `Infinity` and
/// `NaN` at all. Pinning the answers makes that decision show up here as a
/// test to update instead of as a surprise in somebody's game.
#[test]
fn a_zero_divisor_and_an_overflow_do_what_the_platform_does() {
    // `Infinity` and `NaN`, in a `Whole`. Recorded, not endorsed.
    assert_eq!(
        text("state answer is client Text from text of (quotient with value is 1, divisor is 0)\n"),
        "Infinity"
    );
    assert_eq!(
        text("state answer is client Text from text of (mod with value is 1, divisor is 0)\n"),
        "NaN"
    );
    assert_eq!(
        text("state answer is client Text from text of (floor of (1 / 0))\n"),
        "Infinity"
    );

    // Above 2^53 a `Whole` loses precision — §14A.3 chose f64 and said so.
    // It does *not* lose integrality, which is the difference between a
    // documented bound and the defect this branch fixed: every f64 at that
    // magnitude is an integer, so `+`, `-` and `*` cannot produce the
    // fraction `/` used to.
    assert_eq!(
        text("state answer is client Text from text of (9007199254740992 + 1)\n"),
        "9007199254740992"
    );
    assert_eq!(
        text("state answer is client Text from text of (94906266 * 94906266)\n"),
        "9007199326062756"
    );

    // A shift count is taken modulo 32, which is the platform's rule and
    // is why `places is 32` is not "shift everything away".
    assert_eq!(
        text("state answer is client Text from text of (shiftRight with value is 256, places is 0 - 4)\n"),
        "0"
    );
}
