//! Lexing for ZDeceptron.
//!
//! Memory safety is a mechanically verified property of this compiler,
//! not a claim: no crate in this workspace may contain `unsafe`.
#![forbid(unsafe_code)]

pub mod layout;
pub mod raw;
mod span;
pub mod token;

pub use layout::{tokenize, LexError};
pub use raw::{word_to_soft_keyword, word_to_type_ctor, SoftKeyword, TypeCtor};
pub use span::Span;
pub use token::{Token, TokenKind};
