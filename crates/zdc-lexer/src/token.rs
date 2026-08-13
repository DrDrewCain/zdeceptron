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
    /// `trusted state orders …`, `takes key is trusted Text`,
    /// `gives trusted Text`, and a release's endorsement clause — spec
    /// §18.1.1 and §19.10.2. One word in four slots, all of them
    /// declarations.
    Trusted,
    /// `unique id is Whole` — a record field that is the row's identity,
    /// spec §14G.7.7 and issue #2.
    ///
    /// A hard keyword rather than a soft one, and that is what buys the
    /// grammar: `field := ["unique"] IDENT "is" type` is LL(1) only
    /// because the leader cannot also be a field name. §14G.7.7 records
    /// that this is why `key` was rejected for the same job — `key is
    /// Text` is a plausible field and `unique is Text` is not.
    Unique,
    /// `release judge with guess, answer` — spec §19.1.
    Release,
    /// `limit 10 per visitor` — spec §19.1.
    Limit,
    State,
    Function,
    View,
    Record,
    Choice,
    Component,
    Use,
    /// `route` — the declaration that names a site's URLs (spec §14G.2).
    Route,

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
    /// `body contains query` — the one operator §14F.1 adds to the closed
    /// infix set, and the only word §17.4.2 reserves for it.
    Contains,
    Yes,
    No,
    Empty,
    Environment,
    /// `address` — the URL this document was served at (spec §14G.2).
    Address,
    /// §14C.3b, and the mechanism that makes it reachable. `build read
    /// "content/hello.md"` asks the *compiler* for a capability rather
    /// than importing a module, because a build-time call has no host —
    /// the compiler is the host. The capability name that follows is an
    /// identifier in a closed set, not a keyword, so the set can grow
    /// without spending another word from §14G.7.7's budget.
    Build,
    /// `media "(prefers-color-scheme: dark)"` — whether the browser
    /// matches a CSS media query, as a `Truth` that changes when the
    /// browser's answer changes.
    ///
    /// The operand is a text literal rather than an expression, because
    /// `matchMedia` subscribes to one query for the life of the page: a
    /// query that varied would have to re-subscribe, and nothing in the
    /// language says when that would happen.
    Media,
    /// `scroll` — how far down the document the reader is, as a `Decimal`
    /// from 0 to 100 that changes when they move.
    ///
    /// A soft keyword, exactly as `media` is: it means this immediately
    /// after `from` and stays an ordinary name everywhere else, so a signal
    /// called `scroll` is still a signal called `scroll`.
    Scroll,

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
            Release => "release",
            Limit => "limit",
            State => "state",
            Function => "function",
            View => "view",
            Record => "record",
            Choice => "choice",
            Component => "component",
            Use => "use",
            Route => "route",
            For => "for",
            Children => "children",
            Client => "client",
            Static => "static",
            Server => "server",
            Durable => "durable",
            Unique => "unique",
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
            Contains => "contains",
            Yes => "yes",
            No => "no",
            Empty => "empty",
            Environment => "environment",
            Address => "address",
            Build => "build",
            Media => "media",
            Scroll => "scroll",
            Number(_) | Text(_) | Ident(_) | Plus | Minus | Star | Slash | Less | Greater
            | LessEq | GreaterEq | Comma | Dot | LParen | RParen | LBracket | RBracket
            | Newline | Indent | Dedent | Eof => return None,
        })
    }

    /// What this keyword does in the grammar, as a clause that completes
    /// "`from` introduces a pipeline's source".
    ///
    /// A keyword may not be a name, and until this existed the diagnostic
    /// for writing one in a name position said only that the word was a
    /// keyword, which is the part the reader can already see. Saying what
    /// the word is spent on turns "why not?" into "of course", and it is a
    /// fact about the token rather than about the position, so it lives
    /// beside the spelling and is written once.
    ///
    /// Returns `None` for literals, punctuation, and layout tokens, which
    /// have no role to describe: a caller reaching this with one of those
    /// is not looking at a keyword.
    ///
    /// The phrasing is English because [`TokenKind::keyword_spelling`] is:
    /// a dialect (spec §4.6) replaces both together.
    pub fn keyword_role(&self) -> Option<&'static str> {
        use TokenKind::*;
        Some(match self {
            Secret => "marks state the browser may never observe",
            Trusted => "marks a value the program vouches for",
            Release => "begins a declaration that deliberately discloses a secret",
            Limit => "caps how often one session may evaluate a release",
            State => "begins a state declaration",
            Function => "begins a function declaration",
            View => "begins the declaration of the page",
            Record => "begins a record declaration",
            Unique => "marks the record field that is a row's identity",
            Choice => "begins a choice declaration",
            Component => "begins a component declaration",
            Use => "begins an import",
            Route => "begins the declaration that names a site's URLs",
            For => "names what an import borrows",
            Children => "stands for the nodes nested under a component at its call site",
            Client => "places state in browser memory",
            Static => "places a value in the build",
            Server => "places state in a serverless invocation",
            Durable => "places state in persistent storage",
            Starting => "gives state its initial value",
            Emitting => "writes a build-time value into a file in the bundle",
            From => "introduces a pipeline's source, and derives state from other state",
            Of => "joins a type constructor to the type it holds",
            To => "pairs a key with a value, in a map literal and in a map's type",
            Give => "returns a value from a function",
            Set => "writes a value into state",
            Add => "adds to state",
            Subtract => "subtracts from state",
            Append => "appends to a list in state",
            Remove => "removes from a collection in state",
            Keep => "filters a pipeline",
            Sort => "orders a pipeline",
            MapEach => "rewrites every element of a pipeline",
            Take => "shortens a pipeline",
            First => "counts what `take` keeps",
            Where => "carries the condition `keep` filters by",
            By => "carries the ordering `sort` uses",
            When => "matches a choice, one arm per variant",
            Each => "repeats a view node over a list",
            In => "names the list `each` repeats over",
            If => "chooses between two branches",
            Otherwise => "introduces an `if`'s second branch",
            Show => "introduces what a `when` arm draws",
            On => "attaches a handler to an event",
            With => "introduces the arguments of a call and the fields of a variant",
            And => "joins two conditions that must both hold",
            Or => "joins two conditions of which either may hold",
            Not => "negates a condition",
            Is => "separates a name from what it is, in a declaration and in an argument",
            IsNot => "compares two values for difference",
            At => "names one entry of a map or a list",
            Contains => "asks whether a collection holds a value",
            Yes => "is the true value",
            No => "is the false value",
            Empty => "is the empty collection",
            Environment => "reads a value out of a serverless invocation's environment",
            Address => "is the URL this document was served at",
            Build => "asks the compiler for something while it is compiling",
            Media => "asks the browser whether it matches a CSS media query",
            Scroll => "asks the browser how far down the document the reader is",
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
            Number(_) | Text(_) | Ident(_) | Secret | Trusted | Unique | Release | Limit
            | State | Function | View | Record | Choice | Component | Use | Route | For
            | Children | Client | Static | Server | Durable | Starting | Emitting | From | Of
            | To | Give | Set | Add | Subtract | Append | Remove | Keep | Sort | MapEach | Take
            | First | Where | By | When | Each | In | If | Otherwise | Show | On | With | And
            | Or | Not | Is | IsNot | At | Contains | Yes | No | Empty | Environment | Address
            | Build | Media | Scroll | Newline | Indent | Dedent | Eof => return None,
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
            (TokenKind::Release, "release"),
            (TokenKind::Limit, "limit"),
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
            (TokenKind::Contains, "contains"),
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

    /// Every word the lexer reserves can say what it is reserved for.
    ///
    /// The word list is read out of `word_to_kind`'s own source rather than
    /// written here, because a hand-copied list is correct on the day it is
    /// written and silently short on the day a word is added — which is the
    /// day this test needed to fail. Adding a keyword without a role is
    /// then a failing test rather than a diagnostic that trails off.
    #[test]
    fn every_reserved_word_says_what_it_is_reserved_for() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/raw.rs"),
        )
        .expect("the lexer's own source is readable");

        // `word_to_kind`'s body only. A soft keyword is an ordinary name
        // everywhere but one construct, so it is not reserved and has no
        // role to give; reading the whole file would sweep those in.
        let body = {
            let opens = source
                .find("fn word_to_kind(")
                .expect("`word_to_kind` is in the lexer's source");
            let closes = source[opens..]
                .find("\n}\n")
                .expect("`word_to_kind` has an end");
            &source[opens..opens + closes]
        };

        let words: Vec<String> = body
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix('"')?;
                let (word, tail) = rest.split_once('"')?;
                tail.trim_start().strip_prefix("=>")?;
                Some(word.to_string())
            })
            .collect();

        // Non-vacuity: a scan that matched nothing would otherwise report
        // that every keyword has a role.
        assert!(
            words.len() >= 50,
            "the scan found only {} reserved words, so it stopped reading \
             `word_to_kind` rather than the table shrinking: {words:?}",
            words.len()
        );

        // `word_to_kind` rather than `tokenize`, because `first` is a
        // keyword only after `take` and lexing it alone yields a name.
        for word in &words {
            let role = crate::raw::word_to_kind(word)
                .keyword_role()
                .unwrap_or_else(|| panic!("`{word}` is reserved and has no role"));
            assert!(
                !role.is_empty(),
                "`{word}` has an empty role, which says no more than naming it does"
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
