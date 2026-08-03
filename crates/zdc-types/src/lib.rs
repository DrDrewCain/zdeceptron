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
mod events;
mod infer;
mod integrity;
mod placement;
mod table;
mod ty;
mod unify;

use zdc_hir::Hir;
use zdc_lexer::Span;

pub use crate::choice::{Choice, Variant};
pub use crate::events::{event_names, payload_of, EventPayload, EVENTS};
pub use crate::placement::{read_kind, Placements, ReadContext, ReadKind, SignalPlacement};
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

/// Typecheck a resolved program, against the placement pass's answers.
///
/// Returns every type in the program, or every error in it — never the
/// first error alone.
///
/// The `placements` argument is §17.1.4's interface. It replaces the
/// syntax-driven stub this crate used to carry, and it is what makes the
/// type of a cross-placement read a *lookup* rather than a second copy of
/// §14G.1.4's table that can drift.
///
/// Integrity (§18.1) runs after inference rather than beside it, and only
/// when inference succeeded. It walks the same HIR asking a different
/// question, and a program whose types are wrong has expressions whose
/// provenance is not worth reporting on yet.
pub fn check(hir: &Hir, placements: &dyn Placements) -> Result<TypeTable, Vec<TypeError>> {
    let table = infer::Checker::new(hir, placements).run()?;
    let violations = integrity::check(hir, placements);
    if violations.is_empty() {
        Ok(table)
    } else {
        Err(violations)
    }
}
