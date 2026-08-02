//! Lexing for ZDeceptron.
//!
//! Memory safety is a mechanically verified property of this compiler,
//! not a claim: no crate in this workspace may contain `unsafe`.
#![forbid(unsafe_code)]

mod span;
pub mod raw;
pub mod token;

pub use span::Span;
pub use token::{Token, TokenKind};
