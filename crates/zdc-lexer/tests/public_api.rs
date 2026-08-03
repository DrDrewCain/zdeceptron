use zdc_lexer::{
    tokenize, word_to_soft_keyword, word_to_type_ctor, SoftKeyword, TokenKind, TypeCtor,
};

fn assert_span_is_in(source: &str, span: zdc_lexer::Span) {
    assert!(span.start <= span.end, "backwards span: {span:?}");
    assert!(
        span.end as usize <= source.len(),
        "span {span:?} escapes {} source bytes",
        source.len()
    );
    assert!(source.is_char_boundary(span.start as usize));
    assert!(source.is_char_boundary(span.end as usize));
}

#[test]
fn successful_token_streams_have_ordered_bounded_spans_and_one_eof() {
    let source = "state café is client Text starting \"hello 😀\"\n\
                  view\n    Column\n        Text café\n";
    let tokens = tokenize(source).expect("the fixture lexes");

    for token in &tokens {
        assert_span_is_in(source, token.span);
    }
    assert!(
        tokens
            .windows(2)
            .all(|pair| pair[0].span.start <= pair[1].span.start),
        "token starts must remain in source order: {tokens:#?}"
    );
    assert_eq!(
        tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Eof)
            .count(),
        1
    );
    assert_eq!(
        tokens.last().map(|token| &token.kind),
        Some(&TokenKind::Eof)
    );
}

#[test]
fn layout_tokens_are_balanced_before_eof() {
    let source = "view\n    Column\n        Row\n            Text \"x\"\n    Text \"done\"\n";
    let tokens = tokenize(source).expect("the fixture lexes");
    let mut depth = 0usize;
    // The fixture opens three levels and closes them, so the balance below
    // is asserted over a stream that really contains layout tokens: an
    // empty one is balanced too, and would pass while lexing nothing.
    let indents = tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::Indent))
        .count();
    assert_eq!(indents, 3, "the fixture should open three layout levels");

    for token in &tokens {
        match token.kind {
            TokenKind::Indent => depth += 1,
            TokenKind::Dedent => {
                assert!(depth > 0, "a dedent appeared without an open indent");
                depth -= 1;
            }
            TokenKind::Eof => assert_eq!(depth, 0, "EOF must close every layout level"),
            _ => {}
        }
    }
}

#[test]
fn recently_added_reserved_words_round_trip_through_diagnostics() {
    let cases = [
        ("trusted", TokenKind::Trusted),
        ("release", TokenKind::Release),
        ("limit", TokenKind::Limit),
        ("route", TokenKind::Route),
        ("static", TokenKind::Static),
        ("emitting", TokenKind::Emitting),
        ("contains", TokenKind::Contains),
        ("address", TokenKind::Address),
        ("build", TokenKind::Build),
    ];

    for (word, expected) in cases {
        let tokens = tokenize(word).expect("one word lexes");
        assert_eq!(tokens[0].kind, expected);
        assert_eq!(tokens[0].kind.keyword_spelling(), Some(word));
    }
}

#[test]
fn soft_keywords_remain_ordinary_identifiers_until_the_parser_needs_them() {
    let cases = [
        ("foreign", SoftKeyword::Foreign),
        ("as", SoftKeyword::As),
        ("takes", SoftKeyword::Takes),
        ("gives", SoftKeyword::Gives),
        ("anywhere", SoftKeyword::Anywhere),
        ("pure", SoftKeyword::Pure),
        ("per", SoftKeyword::Per),
        ("visitor", SoftKeyword::Visitor),
    ];

    for (word, expected) in cases {
        let tokens = tokenize(word).expect("one word lexes");
        assert_eq!(tokens[0].kind, TokenKind::Ident(word.to_string()));
        assert_eq!(word_to_soft_keyword(word), Some(expected));
        assert_eq!(expected.spelling(), word);
    }
    assert_eq!(word_to_soft_keyword("ordinary"), None);
}

#[test]
fn type_constructors_are_contextual_words_not_reserved_tokens() {
    let cases = [
        ("List", TypeCtor::List),
        ("Option", TypeCtor::Option),
        ("Remote", TypeCtor::Remote),
        ("Map", TypeCtor::Map),
    ];

    for (word, expected) in cases {
        let tokens = tokenize(word).expect("one word lexes");
        assert_eq!(tokens[0].kind, TokenKind::Ident(word.to_string()));
        assert_eq!(word_to_type_ctor(word), Some(expected));
    }
    assert_eq!(word_to_type_ctor("Customer"), None);
}

#[test]
fn every_edit_prefix_returns_only_spans_inside_that_revision() {
    let target = "# 😀\nstate name is client Text starting \"hello\"\nview\n    Text name\n";
    let mut boundaries: Vec<usize> = target.char_indices().map(|(at, _)| at).collect();
    boundaries.push(target.len());

    for end in boundaries {
        let source = &target[..end];
        match tokenize(source) {
            Ok(tokens) => {
                for token in tokens {
                    assert_span_is_in(source, token.span);
                }
            }
            Err(error) => assert_span_is_in(source, error.span),
        }
    }
}

#[test]
fn invisible_invalid_characters_are_escaped_or_named_in_diagnostics() {
    for (source, forbidden) in [
        ("\t", '\t'),
        ("\r", '\r'),
        ("\u{feff}", '\u{feff}'),
        ("\u{a0}", '\u{a0}'),
        ("\u{202e}", '\u{202e}'),
    ] {
        let error = tokenize(source).expect_err("the character must be refused");
        assert_span_is_in(source, error.span);
        assert!(!error.message.contains(forbidden), "{:?}", error.message);
        assert!(!error.message.is_empty());
    }
}
