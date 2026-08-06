#![forbid(unsafe_code)]

pub mod codes;
mod cursor;
mod decl;
mod expr;
mod stmt;
mod view;

pub use cursor::{ParseError, Parser, Suggestion};

/// Parse ZDeceptron source into a syntax tree.
///
/// This is the entry point every later compiler stage calls.
pub fn parse(src: &str) -> Result<zdc_ast::Program, ParseError> {
    // A lexical error is a syntax error a character early: the file does
    // not have one reading, and §4.1's bargain is that a construct has
    // exactly one. It is filed under the same rule for that reason.
    let tokens = zdc_lexer::tokenize(src).map_err(|e| ParseError {
        message: e.message,
        span: e.span,
        label: None,
        suggestion: None,
        code: codes::ONE_VALID_FORM,
    })?;
    Parser::new(tokens).program()
}
