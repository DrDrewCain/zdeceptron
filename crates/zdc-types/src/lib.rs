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
pub mod codes;
mod elements;
mod events;
mod failure;
mod infer;
mod placement;
pub mod routing;
mod table;
mod ty;
mod unify;

use zdc_hir::Hir;
use zdc_lexer::Span;

pub use crate::choice::{code_choice, error_fields, Choice, Variant, ERROR_CODE_FIELD};
pub use crate::events::{
    event_names, is_document_key, payload_of, suggest_key, EventPayload, DOCUMENT_KEY_RULE, EVENTS,
    NAMED_KEYS,
};
pub use crate::failure::FailureCode;
pub use crate::placement::{read_kind, Placements, ReadContext, ReadKind, SignalPlacement};
pub use crate::routing::{Page, Site};
pub use crate::table::{EmptyKind, IndexKind, OperatorKind, TypeTable};
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
    /// The rule this is an instance of, for the errors that name one.
    ///
    /// A code is the handle a reader passes to `zdc explain`, and it is
    /// stable across every rewording of the message. Until this field
    /// existed the type errors were the largest family in the compiler
    /// with no way to look one up: the message was the whole of what a
    /// programmer got, and the type system is where a programmer is most
    /// often learning the language rather than confirming it.
    ///
    /// `Option`, and not a required field as a `ParseError`'s is, because
    /// the codes arrive a family at a time. The `codes` module says which
    /// errors still carry `None` and why, so the remainder is written down
    /// rather than discovered.
    pub code: Option<&'static str>,
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
/// Integrity (§18.1) is **not** here. It is the closed lattice in
/// `zdc-graph`, which the flow pass runs; this crate once carried a
/// second, default-open version of the same pass, and two lattices
/// labelling the same expressions could only disagree.
pub fn check(hir: &Hir, placements: &dyn Placements) -> Result<TypeTable, Vec<TypeError>> {
    let types = infer::Checker::new(hir, placements).run()?;
    // Routing runs after inference because it reads what inference
    // settled: which variant a `when` eliminates, and what a `static`
    // initialiser evaluates to.
    match routing::check(hir) {
        Ok(_) => Ok(types),
        Err(errors) => Err(errors),
    }
}

/// The documents a routed program emits, in URL order.
///
/// Empty for a program with no `route`: an unrouted program is one page,
/// which is what it has always been.
pub fn site(hir: &Hir) -> Site {
    routing::check(hir).unwrap_or_default()
}
