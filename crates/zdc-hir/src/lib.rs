#![forbid(unsafe_code)]

//! High-level intermediate representation for ZDeceptron.
//!
//! HIR nodes live in typed arenas. Each arena has its own index type, so
//! passing (for example) a definition ID to an expression arena is rejected
//! by Rust instead of becoming a wrong lookup at runtime.

mod ids;

pub use ids::{Arena, ArenaId, BlockId, DefId, ExprId, LocalId};
