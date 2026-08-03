#![forbid(unsafe_code)]

//! Name resolution: AST to HIR.
//!
//! Two passes. The first collects every top-level declaration, because
//! they are order-independent: `state b is client Whole from a` may be
//! written above `state a`. The second walks each body with a stack of
//! lexical scopes, replacing every identifier with what it refers to.
//!
//! Both passes report every error they find rather than stopping at the
//! first, so a program with three undefined names produces three
//! diagnostics from one run.

mod collect;
mod instantiate;
pub mod modules;
mod resolve;
mod sandbox;
mod scope;

pub use collect::{collect, collect_linked, GlobalTable, ResolveError};
pub use modules::{load, load_with_entry, Linked, Module};
pub use resolve::{Resolver, BUILTIN_ELEMENTS, BUILTIN_PATTERNS};
pub use scope::Scopes;
