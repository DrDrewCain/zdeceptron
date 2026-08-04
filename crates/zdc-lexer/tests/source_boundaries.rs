use zdc_lexer::{tokenize, Span, TokenKind};

#[test]
fn token_spans_quote_the_original_utf8_source() {
    let src = "state café is client Whole starting 12.5";
    let tokens = tokenize(src).expect("source lexes");

    let quoted = tokens
        .iter()
        .filter(|token| !matches!(token.kind, TokenKind::Newline | TokenKind::Eof))
        .map(|token| {
            let range: std::ops::Range<usize> = token.span.into();
            (token.kind.clone(), &src[range])
        })
        .collect::<Vec<_>>();

    assert_eq!(
        quoted,
        vec![
            (TokenKind::State, "state"),
            (TokenKind::Ident("café".into()), "café"),
            (TokenKind::Is, "is"),
            (TokenKind::Client, "client"),
            (TokenKind::Ident("Whole".into()), "Whole"),
            (TokenKind::Starting, "starting"),
            (TokenKind::Number(12.5), "12.5"),
        ]
    );
}

#[test]
fn combined_is_not_span_includes_the_whole_phrase() {
    let src = "a is    not b";
    let tokens = tokenize(src).expect("source lexes");
    let operator = tokens
        .iter()
        .find(|token| token.kind == TokenKind::IsNot)
        .expect("combined operator");
    let range: std::ops::Range<usize> = operator.span.into();

    assert_eq!(&src[range], "is    not");
}

#[test]
fn invalid_unicode_character_span_uses_byte_offsets() {
    let src = "view\n    😀";
    let err = tokenize(src).unwrap_err();
    let range: std::ops::Range<usize> = err.span.into();

    assert_eq!(&src[range], "😀");
    assert_eq!(err.span, Span::new(9, 13));
    assert!(err.message.contains("😀"), "got: {}", err.message);
    assert!(
        err.message.contains("is not valid ZDeceptron"),
        "got: {}",
        err.message
    );
}

#[test]
fn empty_and_comment_only_sources_emit_only_eof() {
    for src in ["", "# nothing here"] {
        let kinds = tokenize(src)
            .expect("source lexes")
            .into_iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec![TokenKind::Eof], "source: {src:?}");
    }
}

/// **A word long enough to abort the process must be a diagnostic.**
///
/// `logos` compiles the one Unicode-class rule here into mutually
/// recursive state functions whose tail calls the optimiser turns into
/// jumps. Unoptimised it does not, and the scan spends about a kilobyte
/// of stack per character: 8 000 `z` raised `SIGABRT`, which is not a
/// panic, cannot be caught, prints nothing, and takes `zdc lsp` with it.
/// The compiler's totality must not rest on `-O`.
#[test]
fn a_word_longer_than_the_limit_is_reported_not_fatal() {
    let src = format!("{}\n", "z".repeat(200_000));
    let err = tokenize(&src).expect_err("an over-long word must be rejected");
    assert!(err.message.contains("200000"), "got: {}", err.message);
    assert!(
        err.message.contains("without a break"),
        "got: {}",
        err.message
    );
    assert_eq!(
        err.span,
        Span::new(0, 200_000),
        "the span must cover the whole run, not the character that crossed the limit"
    );
}

/// Combining marks are `XID_Continue` too, so a run of them reaches the
/// same scan by a different door.
#[test]
fn a_long_run_of_combining_marks_is_reported_not_fatal() {
    let src = format!("a{}\n", "\u{301}".repeat(50_000));
    tokenize(&src).expect_err("an over-long run of marks must be rejected");
}

/// The limit is on what the `Word` rule scans. A long *string* and a long
/// *comment* never reach it — both are matched by ASCII-only rules that
/// compile to loops — so neither may be caught by the guard.
#[test]
fn long_strings_and_comments_are_unaffected() {
    let text = format!(
        "state a is client Text starting \"{}\"\n",
        "z".repeat(100_000)
    );
    tokenize(&text).expect("a long string literal must still lex");

    let comment = format!("# {}\n", "z".repeat(100_000));
    tokenize(&comment).expect("a long comment must still lex");
}

/// The guard counts a run, not a line, so ordinary source with many
/// tokens on one line is untouched however long the line is.
#[test]
fn a_long_line_of_ordinary_tokens_is_unaffected() {
    let src = format!(
        "state a is client Whole starting {}1\n",
        "1 + ".repeat(20_000)
    );
    tokenize(&src).expect("a long line of short tokens must still lex");
}
