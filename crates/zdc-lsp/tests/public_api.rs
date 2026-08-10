use zdc_hir::Res;
use zdc_lsp::{
    actions, callable_at, complete, declarations, definition, document_declarations, encode, folds,
    highlights, highlights_within, hints, hover, incoming, outgoing, references, signature,
    type_definition, Analysis, CompletionKind, DeclarationKind, LineIndex, Position, SymbolKind,
    TOKEN_MODIFIERS, TOKEN_TYPES,
};

const COUNTER: &str = "state count is client Whole starting 0\n\
                       state doubled is client Whole from count + count\n\
                       view\n    Text doubled\n";

#[test]
fn a_valid_analysis_exposes_every_successful_compiler_stage() {
    let analysis = Analysis::of(COUNTER);

    assert_eq!(analysis.text(), COUNTER);
    assert!(analysis.diagnostics().is_empty());
    assert!(!analysis.tokens().is_empty());
    assert!(!analysis.symbols().iter().collect::<Vec<_>>().is_empty());
    assert!(analysis.hir().is_some());
    assert!(analysis.types().is_some());
}

#[test]
fn an_incomplete_file_keeps_lexical_editor_features_alive() {
    let analysis = Analysis::of("state count is client Whole starting ");

    assert_eq!(analysis.diagnostics().len(), 1);
    assert!(analysis.hir().is_none());
    assert!(analysis.types().is_none());
    assert!(!analysis.tokens().is_empty());

    let highlighted = highlights(&analysis);
    assert!(!highlighted.is_empty());
    assert_eq!(encode(&highlighted).len(), highlighted.len() * 5);
}

#[test]
fn definition_hover_and_symbols_agree_on_a_resolved_reference() {
    let analysis = Analysis::of(COUNTER);
    let use_offset = COUNTER.rfind("doubled").expect("the view reference") as u32;
    let declaration_offset = COUNTER.find("doubled").expect("the declaration") as u32;

    let symbol = analysis
        .symbols()
        .at(use_offset)
        .expect("a symbol at the reference");
    assert_eq!(symbol.name, "doubled");
    assert!(matches!(
        symbol.kind,
        SymbolKind::Use {
            res: Some(Res::Def(_)),
            ..
        }
    ));

    let target = definition(&analysis, use_offset).expect("a definition target");
    assert_eq!(target.start, declaration_offset);

    let (hovered, markdown) = hover(&analysis, use_offset).expect("hover information");
    assert_eq!(hovered, symbol.span);
    assert!(markdown.contains("Whole"), "{markdown}");
    assert!(markdown.contains("client"), "{markdown}");
}

#[test]
fn completion_works_before_the_source_can_parse() {
    let source = "state count is ";
    let analysis = Analysis::of(source);
    let items = complete(&analysis, source.len() as u32);

    let placements: Vec<&str> = items
        .iter()
        .filter(|item| item.kind == CompletionKind::Placement)
        .map(|item| item.label.as_str())
        .collect();
    // Every placement the language has, in `Placement::ALL`'s order, so a
    // fifth one that the completion engine forgets is a failure here
    // rather than a silently short list. This asserted three placements
    // and went on passing after `static` became the fourth.
    let declared: Vec<&str> = zdc_ast::Placement::ALL
        .iter()
        .map(|placement| zdc_types::SignalPlacement::from_ast(*placement).describe())
        .collect();
    assert_eq!(declared.len(), 5, "the placement list shrank");
    assert_eq!(placements, declared);
    assert!(items.iter().all(|item| !item.detail.is_empty()));
}

#[test]
fn line_positions_use_utf16_and_clamp_stale_editor_coordinates() {
    let text = "# 😀 éx\nstate count";
    let lines = LineIndex::new(text);
    let x = text.find('x').expect("the marker") as u32;

    assert_eq!(
        lines.position(text, x),
        Position {
            line: 0,
            character: 6,
        }
    );
    assert_eq!(
        lines.offset(
            text,
            Position {
                line: 0,
                character: 6,
            }
        ),
        x
    );
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
fn every_edit_prefix_is_safe_across_the_public_feature_surface() {
    let target = "# 😀\nstate count is client Whole starting 0\nview\n    Text count\n";
    let mut boundaries: Vec<usize> = target.char_indices().map(|(at, _)| at).collect();
    boundaries.push(target.len());

    for end in boundaries {
        let source = &target[..end];
        let analysis = Analysis::of(source);
        let offsets = [0, source.len() as u32, u32::MAX];

        for offset in offsets {
            let _ = complete(&analysis, offset);
            let _ = definition(&analysis, offset);
            let _ = type_definition(&analysis, offset);
            let _ = hover(&analysis, offset);
            let _ = references(&analysis, offset);
            let _ = signature(&analysis, offset);
            let _ = callable_at(&analysis, offset);
            let _ = analysis.symbols().at(offset);
        }
        let _ = folds(&analysis);
        let _ = declarations(&analysis);
        let _ = hints(&analysis, 0, u32::MAX);
        let _ = actions(&analysis, zdc_lexer::Span::new(0, source.len() as u32));

        let highlighted = highlights(&analysis);
        assert_eq!(encode(&highlighted).len(), highlighted.len() * 5);
        assert!(highlighted.iter().all(|item| {
            (item.token_type as usize) < TOKEN_TYPES.len()
                && item.modifiers < (1 << TOKEN_MODIFIERS.len())
        }));
    }
}

/// The features added for the editor surface all read the same symbol
/// index, and the point of that is that they cannot disagree. This is the
/// one program where every one of them has something to say, so a change
/// that made two of them answer about different declarations fails here.
#[test]
fn the_editor_features_agree_about_one_program() {
    let source = "function twice with n\n    give n + n\n\
                  state doubled is client Whole from twice with 2\n\
                  view\n    Text doubled\n";
    let analysis = Analysis::of(source);
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );

    let call = source.find("twice with 2").expect("the call") as u32;
    let declared = source.find("twice with n").expect("the declaration") as u32;

    // Go-to-definition, find-references and the call hierarchy all agree
    // that the call names the declaration.
    assert_eq!(
        definition(&analysis, call).map(|span| span.start),
        Some(declared)
    );
    assert_eq!(
        references(&analysis, call)
            .into_iter()
            .map(|span| span.start)
            .collect::<Vec<_>>(),
        [declared, call]
    );
    let callable = callable_at(&analysis, call).expect("a callable");
    assert_eq!(callable.selection.start, declared);
    assert!(
        incoming(&analysis, callable.def).is_empty(),
        "the only call to it is in a `state` line, which is not a callable"
    );
    assert!(outgoing(&analysis, callable.def).is_empty());

    // The outline names the same three declarations, in source order.
    let outline: Vec<(String, DeclarationKind)> = document_declarations(&analysis)
        .into_iter()
        .map(|declaration| (declaration.name, declaration.kind))
        .collect();
    assert_eq!(
        outline,
        [
            ("twice".to_string(), DeclarationKind::Function),
            (
                "doubled".to_string(),
                DeclarationKind::Signal(zdc_ast::Placement::Client)
            ),
            ("view".to_string(), DeclarationKind::View),
        ]
    );

    // Signature help, inlay hints and folding each answer for it too.
    let help = signature(&analysis, call + "twice with ".len() as u32).expect("a signature");
    assert_eq!(help.label, "twice with n is Whole");
    assert_eq!(
        hints(&analysis, 0, u32::MAX)
            .into_iter()
            .map(|hint| hint.label)
            .collect::<Vec<_>>(),
        ["is Whole"],
        "the one binder in the program is `twice`'s parameter"
    );
    assert_eq!(folds(&analysis).len(), 2, "the function body and the view");

    // And the range form of the highlighter is the whole document's
    // answer restricted to those lines.
    let whole = highlights(&analysis);
    let first = highlights_within(&analysis, 0, 0);
    assert!(!first.is_empty(), "the first line colours something");
    assert_eq!(
        first,
        whole
            .iter()
            .copied()
            .filter(|token| token.line == 0)
            .collect::<Vec<_>>()
    );
}
