use crate::Span;

/// Every token the language can produce.
///
/// Keyword variants carry no text: their spelling is dialect-supplied
/// (spec §4.6), so nothing downstream may assume the English form.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Number(f64),
    Text(String),
    Ident(String),

    // Declaration keywords
    Secret,
    State,
    Function,
    View,
    Note,

    // Placement keywords
    Client,
    Server,
    Durable,

    // Initializer keywords
    Starting,
    From,

    // Type keywords
    Of,
    To,

    // Statement keywords
    Give,
    Set,
    Add,
    Subtract,
    Keep,
    Sort,
    MapEach,
    Take,
    First,
    Where,
    By,
    When,
    Each,
    In,
    If,
    Otherwise,
    Show,
    On,
    With,

    // Expression keywords
    And,
    Or,
    Not,
    Is,
    IsNot,
    At,
    Yes,
    No,
    Empty,
    Environment,

    // Symbol operators (retained per spec §4.2)
    Plus,
    Minus,
    Star,
    Slash,
    Less,
    Greater,
    LessEq,
    GreaterEq,

    // Punctuation
    Comma,
    Dot,
    LParen,
    RParen,

    // Layout
    Newline,
    Indent,
    Dedent,
    Eof,
}

impl TokenKind {
    /// The one valid spelling of this keyword in the `english` dialect.
    ///
    /// Returns `None` for literals, punctuation, and layout tokens.
    /// Diagnostics use this to name the single correct form (spec §4.1).
    pub fn keyword_spelling(&self) -> Option<&'static str> {
        use TokenKind::*;
        Some(match self {
            Secret => "secret",
            State => "state",
            Function => "function",
            View => "view",
            Note => "note",
            Client => "client",
            Server => "server",
            Durable => "durable",
            Starting => "starting",
            From => "from",
            Of => "of",
            To => "to",
            Give => "give",
            Set => "set",
            Add => "add",
            Subtract => "subtract",
            Keep => "keep",
            Sort => "sort",
            MapEach => "map",
            Take => "take",
            First => "first",
            Where => "where",
            By => "by",
            When => "when",
            Each => "each",
            In => "in",
            If => "if",
            Otherwise => "otherwise",
            Show => "show",
            On => "on",
            With => "with",
            And => "and",
            Or => "or",
            Not => "not",
            Is => "is",
            IsNot => "is not",
            At => "at",
            Yes => "yes",
            No => "no",
            Empty => "empty",
            Environment => "environment",
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Token { kind, span }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_report_their_single_spelling() {
        assert_eq!(TokenKind::Starting.keyword_spelling(), Some("starting"));
        assert_eq!(TokenKind::IsNot.keyword_spelling(), Some("is not"));
        assert_eq!(TokenKind::Ident("x".into()).keyword_spelling(), None);
    }

    #[test]
    fn layout_tokens_are_not_keywords() {
        assert_eq!(TokenKind::Indent.keyword_spelling(), None);
        assert_eq!(TokenKind::Eof.keyword_spelling(), None);
    }
}
