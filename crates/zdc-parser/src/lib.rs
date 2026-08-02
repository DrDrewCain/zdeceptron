#![forbid(unsafe_code)]

mod cursor;
mod decl;
mod expr;
mod stmt;

pub use cursor::{ParseError, Parser};
