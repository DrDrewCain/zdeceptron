use crate::codes;
use zdc_lexer::{SoftKeyword, Span, Token, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
    /// What the caret says about the span, when this site knows something
    /// worth saying.
    ///
    /// The renderer used to print the word `here` under every caret in the
    /// compiler, which is where the caret already is. A site that knows it
    /// is looking at a type, or at a keyword, or at the wrong kind of
    /// literal, can spend that line instead. `None` falls back to the
    /// code's own label rather than to a generic phrase.
    pub label: Option<String>,
    /// An edit that would make this line parse, for the sites that can
    /// name one exactly.
    pub suggestion: Option<Suggestion>,
    /// The rule this error is an instance of.
    ///
    /// Not an `Option`: every parse error is explainable, and the field is
    /// required so that a new error cannot be added without deciding which
    /// rule it belongs to. Parse errors were the ones a beginner hits
    /// first and the only ones `zdc explain` could not answer for.
    pub code: &'static str,
}

/// One edit, expressed against the source.
///
/// The parser knows the byte range and the text that belongs in it, and
/// the renderer turns the pair into the whole corrected line. Carrying the
/// edit rather than a formatted string is what lets the shown line be the
/// reader's own, and is the value an editor's quick fix would apply.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// The byte range replaced. An empty range is an insertion.
    pub span: Span,
    /// What goes in that range.
    pub replacement: String,
}

impl ParseError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>, span: Span) -> ParseError {
        ParseError {
            message: message.into(),
            span,
            label: None,
            suggestion: None,
            code,
        }
    }

    /// Say what the caret is pointing at.
    pub(crate) fn labelled(mut self, label: impl Into<String>) -> ParseError {
        self.label = Some(label.into());
        self
    }

    /// Name the edit that would make the line parse.
    pub(crate) fn suggesting(mut self, span: Span, replacement: impl Into<String>) -> ParseError {
        self.suggestion = Some(Suggestion {
            span,
            replacement: replacement.into(),
        });
        self
    }
}

/// A kind of nesting the parser counts, so that source which nests
/// further than any program does is reported rather than followed.
///
/// Recursive descent turns nesting in the source into frames on the
/// stack, and running out of stack is not an error a user ever sees: it
/// raises `SIGABRT`, which cannot be caught or unwound, so the process
/// dies with no diagnostic at all. A truncated download, or a file that
/// is not ZDeceptron at all, can nest thousands deep.
///
/// The limits differ because the frames do, and both are **measured
/// rather than guessed**. In an unoptimised build a level of expression
/// now costs roughly 8 KB of stack and a level of indentation roughly
/// 2 KB — the reverse of the ratio these limits were first set from,
/// because an expression frame grew every time the grammar did while a
/// block frame did not.
///
/// They were 256 and 64, chosen when a level of expression was said to
/// cost under 2 KB. At 8 KB the worst case — indentation nested to its
/// limit with an expression nested to its limit inside the innermost
/// block — needed more than the 2 MiB a default thread stack has, so the
/// guard aborted in exactly the case it exists to report. Re-derived: the
/// worst case at 32 and 96 fits inside 1.5 MiB, which leaves a 2 MiB
/// stack a quarter of itself spare.
///
/// Both are still far above anything a person writes: 32 levels of
/// indentation is 128 spaces of it, and 96 levels of expression nesting
/// is not reachable without generating the file.
#[derive(Clone, Copy)]
pub(crate) enum Nesting {
    Expression,
    Type,
    Block,
}

impl Nesting {
    fn limit(self) -> usize {
        match self {
            Nesting::Expression | Nesting::Type => 96,
            Nesting::Block => 32,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Nesting::Expression => "expression",
            Nesting::Type => "type",
            Nesting::Block => "indented block",
        }
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// The span of the last consumed token that carries source text.
    last_end: Span,
    /// How many expressions and types are currently being parsed.
    expr_depth: usize,
    /// How many indented blocks are currently being parsed.
    block_depth: usize,
    /// Whether the expression being parsed is an argument's value.
    ///
    /// Argument lists are comma-separated and `with` takes a
    /// comma-separated list of its own, so `Link Photo with album is slug,
    /// padding is 8` has two readings: `padding is 8` is either a second
    /// argument to `Link` or a second argument to `Photo`. Spec §14G.1.1
    /// resolves it by requiring the parentheses — `Link (Photo with album
    /// is slug), padding is 8` — rather than by picking a winner, because
    /// §4.1 admits exactly one phrasing per construct and a phrasing with
    /// two meanings is the same defect from the other side.
    ///
    /// Explicit parentheses clear this, so a call may nest as deeply as it
    /// likes as long as each level says where it ends.
    in_argument_value: bool,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        assert!(!tokens.is_empty(), "token stream always ends with Eof");
        let start = tokens[0].span.start;
        Parser {
            tokens,
            pos: 0,
            last_end: Span::new(start, start),
            expr_depth: 0,
            block_depth: 0,
            in_argument_value: false,
        }
    }

    /// Whether the expression currently being parsed is an argument's
    /// value, and therefore may not introduce a call with a bare `with`
    /// (spec §14G.1.1).
    pub(crate) fn in_argument_value(&self) -> bool {
        self.in_argument_value
    }

    /// Set the argument-position restriction, returning the previous
    /// value so the caller can restore it.
    ///
    /// Returning the old value rather than a plain setter is what lets
    /// parentheses nest: each `(` clears the restriction and restores
    /// whatever was in force outside it, so `f with a is (g with b is
    /// (h with c))` is legal at every level.
    pub(crate) fn set_argument_value(&mut self, value: bool) -> bool {
        std::mem::replace(&mut self.in_argument_value, value)
    }

    pub fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos.min(self.tokens.len() - 1)].kind
    }

    pub fn peek_span(&self) -> Span {
        self.tokens[self.pos.min(self.tokens.len() - 1)].span
    }

    pub fn peek_at(&self, offset: usize) -> &TokenKind {
        let index = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[index].kind
    }

    pub fn at(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(kind)
    }

    /// How far through the stream the cursor is.
    ///
    /// Only error recovery reads this, and it reads it for one thing:
    /// whether a failed parse consumed anything. A recovery loop that
    /// cannot tell is a recovery loop that can spin.
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    /// Whether the token here is the first on its line.
    ///
    /// True at the start of the file, and after the layout tokens that
    /// end a line — `Newline`, and the `Indent` or `Dedent` run that a
    /// `Newline` introduces. Used by recovery to tell a declaration
    /// keyword that opens a line from one that appears inside it.
    pub(crate) fn at_line_start(&self) -> bool {
        match self.pos.checked_sub(1) {
            None => true,
            Some(previous) => matches!(
                self.tokens[previous].kind,
                TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent
            ),
        }
    }

    pub fn bump(&mut self) -> Token {
        let token = self.tokens[self.pos.min(self.tokens.len() - 1)].clone();
        if !is_layout(&token.kind) {
            self.last_end = token.span;
        }
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        token
    }

    /// The span of the most recently consumed token that carries source
    /// text, ignoring layout.
    ///
    /// A layout token's span is not part of the construct it closes: a
    /// `Newline` or `Dedent` carries the line break *and the following
    /// line's indentation*, so a node whose span ended at one would run
    /// past its own last character and into the gap before its next
    /// sibling. Ending at the last real token keeps the span tree a tree.
    pub(crate) fn last_span(&self) -> Span {
        self.last_end
    }

    /// Consume the token if it matches, reporting whether it did.
    pub fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Consume the token or fail with a message naming the single valid
    /// form, as required by spec §4.1.
    pub fn expect(&mut self, kind: TokenKind, context: &str) -> Result<Token, ParseError> {
        if self.at(&kind) {
            return Ok(self.bump());
        }
        let expected = describe_expected(&kind);
        let found = describe_found(self.peek());
        // The sentence "ZDeceptron has exactly one way to write this" used
        // to end this message. It is the rule rather than the claim, it
        // was the same on every one of these, and it is now one line of
        // `zdc explain E0103` away. What replaces it on the caret is the
        // part that differs: what is actually written there.
        Err(ParseError::new(
            codes::ONE_VALID_FORM,
            format!("Expected {expected} {context}."),
            self.peek_span(),
        )
        .labelled(format!("{expected} belongs here, and this is {found}")))
    }

    /// Consume a name, or explain why what is written cannot be one.
    ///
    /// The keyword case is separated because it is a different mistake
    /// with a different repair. A keyword in a name position is not a
    /// misspelling: the word is one of the sixty-odd the grammar has
    /// already spent, and several of them — `from`, `to`, `route`,
    /// `limit` — are the natural name for a piece of data. Naming the
    /// keyword and stopping told the reader what they could already see,
    /// so the message states the rule and says what the word is spent on.
    pub fn expect_ident(&mut self, context: &str) -> Result<zdc_ast::Ident, ParseError> {
        let span = self.peek_span();
        let found = self.peek().clone();
        if let TokenKind::Ident(text) = found {
            self.bump();
            return Ok(zdc_ast::Ident { text, span });
        }
        if let (Some(word), Some(role)) = (found.keyword_spelling(), found.keyword_role()) {
            return Err(ParseError::new(
                codes::KEYWORD_AS_NAME,
                format!("Expected a name {context}. No keyword may be a name: `{word}` {role}."),
                span,
            )
            .labelled(format!("`{word}` is a keyword, so it cannot be a name")));
        }
        Err(ParseError::new(
            codes::ONE_VALID_FORM,
            format!(
                "Expected a name {context}, found {}.",
                describe_found(&found)
            ),
            span,
        )
        .labelled("a name belongs here"))
    }

    pub fn expect_text(&mut self, context: &str) -> Result<String, ParseError> {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::Text(value) => {
                self.bump();
                Ok(value)
            }
            other => Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "Expected quoted text {context}, found {}.",
                    describe_found(&other)
                ),
                span,
            )
            .labelled("quoted text belongs here")),
        }
    }

    /// Whether the current token is the word a construct expects here.
    ///
    /// A soft keyword is an ordinary `Ident` everywhere else (see
    /// [`zdc_lexer::word_to_soft_keyword`]), so this is how the `foreign`
    /// grammar reads `takes` and `gives` without taking either word away
    /// from programs that want it as a name.
    pub(crate) fn at_soft(&self, word: SoftKeyword) -> bool {
        match self.peek() {
            TokenKind::Ident(text) => zdc_lexer::word_to_soft_keyword(text) == Some(word),
            _ => false,
        }
    }

    /// [`Cursor::at_soft`] `offset` tokens ahead.
    ///
    /// Two-token lookahead, for the one place a soft keyword is only the
    /// construct when the *next* word confirms it: `durable per visitor`
    /// is a placement refusal and `durable per` is a signal named `per`.
    /// Committing to the first word alone would turn the second case into
    /// the first case's diagnostic.
    pub(crate) fn at_soft_at(&self, offset: usize, word: SoftKeyword) -> bool {
        match self.peek_at(offset) {
            TokenKind::Ident(text) => zdc_lexer::word_to_soft_keyword(text) == Some(word),
            _ => false,
        }
    }

    pub(crate) fn eat_soft(&mut self, word: SoftKeyword) -> bool {
        if self.at_soft(word) {
            self.bump();
            true
        } else {
            false
        }
    }

    pub(crate) fn expect_soft(
        &mut self,
        word: SoftKeyword,
        context: &str,
    ) -> Result<(), ParseError> {
        if self.eat_soft(word) {
            return Ok(());
        }
        Err(ParseError::new(
            codes::ONE_VALID_FORM,
            format!(
                "Expected `{}` {context}, found {}.",
                word.spelling(),
                describe_found(self.peek())
            ),
            self.peek_span(),
        )
        .labelled(format!("`{}` belongs here", word.spelling())))
    }

    /// Skip layout tokens that carry no meaning at this position.
    pub fn skip_newlines(&mut self) {
        while self.at(&TokenKind::Newline) {
            self.bump();
        }
    }

    /// Parse `f` one level deeper, reporting an error rather than
    /// recursing past the limit for this kind of nesting.
    ///
    /// The depth is restored whether `f` succeeds or fails, so an error
    /// on one path does not leak a level onto the next.
    pub(crate) fn nested<T>(
        &mut self,
        kind: Nesting,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        self.deepen(kind)?;
        let parsed = f(self);
        *self.depth_mut(kind) -= 1;
        parsed
    }

    /// Charge one level of `kind` without opening a frame for it.
    ///
    /// **What `nested` alone does not bound.** It counts the parser's own
    /// recursion, and a left-associative loop grows the tree one level per
    /// iteration while staying in a single frame: `1 + 1 + …` and
    /// `x.f.f.f…` both parse at depth 1 and produce a spine as long as the
    /// source. Every later pass — lowering, inference, the graph passes,
    /// emission — walks that spine recursively, so the abort this guard
    /// exists to prevent simply moved out of the parser and into whichever
    /// crate walked first. Twenty thousand `+` did exactly that.
    ///
    /// The caller charges a level per iteration and hands the budget back
    /// with [`Parser::unwind_to`] once the spine is built, which bounds the
    /// **tree** rather than the frames that happened to build it.
    pub(crate) fn deepen(&mut self, kind: Nesting) -> Result<(), ParseError> {
        if self.depth(kind) >= kind.limit() {
            return Err(ParseError::new(
                codes::TOO_DEEP,
                format!(
                    "This {} is nested more than {} levels deep. Give the inner parts names and \
                     refer to them instead.",
                    kind.noun(),
                    kind.limit()
                ),
                self.peek_span(),
            )
            .labelled(format!("the {} reaches its limit here", kind.noun())));
        }
        *self.depth_mut(kind) += 1;
        Ok(())
    }

    /// The current depth, to be handed back to [`Parser::unwind_to`].
    pub(crate) fn depth_mark(&self, kind: Nesting) -> usize {
        self.depth(kind)
    }

    /// Give back every level charged since `mark`, on the failing path as
    /// well as the succeeding one.
    pub(crate) fn unwind_to(&mut self, kind: Nesting, mark: usize) {
        *self.depth_mut(kind) = mark;
    }

    fn depth(&self, kind: Nesting) -> usize {
        match kind {
            Nesting::Expression | Nesting::Type => self.expr_depth,
            Nesting::Block => self.block_depth,
        }
    }

    fn depth_mut(&mut self, kind: Nesting) -> &mut usize {
        match kind {
            Nesting::Expression | Nesting::Type => &mut self.expr_depth,
            Nesting::Block => &mut self.block_depth,
        }
    }

    /// A newline-introduced, indented run of items: the one place the
    /// language's block structure is implemented.
    ///
    /// Statements, view nodes, and both flavours of match arm are all
    /// indented runs, and previously each parsed its own — which is how
    /// the statement side came to compute a correct span and the view
    /// side an over-running one. `before` and `to_open` name what the
    /// block belongs to, so error messages stay specific.
    ///
    /// The returned span runs from the line break that opens the block to
    /// the last character of its last item, never to the `Dedent` that
    /// closes it (see `last_span`).
    pub(crate) fn indented<T>(
        &mut self,
        before: &str,
        to_open: &str,
        mut item: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<(Vec<T>, Span), ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Newline, before)?;
        let open = self.peek_span();
        self.expect(TokenKind::Indent, to_open)?;

        let items = self.nested(Nesting::Block, |p| {
            let mut items = Vec::new();
            loop {
                p.skip_newlines();
                if p.at(&TokenKind::Dedent) || p.at(&TokenKind::Eof) {
                    break;
                }
                items.push(item(p)?);
            }
            Ok(items)
        })?;

        // An `Indent` is only emitted for a line that has content, so an
        // empty block cannot arise from real source; fall back to the
        // block's opening position rather than to an unrelated token.
        let end = if items.is_empty() {
            open
        } else {
            self.last_span()
        };
        self.eat(&TokenKind::Dedent);
        Ok((items, start.to(end)))
    }
}

/// Layout tokens stand for the shape of the file, not for characters a
/// construct owns.
fn is_layout(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
    )
}

/// A user-facing name for a token kind, for use in "expected ..." messages.
///
/// Keywords and punctuation get their single valid spelling, backtick-quoted.
/// Layout tokens and literal-carrying tokens get a plain-English description
/// instead of a symbol, since they have no surface spelling a user would
/// type. No arm may fall back to `{:?}` — an enum variant name must never
/// reach a user-facing string (spec §7.3).
fn describe_expected(kind: &TokenKind) -> String {
    if let Some(word) = kind.keyword_spelling() {
        return format!("`{word}`");
    }
    if let Some(symbol) = kind.punctuation_spelling() {
        return format!("`{symbol}`");
    }
    match kind {
        TokenKind::Newline => "a line break".to_string(),
        TokenKind::Indent => "an indented block".to_string(),
        TokenKind::Dedent => "the end of an indented block".to_string(),
        TokenKind::Eof => "the end of the file".to_string(),
        TokenKind::Number(_) => "a number".to_string(),
        TokenKind::Text(_) => "quoted text".to_string(),
        TokenKind::Ident(_) => "a name".to_string(),
        _ => unreachable!(
            "every other token kind has a keyword or punctuation spelling, handled above"
        ),
    }
}

/// The word a token was written as, when it has one.
///
/// `describe_found` answers "what kind of thing is this", which is what a
/// message needs when the kind is the mistake. When the *word* is the
/// mistake, quoting it back is what makes the diagnostic about the
/// reader's own program: "found a name" and "found `Map`" cost the same
/// line and only one of them can be acted on.
///
/// Returns `None` for layout and punctuation, which have no word a reader
/// wrote, and for text literals, whose contents are not a word and can be
/// arbitrarily long.
pub(crate) fn found_word(kind: &TokenKind) -> Option<String> {
    match kind {
        TokenKind::Ident(text) => Some(text.clone()),
        TokenKind::Number(value) => Some(format!("{value}")),
        other => other.keyword_spelling().map(str::to_string),
    }
}

/// A user-facing description of a token that was actually encountered, for
/// the "found ..." half of a parse error message.
///
/// This is the counterpart to `describe_expected`: keywords are named by
/// spelling (`the keyword \`state\``) rather than bare-quoted, since "found
/// `state`" reads as though `state` were a symbol. Punctuation, layout, and
/// literal-carrying tokens are described the same way as on the "expected"
/// side. No arm may fall back to `{:?}` — an enum variant name must never
/// reach a user-facing string (spec §7.3).
pub(crate) fn describe_found(kind: &TokenKind) -> String {
    if let Some(word) = kind.keyword_spelling() {
        return format!("the keyword `{word}`");
    }
    if let Some(symbol) = kind.punctuation_spelling() {
        return format!("`{symbol}`");
    }
    match kind {
        TokenKind::Newline => "a line break".to_string(),
        TokenKind::Indent => "an indented block".to_string(),
        TokenKind::Dedent => "the end of an indented block".to_string(),
        TokenKind::Eof => "the end of the file".to_string(),
        TokenKind::Number(_) => "a number".to_string(),
        TokenKind::Text(_) => "a piece of text".to_string(),
        TokenKind::Ident(_) => "a name".to_string(),
        _ => unreachable!(
            "every other token kind has a keyword or punctuation spelling, handled above"
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::codes;
    #[test]
    fn unclosed_paren_names_the_symbol_not_the_variant() {
        let src = "view\n    Text (1 + 2\n";
        let err = crate::parse(src).unwrap_err();
        assert!(
            err.message.contains("`)`"),
            "missing the closing paren symbol:\n{}",
            err.message
        );
        assert!(
            !err.message.contains("RParen"),
            "leaked the enum variant name:\n{}",
            err.message
        );
    }

    #[test]
    fn missing_newline_before_a_block_names_a_line_break_not_the_variant() {
        let src = "view\n    Text \"a\" Text \"b\"\n";
        let err = crate::parse(src).unwrap_err();
        assert!(
            !err.message.contains("Newline"),
            "leaked the enum variant name:\n{}",
            err.message
        );
        assert!(
            !err.message.contains("Indent"),
            "leaked the enum variant name:\n{}",
            err.message
        );
        assert!(
            !err.message.contains("Dedent"),
            "leaked the enum variant name:\n{}",
            err.message
        );
    }

    /// This asserted `the keyword \`state\`` when the message's whole
    /// content was that the word is a keyword. It now says the rule and
    /// what the word is spent on, so the spelling is asserted directly and
    /// the rule beside it.
    #[test]
    fn expect_ident_on_a_keyword_states_the_rule_and_names_the_keyword() {
        let src = "state state is client Int starting empty";
        let err = crate::parse(src).unwrap_err();
        assert!(
            err.message.contains("`state`"),
            "missing the keyword spelling:\n{}",
            err.message
        );
        assert!(
            err.message.contains("No keyword may be a name"),
            "missing the rule:\n{}",
            err.message
        );
        assert!(
            err.message.contains("begins a state declaration"),
            "missing what the word is spent on:\n{}",
            err.message
        );
        assert!(
            !err.message.contains("State"),
            "leaked the enum variant name:\n{}",
            err.message
        );
        assert_eq!(err.code, codes::KEYWORD_AS_NAME);
    }

    #[test]
    fn expect_ident_on_a_number_describes_it_as_a_number() {
        let src = "state 5 is client Int starting empty";
        let err = crate::parse(src).unwrap_err();
        assert!(
            err.message.contains("a number"),
            "missing the number description:\n{}",
            err.message
        );
        assert!(
            !err.message.contains("Number"),
            "leaked the enum variant name:\n{}",
            err.message
        );
        assert!(
            !err.message.contains('('),
            "leaked the literal's Debug form:\n{}",
            err.message
        );
    }

    /// No parse error, from any code path, may let a `TokenKind` variant
    /// name reach the user. This is the guard for the whole class of bug,
    /// not just the specific instances found so far (spec §7.3).
    #[test]
    fn no_malformed_program_leaks_a_token_kind_variant_name() {
        let malformed_programs = [
            "state 5 is client Int starting empty",
            "state state is client Int starting empty",
            "view\n    Text (1 + 2\n",
            "view Text",
            "state x is 5 starting empty",
            "view\n    5\n",
            "5",
            "function f\n    5\n",
        ];

        let forbidden = [
            "Ident",
            "TokenKind",
            "Number(",
            "Text(",
            "RParen",
            "LParen",
            "Newline",
            "Indent",
            "Dedent",
            "Eof",
        ];

        for src in malformed_programs {
            let err = crate::parse(src).unwrap_err();
            for needle in forbidden {
                assert!(
                    !err.message.contains(needle),
                    "message for {src:?} leaked `{needle}`:\n{}",
                    err.message
                );
            }
        }
    }
}
