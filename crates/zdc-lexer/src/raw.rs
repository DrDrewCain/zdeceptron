use crate::{Span, TokenKind};
use logos::Logos;

/// A token before layout processing.
///
/// `LineStart(n)` is a newline plus the `n` spaces that follow it; the
/// layout pass in Task 4 turns these into `Newline`/`Indent`/`Dedent`.
#[derive(Debug, Clone, PartialEq)]
pub enum RawToken {
    Kw(TokenKind),
    LineStart(u32),
    Error,
}

#[derive(Logos, Debug, Clone, PartialEq)]
enum Lexeme {
    // A newline followed by its indentation. Longest-match beats `Space`
    // because this alternative starts with `\n`.
    #[regex(r"\n[ ]*", line_start_width)]
    LineStart(u32),

    #[regex(r"[ ]+", logos::skip)]
    Space,

    // `#` to end of line. A symbol, not a word: the syntax-evidence study
    // that motivated word-based tokens (spec §4.2) covers operators and
    // control flow, not comment markers; Python uses `#` for comments and
    // scored well in that same study; and `#` cannot begin an identifier
    // in any script, so it needs no entry in `word_to_kind` and is
    // dialect-neutral with no English word to relocate.
    #[regex(r"#[^\n]*", logos::skip)]
    Comment,

    #[regex(r"[0-9]+(\.[0-9]+)?", |lex| lex.slice().parse::<f64>().ok())]
    Number(f64),

    #[regex(r#""[^"\n]*""#, |lex| {
        let s = lex.slice();
        s[1..s.len() - 1].to_string()
    })]
    Text(String),

    // UAX #31 identifier rules (spec §4.6): non-Latin identifiers must
    // work from day one so no lexer rewrite is needed for future dialects.
    #[regex(r"[\p{XID_Start}_][\p{XID_Continue}]*", |lex| lex.slice().to_string())]
    Word(String),

    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("<=")]
    LessEq,
    #[token(">=")]
    GreaterEq,
    #[token("<")]
    Less,
    #[token(">")]
    Greater,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
}

/// The indent width of the line following a `\n[ ]*` match (its length
/// minus the newline byte itself).
fn line_start_width(lex: &mut logos::Lexer<Lexeme>) -> u32 {
    (lex.slice().len() - 1) as u32
}

/// Map a bare word to its keyword, or to an identifier.
///
/// This table is the `english` dialect. Task 9 and later dialect work
/// replace this function rather than editing call sites (spec §4.6).
fn word_to_kind(word: &str) -> TokenKind {
    use TokenKind::*;
    match word {
        "secret" => Secret,
        "state" => State,
        "function" => Function,
        "view" => View,
        "client" => Client,
        "server" => Server,
        "durable" => Durable,
        "starting" => Starting,
        "from" => From,
        "of" => Of,
        "to" => To,
        "give" => Give,
        "set" => Set,
        "add" => Add,
        "subtract" => Subtract,
        "keep" => Keep,
        "sort" => Sort,
        "map" => MapEach,
        "take" => Take,
        "first" => First,
        "where" => Where,
        "by" => By,
        "when" => When,
        "each" => Each,
        "in" => In,
        "if" => If,
        "otherwise" => Otherwise,
        "show" => Show,
        "on" => On,
        "with" => With,
        "and" => And,
        "or" => Or,
        "not" => Not,
        "is" => Is,
        "at" => At,
        "yes" => Yes,
        "no" => No,
        "empty" => Empty,
        "environment" => Environment,
        other => Ident(other.to_string()),
    }
}

/// A word that names a type built from another type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCtor {
    /// `List of T`
    List,
    /// `Option of T`
    Option,
    /// `Remote of T`
    Remote,
    /// `Map of K to V`
    Map,
}

/// Map a capitalised word to the type it constructs, if it names one.
///
/// This is the second half of the `english` dialect, and it exists so
/// that `word_to_kind` and this function are between them the *only*
/// places an English spelling is recognised: a dialect replaces the pair
/// and nothing else in the compiler knows what `List` means. The parser
/// previously compared identifier text against `"List"`, `"Option"`,
/// `"Remote"`, and `"Map"` itself, which left four English words wired
/// into it that no dialect could reach.
///
/// These are not `TokenKind`s, because they are only type constructors
/// where a type is expected. Making them keywords would reserve four
/// ordinary nouns everywhere — `List` is exactly the sort of word a view
/// element or a field is named — for a meaning they only have after a
/// placement. The lexer keeps the spellings; the position decides
/// whether they are read as a constructor.
pub fn word_to_type_ctor(word: &str) -> Option<TypeCtor> {
    Some(match word {
        "List" => TypeCtor::List,
        "Option" => TypeCtor::Option,
        "Remote" => TypeCtor::Remote,
        "Map" => TypeCtor::Map,
        _ => return None,
    })
}

pub fn tokenize_raw(src: &str) -> Vec<(RawToken, Span)> {
    let mut out: Vec<(RawToken, Span)> = Vec::new();
    let mut lexer = Lexeme::lexer(src);

    while let Some(result) = lexer.next() {
        let range = lexer.span();
        let span = Span::new(range.start as u32, range.end as u32);

        let token = match result {
            Err(()) => RawToken::Error,
            Ok(Lexeme::LineStart(width)) => RawToken::LineStart(width),
            Ok(Lexeme::Space) | Ok(Lexeme::Comment) => continue,
            Ok(Lexeme::Number(n)) => RawToken::Kw(TokenKind::Number(n)),
            Ok(Lexeme::Text(s)) => RawToken::Kw(TokenKind::Text(s)),
            Ok(Lexeme::Word(w)) => RawToken::Kw(word_to_kind(&w)),
            Ok(Lexeme::Plus) => RawToken::Kw(TokenKind::Plus),
            Ok(Lexeme::Minus) => RawToken::Kw(TokenKind::Minus),
            Ok(Lexeme::Star) => RawToken::Kw(TokenKind::Star),
            Ok(Lexeme::Slash) => RawToken::Kw(TokenKind::Slash),
            Ok(Lexeme::LessEq) => RawToken::Kw(TokenKind::LessEq),
            Ok(Lexeme::GreaterEq) => RawToken::Kw(TokenKind::GreaterEq),
            Ok(Lexeme::Less) => RawToken::Kw(TokenKind::Less),
            Ok(Lexeme::Greater) => RawToken::Kw(TokenKind::Greater),
            Ok(Lexeme::Comma) => RawToken::Kw(TokenKind::Comma),
            Ok(Lexeme::Dot) => RawToken::Kw(TokenKind::Dot),
            Ok(Lexeme::LParen) => RawToken::Kw(TokenKind::LParen),
            Ok(Lexeme::RParen) => RawToken::Kw(TokenKind::RParen),
        };

        // `is` followed by `not` is a single operator (spec §4.2).
        if token == RawToken::Kw(TokenKind::Not) {
            if let Some((RawToken::Kw(TokenKind::Is), is_span)) = out.last().cloned() {
                out.pop();
                out.push((RawToken::Kw(TokenKind::IsNot), is_span.to(span)));
                continue;
            }
        }

        out.push((token, span));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<RawToken> {
        tokenize_raw(src).into_iter().map(|(t, _)| t).collect()
    }

    #[test]
    fn lexes_a_state_declaration() {
        assert_eq!(
            kinds("state votes is durable"),
            vec![
                RawToken::Kw(TokenKind::State),
                RawToken::Kw(TokenKind::Ident("votes".into())),
                RawToken::Kw(TokenKind::Is),
                RawToken::Kw(TokenKind::Durable),
            ]
        );
    }

    #[test]
    fn is_not_lexes_as_one_token() {
        assert_eq!(
            kinds("a is not b"),
            vec![
                RawToken::Kw(TokenKind::Ident("a".into())),
                RawToken::Kw(TokenKind::IsNot),
                RawToken::Kw(TokenKind::Ident("b".into())),
            ]
        );
    }

    #[test]
    fn bare_not_after_and_does_not_merge_into_is_not() {
        // The `is not` merge must only fire when the immediately preceding
        // token is `Is`; a bare prefix `not` (as in `and not item.hidden`)
        // must lex as separate `And`, `Not` tokens.
        assert_eq!(
            kinds("a and not b"),
            vec![
                RawToken::Kw(TokenKind::Ident("a".into())),
                RawToken::Kw(TokenKind::And),
                RawToken::Kw(TokenKind::Not),
                RawToken::Kw(TokenKind::Ident("b".into())),
            ]
        );
    }

    #[test]
    fn line_start_carries_indent_width() {
        assert_eq!(
            kinds("view\n    Column"),
            vec![
                RawToken::Kw(TokenKind::View),
                RawToken::LineStart(4),
                RawToken::Kw(TokenKind::Ident("Column".into())),
            ]
        );
    }

    #[test]
    fn hash_comments_are_skipped() {
        assert_eq!(
            kinds("# this is skipped\nnotebook"),
            vec![
                RawToken::LineStart(0),
                RawToken::Kw(TokenKind::Ident("notebook".into())),
            ]
        );
    }

    // Regression test: with `note` as the (former, round-1) comment marker,
    // a field or variable literally named `note` had its rest-of-line
    // silently swallowed with no diagnostic (`item.note` lexed as
    // `[Ident("item"), Dot]`, dropping the trailing `note` entirely). The
    // comment marker is now the symbol `#`, which cannot begin an
    // identifier in any script, so `note` is always an ordinary word.
    #[test]
    fn note_is_an_ordinary_identifier_not_a_comment_marker() {
        assert_eq!(
            kinds("item.note"),
            vec![
                RawToken::Kw(TokenKind::Ident("item".into())),
                RawToken::Kw(TokenKind::Dot),
                RawToken::Kw(TokenKind::Ident("note".into())),
            ]
        );
        assert_eq!(
            kinds("note"),
            vec![RawToken::Kw(TokenKind::Ident("note".into()))]
        );
    }

    #[test]
    fn comment_at_end_of_line_does_not_eat_following_indentation() {
        assert_eq!(
            kinds("view # hi\n    Column"),
            vec![
                RawToken::Kw(TokenKind::View),
                RawToken::LineStart(4),
                RawToken::Kw(TokenKind::Ident("Column".into())),
            ]
        );
    }

    #[test]
    fn unicode_identifiers_are_accepted() {
        assert_eq!(
            kinds("état"),
            vec![RawToken::Kw(TokenKind::Ident("état".into()))]
        );
    }

    #[test]
    fn tabs_are_rejected() {
        assert_eq!(
            kinds("\tx"),
            vec![RawToken::Error, RawToken::Kw(TokenKind::Ident("x".into()))]
        );
    }

    /// English spellings live in exactly two functions, so a dialect
    /// replaces the pair and the compiler knows nothing else.
    #[test]
    fn type_constructors_are_recognised_by_the_dialect_table() {
        assert_eq!(word_to_type_ctor("List"), Some(TypeCtor::List));
        assert_eq!(word_to_type_ctor("Option"), Some(TypeCtor::Option));
        assert_eq!(word_to_type_ctor("Remote"), Some(TypeCtor::Remote));
        assert_eq!(word_to_type_ctor("Map"), Some(TypeCtor::Map));
    }

    #[test]
    fn an_ordinary_word_constructs_no_type() {
        assert_eq!(word_to_type_ctor("Item"), None);
        assert_eq!(word_to_type_ctor("list"), None);
        assert_eq!(word_to_type_ctor("Column"), None);
    }

    /// A type constructor is still an ordinary identifier token: nothing
    /// is reserved, so `List` remains available as an element or field
    /// name.
    #[test]
    fn a_type_constructor_still_lexes_as_an_identifier() {
        assert_eq!(
            kinds("List of Item"),
            vec![
                RawToken::Kw(TokenKind::Ident("List".into())),
                RawToken::Kw(TokenKind::Of),
                RawToken::Kw(TokenKind::Ident("Item".into())),
            ]
        );
    }

    #[test]
    fn lexes_numbers_and_text() {
        assert_eq!(
            kinds(r#"8 "search""#),
            vec![
                RawToken::Kw(TokenKind::Number(8.0)),
                RawToken::Kw(TokenKind::Text("search".into())),
            ]
        );
    }
}
