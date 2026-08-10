use std::collections::HashSet;

use zdc_lexer::Span;
use zdc_lsp::{
    encode, highlights, Analysis, Highlight, IsRole, LineIndex, Position, SymbolKind,
    TOKEN_MODIFIERS, TOKEN_TYPES,
};

#[test]
fn semantic_token_legend_is_stable_and_unique() {
    assert_eq!(
        TOKEN_TYPES,
        [
            "keyword",
            "operator",
            "type",
            "class",
            "enumMember",
            "variable",
            "function",
            "method",
            "parameter",
            "property",
            "string",
            "number",
        ]
    );
    assert_eq!(
        TOKEN_MODIFIERS,
        [
            "declaration",
            "readonly",
            "defaultLibrary",
            "client",
            "static",
            "server",
            "durable",
            "remembered",
        ]
    );
    assert_eq!(
        TOKEN_TYPES.iter().copied().collect::<HashSet<_>>().len(),
        TOKEN_TYPES.len()
    );
    assert_eq!(
        TOKEN_MODIFIERS
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len(),
        TOKEN_MODIFIERS.len()
    );
}

#[test]
fn semantic_tokens_use_protocol_delta_encoding() {
    let input = [
        Highlight {
            line: 0,
            start: 2,
            length: 4,
            token_type: 5,
            modifiers: 1,
        },
        Highlight {
            line: 0,
            start: 9,
            length: 3,
            token_type: 6,
            modifiers: 0,
        },
        Highlight {
            line: 3,
            start: 4,
            length: 2,
            token_type: 10,
            modifiers: 8,
        },
    ];

    assert_eq!(
        encode(&input),
        [0, 2, 4, 5, 1, 0, 7, 3, 6, 0, 3, 4, 2, 10, 8]
    );
    assert!(encode(&[]).is_empty());
}

#[test]
fn line_index_counts_terminal_newlines_as_empty_lines() {
    for (text, lines) in [("", 1), ("one", 1), ("one\n", 2), ("one\n\n", 3)] {
        assert_eq!(LineIndex::new(text).line_count(), lines, "text {text:?}");
    }
}

#[test]
fn line_index_clamps_stale_spans_past_the_document_end() {
    let text = "first\n😀 second";
    let lines = LineIndex::new(text);
    let (start, end) = lines.range(text, Span::new(u32::MAX - 1, u32::MAX));

    assert_eq!(start, lines.position(text, text.len() as u32));
    assert_eq!(end, start);
}

#[test]
fn utf16_positions_round_trip_at_every_character_boundary() {
    let text = "a😀é\n中文\n";
    let lines = LineIndex::new(text);
    for (offset, _) in text.char_indices() {
        let position = lines.position(text, offset as u32);
        assert_eq!(lines.offset(text, position), offset as u32);
    }
    assert_eq!(
        lines.offset(
            text,
            Position {
                line: u32::MAX,
                character: u32::MAX,
            }
        ),
        text.len() as u32
    );
}

#[test]
fn symbol_lookup_includes_the_caret_immediately_after_a_name() {
    let source = "state count is client Whole starting 0\nview\n    Text count\n";
    let analysis = Analysis::of(source);
    let start = source.rfind("count").expect("the view reference") as u32;
    let end = start + "count".len() as u32;

    assert_eq!(
        analysis.symbols().at(start).map(|s| s.name.as_str()),
        Some("count")
    );
    assert_eq!(
        analysis.symbols().at(end).map(|s| s.name.as_str()),
        Some("count")
    );
    assert!(analysis.symbols().at(u32::MAX).is_none());
}

#[test]
fn the_three_public_is_roles_are_distinguished_in_one_program() {
    let source = r#"state left is client Whole starting 1
function same with right
    if left is right
        give right
    give left
state answer is client Whole from same with right is left
view
    Text answer
"#;
    let analysis = Analysis::of(source);
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );

    let roles = analysis
        .symbols()
        .iter()
        .filter_map(|symbol| match symbol.kind {
            SymbolKind::Is(role) => Some(role),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(roles.contains(&IsRole::Declaration));
    assert!(roles.contains(&IsRole::NamedArgument));
    assert!(roles.contains(&IsRole::Equality));
}

#[test]
fn every_highlight_fits_the_declared_protocol_legend() {
    let source = r#"state typed is client Text starting ""
state saved is durable Text starting ""
function copy with value
    give value
view
    Input typed, hint is "edit"
"#;
    let analysis = Analysis::of(source);
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );

    let tokens = highlights(&analysis);
    assert!(!tokens.is_empty());
    assert!(tokens
        .windows(2)
        .all(|pair| { (pair[0].line, pair[0].start) <= (pair[1].line, pair[1].start) }));
    assert!(tokens.iter().all(|token| {
        token.length > 0
            && token.token_type < TOKEN_TYPES.len() as u32
            && token.modifiers < (1 << TOKEN_MODIFIERS.len())
    }));
}
