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
    /// The integrity direction of the lattice (spec §18.1.1).
    ///
    /// `secret` answers *who may learn this value*; `trusted` answers *who
    /// chose it*. One word in the slots `secret` already occupies, so
    /// `stateDecl` stays LL(1) at its decision point.
    Trusted,
    State,
    Function,
    View,
    Record,
    Choice,
    Component,
    Use,

    // Module keywords
    For,

    /// The nodes nested under a component at its call site (spec §14D.1).
    ///
    /// A keyword rather than a conventional parameter name, so a dialect
    /// relocates it with the rest of the language rather than leaving one
    /// English word wired into the parser (spec §4.6).
    Children,

    // Placement keywords
    Client,
    /// §14C.3b. Build-time state: evaluated once by `zdc build` and
    /// inlined into the bundle, so reading it from the browser crosses no
    /// boundary at all.
    Static,
    Server,
    Durable,

    // Initializer keywords
    Starting,
    From,
    /// §14C.3b. The build-time output clause: the value of a `static`
    /// signal, written to a path in the bundle rather than only read from
    /// it. `rss.xml` and `llms.txt` are files, not endpoints.
    Emitting,

    // Type keywords
    Of,
    To,

    // Statement keywords
    Give,
    Set,
    Add,
    Subtract,
    Append,
    Remove,
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
    /// §14C.3b, and the mechanism that makes it reachable. `build read
    /// "content/hello.md"` asks the *compiler* for a capability rather
    /// than importing a module, because a build-time call has no host —
    /// the compiler is the host. The capability name that follows is an
    /// identifier in a closed set, not a keyword, so the set can grow
    /// without spending another word from §14G.7.7's budget.
    Build,

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
    LBracket,
    RBracket,

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
            Trusted => "trusted",
            State => "state",
            Function => "function",
            View => "view",
            Record => "record",
            Choice => "choice",
            Component => "component",
            Use => "use",
            For => "for",
            Children => "children",
            Client => "client",
            Static => "static",
            Server => "server",
            Durable => "durable",
            Starting => "starting",
            Emitting => "emitting",
            From => "from",
            Of => "of",
            To => "to",
            Give => "give",
            Set => "set",
            Add => "add",
            Subtract => "subtract",
            Append => "append",
            Remove => "remove",
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
            Build => "build",
            Number(_) | Text(_) | Ident(_) | Plus | Minus | Star | Slash | Less | Greater
            | LessEq | GreaterEq | Comma | Dot | LParen | RParen | LBracket | RBracket
            | Newline | Indent | Dedent | Eof => return None,
        })
    }

    /// The surface spelling of a punctuation or literal token, for diagnostics.
    ///
    /// Returns `None` for layout tokens (`Newline`, `Indent`, `Dedent`, `Eof`)
    /// and for tokens carrying user text, which callers describe differently.
    pub fn punctuation_spelling(&self) -> Option<&'static str> {
        use TokenKind::*;
        Some(match self {
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Less => "<",
            Greater => ">",
            LessEq => "<=",
            GreaterEq => ">=",
            Comma => ",",
            Dot => ".",
            LParen => "(",
            RParen => ")",
            LBracket => "[",
            RBracket => "]",
            Number(_) | Text(_) | Ident(_) | Secret | Trusted | State | Function | View
            | Record | Choice | Component | Use | For | Children | Client | Static | Server
            | Durable | Starting | Emitting | From | Of | To | Give | Set | Add | Subtract
            | Append | Remove | Keep | Sort | MapEach | Take | First | Where | By | When | Each
            | In | If | Otherwise | Show | On | With | And | Or | Not | Is | IsNot | At | Yes
            | No | Empty | Environment | Build | Newline | Indent | Dedent | Eof => return None,
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

    #[test]
    fn all_keyword_variants_have_correct_spelling() {
        let keywords: &[(TokenKind, &str)] = &[
            // Declaration keywords
            (TokenKind::Secret, "secret"),
            (TokenKind::Trusted, "trusted"),
            (TokenKind::State, "state"),
            (TokenKind::Function, "function"),
            (TokenKind::View, "view"),
            (TokenKind::Record, "record"),
            (TokenKind::Choice, "choice"),
            (TokenKind::Component, "component"),
            (TokenKind::Use, "use"),
            (TokenKind::For, "for"),
            (TokenKind::Children, "children"),
            // Placement keywords
            (TokenKind::Client, "client"),
            (TokenKind::Static, "static"),
            (TokenKind::Server, "server"),
            (TokenKind::Durable, "durable"),
            // Initializer keywords
            (TokenKind::Starting, "starting"),
            (TokenKind::Emitting, "emitting"),
            (TokenKind::From, "from"),
            // Type keywords
            (TokenKind::Of, "of"),
            (TokenKind::To, "to"),
            // Statement keywords
            (TokenKind::Give, "give"),
            (TokenKind::Set, "set"),
            (TokenKind::Add, "add"),
            (TokenKind::Subtract, "subtract"),
            (TokenKind::Append, "append"),
            (TokenKind::Remove, "remove"),
            (TokenKind::Keep, "keep"),
            (TokenKind::Sort, "sort"),
            (TokenKind::MapEach, "map"),
            (TokenKind::Take, "take"),
            (TokenKind::First, "first"),
            (TokenKind::Where, "where"),
            (TokenKind::By, "by"),
            (TokenKind::When, "when"),
            (TokenKind::Each, "each"),
            (TokenKind::In, "in"),
            (TokenKind::If, "if"),
            (TokenKind::Otherwise, "otherwise"),
            (TokenKind::Show, "show"),
            (TokenKind::On, "on"),
            (TokenKind::With, "with"),
            // Expression keywords
            (TokenKind::And, "and"),
            (TokenKind::Or, "or"),
            (TokenKind::Not, "not"),
            (TokenKind::Is, "is"),
            (TokenKind::IsNot, "is not"),
            (TokenKind::At, "at"),
            (TokenKind::Yes, "yes"),
            (TokenKind::No, "no"),
            (TokenKind::Empty, "empty"),
            (TokenKind::Environment, "environment"),
            (TokenKind::Build, "build"),
        ];

        for (variant, expected_spelling) in keywords {
            assert_eq!(
                variant.keyword_spelling(),
                Some(*expected_spelling),
                "keyword variant {:?} should have spelling '{}'",
                variant,
                expected_spelling
            );
        }
    }

    #[test]
    fn non_keyword_variants_return_none() {
        let non_keywords = [
            TokenKind::Number(1.0),
            TokenKind::Text("hello".into()),
            TokenKind::Ident("foo".into()),
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Less,
            TokenKind::Greater,
            TokenKind::LessEq,
            TokenKind::GreaterEq,
            TokenKind::Comma,
            TokenKind::Dot,
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBracket,
            TokenKind::RBracket,
            TokenKind::Newline,
            TokenKind::Indent,
            TokenKind::Dedent,
            TokenKind::Eof,
        ];

        for variant in &non_keywords {
            assert_eq!(
                variant.keyword_spelling(),
                None,
                "non-keyword variant {:?} should return None",
                variant
            );
        }
    }

    #[test]
    fn punctuation_variants_report_their_symbol() {
        let punctuation: &[(TokenKind, &str)] = &[
            (TokenKind::Plus, "+"),
            (TokenKind::Minus, "-"),
            (TokenKind::Star, "*"),
            (TokenKind::Slash, "/"),
            (TokenKind::Less, "<"),
            (TokenKind::Greater, ">"),
            (TokenKind::LessEq, "<="),
            (TokenKind::GreaterEq, ">="),
            (TokenKind::Comma, ","),
            (TokenKind::Dot, "."),
            (TokenKind::LParen, "("),
            (TokenKind::RParen, ")"),
            (TokenKind::LBracket, "["),
            (TokenKind::RBracket, "]"),
        ];

        for (variant, expected_symbol) in punctuation {
            assert_eq!(
                variant.punctuation_spelling(),
                Some(*expected_symbol),
                "punctuation variant {:?} should have symbol '{}'",
                variant,
                expected_symbol
            );
        }
    }

    #[test]
    fn layout_tokens_have_no_punctuation_spelling() {
        assert_eq!(TokenKind::Newline.punctuation_spelling(), None);
        assert_eq!(TokenKind::Indent.punctuation_spelling(), None);
        assert_eq!(TokenKind::Dedent.punctuation_spelling(), None);
        assert_eq!(TokenKind::Eof.punctuation_spelling(), None);
    }

    #[test]
    fn keywords_and_literals_have_no_punctuation_spelling() {
        assert_eq!(TokenKind::State.punctuation_spelling(), None);
        assert_eq!(TokenKind::IsNot.punctuation_spelling(), None);
        assert_eq!(TokenKind::Ident("x".into()).punctuation_spelling(), None);
        assert_eq!(TokenKind::Number(1.0).punctuation_spelling(), None);
        assert_eq!(TokenKind::Text("hi".into()).punctuation_spelling(), None);
    }
}
