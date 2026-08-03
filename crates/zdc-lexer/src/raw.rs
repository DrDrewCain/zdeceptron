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
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
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
        "record" => Record,
        "choice" => Choice,
        "component" => Component,
        "use" => Use,
        "for" => For,
        "children" => Children,
        "client" => Client,
        "static" => Static,
        "server" => Server,
        "durable" => Durable,
        "starting" => Starting,
        "emitting" => Emitting,
        "from" => From,
        "of" => Of,
        "to" => To,
        "give" => Give,
        "set" => Set,
        "add" => Add,
        "subtract" => Subtract,
        "append" => Append,
        "remove" => Remove,
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
        "contains" => Contains,
        "yes" => Yes,
        "no" => No,
        "empty" => Empty,
        "environment" => Environment,
        other => Ident(other.to_string()),
    }
}

/// A word that means a keyword only where one construct expects it.
///
/// The `foreign` declaration (§14E.1 as amended by §17.4.2) needs five more
/// words, and §14G.7.7's accounting budgets exactly one new reserved
/// identifier — `contains`. Reserving `as`, `takes`, `gives` and `anywhere`
/// outright would take four ordinary nouns away from every program for a
/// meaning they only have inside one indented block, which is the same
/// argument [`word_to_type_ctor`] makes about `List`.
///
/// They stay `Ident` tokens, and the parser asks *this table* whether the
/// word it is looking at is the one that construct wants. The spelling
/// therefore still lives in the lexer, so a dialect replaces this function
/// with the other two and nothing downstream knows any English.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftKeyword {
    /// Begins a `foreign` declaration.
    Foreign,
    /// `from "zd:text" as "length"` — the symbol within the module.
    As,
    /// Introduces a `foreign`'s parameter list.
    Takes,
    /// Introduces a `foreign`'s result type.
    Gives,
    /// A `foreign` that may run in any placement.
    Anywhere,
}

impl SoftKeyword {
    /// The one valid spelling of this word in the `english` dialect, for
    /// the diagnostic that names the single correct form (§4.1).
    pub fn spelling(self) -> &'static str {
        match self {
            SoftKeyword::Foreign => "foreign",
            SoftKeyword::As => "as",
            SoftKeyword::Takes => "takes",
            SoftKeyword::Gives => "gives",
            SoftKeyword::Anywhere => "anywhere",
        }
    }
}

pub fn word_to_soft_keyword(word: &str) -> Option<SoftKeyword> {
    Some(match word {
        "foreign" => SoftKeyword::Foreign,
        "as" => SoftKeyword::As,
        "takes" => SoftKeyword::Takes,
        "gives" => SoftKeyword::Gives,
        "anywhere" => SoftKeyword::Anywhere,
        _ => return None,
    })
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

/// The longest unbroken run of token characters the lexer will scan.
///
/// **Why a limit exists at all.** `logos` compiles the `Word` pattern —
/// `[\p{XID_Start}_][\p{XID_Continue}]*`, the only rule here over a
/// Unicode class rather than an ASCII one — into a set of mutually
/// recursive state functions, and relies on the optimiser to turn their
/// tail calls into jumps. It does at `-O`; it does not at `-O0`, where
/// the scan costs roughly a kilobyte of stack per character and an
/// 8 000-character word aborts the process. That abort is `SIGABRT`: no
/// panic, no diagnostic, nothing `catch_unwind` can hold, and `zdc lsp`
/// simply dies mid-keystroke. Bounding the run bounds the recursion, and
/// bounds it identically in both profiles rather than leaving the
/// compiler's totality resting on an optimisation setting.
///
/// A kilobyte is three orders of magnitude past the longest name in any
/// example here and three orders short of where the unoptimised scan
/// fails, so nothing a person writes is near either edge.
pub const MAX_TOKEN_CHARS: usize = 1024;

/// Whether `c` ends a token rather than continuing one.
///
/// Deliberately an **over**-approximation of "not `XID_Continue`": every
/// character the lexer has its own rule for, plus whitespace. Anything
/// else is treated as continuing a run, so a run that is too long is
/// caught whether it is a name, a number, or bytes that are not
/// ZDeceptron at all — and the last of those is the case that matters,
/// because it is the one a truncated download produces.
fn separates(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '+' | '-' | '*' | '/' | '<' | '>' | ',' | '.' | '(' | ')' | '[' | ']' | '"' | '#'
        )
}

/// The span of the first run longer than [`MAX_TOKEN_CHARS`], if there is
/// one, skipping string bodies and comments — neither reaches the `Word`
/// rule, and a long one of either is legitimate.
pub fn over_long_run(src: &str) -> Option<(Span, usize)> {
    let mut chars = src.char_indices();
    let mut start = 0usize;
    let mut length = 0usize;

    while let Some((at, c)) = chars.next() {
        if c == '#' || c == '"' {
            let closes = if c == '#' { '\n' } else { '"' };
            for (_, inner) in chars.by_ref() {
                if inner == closes || inner == '\n' {
                    break;
                }
            }
            length = 0;
            continue;
        }
        if separates(c) {
            length = 0;
            continue;
        }
        if length == 0 {
            start = at;
        }
        length += 1;
        if length <= MAX_TOKEN_CHARS {
            continue;
        }

        // Report the whole run, not the character that crossed the line:
        // a caret under the 1025th `z` of 200 000 explains nothing.
        let mut end = at + c.len_utf8();
        for (next_at, next) in chars.by_ref() {
            if separates(next) {
                break;
            }
            end = next_at + next.len_utf8();
            length += 1;
        }
        return Some((Span::new(start as u32, end as u32), length));
    }
    None
}

/// `first` is a keyword only in `take first`, and an ordinary name
/// everywhere else.
///
/// The pipeline clause is the one place the word means anything, and it
/// always means it immediately after `take` — the same one-token context
/// the `is not` merge below uses. Reserving it outright would forbid
/// `function first of items` and `min with first, second`, both of which
/// §17.4.9 writes, so this is what lets the prelude be spelled the way the
/// spec spells it. The written word is carried through, so a dialect that
/// spells this clause differently keeps the identifier it wrote.
fn demote_first(kind: TokenKind, word: String, out: &[(RawToken, Span)]) -> TokenKind {
    if kind != TokenKind::First {
        return kind;
    }
    match out.last() {
        Some((RawToken::Kw(TokenKind::Take), _)) => TokenKind::First,
        _ => TokenKind::Ident(word),
    }
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
            Ok(Lexeme::Word(w)) => RawToken::Kw(demote_first(word_to_kind(&w), w, &out)),
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
            Ok(Lexeme::LBracket) => RawToken::Kw(TokenKind::LBracket),
            Ok(Lexeme::RBracket) => RawToken::Kw(TokenKind::RBracket),
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

    /// §14B.1 and §14B.2: the declaration and mutation words are keywords,
    /// so a dialect relocates them with the rest rather than matching text.
    #[test]
    fn the_declaration_and_membership_words_are_keywords() {
        assert_eq!(
            kinds("record choice append remove"),
            vec![
                RawToken::Kw(TokenKind::Record),
                RawToken::Kw(TokenKind::Choice),
                RawToken::Kw(TokenKind::Append),
                RawToken::Kw(TokenKind::Remove),
            ]
        );
    }

    /// §14D's four words are keywords for the same reason: `children` in
    /// particular would otherwise be one English spelling the parser
    /// matched by text, which no dialect could relocate.
    #[test]
    fn the_component_and_module_words_are_keywords() {
        assert_eq!(
            kinds("component use for children"),
            vec![
                RawToken::Kw(TokenKind::Component),
                RawToken::Kw(TokenKind::Use),
                RawToken::Kw(TokenKind::For),
                RawToken::Kw(TokenKind::Children),
            ]
        );
    }

    /// §14B.4 puts collection literals in brackets, which no other
    /// construct uses, so they need no lookahead to recognise.
    #[test]
    fn brackets_lex_as_their_own_tokens() {
        assert_eq!(
            kinds("[1, 2]"),
            vec![
                RawToken::Kw(TokenKind::LBracket),
                RawToken::Kw(TokenKind::Number(1.0)),
                RawToken::Kw(TokenKind::Comma),
                RawToken::Kw(TokenKind::Number(2.0)),
                RawToken::Kw(TokenKind::RBracket),
            ]
        );
    }

    /// §14F.1 closes the infix set at `and`, `or`, `not`, `is`, `is not`,
    /// `at` and `contains`, so the last of those is a real operator token
    /// and the one identifier §14G.7.7's accounting spends on the library.
    #[test]
    fn contains_is_the_one_operator_the_library_adds() {
        assert_eq!(
            kinds("body contains query"),
            vec![
                RawToken::Kw(TokenKind::Ident("body".into())),
                RawToken::Kw(TokenKind::Contains),
                RawToken::Kw(TokenKind::Ident("query".into())),
            ]
        );
    }

    /// `first` means the pipeline clause only where the clause is, which
    /// is what lets `prelude/list.zd` declare `function first of items`
    /// and `prelude/number.zd` write `min with first, second`.
    #[test]
    fn first_is_a_keyword_only_directly_after_take() {
        assert_eq!(
            kinds("take first 25"),
            vec![
                RawToken::Kw(TokenKind::Take),
                RawToken::Kw(TokenKind::First),
                RawToken::Kw(TokenKind::Number(25.0)),
            ]
        );
        assert_eq!(
            kinds("function first of items"),
            vec![
                RawToken::Kw(TokenKind::Function),
                RawToken::Kw(TokenKind::Ident("first".into())),
                RawToken::Kw(TokenKind::Of),
                RawToken::Kw(TokenKind::Ident("items".into())),
            ]
        );
        assert_eq!(
            kinds("min with first, second"),
            vec![
                RawToken::Kw(TokenKind::Ident("min".into())),
                RawToken::Kw(TokenKind::With),
                RawToken::Kw(TokenKind::Ident("first".into())),
                RawToken::Kw(TokenKind::Comma),
                RawToken::Kw(TokenKind::Ident("second".into())),
            ]
        );
    }

    /// The `foreign` grammar needs five more words, and §14G.7.7 budgets
    /// one. They stay ordinary identifiers, and the parser asks this table
    /// whether the word in front of it is the one that construct wants —
    /// so the spelling is still the lexer's, and a dialect replaces it
    /// with the other two.
    #[test]
    fn the_foreign_grammar_reserves_nothing() {
        for word in ["foreign", "as", "takes", "gives", "anywhere"] {
            assert_eq!(
                kinds(word),
                vec![RawToken::Kw(TokenKind::Ident(word.into()))],
                "`{word}` must stay available as a name"
            );
            assert!(
                word_to_soft_keyword(word).is_some(),
                "`{word}` must be recognisable where the grammar wants it"
            );
        }
        assert_eq!(word_to_soft_keyword("item"), None);
    }

    #[test]
    fn every_soft_keyword_round_trips_through_its_spelling() {
        for word in ["foreign", "as", "takes", "gives", "anywhere"] {
            let soft = word_to_soft_keyword(word).expect("a soft keyword");
            assert_eq!(soft.spelling(), word);
        }
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
