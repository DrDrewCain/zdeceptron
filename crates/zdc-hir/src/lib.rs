#![forbid(unsafe_code)]

//! High-level intermediate representation for ZDeceptron.
//!
//! HIR nodes live in typed arenas. Each arena has its own index type, so
//! passing (for example) a definition ID to an expression arena is rejected
//! by Rust instead of becoming a wrong lookup at runtime.

mod ids;
mod nodes;
mod url;

pub use ids::{Arena, ArenaId, BlockId, DefId, ExprId, LocalId};
pub use nodes::*;
pub use url::{
    is_event_attribute, is_url_attribute, url_is_safe, url_scheme, URL_ATTRIBUTES, URL_SCHEMES,
};
