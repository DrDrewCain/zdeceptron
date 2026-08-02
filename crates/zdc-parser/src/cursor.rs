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
        let expected = match kind.keyword_spelling() {
            Some(word) => format!("`{word}`"),
            None => format!("{kind:?}"),
        };
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
