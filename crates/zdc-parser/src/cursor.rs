use zdc_lexer::{Span, Token, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// The span of the last consumed token that carries source text.
    last_end: Span,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        assert!(!tokens.is_empty(), "token stream always ends with Eof");
        let start = tokens[0].span.start;
        Parser {
            tokens,
            pos: 0,
            last_end: Span::new(start, start),
        }
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

    /// Skip layout tokens that carry no meaning at this position.
    pub fn skip_newlines(&mut self) {
        while self.at(&TokenKind::Newline) {
            self.bump();
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

        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::Dedent) || self.at(&TokenKind::Eof) {
                break;
            }
            items.push(item(self)?);
        }

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
        let src = "view Text";
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
