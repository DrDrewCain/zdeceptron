//! Semantic highlighting: the scopes a regular expression cannot compute.
//!
//! `editors/vscode/README.md` names the two cases the TextMate grammar
//! gives up on, and both are answered here from the compiler rather than
//! guessed: `is` is coloured by which of its three jobs it is doing, and a
//! capitalised name is coloured by whether resolution made it a type or a
//! view element.
//!
//! There is a third thing the grammar could never have known, and it is the
//! one worth having: a reference to a signal carries a modifier naming the
//! placement it was declared with, so `client`, `server` and `durable`
//! state look different while the code is being written.
//!
//! Classification degrades in three steps rather than switching off. With
//! types, everything below applies. Without them — the file resolves but
//! does not typecheck — nothing here changes, since none of it needs a
//! type. Without resolution, syntactic roles survive and placement
//! modifiers do not. Without a parse, the lexer's own token kinds are still
//! emitted, so a file being typed into keeps its colours.

use std::collections::HashMap;

use zdc_ast::Placement;
use zdc_hir::{DefKind, Res};
use zdc_lexer::{Token, TokenKind};

use crate::analysis::Analysis;
use crate::symbols::{IsRole, Symbol, SymbolKind};

/// The token types this server emits, in the order the protocol indexes
/// them. Every name is one the protocol already defines, so a client that
/// has never heard of ZDeceptron still colours the file.
pub const TOKEN_TYPES: &[&str] = &[
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
];

/// The modifiers, in bit order.
///
/// The first three are the protocol's. The last three are this language's:
/// a client that does not know them ignores them, which is why placement
/// is expressed as a modifier on an ordinary `variable` rather than as a
/// token type of its own that would leave the name uncoloured.
pub const TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "readonly",
    "defaultLibrary",
    "client",
    "server",
    "durable",
];

const KEYWORD: u32 = 0;
const OPERATOR: u32 = 1;
const TYPE: u32 = 2;
const CLASS: u32 = 3;
const ENUM_MEMBER: u32 = 4;
const VARIABLE: u32 = 5;
const FUNCTION: u32 = 6;
const METHOD: u32 = 7;
const PARAMETER: u32 = 8;
const PROPERTY: u32 = 9;
const STRING: u32 = 10;
const NUMBER: u32 = 11;

const DECLARATION: u32 = 1 << 0;
const READONLY: u32 = 1 << 1;
const DEFAULT_LIBRARY: u32 = 1 << 2;
const CLIENT: u32 = 1 << 3;
const SERVER: u32 = 1 << 4;
const DURABLE: u32 = 1 << 5;

/// One highlighted token, before the protocol's delta encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Highlight {
    pub line: u32,
    pub start: u32,
    /// Length in UTF-16 code units, which is what the protocol counts.
    pub length: u32,
    pub token_type: u32,
    pub modifiers: u32,
}

/// Every highlighted token in the file, in source order.
pub fn highlights(analysis: &Analysis) -> Vec<Highlight> {
    let text = analysis.text();
    let lines = analysis.lines();
    let by_start = analysis.symbols().by_start();

    let mut out = Vec::new();
    for token in analysis.tokens() {
        let Some((token_type, modifiers)) = classify(analysis, &by_start, token) else {
            continue;
        };

        let start = lines.position(text, token.span.start);
        let end = lines.position(text, token.span.end);
        // The protocol has no representation for a token that spans lines,
        // and a client given one draws the rest of the file wrong.
        if start.line != end.line || end.character < start.character {
            continue;
        }
        out.push(Highlight {
            line: start.line,
            start: start.character,
            length: end.character - start.character,
            token_type,
            modifiers,
        });
    }
    out
}

/// The protocol's delta encoding: five integers per token, each position
/// relative to the previous token's.
pub fn encode(highlights: &[Highlight]) -> Vec<u32> {
    let mut out = Vec::with_capacity(highlights.len() * 5);
    let (mut line, mut start) = (0, 0);
    for highlight in highlights {
        let delta_line = highlight.line.saturating_sub(line);
        let delta_start = if delta_line == 0 {
            highlight.start.saturating_sub(start)
        } else {
            highlight.start
        };
        out.extend_from_slice(&[
            delta_line,
            delta_start,
            highlight.length,
            highlight.token_type,
            highlight.modifiers,
        ]);
        line = highlight.line;
        start = highlight.start;
    }
    out
}

/// What a token is, preferring what the compiler decided over what the
/// token kind alone can say.
fn classify(
    analysis: &Analysis,
    by_start: &HashMap<u32, &Symbol>,
    token: &Token,
) -> Option<(u32, u32)> {
    if let Some(symbol) = by_start.get(&token.span.start) {
        if symbol.span == token.span {
            return Some(from_symbol(analysis, symbol));
        }
    }
    from_token_kind(&token.kind)
}

fn from_symbol(analysis: &Analysis, symbol: &Symbol) -> (u32, u32) {
    match &symbol.kind {
        SymbolKind::Signal {
            placement, source, ..
        } => (
            VARIABLE,
            DECLARATION | placement_bit(*placement) | if *source { 0 } else { READONLY },
        ),
        SymbolKind::Function { .. } => (FUNCTION, DECLARATION),
        // A component is used exactly where a built-in element is, so it
        // takes the same colour minus the modifier that says the language
        // provided it (spec §14D.1).
        SymbolKind::Component { .. } => (CLASS, DECLARATION),
        SymbolKind::View => (KEYWORD, DECLARATION),
        SymbolKind::Binding { parameter, .. } => {
            (if *parameter { PARAMETER } else { VARIABLE }, DECLARATION)
        }
        SymbolKind::Use { res, .. } => from_res(analysis, *res),
        SymbolKind::Element => (CLASS, DEFAULT_LIBRARY),
        SymbolKind::Variant => (ENUM_MEMBER, DEFAULT_LIBRARY),
        SymbolKind::TypeName { builtin } => (TYPE, if *builtin { DEFAULT_LIBRARY } else { 0 }),
        SymbolKind::Label => (PARAMETER, 0),
        SymbolKind::Field => (PROPERTY, 0),
        SymbolKind::Event => (METHOD, DEFAULT_LIBRARY),
        // The three jobs of `is`, told apart. A declaration's `is` is part
        // of the declaration syntax; the other two are operators, and the
        // one that binds a named argument is marked as declaring it.
        SymbolKind::Is(IsRole::Declaration) => (KEYWORD, DECLARATION),
        SymbolKind::Is(IsRole::NamedArgument) => (OPERATOR, DECLARATION),
        SymbolKind::Is(IsRole::Equality) => (OPERATOR, 0),
    }
}

/// A reference takes its colour from what it refers to — including, for a
/// signal, where that value lives.
fn from_res(analysis: &Analysis, res: Option<Res>) -> (u32, u32) {
    let Some(res) = res else {
        return (VARIABLE, 0);
    };
    match res {
        Res::Builtin(zdc_hir::Builtin::Element) => (CLASS, DEFAULT_LIBRARY),
        Res::Builtin(zdc_hir::Builtin::Type) => (TYPE, DEFAULT_LIBRARY),
        Res::Local(_) => (VARIABLE, 0),
        // A variant of a declared `choice` is the same kind of thing as a
        // built-in variant such as `Ready`, minus the modifier that says
        // the language provided it.
        Res::Variant { .. } => (ENUM_MEMBER, 0),
        Res::Def(def) => {
            let Some(hir) = analysis.hir() else {
                return (VARIABLE, 0);
            };
            match &hir.defs[def].kind {
                DefKind::Signal(signal) => (
                    VARIABLE,
                    placement_bit(signal.placement) | if signal.is_source { 0 } else { READONLY },
                ),
                DefKind::Function(_) => (FUNCTION, 0),
                DefKind::View(_) => (KEYWORD, 0),
                // A declared record or choice is named where a type is
                // written, so it colours as a type the program provided.
                DefKind::Record(_) | DefKind::Choice(_) => (TYPE, 0),
                DefKind::Component(_) => (CLASS, 0),
            }
        }
    }
}

fn placement_bit(placement: Placement) -> u32 {
    match placement {
        Placement::Client => CLIENT,
        Placement::Server => SERVER,
        Placement::Durable => DURABLE,
    }
}

/// What the lexer alone can say. This is what a file that does not parse
/// falls back to, so it must never be wrong — only less specific.
fn from_token_kind(kind: &TokenKind) -> Option<(u32, u32)> {
    use TokenKind::*;
    Some(match kind {
        Number(_) => (NUMBER, 0),
        Text(_) => (STRING, 0),
        // Without a tree there is nothing to say about a name beyond that
        // it is one.
        Ident(_) => (VARIABLE, 0),
        // Word operators. `is` defaults to equality, which is what it is
        // everywhere the tree is unavailable to say otherwise.
        And | Or | Not | Is | IsNot | At => (OPERATOR, 0),
        Plus | Minus | Star | Slash | Less | Greater | LessEq | GreaterEq => (OPERATOR, 0),
        // Punctuation and layout carry no meaning worth colouring, and
        // leaving them out lets the TextMate grammar keep handling them.
        Comma | Dot | LParen | RParen | Newline | Indent | Dedent | Eof => return None,
        other => {
            // Everything left is a keyword, and the lexer proves it by
            // having a spelling for it.
            other.keyword_spelling()?;
            (KEYWORD, 0)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(src: &str, needle: &str) -> Highlight {
        let analysis = Analysis::of(src);
        let text = analysis.text();
        let lines = analysis.lines();
        let offset = src.find(needle).expect("the needle is in the source") as u32;
        let position = lines.position(text, offset);
        highlights(&analysis)
            .into_iter()
            .find(|h| h.line == position.line && h.start == position.character)
            .unwrap_or_else(|| panic!("no highlight at {needle:?}"))
    }

    /// The distinction the README says a regular expression cannot make.
    #[test]
    fn the_three_jobs_of_is_get_three_different_scopes() {
        // `is` three times over. Note that the one in the view is a
        // named argument and not a comparison, which is exactly the
        // distinction being asserted.
        let src = "state open is client Truth starting no\n\
                   state shown is client Truth from open is yes\n\
                   view\n    Checkbox open, hint is \"search\"\n";
        let analysis = Analysis::of(src);
        let all = highlights(&analysis);
        let lines = analysis.lines();
        let text = analysis.text();

        let find = |offset: usize| {
            let position = lines.position(text, offset as u32);
            all.iter()
                .find(|h| h.line == position.line && h.start == position.character)
                .copied()
                .expect("a highlight")
        };

        let declaration = find(src.find(" is ").expect("declaration") + 1);
        let named = find(src.find("hint is").expect("named") + 5);
        let equality = find(src.rfind("open is yes").expect("equality") + 5);

        assert_eq!(
            (declaration.token_type, declaration.modifiers),
            (KEYWORD, DECLARATION)
        );
        assert_eq!((named.token_type, named.modifiers), (OPERATOR, DECLARATION));
        assert_eq!((equality.token_type, equality.modifiers), (OPERATOR, 0));

        // All three are distinguishable from each other, which is the
        // property that actually matters.
        let scopes = [
            (declaration.token_type, declaration.modifiers),
            (named.token_type, named.modifiers),
            (equality.token_type, equality.modifiers),
        ];
        assert_eq!(
            scopes
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
    }

    /// The other distinction: both are capitalised, and only resolution
    /// tells them apart.
    #[test]
    fn a_type_and_an_element_get_different_scopes() {
        let src = "state name is client Text starting \"\"\nview\n    Text name\n";
        let as_type = at(src, "Text starting");
        let as_element = at(src, "Text name");
        assert_eq!(as_type.token_type, TYPE);
        assert_eq!(as_element.token_type, CLASS);
    }

    /// The one no grammar could have computed: where the value lives.
    #[test]
    fn a_reference_carries_the_placement_of_what_it_refers_to() {
        let src = "state here is client Whole starting 0\n\
                   state kept is durable Whole starting 0\n\
                   state sum is server Whole from here + kept\n\
                   view\n    Text here\n";
        let analysis = Analysis::of(src);
        // Not `diagnostics().is_empty()`: `server` and `durable` placement
        // is refused by the emitter until M6 (§16.5), and the editor now
        // shows that refusal because `zdc check` does. What this test needs
        // is that the program resolved and typechecked, which is what makes
        // a token's placement answerable at all.
        assert!(analysis.types().is_some(), "{:?}", analysis.diagnostics());

        assert_eq!(at(src, "here + kept").modifiers & CLIENT, CLIENT);
        assert_eq!(at(src, "kept\n").modifiers & DURABLE, DURABLE);
        assert_eq!(at(src, "sum is").modifiers & SERVER, SERVER);
    }

    /// A `from` signal is recomputed and cannot be assigned to, which is
    /// exactly what `readonly` means.
    #[test]
    fn a_derived_signal_is_marked_readonly_and_a_source_is_not() {
        let src = "state count is client Whole starting 0\n\
                   state twice is client Whole from count * 2\n";
        assert_eq!(at(src, "twice").modifiers & READONLY, READONLY);
        assert_eq!(at(src, "count is").modifiers & READONLY, 0);
    }

    #[test]
    fn a_named_argument_label_is_not_a_variable() {
        let src = "state q is client Text starting \"\"\nview\n    Input q, hint is \"go\"\n";
        assert_eq!(at(src, "hint").token_type, PARAMETER);
    }

    #[test]
    fn a_field_is_a_property_and_a_variant_is_an_enum_member() {
        let src = "state items is server List of Item starting empty\n\
                   view\n    when items\n        Loading show Spinner\n\
                   \x20       Failed with error show ErrorBar message is error.message\n\
                   \x20       Ready with ready show Text \"ok\"\n";
        assert_eq!(at(src, "Loading").token_type, ENUM_MEMBER);
        assert_eq!(at(src, "message\n").token_type, PROPERTY);
    }

    /// A file mid-keystroke must not go plain. The lexer's own kinds are
    /// enough to keep keywords, strings and numbers coloured.
    #[test]
    fn a_file_that_does_not_parse_still_highlights_its_tokens() {
        let src = "state count is client Whole starting 0\nview\n    Text ";
        let analysis = Analysis::of(src);
        assert!(!analysis.diagnostics().is_empty(), "expected a parse error");

        let all = highlights(&analysis);
        assert!(!all.is_empty(), "highlighting went dark on a partial file");
        assert!(all.iter().any(|h| h.token_type == KEYWORD));
        assert!(all.iter().any(|h| h.token_type == NUMBER));
    }

    #[test]
    fn every_highlight_stays_on_one_line_and_within_its_line() {
        let src = "# an em dash \u{2014} here\n\
                   state \u{e9} is client Text starting \"\u{4e2d}\u{6587}\"\n\
                   view\n    Text \u{e9}\n";
        let analysis = Analysis::of(src);
        let lines = analysis.lines();
        for highlight in highlights(&analysis) {
            assert!((highlight.line as usize) < lines.line_count());
            assert!(
                highlight.length > 0,
                "a zero-length highlight: {highlight:?}"
            );
        }
    }

    /// Two-byte and four-byte characters inside a token must count as
    /// UTF-16 code units, not as bytes, or every colour after them slides.
    #[test]
    fn lengths_and_columns_are_counted_in_utf16_code_units() {
        let src = "state x is client Text starting \"\u{e9}\u{1f600}\"\n";
        let highlight = at(src, "\"\u{e9}");

        // The head of the line is 32 ASCII characters.
        assert_eq!(highlight.start, 32);
        // Bytes: two quotes, two for "é", four for the emoji — eight.
        assert_eq!(
            highlight.length as usize, 5,
            "counted bytes, not code units"
        );
        assert_eq!(highlight.token_type, STRING);
    }

    /// Identifiers may be non-Latin (spec §4.6 needs that for dialects),
    /// so a name whose characters are two bytes each must still be given
    /// the right column and width.
    #[test]
    fn a_non_latin_identifier_is_measured_in_code_units() {
        let src = "state \u{4e2d}\u{6587} is client Whole starting 0\n";
        let highlight = at(src, "\u{4e2d}\u{6587}");
        assert_eq!(highlight.start, 6);
        assert_eq!(highlight.length, 2);
    }

    #[test]
    fn the_delta_encoding_starts_from_the_origin_and_advances() {
        let encoded = encode(&[
            Highlight {
                line: 0,
                start: 0,
                length: 5,
                token_type: KEYWORD,
                modifiers: 0,
            },
            Highlight {
                line: 0,
                start: 6,
                length: 5,
                token_type: VARIABLE,
                modifiers: CLIENT,
            },
            Highlight {
                line: 2,
                start: 4,
                length: 4,
                token_type: CLASS,
                modifiers: DEFAULT_LIBRARY,
            },
        ]);
        assert_eq!(
            encoded,
            vec![
                0,
                0,
                5,
                KEYWORD,
                0, //
                0,
                6,
                5,
                VARIABLE,
                CLIENT, //
                2,
                4,
                4,
                CLASS,
                DEFAULT_LIBRARY,
            ]
        );
    }

    /// The encoded stream is what a client applies blindly. Every index
    /// into the legend must exist, or it colours with something else.
    #[test]
    fn every_emitted_index_is_within_the_legend() {
        for path in std::fs::read_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples"),
        )
        .expect("the examples directory")
        {
            let path = path.expect("a directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("zd") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("a readable example");
            let analysis = Analysis::of(&src);
            for highlight in highlights(&analysis) {
                assert!(
                    (highlight.token_type as usize) < TOKEN_TYPES.len(),
                    "{}: token type {} is not in the legend",
                    path.display(),
                    highlight.token_type
                );
                assert!(
                    highlight.modifiers >> TOKEN_MODIFIERS.len() == 0,
                    "{}: modifiers {:#b} exceed the legend",
                    path.display(),
                    highlight.modifiers
                );
            }
        }
    }

    #[test]
    fn nonsense_produces_no_highlights_rather_than_a_panic() {
        for src in ["", "\u{0}\u{1}", "{\"json\": 1}", "((((", "state x is"] {
            let analysis = Analysis::of(src);
            let _ = encode(&highlights(&analysis));
        }
    }
}
