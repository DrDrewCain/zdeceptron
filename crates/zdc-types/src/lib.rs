#![forbid(unsafe_code)]

//! Type inference and checking: HIR in, types out.
//!
//! Spec §5.4 asks for Hindley–Milner with let-polymorphism over `Text`,
//! `Whole`, `Decimal`, `Truth`, `List of T`, `Map of K to V`, `Option of
//! T`, `Remote of T` and user-declared shapes, with no higher-rank types
//! and no typeclasses. That is what this is.
//!
//! Three things make it more than a textbook exercise.
//!
//! * **`Remote of T` is a type, and placement is not.** Reading a
//!   `server` signal from the view yields `Remote of Text`; reading the
//!   same signal from a view-rooted server derivation yields `Text`
//!   (§5.2 as amended by §14G.1.4). The type of the read is this pass's
//!   job; deciding *where* code runs is `zdc-graph`'s. The whole of that
//!   boundary is [`placement`], including a stub that answers it from
//!   syntax until `zdc-graph` exists.
//! * **`when` must be exhaustive, everywhere.** §14G.1.6: all three arms
//!   of a `Remote` in every context, including arms the compiler can
//!   prove unreachable. Without that verdict a missing arm is a runtime
//!   throw, which is why codegen refuses to emit a `when` without one.
//! * **Every error is reported.** A programmer with three type errors
//!   sees three diagnostics from one run, matching name resolution.
//!
//! The result is a [`TypeTable`], which answers every blocking entry on
//! §16.7's list of what code generation needs.

mod choice;
mod elements;
mod infer;
mod integrity;
mod placement;
pub mod routing;
mod table;
mod ty;
mod unify;

use zdc_hir::Hir;
use zdc_lexer::Span;

pub use crate::choice::{Choice, Variant};
pub use crate::integrity::trusted_signals;
pub use crate::placement::{read_kind, ReadContext, ReadKind, SignalPlacement};
pub use crate::routing::{Page, Site};
pub use crate::table::{EmptyKind, IndexKind, TypeTable};
pub use crate::ty::{Constraint, Type};

/// A type error, pointing at the source that caused it.
///
/// Spec §7.3 makes diagnostics a primary deliverable, so every one of
/// these names what was expected, what was found, and where. No Rust type
/// name ever reaches one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

/// Typecheck a resolved program.
///
/// Returns every type in the program, or every error in it — never the
/// first error alone.
pub fn check(hir: &Hir) -> Result<TypeTable, Vec<TypeError>> {
    let types = infer::Checker::new(hir).run()?;
    // Routing and integrity run after inference because both read what
    // inference settled — which variant a `when` eliminates, and what a
    // `static` initialiser evaluates to. Both report every problem they
    // find, and both report alongside the other, so a program with a URL
    // collision and an untrusted index sees both from one run.
    let mut errors = Vec::new();
    if let Err(found) = routing::check(hir) {
        errors.extend(found);
    }
    if let Err(found) = integrity::check(hir) {
        errors.extend(found);
    }
    if errors.is_empty() {
        Ok(types)
    } else {
        Err(errors)
    }
}

/// The documents a routed program emits, in URL order.
///
/// Empty for a program with no `route`: an unrouted program is one page,
/// which is what it has always been.
pub fn site(hir: &Hir) -> Site {
    routing::check(hir).unwrap_or_default()
}
