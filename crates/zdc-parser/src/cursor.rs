use zdc_lexer::{SoftKeyword, Span, Token, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
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
        Err(ParseError {
            message: format!(
                "Expected {expected} {context}. ZDeceptron has exactly one way to write this."
            ),
            span: self.peek_span(),
        })
    }

    pub fn expect_ident(&mut self, context: &str) -> Result<zdc_ast::Ident, ParseError> {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::Ident(text) => {
                self.bump();
                Ok(zdc_ast::Ident { text, span })
            }
            other => Err(ParseError {
                message: format!(
                    "Expected a name {context}, found {}.",
                    describe_found(&other)
                ),
                span,
            }),
        }
    }

    pub fn expect_text(&mut self, context: &str) -> Result<String, ParseError> {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::Text(value) => {
                self.bump();
                Ok(value)
            }
            other => Err(ParseError {
                message: format!(
                    "Expected quoted text {context}, found {}.",
                    describe_found(&other)
                ),
                span,
            }),
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
        Err(ParseError {
            message: format!(
                "Expected `{}` {context}, found {}. ZDeceptron has exactly one way to write this.",
                word.spelling(),
                describe_found(self.peek())
            ),
            span: self.peek_span(),
        })
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
            return Err(ParseError {
                message: format!(
                    "This {} is nested more than {} levels deep. Give the inner parts names and \
                     refer to them instead.",
                    kind.noun(),
                    kind.limit()
                ),
                span: self.peek_span(),
            });
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

    #[test]
    fn expect_ident_on_a_keyword_names_the_keyword_not_the_variant() {
        let src = "state state is client Int starting empty";
        let err = crate::parse(src).unwrap_err();
        assert!(
            err.message.contains("the keyword `state`"),
            "missing the keyword spelling:\n{}",
            err.message
        );
        assert!(
            !err.message.contains("State"),
            "leaked the enum variant name:\n{}",
            err.message
        );
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
