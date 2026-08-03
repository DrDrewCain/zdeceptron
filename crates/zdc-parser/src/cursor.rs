use zdc_lexer::{Span, Token, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        assert!(!tokens.is_empty(), "token stream always ends with Eof");
        Parser { tokens, pos: 0 }
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
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        token
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
                message: format!("Expected a name {context}, found {other:?}."),
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
}
