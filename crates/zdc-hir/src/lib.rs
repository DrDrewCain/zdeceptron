#![forbid(unsafe_code)]

//! High-level intermediate representation for ZDeceptron.
//!
//! HIR nodes live in typed arenas. Each arena has its own index type, so
//! passing (for example) a definition ID to an expression arena is rejected
//! by Rust instead of becoming a wrong lookup at runtime.

mod ids;
mod nodes;
/// The one rule that bounds every path a program can make the build open.
///
/// Here rather than in `zdc-resolve` because the two callers sit on
/// opposite sides of it: `use` opens a module during resolution, and
/// `build read` / `build list` open a file during emission. A rule with
/// two callers in two crates lives under the crate they both already
/// depend on, beside [`url`] — which is here for the same reason.
pub mod sandbox;
mod url;

pub use ids::{Arena, ArenaId, BlockId, DefId, ExprId, LocalId, PlaceId};
pub use nodes::*;
pub use url::{
    destination, is_event_attribute, is_url_attribute, url_is_safe, url_scheme, Destination,
    FETCHING_SCHEMES, URL_ATTRIBUTES, URL_SCHEMES,
};
