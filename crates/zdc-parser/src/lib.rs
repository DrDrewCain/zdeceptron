#![forbid(unsafe_code)]

mod cursor;
mod decl;
mod expr;
mod stmt;
mod view;

pub use cursor::{ParseError, Parser};

/// Parse ZDeceptron source into a syntax tree.
///
/// This is the entry point every later compiler stage calls.
pub fn parse(src: &str) -> Result<zdc_ast::Program, ParseError> {
    let tokens = zdc_lexer::tokenize(src).map_err(|e| ParseError {
        message: e.message,
        span: e.span,
    })?;
    Parser::new(tokens).program()
}
