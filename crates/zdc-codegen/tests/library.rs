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

/// What the type checker refuses a program for.
///
/// `support::refusals` reports *codegen* refusals, and a type error is not
/// one: `try_compile` runs the checker with `unwrap_or_default` so that a
/// test about emission is not also a test about inference. §14A.3's ruling
/// is enforced by the checker, so this asks the checker.
fn type_errors(source: &str) -> Vec<String> {
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("{}", e.message));
    let hir = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{}", errors[0].message));
    let split = zdc_graph::split(&hir);
    match zdc_types::check(&hir, &split) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|error| error.message).collect(),
    }
}

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

/// **Construction at a length the source does not write out.** `indices`
/// is the one that unlocks the rest: `map each` binds the element and not
/// its position, so a program that wants a grid has to start from the
/// positions themselves.
#[test]
fn indices_builds_a_list_the_source_never_spelled_out() {
    assert_eq!(
        text(
            "state size is client Whole starting 5\n\
             state xs is client List of Whole from indices of size\n\
             state parts is client List of Text from labels of xs\n\
             state answer is client Text from join with parts is parts, using is \"|\"\n\
             function labels of xs\n    from xs\n    map each n to text of n\n"
        ),
        "0|1|2|3|4"
    );
}

/// The count is a value, not a literal — which is the whole point — so a
/// count of zero and a negative count have to answer rather than run away.
#[test]
fn a_count_of_zero_or_less_gives_the_empty_list() {
    assert_eq!(
        text("state answer is client Text from text of (length of (indices of 0))\n"),
        "0"
    );
    assert_eq!(
        text("state answer is client Text from text of (length of (indices of (0 - 3)))\n"),
        "0",
        "a negative count is empty, not an unbounded loop"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of \
             (length of (filled with value is \"x\", count is 0))\n"
        ),
        "0"
    );
}

/// `Array(n).fill(x)`, which is what four of the ported engines open with.
#[test]
fn filled_repeats_one_value_a_computed_number_of_times() {
    assert_eq!(
        text(
            "state n is client Whole starting 4\n\
             state tiles is client List of Text from filled with value is \"wall\", count is n\n\
             state answer is client Text from join with parts is tiles, using is \",\"\n"
        ),
        "wall,wall,wall,wall"
    );
}

/// A grid at a size the program chose, which is the thing `createBoard(w, h)`
/// needed and could not have: `indices` gives the positions and `map each`
/// turns each position into its cell.
#[test]
fn a_grid_can_be_built_at_a_size_the_program_computes() {
    assert_eq!(
        text(
            "state w is client Whole starting 3\n\
             state h is client Whole starting 2\n\
             state cells is client List of Text from gridOf with width is w, height is h\n\
             state answer is client Text from join with parts is cells, using is \" \"\n\
             function gridOf with width, height\n\
             \x20   from indices of (width * height)\n\
             \x20   map each i to (cellName with index is i, width is width)\n\
             function cellName with index, width\n\
             \x20   with row is (valueOr with maybe is (quotient with value is index, divisor is width), fallback is 0)\n\
             \x20   give text of (index - (row * width)) + \",\" + text of row\n"
        ),
        "0,0 1,0 2,0 0,1 1,1 2,1"
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

/// Render a library answer that is an `Option of Whole`: the number it
/// holds, or `none`.
///
/// §14A.3 makes the `Decimal`-to-`Whole` narrowing partial, so `floor of`,
/// `round of`, `quotient`, `mod` and `randomBelow` all give an `Option`.
/// Eliminating it with an ordinary `when` is what a program does, so it is
/// what these tests do — and it lets one assertion distinguish "the number
/// two" from "no answer" without a sentinel that could be mistaken for
/// either.
fn optional(expr: &str) -> String {
    text(&format!(
        "function shownOption of maybe\n    \
         when maybe\n        \
         Some with whole\n            \
         give text of whole\n        \
         None\n            \
         give \"none\"\n\
         state answer is client Text from shownOption of ({expr})\n"
    ))
}

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
    assert_eq!(optional("quotient with value is 7, divisor is 3"), "2");
    assert_eq!(optional("mod with value is 7, divisor is 3"), "1");
    // The `Decimal` that `/` gives is the true quotient and says so.
    assert_eq!(
        text("state answer is client Text from text of (7 / 3)\n"),
        "2.3333333333333335"
    );
    // Floored, not truncated, so a remainder is never negative for a
    // positive divisor — this is the property a torus index needs.
    assert_eq!(
        optional("quotient with value is (0 - 7), divisor is 3"),
        "-3"
    );
    assert_eq!(optional("mod with value is (0 - 7), divisor is 3"), "2");
    assert_eq!(optional("mod with value is (0 - 1), divisor is 8"), "7");
}

/// `divisor * quotient + mod` is the value, which is the identity that
/// makes the pair a division rather than two unrelated functions.
///
/// Both halves are eliminated with `valueOr` and a fallback that cannot be
/// mistaken for an answer: if either were `None` for a non-zero divisor
/// the sum would miss by a mile rather than by one, which is the failure
/// this test is here to make loud.
#[test]
fn quotient_and_remainder_reconstruct_the_value() {
    for (value, divisor) in [(17, 5), (-17, 5), (17, -5), (-17, -5), (0, 7), (9, 3)] {
        let source = format!(
            "state answer is client Text from text of \
             (({divisor} * (valueOr with maybe is \
             (quotient with value is {value}, divisor is {divisor}), fallback is 1000000)) \
             + (valueOr with maybe is \
             (mod with value is {value}, divisor is {divisor}), fallback is 1000000))\n"
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
        let answer = optional(&format!("randomBelow with seed is {seed}, bound is 6"));
        let roll: i64 = answer.parse().unwrap_or_else(|_| panic!("{answer}"));
        assert!((0..6).contains(&roll), "seed {seed} gave {roll}");
    }
    // **The reported case.** A port of real application code hit
    // `randomBelow with seed is …, bound is emptyCount` on a full 2,048
    // board: `emptyCount` was 0, the draw was `NaN` in a value typed
    // `Whole`, the spawn was silently skipped and nothing said anything.
    // A bound of zero is a range with no members, so there is nothing to
    // draw, and the type says so instead of handing back a number the
    // caller would index with.
    assert_eq!(optional("randomBelow with seed is 1, bound is 0"), "none");
}

/// **The acceptance test for the `Whole` finiteness ruling.**
///
/// This test used to be called
/// `a_zero_divisor_and_an_overflow_do_what_the_platform_does`, and its
/// first three assertions read `"Infinity"`, `"NaN"` and `"Infinity"` under
/// the heading *recorded, not endorsed*. That recording was correct and is
/// deliberately not deleted: it is restated here as the behaviour that is
/// now gone. §14A.3 rules that a `Whole` is a **finite** integral f64 and
/// that a `Decimal` is every f64, so `floor of` and `round of` — the only
/// narrowing between the two — give `Option of Whole`, and the three
/// answers are `None`.
///
/// The rest of the test is unchanged, because the 2^53 precision bound is
/// a documented limit rather than a defect and this ruling does not touch
/// it.
#[test]
fn a_zero_divisor_has_no_whole_answer_and_says_so() {
    // Was `Infinity`, `NaN` and `Infinity`, each held in a value whose
    // type said `Whole`. All three are `None` now.
    assert_eq!(optional("quotient with value is 1, divisor is 0"), "none");
    assert_eq!(optional("mod with value is 1, divisor is 0"), "none");
    assert_eq!(optional("floor of (1 / 0)"), "none");
    // The other two ways out of the finite `Decimal`s, for completeness:
    // `-Infinity` and the `NaN` that `0 / 0` is.
    assert_eq!(optional("floor of (0 - (1 / 0))"), "none");
    assert_eq!(optional("floor of (0 / 0)"), "none");
    assert_eq!(optional("round of (1 / 0)"), "none");

    // `Decimal` keeps them, and that is the point of the split: `/` is
    // total, the narrowing is not. A program that wants to see what the
    // division did still can.
    assert_eq!(
        text("state answer is client Text from text of (1 / 0)\n"),
        "Infinity"
    );

    // And the ordinary narrowing is untouched — a finite `Decimal` still
    // becomes the `Whole` it always did.
    assert_eq!(optional("floor of (7 / 2)"), "3");
    assert_eq!(optional("round of (7 / 2)"), "4");
    assert_eq!(optional("floor of (0 - (7 / 2))"), "-4");

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

/// The old behaviour is not merely absent, it is unwritable.
///
/// `state answer is client Whole from floor of (1 / 0)` compiled before
/// this branch and put `Infinity` in the signal. There is no longer a
/// spelling that lands a narrowing in a `Whole` without eliminating the
/// `Option` first, which is what makes the ruling a property of the type
/// system rather than a convention the library observes.
#[test]
fn a_narrowing_can_no_longer_be_stored_as_a_whole() {
    for source in [
        "state answer is client Whole from floor of (1 / 0)\nview\n    Text answer\n",
        "state answer is client Whole from round of (7 / 2)\nview\n    Text answer\n",
        "state answer is client Whole from quotient with value is 1, divisor is 0\n\
         view\n    Text answer\n",
        // And it cannot be laundered through arithmetic either: `Option of
        // Whole` is not a number, so there is no expression that adds one
        // to it.
        "state answer is client Whole from (floor of (1 / 0)) + 1\nview\n    Text answer\n",
        // Nor shown, which is how the old `Infinity` reached a page at all.
        "state answer is client Text from text of (floor of (1 / 0))\n\
         view\n    Text answer\n",
    ] {
        let errors = type_errors(source);
        assert!(
            errors.iter().any(|message| message.contains("Option")),
            "the refusal must name the `Option`: {errors:?}\n{source}"
        );
    }
}

/// `$listAt` is hardened at the sink, not only at the source.
///
/// The guard was `i >= 0 && i < xs.length`, which rejects `NaN` and both
/// infinities by accident of IEEE comparison but *admits* a finite
/// fraction — and `xs[1.5]` is `undefined`, so the helper could return a
/// `Some` wrapping nothing: a `None`-shaped failure wearing a `Some`.
/// §14A.3's ruling makes a fractional `Whole` unreachable through the type
/// system. Unreachable is not impossible, so the sink is checked too, and
/// this test calls the shipped helper directly rather than through a
/// program the checker would now refuse to write.
#[test]
fn an_index_that_is_not_a_whole_number_finds_nothing() {
    let bundle = compile_source(
        "state xs is client List of Whole starting [10, 20, 30]\n\
         state one is client Option of Whole from xs at 0\n\
         view\n\
         \x20   when one\n\
         \x20       Some with value show Text value\n\
         \x20       None            show Text \"none\"\n",
    );
    let mut context = context(false);
    let tags = run(
        &mut context,
        &bundle.client_js,
        "const probe = (i) => $listAt([10, 20, 30], i).tag;\n\
         [probe(1), probe(1.5), probe(0.5), probe(NaN), probe(Infinity), \
         probe(-Infinity), probe(-1), probe(3)].join(',')",
    );
    assert_eq!(tags, "Some,None,None,None,None,None,None,None");
}

/// The same guard on `textAt`, which indexes a code-point array and had
/// the identical hole.
#[test]
fn a_fractional_text_index_finds_nothing() {
    let bundle = compile_source(
        "state answer is client Text from valueOr with maybe is \
         (textAt with value is \"abc\", index is 0), fallback is \"\"\n\
         view\n    Text answer\n",
    );
    let mut context = context(false);
    let tags = run(
        &mut context,
        &bundle.client_js,
        "const probe = (i) => $textAt('abc', i).tag;\n\
         [probe(1), probe(1.5), probe(NaN), probe(Infinity)].join(',')",
    );
    assert_eq!(tags, "Some,None,None,None");
}

// --- the folds this library could not previously spell --------------------
//
// Every function below is written in ZDeceptron over `listAt` and
// `listLength`, in the shape `list.zd` records: one element per step, the
// answer travelling as a parameter, and a call and nothing else at the end.
//
// None of them takes a function, because the language has none to take
// (§17.2: "the language has no first-class functions"). So `anyOf` folds a
// list of `Truth` rather than applying a predicate, and the predicate is
// applied by the pipeline before the fold sees it. That is not a
// workaround: `map each` and `keep each` are the language's way of saying
// "apply this to every element", and a prelude function that duplicated
// them would need a value the language cannot construct.

#[test]
fn any_and_all_fold_a_list_of_truths() {
    assert_eq!(
        text("state answer is client Text from text of (anyOf of [no, yes, no])\n"),
        "yes"
    );
    assert_eq!(
        text("state answer is client Text from text of (anyOf of [no, no])\n"),
        "no"
    );
    assert_eq!(
        text("state answer is client Text from text of (allOf of [yes, yes])\n"),
        "yes"
    );
    assert_eq!(
        text("state answer is client Text from text of (allOf of [yes, no])\n"),
        "no"
    );
}

/// The empty list, which is where every fold's identity shows.
///
/// `anyOf` of nothing is `no` and `allOf` of nothing is `yes`, which
/// surprises people until they write the fold: the answer is whatever
/// leaves the fold unchanged.
#[test]
fn the_empty_list_gives_each_fold_its_identity() {
    assert_eq!(
        text("state answer is client Text from text of (anyOf of empty)\n"),
        "no"
    );
    assert_eq!(
        text("state answer is client Text from text of (allOf of empty)\n"),
        "yes"
    );
    assert_eq!(
        text("state answer is client Text from text of (countOf of empty)\n"),
        "0"
    );
}

#[test]
fn count_of_counts_the_yeses() {
    assert_eq!(
        text("state answer is client Text from text of (countOf of [yes, no, yes, yes])\n"),
        "3"
    );
}

#[test]
fn the_smallest_and_largest_of_a_list_are_optional() {
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (minOf of [3, 1, 2]), fallback is 0)\n"
        ),
        "1"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (maxOf of [3, 1, 2]), fallback is 0)\n"
        ),
        "3"
    );
    // Nothing has no smallest element, and saying so is what `Option` is
    // for. A sentinel would be a lie that typechecks.
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (minOf of empty), fallback is 0 - 1)\n"
        ),
        "-1"
    );
}

#[test]
fn take_and_drop_split_a_list_at_a_count() {
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (listTake with items is [\"a\", \"b\", \"c\"], count is 2), using is \",\"\n"
        ),
        "a,b"
    );
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (listDrop with items is [\"a\", \"b\", \"c\"], count is 2), using is \",\"\n"
        ),
        "c"
    );
}

/// A count outside the list is not an error, because there is no error to
/// be: both directions saturate, which is what makes pagination past the
/// end give an empty page rather than a refusal.
#[test]
fn a_count_past_either_end_saturates() {
    assert_eq!(
        text(
            "state answer is client Text from text of (listLength of \
             (listTake with items is [\"a\"], count is 9))\n"
        ),
        "1"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (listLength of \
             (listDrop with items is [\"a\"], count is 9))\n"
        ),
        "0"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (listLength of \
             (listTake with items is [\"a\", \"b\"], count is 0 - 3))\n"
        ),
        "0"
    );
}

#[test]
fn a_list_can_be_edited_at_a_position() {
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (insertAt with items is [\"a\", \"b\"], index is 1, item is \"x\"), using is \",\"\n"
        ),
        "a,x,b"
    );
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (removeAt with items is [\"a\", \"b\", \"c\"], index is 1), using is \",\"\n"
        ),
        "a,c"
    );
    // Inserting at the length appends, which is the one position an
    // insert has that a replace does not.
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (insertAt with items is [\"a\"], index is 1, item is \"b\"), using is \",\"\n"
        ),
        "a,b"
    );
    // An index nothing occupies removes nothing, for the same reason a
    // count past the end saturates.
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (removeAt with items is [\"a\"], index is 5), using is \",\"\n"
        ),
        "a"
    );
}

#[test]
fn a_list_of_lists_flattens_in_order() {
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (flatten of [[\"a\", \"b\"], empty, [\"c\"]]), using is \",\"\n"
        ),
        "a,b,c"
    );
}

/// Duplicates go by `is`, the same equality `contains` uses, and the
/// first occurrence is the one that stays.
#[test]
fn duplicates_are_dropped_keeping_the_first() {
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (withoutDuplicates of [\"b\", \"a\", \"b\", \"a\"]), using is \",\"\n"
        ),
        "b,a"
    );
}

#[test]
fn a_range_counts_up_to_but_not_including_its_stop() {
    assert_eq!(
        text(
            "state answer is client Text from text of (sumOf of \
             (range with start is 2, stop is 5))\n"
        ),
        "9"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (listLength of \
             (range with start is 5, stop is 5))\n"
        ),
        "0"
    );
    // A stop below the start is empty rather than counting down. One
    // direction, one meaning: counting down is `reverse of`.
    assert_eq!(
        text(
            "state answer is client Text from text of (listLength of \
             (range with start is 5, stop is 2))\n"
        ),
        "0"
    );
}

// --- the text operations tables cannot supply -----------------------------

#[test]
fn padding_reaches_a_width_and_never_truncates() {
    assert_eq!(
        text(
            "state answer is client Text from padStart with value is \"7\", \
             width is 3, using is \"0\"\n"
        ),
        "007"
    );
    assert_eq!(
        text(
            "state answer is client Text from padEnd with value is \"7\", \
             width is 3, using is \".\"\n"
        ),
        "7.."
    );
    // Already wide enough is left alone. Padding that truncated would be
    // two operations wearing one name.
    assert_eq!(
        text(
            "state answer is client Text from padStart with value is \"abcd\", \
             width is 2, using is \"0\"\n"
        ),
        "abcd"
    );
}

#[test]
fn repeat_concatenates_a_count_of_copies() {
    assert_eq!(
        text("state answer is client Text from repeat with value is \"ab\", count is 3\n"),
        "ababab"
    );
    assert_eq!(
        text("state answer is client Text from repeat with value is \"ab\", count is 0\n"),
        ""
    );
}

/// Case-insensitive comparison, written once here so that every program
/// with a search box does not write it again with two allocations.
#[test]
fn text_compares_ignoring_case() {
    assert_eq!(
        text(
            "state answer is client Text from text of (equalsIgnoringCase \
             with value is \"HeLLo\", other is \"hello\")\n"
        ),
        "yes"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (equalsIgnoringCase \
             with value is \"hello\", other is \"help\")\n"
        ),
        "no"
    );
}

// --- reading a number out of text -----------------------------------------

/// **What counts as a number, spelled out.**
///
/// `parseDecimal` accepts an optional sign, digits with an optional
/// fractional part, and an optional exponent, with any amount of
/// surrounding whitespace. That is the language's own numeric literal plus
/// a sign, and deliberately not JavaScript's `Number`, which reads the
/// empty text as zero, `"0x1f"` as thirty-one and `"Infinity"` as an
/// infinity. Each of those three is asserted below, because each is a
/// number a form field would otherwise produce from text nobody meant as
/// one.
#[test]
fn parsing_a_decimal_accepts_a_number_and_nothing_else() {
    assert_eq!(optional("parseDecimal of \"42\""), "42");
    assert_eq!(optional("parseDecimal of \"-1.5\""), "-1.5");
    assert_eq!(optional("parseDecimal of \"+7\""), "7");
    assert_eq!(optional("parseDecimal of \"1e3\""), "1000");
    assert_eq!(optional("parseDecimal of \".5\""), "0.5");
    // Surrounding space is what a pasted field contains, and it is not an
    // error.
    assert_eq!(optional("parseDecimal of \"  42  \""), "42");

    // The empty field is not zero. `Number("")` is, which is the single
    // most common way a form silently invents a value.
    assert_eq!(optional("parseDecimal of \"\""), "none");
    assert_eq!(optional("parseDecimal of \"   \""), "none");
    // Trailing rubbish is not a number, however numeric its prefix.
    // `parseFloat("12abc")` is 12; this is `None`.
    assert_eq!(optional("parseDecimal of \"12abc\""), "none");
    assert_eq!(optional("parseDecimal of \"abc\""), "none");
    assert_eq!(optional("parseDecimal of \"1,000\""), "none");
    assert_eq!(optional("parseDecimal of \"0x1f\""), "none");
    assert_eq!(optional("parseDecimal of \"Infinity\""), "none");
    assert_eq!(optional("parseDecimal of \"NaN\""), "none");
    // Overflowing the literal is not an infinity either. §14A.3 makes
    // `Decimal` every f64, so this one is a choice rather than a
    // consequence: text that came from outside the program is not an IEEE
    // operation, so there is no division here whose answer `Infinity`
    // would be. `1 / 0` is still the way to write one.
    assert_eq!(optional("parseDecimal of \"1e400\""), "none");
}

/// A `Whole` parse is exact: a fractional input has no whole number, and
/// says so rather than rounding towards one the caller did not ask for.
#[test]
fn parsing_a_whole_refuses_a_fraction_rather_than_rounding_it() {
    assert_eq!(optional("parseWhole of \"42\""), "42");
    assert_eq!(optional("parseWhole of \"-7\""), "-7");
    // A whole number written with a redundant point is still whole.
    assert_eq!(optional("parseWhole of \"12.0\""), "12");
    assert_eq!(optional("parseWhole of \"1e3\""), "1000");

    // Neither truncated nor rounded. A caller that wants either writes it:
    // `floor of` over `decimalOr` is the truncation, and it is one line.
    assert_eq!(optional("parseWhole of \"12.5\""), "none");
    assert_eq!(optional("parseWhole of \"-0.5\""), "none");
    assert_eq!(optional("parseWhole of \"\""), "none");
    assert_eq!(optional("parseWhole of \"12abc\""), "none");
    // Beyond the finite range there is no `Whole`, which is §14A.3's
    // ruling arriving here for free rather than being restated.
    assert_eq!(optional("parseWhole of \"1e400\""), "none");
}

/// The two the issue asked for: the fallback form, shaped like `atOr`,
/// `valueOr` and `readyOr`, so a numeric field is one expression.
#[test]
fn the_fallback_forms_eliminate_the_option_at_the_call_site() {
    assert_eq!(
        text(
            "state answer is client Text from text of \
             (wholeOr with value is \"42\", fallback is 0)\n"
        ),
        "42"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of \
             (wholeOr with value is \"\", fallback is 0)\n"
        ),
        "0"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of \
             (decimalOr with value is \"2.5\", fallback is 0)\n"
        ),
        "2.5"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of \
             (decimalOr with value is \"oops\", fallback is 0 - 1)\n"
        ),
        "-1"
    );
    // The fallback is the caller's and is not consulted when the parse
    // succeeds, which is what makes it a fallback rather than a default.
    assert_eq!(
        text(
            "state answer is client Text from text of \
             (wholeOr with value is \"0\", fallback is 9)\n"
        ),
        "0"
    );
}

/// **The acceptance test for issue #34.** A text box yields a number.
///
/// The parse is only worth having if it survives the whole compiler from
/// the element the user typed into, so this types into a real `Input`,
/// fires the event the runtime wires, and reads the computed number back
/// out of the rendered page. Nothing here inspects the generated source.
#[test]
fn a_text_box_yields_a_number_the_program_computes_with() {
    let bundle = compile_source(
        "state typed is client Text starting \"\"\n\
         state doubled is client Whole from (wholeOr with value is typed, fallback is 0) * 2\n\
         view\n    \
         Column\n        \
         Input typed, hint is \"how many\"\n        \
         Text (text of doubled)\n",
    );
    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $input = walk($host).find((n) => n.tagName === 'input');\n\
         const $frames = [];\n\
         $input.value = '21';\n\
         $input.fire('input');\n\
         $frames.push(html($host));\n\
         $input.value = 'not a number';\n\
         $input.fire('input');\n\
         $frames.push(html($host));\n\
         $frames.join('\\n')",
    );
    let frames: Vec<&str> = rendered.lines().collect();
    assert!(
        frames[0].contains(">42<"),
        "twenty-one typed into the box must double to forty-two:\n{}",
        frames[0]
    );
    // And rubbish falls back rather than putting a `NaN` into a `Whole`.
    assert!(
        frames[1].contains(">0<"),
        "text that is not a number must reach the fallback:\n{}",
        frames[1]
    );
}

// --- a root and an exponent -----------------------------------------------

/// The ordinary answers, which have to be the platform's own to the last
/// bit: a `sqrt` written in ZDeceptron by Newton's method would be a
/// second, differently rounded answer to a question that already has one.
#[test]
fn sqrt_and_power_agree_with_the_platform_to_the_last_bit() {
    assert_eq!(optional("sqrt of 9"), "3");
    assert_eq!(optional("sqrt of 0"), "0");
    // The value every implementation of a square root is compared against.
    assert_eq!(optional("sqrt of 2"), "1.4142135623730951");
    assert_eq!(optional("power with value is 2, exponent is 10"), "1024");
    assert_eq!(optional("power with value is 2, exponent is 0"), "1");
    // A fractional exponent is a root, and it is the reason `power` cannot
    // be repeated multiplication.
    assert_eq!(
        optional("power with value is 2, exponent is 0.5"),
        "1.4142135623730951"
    );
    assert_eq!(optional("power with value is 2, exponent is 0 - 1"), "0.5");
}

/// **The failure cases, which are the reason both give an `Option`.**
///
/// Issue #5 is open: `Whole` arithmetic on the client path is not guarded,
/// so nothing elsewhere in the language stops an `Infinity` being made.
/// That is an argument for these two not adding another way, not an excuse
/// to. The rule is one line and covers both: **`None` unless the answer is
/// a finite number.**
#[test]
fn a_root_of_a_negative_and_an_overflowing_power_have_no_answer() {
    // `Math.sqrt(-1)` is `NaN`, and a `NaN` wearing the name of a length
    // is how a distance calculation goes wrong three functions later.
    assert_eq!(optional("sqrt of (0 - 1)"), "none");
    assert_eq!(optional("sqrt of (0 / 0)"), "none");
    // `Math.sqrt(Infinity)` is `Infinity`. An infinite input has no finite
    // root, and this says so rather than passing the infinity along.
    assert_eq!(optional("sqrt of (1 / 0)"), "none");

    // `Math.pow(10, 400)` is `Infinity`: the overflow the issue names.
    assert_eq!(optional("power with value is 10, exponent is 400"), "none");
    assert_eq!(optional("power with value is 10, exponent is 0 - 400"), "0");
    // A negative base under a fractional exponent is `NaN`, because the
    // real cube root of a negative is not what `Math.pow` computes.
    assert_eq!(
        optional("power with value is 0 - 8, exponent is 0.5"),
        "none"
    );
    // `Math.pow(0, -1)` is `Infinity`, which is division by zero wearing
    // another name — and `quotient` already refuses that one.
    assert_eq!(optional("power with value is 0, exponent is 0 - 1"), "none");
}

/// **Worked case: the two things the issue said were unwritable.**
///
/// A distance, and the Brier score the JudgeHuman target is built on. Both
/// are one expression now and neither was expressible before.
#[test]
fn a_distance_and_a_brier_score_are_expressions_now() {
    // 3, 4, 5. `sqrt` of a sum of squares, with both squares from `power`.
    assert_eq!(
        optional(
            "sqrt of ((valueOr with maybe is (power with value is 3, exponent is 2), \
             fallback is 0 - 1) \
             + (valueOr with maybe is (power with value is 4, exponent is 2), \
             fallback is 0 - 1))"
        ),
        "5"
    );
    // The Brier score of one forecast: `(forecast - outcome) ^ 2`. A
    // confident forecast that was right scores near zero.
    assert_eq!(
        optional("power with value is (0.9 - 1), exponent is 2"),
        "0.009999999999999995"
    );
    // And a confident forecast that was wrong scores near one.
    assert_eq!(
        optional("power with value is (0.9 - 0), exponent is 2"),
        "0.81"
    );
}

// --- formatting a number for display --------------------------------------

/// Precision, which is the one third of this that is a primitive.
///
/// `text of n` is the platform's shortest form that round-trips and offers
/// no control at all; this is the fixed-point form, and it is the only
/// piece of formatting the language cannot express, because writing one
/// means reading the digits of an f64.
#[test]
fn fixed_point_text_gives_the_digits_asked_for() {
    assert_eq!(
        optional("fixedText with value is 3.14159, digits is 2"),
        "3.14"
    );
    assert_eq!(
        optional("fixedText with value is 3.14159, digits is 0"),
        "3"
    );
    // Trailing zeroes are the point of asking: a price is "2.50", not
    // "2.5", and `text of` cannot say so.
    assert_eq!(optional("fixedText with value is 2.5, digits is 2"), "2.50");
    assert_eq!(optional("fixedText with value is 2, digits is 2"), "2.00");
    // The f64 answer, not the decimal one. 1.005 is not representable, and
    // the nearest double is slightly below it, so this rounds down. It is
    // recorded rather than endorsed: the alternative is decimal
    // arithmetic, which §14A.3 did not choose.
    assert_eq!(
        optional("fixedText with value is 1.005, digits is 2"),
        "1.00"
    );
    assert_eq!(
        optional("fixedText with value is 0 - 1.25, digits is 1"),
        "-1.3"
    );
}

/// The failure cases, which are why this gives an `Option` rather than
/// letting a `RangeError` cross the boundary.
#[test]
fn fixed_point_text_refuses_what_it_cannot_render_as_digits() {
    // `toFixed` throws a `RangeError` outside 0 … 100, and a throw from
    // inside a signal is not a value the program can handle.
    assert_eq!(optional("fixedText with value is 1, digits is 101"), "none");
    assert_eq!(
        optional("fixedText with value is 1, digits is 0 - 1"),
        "none"
    );
    // A non-finite number has no fixed-point form. `(1 / 0).toFixed(2)` is
    // the text "Infinity", which would then be grouped into nonsense.
    assert_eq!(
        optional("fixedText with value is 1 / 0, digits is 2"),
        "none"
    );
    assert_eq!(
        optional("fixedText with value is 0 / 0, digits is 2"),
        "none"
    );
    // At or above 1e21 `toFixed` gives exponential notation rather than
    // digits, which would break the promise everything above this relies
    // on: the answer is a sign, digits and at most one point.
    assert_eq!(
        optional("fixedText with value is 1000000000000000000000, digits is 2"),
        "none"
    );
}

/// Grouping, which is **not** a primitive: it is a fold over the digits
/// `fixedText` produced, written in ZDeceptron like the rest of the
/// library.
#[test]
fn grouping_puts_a_separator_between_every_three_digits() {
    let grouped = |value: &str, digits: &str, separator: &str| {
        optional(&format!(
            "numberText with value is {value}, digits is {digits}, separator is \"{separator}\""
        ))
    };
    assert_eq!(grouped("1234567.891", "2", ","), "1,234,567.89");
    assert_eq!(grouped("1234", "0", ","), "1,234");
    assert_eq!(grouped("123", "0", ","), "123");
    assert_eq!(grouped("12", "0", ","), "12");
    assert_eq!(grouped("0", "2", ","), "0.00");
    // The boundary a naive implementation gets wrong: a digit count that
    // is an exact multiple of three must not begin with a separator.
    assert_eq!(grouped("123456", "0", ","), "123,456");
    assert_eq!(grouped("100000", "0", ","), "100,000");
    // The sign stays outside the grouping.
    assert_eq!(grouped("0 - 1234567", "0", ","), "-1,234,567");
    // The fraction is never grouped, however long it is.
    assert_eq!(grouped("1234.5", "4", ","), "1,234.5000");
    // A separator that is not a comma, because a thin space is what most
    // of the world writes. The decimal point stays the language's own;
    // swapping it is one `replace` at the call site and the library does
    // not guess.
    assert_eq!(grouped("1234567", "0", " "), "1 234 567");
    // No separator at all is the empty text, and it is not a special case.
    assert_eq!(grouped("1234567", "0", ""), "1234567");
    // And the `None` travels: an unrenderable number does not become a
    // grouped "Infinity".
    assert_eq!(
        optional("numberText with value is 1 / 0, digits is 2, separator is \",\""),
        "none"
    );
}

/// Currency, which is the third thing the issue named and is also written
/// in ZDeceptron: a symbol, and the one rule about where it goes.
#[test]
fn money_puts_the_symbol_inside_the_sign() {
    let money = |value: &str, digits: &str, symbol: &str| {
        optional(&format!(
            "moneyText with value is {value}, digits is {digits}, \
             separator is \",\", symbol is \"{symbol}\""
        ))
    };
    assert_eq!(money("1234.5", "2", "$"), "$1,234.50");
    // A currency with no minor unit says so with `digits is 0` rather than
    // with a table of currency codes the library would have to carry.
    assert_eq!(money("1234.5", "0", "¥"), "¥1,235");
    // The sign goes outside the symbol. "$-1,234.50" is what putting the
    // symbol on the front unconditionally produces, and it is wrong
    // everywhere.
    assert_eq!(money("0 - 1234.5", "2", "$"), "-$1,234.50");
    // A symbol that follows the amount is one `+` at the call site, which
    // is why there is no argument here for which side it goes on.
    assert_eq!(money("1234.5", "2", ""), "1,234.50");
}

// --- encoding -------------------------------------------------------------

/// Percent-encoding, over the UTF-8 bytes of a `Text`.
///
/// The characters asserted here are exactly the ones that give a URL its
/// structure. A value carrying any of them, concatenated into a URL, does
/// not land where the program meant it to.
#[test]
fn url_encoding_escapes_every_character_that_structures_a_url() {
    let encoded = |value: &str| {
        text(&format!(
            "state answer is client Text from urlEncoded of \"{value}\"\n"
        ))
    };
    assert_eq!(encoded("a b"), "a%20b");
    assert_eq!(encoded("a/b"), "a%2Fb");
    assert_eq!(encoded("a?b"), "a%3Fb");
    assert_eq!(encoded("a&b"), "a%26b");
    assert_eq!(encoded("a=b"), "a%3Db");
    assert_eq!(encoded("a#b"), "a%23b");
    assert_eq!(encoded("a%b"), "a%25b");
    assert_eq!(encoded("a+b"), "a%2Bb");
    // Bytes, not code units: a character outside ASCII becomes its UTF-8
    // encoding, which is what a URL carries and what the language cannot
    // see for itself.
    assert_eq!(encoded("é"), "%C3%A9");
    assert_eq!(encoded("🎉"), "%F0%9F%8E%89");
    // The unreserved set is left alone, so an ordinary word is unchanged
    // and a program can read what it built.
    assert_eq!(encoded("abcXYZ019-_.~"), "abcXYZ019-_.~");
    assert_eq!(encoded(""), "");
}

/// **The reason this is a security affordance and not a convenience.**
///
/// `href` and `src` are sink 7 of §14G.1.3(c)'s closed list, and a secret
/// reaching one is E-IFC-11 — a working exfiltration, because the value
/// names the host the browser fetches from before anything is painted.
/// The flow pass stops a *secret* getting there. Nothing stops an ordinary
/// value getting there malformed, and a value concatenated into a URL
/// without encoding is exactly that: `q=` plus text containing `&admin=1`
/// is two parameters, not one.
///
/// `queryPart` is the spelling that cannot go wrong, and this asserts it
/// against the concatenation it exists to replace.
#[test]
fn a_query_parameter_carries_its_value_rather_than_extending_the_url() {
    // Concatenation, which is what a program writes without this.
    assert_eq!(
        text("state answer is client Text from \"q=\" + \"a&admin=1\"\n"),
        "q=a&admin=1"
    );
    // The same value, encoded: one parameter, whose contents survive.
    assert_eq!(
        text(
            "state answer is client Text from queryPart with name is \"q\", \
             value is \"a&admin=1\"\n"
        ),
        "q=a%26admin%3D1"
    );
    // The name is encoded too. A parameter name that came from data is
    // no safer than a value that did.
    assert_eq!(
        text(
            "state answer is client Text from queryPart with name is \"a b\", \
             value is \"c d\"\n"
        ),
        "a%20b=c%20d"
    );
    // A whole map, in insertion order, with `&` between and both halves
    // of every entry encoded.
    assert_eq!(
        text(
            "state answer is client Text from queryText of \
             [\"name\" to \"a b\", \"tag\" to \"x&y\"]\n"
        ),
        "name=a%20b&tag=x%26y"
    );
    // Nothing to send is the empty text, not a stray `&`. That is the
    // case a fold which always writes its separator first gets wrong.
    assert_eq!(
        text(
            "state nothing is client Map of Text to Text starting empty\n\
             state answer is client Text from queryText of nothing\n"
        ),
        ""
    );
}

/// JSON string escaping, including the part the language cannot write.
///
/// A backslash and a quote are the two a reader thinks of; the control
/// characters are the ones that make this a primitive, because the lexer's
/// string rule admits no escapes, so no ZDeceptron literal can name them
/// and no `replace` in the language can match one.
#[test]
fn json_encoding_quotes_and_escapes_a_text() {
    let encoded = |expr: &str| {
        text(&format!(
            "state answer is client Text from jsonEncoded of ({expr})\n"
        ))
    };
    // The quotes are part of the answer: this is a JSON *value*, ready to
    // be concatenated into a body, not a fragment needing more punctuation.
    assert_eq!(encoded("\"ab\""), "\"ab\"");
    assert_eq!(encoded("\"\""), "\"\"");
    // A backslash is doubled. A ZDeceptron literal can hold one, because
    // the string rule treats it as an ordinary character rather than an
    // escape.
    assert_eq!(encoded("\"a\\b\""), "\"a\\\\b\"");
    // The line break, which no literal can spell and which `newline`
    // supplies. Unescaped it would end the JSON string.
    assert_eq!(encoded("newline"), "\"\\n\"");
    // Not ASCII, and not escaped either: JSON carries these as themselves.
    assert_eq!(encoded("\"é🎉\""), "\"é🎉\"");
}

/// A quote, which is the character the language cannot put in a literal
/// and therefore the one this has to be driven to reach.
///
/// It is typed into an `Input`, which is also the only route by which text
/// the program did not write reaches it in the first place, and that is
/// precisely the text an encoder exists for.
#[test]
fn json_encoding_escapes_a_quote_that_was_typed_rather_than_written() {
    let bundle = compile_source(
        "state typed is client Text starting \"\"\n\
         view\n    \
         Column\n        \
         Input typed, hint is \"anything\"\n        \
         Text (jsonEncoded of typed)\n",
    );
    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $input = walk($host).find((n) => n.tagName === 'input');\n\
         $input.value = 'a\\\"b';\n\
         $input.fire('input');\n\
         html($host)",
    );
    assert!(
        rendered.contains("<span>\"a\\\"b\"</span>"),
        "a typed quote must come back escaped:\n{rendered}"
    );
}

/// Base64, over the UTF-8 bytes, with the padding the standard alphabet
/// uses. The three lengths modulo three are all asserted, because the
/// padding is where a hand-written encoder goes wrong.
#[test]
fn base64_encodes_the_utf8_bytes_with_padding() {
    let encoded = |value: &str| {
        text(&format!(
            "state answer is client Text from base64Encoded of \"{value}\"\n"
        ))
    };
    assert_eq!(encoded(""), "");
    assert_eq!(encoded("a"), "YQ==");
    assert_eq!(encoded("ab"), "YWI=");
    assert_eq!(encoded("abc"), "YWJj");
    assert_eq!(encoded("abcd"), "YWJjZA==");
    assert_eq!(encoded("hello world"), "aGVsbG8gd29ybGQ=");
    // Bytes again, not code units. `btoa` throws on both of these, which
    // is why this is not `btoa`.
    assert_eq!(encoded("é"), "w6k=");
    assert_eq!(encoded("🎉"), "8J+OiQ==");
    // Every byte value the alphabet has to reach, so a wrong index in the
    // table shows up here rather than in somebody's payload.
    assert_eq!(
        encoded("The quick brown fox jumps over the lazy dog"),
        "VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZw=="
    );
}

// --- the civil calendar --------------------------------------------------
//
// Issue #118's date layer, which was blocked on #15's local bindings and
// on nothing else. Every answer below is checked against the moment the
// platform's own `Date` reports for the same instant, and the arithmetic
// is Howard Hinnant's, so a disagreement here is a defect in `time.zd`
// rather than a difference of opinion about calendars.

/// The civil date of a moment, as `year/month/day`.
fn civil_date(moment: &str) -> String {
    text(&format!(
        "state answer is client Text from \
         (text of (civilDateOf of {moment}).year) + \"/\" + \
         (text of (civilDateOf of {moment}).month) + \"/\" + \
         (text of (civilDateOf of {moment}).day)\n"
    ))
}

/// The time of day of a moment, as `hour:minute:second.millisecond`.
fn civil_time(moment: &str) -> String {
    text(&format!(
        "state answer is client Text from \
         (text of (civilTimeOf of {moment}).hour) + \":\" + \
         (text of (civilTimeOf of {moment}).minute) + \":\" + \
         (text of (civilTimeOf of {moment}).second) + \".\" + \
         (text of (civilTimeOf of {moment}).millisecond)\n"
    ))
}

#[test]
fn a_moment_is_read_apart_into_the_civil_date_it_falls_on() {
    // The epoch itself, which is the one date every other answer is
    // measured from.
    assert_eq!(civil_date("0"), "1970/1/1");
    assert_eq!(civil_date("1717243496000"), "2024/6/1");

    // A leap day in a year divisible by 4, and the century that is a leap
    // year because it is divisible by 400. Both are the cases a rule
    // written as "every fourth year" gets wrong.
    assert_eq!(civil_date("1709164800000"), "2024/2/29");
    assert_eq!(civil_date("951782400000"), "2000/2/29");

    // 2100 is not a leap year, so the day after 2100-02-28 is March.
    assert_eq!(civil_date("4107456000000"), "2100/2/28");
    assert_eq!(civil_date("4107542400000"), "2100/3/1");

    // Before the epoch. The day number is floored rather than truncated,
    // so the last millisecond of 1969 is in 1969 and not in 1970.
    assert_eq!(civil_date("(0 - 1)"), "1969/12/31");
    assert_eq!(civil_date("(0 - 86400000)"), "1969/12/31");
}

#[test]
fn a_moment_is_read_apart_into_the_time_of_day_it_falls_at() {
    assert_eq!(civil_time("0"), "0:0:0.0");
    assert_eq!(civil_time("1717243496000"), "12:4:56.0");
    assert_eq!(civil_time("1717243496789"), "12:4:56.789");

    // The last millisecond of the day before the epoch is 23:59:59.999,
    // not a negative hour. This is the answer JavaScript's own `%` gets
    // wrong and `mod`'s floored remainder gets right.
    assert_eq!(civil_time("(0 - 1)"), "23:59:59.999");
}

#[test]
fn the_weekday_of_a_moment_counts_from_sunday() {
    // 1970-01-01 was a Thursday.
    assert_eq!(
        text("state answer is client Text from text of (weekdayOf of 0)\n"),
        "4"
    );
    // 2024-06-01 was a Saturday.
    assert_eq!(
        text("state answer is client Text from text of (weekdayOf of 1717243496000)\n"),
        "6"
    );
    // And before the epoch it still counts forwards: 1969-12-31 was a
    // Wednesday.
    assert_eq!(
        text("state answer is client Text from text of (weekdayOf of (0 - 1))\n"),
        "3"
    );
}

/// `momentOf` is the exact inverse of `civilDateOf` and `civilTimeOf`
/// together, which is a stronger statement than either half against a
/// table: a shared error in the shift arithmetic would cancel in one
/// direction and this checks both.
#[test]
fn a_moment_and_a_civil_date_round_trip_in_both_directions() {
    // Moment to fields and back.
    let there_and_back = |moment: &str| {
        text(&format!(
            "state answer is client Text from text of (momentOf with \
             date is (civilDateOf of {moment}), time is (civilTimeOf of {moment}))\n"
        ))
    };
    assert_eq!(there_and_back("0"), "0");
    assert_eq!(there_and_back("1717243496789"), "1717243496789");
    assert_eq!(there_and_back("951782400000"), "951782400000");
    assert_eq!(there_and_back("(0 - 1)"), "-1");

    // Fields to moment and back.
    assert_eq!(
        civil_date(
            "(momentOf with date is (CivilDate with year is 2024, month is 6, day is 1), \
             time is (CivilTime with hour is 12, minute is 4, second is 56, \
             millisecond is 0))"
        ),
        "2024/6/1"
    );
}

/// An out-of-range field carries rather than being refused, which is what
/// makes `momentOf` usable for date arithmetic.
#[test]
fn a_month_past_december_is_the_january_after() {
    assert_eq!(
        civil_date(
            "(momentOf with date is (CivilDate with year is 2024, month is 13, day is 1), \
             time is (CivilTime with hour is 0, minute is 0, second is 0, \
             millisecond is 0))"
        ),
        "2025/1/1"
    );
    // And a 32nd of January is the 1st of February.
    assert_eq!(
        civil_date(
            "(momentOf with date is (CivilDate with year is 2024, month is 1, day is 32), \
             time is (CivilTime with hour is 0, minute is 0, second is 0, \
             millisecond is 0))"
        ),
        "2024/2/1"
    );
}

/// **The number issue #15 exists for, counted at run time.**
///
/// §17.7's complaint was not that a date could not be computed but that it
/// cost about a hundred `floor` calls to compute one, because without a
/// place to name an intermediate each of the algorithm's nine became a
/// top-level function recomputing all of its predecessors. With bindings
/// it costs ten: Hinnant's nine, plus the division that turns
/// milliseconds into a day number.
///
/// Counted rather than read off the source, by replacing the `$floor`
/// helper the bundle ships with one that tallies its calls. A static count
/// of `$floor(` would have been a count of *call sites*, and the claim is
/// about calls.
#[test]
fn one_civil_date_costs_ten_floor_calls() {
    let bundle = compile_source(
        "state answer is client Text from text of (civilDateOf of 1717243496000).year\n\
         view\n\
         \x20   Text answer\n",
    );
    let module = support::flatten(&bundle.client_js);
    let counted = module.replace(
        "const $floor = (n) =>",
        "globalThis.$floors = 0;\n\
         const $floor = (n) => { globalThis.$floors += 1; return $floorBody(n); };\n\
         const $floorBody = (n) =>",
    );
    assert_ne!(
        counted, module,
        "the bundle must ship the `$floor` helper for this to count anything"
    );

    let mut context = context(false);
    let answer = run(
        &mut context,
        &counted,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    assert!(
        answer.contains("2024"),
        "the count is only worth having if the answer is right: {answer}"
    );

    let calls = run(&mut context, "", "String(globalThis.$floors)");
    assert_eq!(
        calls, "10",
        "one civil date is ten `floor` calls, and §17.7 measured about a hundred"
    );
}
