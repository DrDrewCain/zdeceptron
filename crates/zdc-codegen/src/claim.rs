//! What a `test` declaration's claim came to — issue #169.
//!
//! Its own module, outside the `evaluate` feature, and the reason is a
//! rule this workspace already states in `zdc-diagnostics`'s manifest:
//! **rendering a diagnostic does not need an interpreter.**
//!
//! [`Broken`] is produced by running a program and consumed by printing
//! one. Those are two different jobs on two different sides of the
//! feature: `evaluate` carries `boa_engine`, which cannot be built for
//! `wasm32-unknown-unknown`, and `zdc-diagnostics` therefore depends on
//! this crate with `default-features = false` so a browser build gets the
//! renderer without the engine. A `Broken` behind that gate makes
//! `zdc-diagnostics` unbuildable in exactly the configuration the gate
//! exists to allow — which is how it was found, by `ci.yml`'s
//! `wasm32-wasip1` build and by nothing else, because a `--workspace`
//! build unifies features and hands `evaluate` to everyone.
//!
//! Nothing here runs anything. It is four fields of plain data describing
//! a claim that turned out to be false.

use zdc_lexer::Span;

/// A claim the program contradicted — issue #169.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken {
    /// Carried rather than written by the renderer, exactly as
    /// [`zdc_graph::GraphError`] carries its own. The code and the site
    /// that raises it belong in one file, so `zdc explain`'s coverage gate
    /// can enumerate the codes from the source that produces them.
    pub code: &'static str,
    pub claim: String,
    pub span: Span,
    /// What each side of a top-level `is` came to, when the expectation
    /// had two sides. `None` when it did not — `a and b`, `xs contains y`,
    /// a call returning a `Truth` — because there is no pair to show and
    /// inventing one would point the reader at the wrong values.
    pub sides: Option<(String, String)>,
}
