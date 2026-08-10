//! The parser reports every syntax error, and invents none.
//!
//! Two properties, and the second is the one that is easy to lose. A
//! parser that resumes anywhere will report a second, third and fourth
//! error about the wreckage of the first, and a report where three of four
//! diagnostics are fiction is worse than the single true one it replaced —
//! the reader now has to decide which to believe. So every test here comes
//! in a pair: a file with two independent mistakes reports two, and a file
//! with one mistake reports exactly one, however much text follows it.
//!
//! The recovery point is the start of the next top-level declaration and
//! nothing else, which is why the "exactly one" direction holds: there is
//! no partial construct for the parser to carry on guessing about.

use zdc_parser::parse_all;

/// The errors a source produces, as `(code, quoted source)` pairs.
///
/// The span is resolved back to the text it covers so an assertion says
/// *where* rather than at which byte, and so it fails legibly when a span
/// moves.
fn errors(source: &str) -> Vec<(&'static str, String)> {
    match parse_all(source) {
        Ok(program) => panic!(
            "this source was supposed to fail and produced {} declarations",
            program.decls.len()
        ),
        Err(errors) => errors
            .into_iter()
            .map(|error| {
                let at = &source[error.span.start as usize..error.span.end as usize];
                (error.code, at.to_string())
            })
            .collect(),
    }
}

/// **Two independent mistakes are both reported.** This is the issue.
#[test]
fn two_declarations_with_a_mistake_each_produce_two_errors() {
    let found = errors(concat!(
        "state votes is Map of Id to Int starting empty\n",
        "state names is List of Text starting empty\n",
        "\n",
        "view\n",
        "    Column\n",
        "        Text \"hi\"\n",
    ));

    assert_eq!(
        found,
        vec![("E0101", "Map".to_string()), ("E0101", "List".to_string()),],
        "both placements are missing, and both should be reported"
    );
}

/// **One mistake stays one mistake.** The same file with the second
/// declaration written correctly reports one error, not one plus whatever
/// the recovery made of the rest.
#[test]
fn one_mistake_is_reported_once_however_much_follows_it() {
    let found = errors(concat!(
        "state votes is Map of Id to Int starting empty\n",
        "state names is client List of Text starting empty\n",
        "\n",
        "record Edge\n",
        "    left is Whole\n",
        "    right is Whole\n",
        "\n",
        "function double with n\n",
        "    give n + n\n",
        "\n",
        "view\n",
        "    Column\n",
        "        Text \"hi\"\n",
    ));

    assert_eq!(found, vec![("E0101", "Map".to_string())]);
}

/// A mistake inside a deeply nested view is still one mistake. The parser
/// is several blocks down when it fails, and everything between there and
/// the next declaration is skipped rather than reinterpreted.
#[test]
fn a_mistake_inside_a_nested_block_does_not_cascade_out_of_it() {
    let found = errors(concat!(
        "view\n",
        "    Column\n",
        "        Row\n",
        "            Text (1 + 2\n",
        "            Text \"after\"\n",
        "            Text \"and after that\"\n",
    ));

    assert_eq!(found.len(), 1, "one unclosed bracket, one error: {found:?}");
    assert_eq!(found[0].0, "E0103");
}

/// The same file with a declaration after the broken block: the error
/// inside the view is reported, the declaration after it is parsed, and
/// nothing is reported about the lines in between.
#[test]
fn recovery_resumes_at_the_next_declaration_and_not_before_it() {
    let found = errors(concat!(
        "view\n",
        "    Column\n",
        "        Text (1 + 2\n",
        "        Text \"skipped\"\n",
        "\n",
        "state total is Whole starting 1\n",
    ));

    // The codes rather than the quoted text for the first: an unclosed
    // bracket is reported where the closing one should have gone, which is
    // the line break, and quoting a line break in an assertion says less
    // than naming the rule does.
    let codes: Vec<&str> = found.iter().map(|(code, _)| *code).collect();
    assert_eq!(
        codes,
        vec!["E0103", "E0101"],
        "the unclosed bracket and the missing placement, and nothing else: {found:?}"
    );
    assert_eq!(
        found[1].1, "Whole",
        "the second error is the declaration after the broken block"
    );
}

/// Three mistakes, three errors, in source order. Counted rather than
/// sampled, because "reports more than one" would also be true of a parser
/// that reported one real error and one invented one.
#[test]
fn every_broken_declaration_is_reported_in_source_order() {
    let source = concat!(
        "state a is Whole starting 1\n",
        "state b is Whole starting 2\n",
        "state c is Whole starting 3\n",
        "\n",
        "view\n",
        "    Column\n",
        "        Text \"x\"\n",
    );
    let found = errors(source);

    assert_eq!(found.len(), 3, "{found:?}");
    let mut previous = 0;
    for error in parse_all(source).expect_err("the source does not parse") {
        assert!(
            error.span.start >= previous,
            "errors must be reported in source order"
        );
        previous = error.span.start;
    }
}

/// Text that cannot begin a declaration at all is one error, and the
/// declarations after it are still parsed.
#[test]
fn a_line_that_cannot_begin_a_declaration_is_reported_once() {
    let found = errors(concat!(
        "5\n",
        "\n",
        "state total is Whole starting 1\n",
        "\n",
        "view\n",
        "    Column\n",
        "        Text \"x\"\n",
    ));

    assert_eq!(
        found,
        vec![("E0104", "5".to_string()), ("E0101", "Whole".to_string())]
    );
}

/// A file whose last declaration is the broken one terminates. A recovery
/// loop that did not guarantee progress would spin here rather than fail,
/// which is a worse failure than the one it was fixing.
#[test]
fn a_mistake_in_the_last_declaration_terminates() {
    let found = errors("view\n    Column\n        Text \"hi\"\n\nstate a is Whole starting 1\n");

    assert_eq!(found, vec![("E0101", "Whole".to_string())]);
}

/// A file with no mistakes still parses, and every declaration survives
/// the new loop.
#[test]
fn a_file_with_no_mistakes_still_produces_every_declaration() {
    let program = parse_all(concat!(
        "state a is client Whole starting 1\n",
        "\n",
        "function double with n\n",
        "    give n + n\n",
        "\n",
        "view\n",
        "    Column\n",
        "        Text a\n",
    ))
    .expect("this source parses");

    assert_eq!(program.decls.len(), 3);
}

/// A lexical error is still reported alone. The layout pass turns
/// indentation into tokens, so a file the lexer refused has no token
/// stream to resynchronise within — every span after the refusal would be
/// invented.
#[test]
fn a_lexical_error_is_reported_alone() {
    let found = errors("state a is client Whole starting 1\nstate b is $ starting 2\n");

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "E0103");
}

/// `parse` is `parse_all`'s first error, unchanged. Every caller that
/// wants one error still gets the one it used to get.
#[test]
fn the_single_error_entry_point_returns_the_first_of_the_list() {
    let source = concat!(
        "state votes is Map of Id to Int starting empty\n",
        "state names is List of Text starting empty\n",
    );

    let first = zdc_parser::parse(source).expect_err("the source does not parse");
    let all = parse_all(source).expect_err("the source does not parse");

    assert_eq!(all.len(), 2);
    assert_eq!(first.message, all[0].message);
    assert_eq!(first.span, all[0].span);
    assert_eq!(first.code, all[0].code);
}
