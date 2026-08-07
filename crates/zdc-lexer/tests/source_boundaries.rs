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

// --- a whole-number literal is the value in the file, or it is refused ---

fn refusal(src: &str) -> String {
    tokenize(src)
        .expect_err("this literal must be refused")
        .message
}

/// **The acceptance criterion for #183.** `9007199254740993` is 2^53 + 1,
/// and a build used to emit `9007199254740992`. The compiler has the
/// digits in hand at the moment it decides, so refusing costs nothing and
/// happens here.
#[test]
fn a_whole_literal_that_is_not_exactly_representable_is_refused() {
    let message = refusal("state n is client Whole starting 9007199254740993\n");
    assert!(message.contains("9007199254740993"), "{message}");
    assert!(
        message.contains("9007199254740992"),
        "the message must name the nearest value that is held exactly: {message}"
    );

    let message = refusal("state n is client Whole starting 99999999999999999999999999\n");
    assert!(message.contains("99999999999999999999999999"), "{message}");
    assert!(
        message.contains("100000000000000000000000000"),
        "the message must name the nearest value that is held exactly: {message}"
    );
}

/// The refusal is at the literal, not at the line or the declaration.
#[test]
fn the_refused_literal_is_the_span_of_the_literal() {
    let src = "state n is client Whole starting 9007199254740993\n";
    let error = tokenize(src).expect_err("this literal must be refused");
    let at = src.find("9007").expect("the literal is in the source");
    assert_eq!(
        error.span,
        Span::new(at as u32, (at + "9007199254740993".len()) as u32)
    );
}

/// Every whole number that *is* held exactly still lexes, including the
/// largest one and the constants the prelude is written with. A rule that
/// refused these would be a different and worse bug.
#[test]
fn a_whole_literal_that_is_exactly_representable_still_lexes() {
    for literal in [
        "0",
        "007",
        "42",
        "86400000",
        "4294967295",
        "4294967296",
        "9007199254740991",
        "9007199254740992",
        // Large, and exact: 10^22 is a power of two times a power of five.
        "10000000000000000000000",
    ] {
        let src = format!("state n is client Whole starting {literal}\n");
        tokenize(&src).unwrap_or_else(|e| panic!("`{literal}` must lex: {}", e.message));
    }
}

/// An unknown escape says which four exist, because a reader who wrote
/// `\r` needs to know what to write instead and not only that this is
/// wrong (§7.3).
#[test]
fn an_unknown_escape_names_the_escapes_that_exist() {
    let message = refusal("state s is client Text starting \"a\\rb\"\n");
    for escape in ["\\n", "\\t", "\\\"", "\\\\"] {
        assert!(
            message.contains(escape),
            "the message must name `{escape}`: {message}"
        );
    }
}

/// **The `Decimal` decision, made deliberately and separately.** A
/// literal written with a fractional part is a `Decimal`, and a `Decimal`
/// is an f64: `0.1234567890123456789` becomes `0.12345678901234568` and
/// the language inherits that rather than pretending otherwise. Refusing
/// it would mean refusing `0.1`, which is not exactly representable
/// either and which every program writes.
#[test]
fn a_decimal_literal_inherits_f64_rounding_rather_than_being_refused() {
    for literal in ["0.1", "0.1234567890123456789", "3.141592653589793238462643"] {
        let src = format!("state n is client Decimal starting {literal}\n");
        tokenize(&src).unwrap_or_else(|e| panic!("`{literal}` must lex: {}", e.message));
    }
}
