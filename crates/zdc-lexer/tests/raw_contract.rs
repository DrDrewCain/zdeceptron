use zdc_lexer::raw::{over_long_run, tokenize_raw, RawToken, MAX_TOKEN_CHARS};
use zdc_lexer::{Span, TokenKind};

fn kinds(source: &str) -> Vec<RawToken> {
    tokenize_raw(source)
        .into_iter()
        .map(|(token, _)| token)
        .collect()
}

#[test]
fn raw_tokens_preserve_newline_indentation_without_layout_tokens() {
    assert_eq!(
        kinds("view\n    Column\n  Text \"x\""),
        [
            RawToken::Kw(TokenKind::View),
            RawToken::LineStart(4),
            RawToken::Kw(TokenKind::Ident("Column".into())),
            RawToken::LineStart(2),
            RawToken::Kw(TokenKind::Ident("Text".into())),
            RawToken::Kw(TokenKind::Text("x".into())),
        ]
    );
}

#[test]
fn raw_token_spans_quote_every_non_skipped_source_fragment() {
    let source = "a <= b # note\n  c";
    let quoted = tokenize_raw(source)
        .into_iter()
        .map(|(token, span)| {
            let range: std::ops::Range<usize> = span.into();
            (token, &source[range])
        })
        .collect::<Vec<_>>();

    assert_eq!(
        quoted,
        [
            (RawToken::Kw(TokenKind::Ident("a".into())), "a"),
            (RawToken::Kw(TokenKind::LessEq), "<="),
            (RawToken::Kw(TokenKind::Ident("b".into())), "b"),
            (RawToken::LineStart(2), "\n  "),
            (RawToken::Kw(TokenKind::Ident("c".into())), "c"),
        ]
    );
}

#[test]
fn is_not_combines_only_adjacent_raw_tokens_and_preserves_the_gap() {
    let source = "a is   not b\nis # comment\nnot";
    let tokens = tokenize_raw(source);
    assert_eq!(tokens[1].0, RawToken::Kw(TokenKind::IsNot));
    let range: std::ops::Range<usize> = tokens[1].1.into();
    assert_eq!(&source[range], "is   not");
    assert_eq!(tokens[4].0, RawToken::Kw(TokenKind::Is));
    assert_eq!(tokens[6].0, RawToken::Kw(TokenKind::Not));
}

#[test]
fn first_is_contextual_at_the_raw_token_boundary() {
    assert_eq!(
        kinds("first take first take # comment\n first"),
        [
            RawToken::Kw(TokenKind::Ident("first".into())),
            RawToken::Kw(TokenKind::Take),
            RawToken::Kw(TokenKind::First),
            RawToken::Kw(TokenKind::Take),
            RawToken::LineStart(1),
            RawToken::Kw(TokenKind::Ident("first".into())),
        ]
    );
}

#[test]
fn invalid_characters_are_raw_errors_with_exact_byte_spans() {
    let source = "é\t😀";
    let tokens = tokenize_raw(source);
    assert_eq!(tokens[0].0, RawToken::Kw(TokenKind::Ident("é".into())));
    assert_eq!(tokens[1], (RawToken::Error, Span::new(2, 3)));
    assert_eq!(tokens[2], (RawToken::Error, Span::new(3, 7)));
}

#[test]
fn token_run_limit_accepts_the_boundary_and_rejects_the_next_character() {
    let at_limit = "z".repeat(MAX_TOKEN_CHARS);
    assert_eq!(over_long_run(&at_limit), None);

    let over_limit = "é".repeat(MAX_TOKEN_CHARS + 1);
    assert_eq!(
        over_long_run(&over_limit),
        Some((
            Span::new(0, (2 * (MAX_TOKEN_CHARS + 1)) as u32),
            MAX_TOKEN_CHARS + 1,
        ))
    );
}

#[test]
fn token_run_limit_reports_only_the_first_overlong_run() {
    let source = format!(
        "ok {} {}",
        "a".repeat(MAX_TOKEN_CHARS + 2),
        "b".repeat(MAX_TOKEN_CHARS + 3)
    );
    assert_eq!(
        over_long_run(&source),
        Some((
            Span::new(3, (3 + MAX_TOKEN_CHARS + 2) as u32),
            MAX_TOKEN_CHARS + 2
        ))
    );
}

#[test]
fn long_block_literal_lines_do_not_count_as_identifier_runs() {
    let body = "z".repeat(MAX_TOKEN_CHARS * 2);
    let source = format!("\"\"\"\n{body}\n\"\"\"");

    assert_eq!(over_long_run(&source), None);
    assert_eq!(
        kinds(&source),
        [RawToken::Kw(TokenKind::Text(body))],
        "the raw lexer must still receive the complete block literal"
    );
}

#[test]
fn long_comments_and_single_line_strings_do_not_count_as_runs() {
    let long = "z".repeat(MAX_TOKEN_CHARS * 2);
    for source in [format!("# {long}"), format!("\"{long}\"")] {
        assert_eq!(over_long_run(&source), None);
    }
}
