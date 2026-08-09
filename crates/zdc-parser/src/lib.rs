#![forbid(unsafe_code)]

pub mod codes;
mod cursor;
mod decl;
mod expr;
mod stmt;
mod view;

pub use cursor::{ParseError, Parser, Suggestion};

/// Parse ZDeceptron source into a syntax tree, or into the first thing
/// that stopped it.
///
/// [`parse_all`] is the same parse reporting every error. This exists for
/// the callers that want one — a test asserting on a specific message, or
/// anything that only needs to know whether the file parses at all.
pub fn parse(src: &str) -> Result<zdc_ast::Program, ParseError> {
    parse_all(src).map_err(|mut errors| errors.remove(0))
}

/// Parse ZDeceptron source into a syntax tree, or into **every** syntax
/// error in it.
///
/// This is the entry point every later compiler stage calls. The recovery
/// that makes more than one error possible is described on
/// [`Parser::program_all`]; the short version is that it resumes only at
/// the start of the next top-level declaration, so a file with one mistake
/// still reports exactly one.
///
/// A **lexical** error is still fatal and still alone. There is nothing to
/// resynchronise on: the layout pass turns indentation into tokens, so a
/// file the lexer refused has no token stream at all — not a damaged one —
/// and every span after the refusal would be invented.
pub fn parse_all(src: &str) -> Result<zdc_ast::Program, Vec<ParseError>> {
    // A lexical error is a syntax error a character early: the file does
    // not have one reading, and §4.1's bargain is that a construct has
    // exactly one. It is filed under the same rule for that reason.
    let tokens = zdc_lexer::tokenize(src).map_err(|e| {
        vec![ParseError {
            message: e.message,
            span: e.span,
            label: None,
            suggestion: None,
            code: codes::ONE_VALID_FORM,
        }]
    })?;
    Parser::new(tokens).program_all()
}
