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
