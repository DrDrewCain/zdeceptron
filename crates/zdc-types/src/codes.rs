//! The rules a type error can be an instance of.
//!
//! Until these existed the type errors were the largest family of
//! diagnostics in the compiler with no way to look one up. A programmer
//! met ``a value starts as a number, but `Whole` is expected here`` and
//! that sentence was the whole of what they got: no code, so nothing to
//! pass to `zdc explain`, so no statement of the rule and no worked
//! repair. The type system is where a reader is *learning* the language
//! rather than confirming what they already know, which makes it the
//! family that could least afford to be the one without a handle (#148).
//!
//! # Why `E02xx`
//!
//! The numbering follows the pipeline, and two of its three stages were
//! already allocated: `zdc-parser` owns `E01xx` for syntax (§4.1), and
//! `zdc-graph` owns `E03xx` for placement, the signal graph and the
//! capabilities (§17.2.4). Types run between them — a program is parsed,
//! then typed, then placed — so `E02xx` is both the free range and the one
//! whose position in the number line means something. A reader who has
//! seen `E0104` and `E0322` can guess which pass produced `E0223` before
//! looking it up, which is the only thing a numeric code is good for.
//!
//! Nothing else in the compiler has ever taken an `E02xx`, and the block
//! is allocated whole rather than a decade at a time, so a later family
//! does not have to interleave.
//!
//! # Why a code names a rule and not a call site
//!
//! `zdc-parser`'s `codes` module already made this argument and it holds
//! harder here: `infer.rs` reports from eighty-odd places, and eighty
//! codes would be eighty `zdc explain` pages saying the same handful of
//! things in slightly different words. A reader who had read one would
//! learn nothing from the next, and the numbering would carry no
//! information at all.
//!
//! So each constant below is a *rule*, and the sites that enforce it share
//! it. The message still names the specific — which element, which field,
//! which two types — because naming the specific is what a message is for.
//!
//! The decades group the rules the way `E03xx`'s do:
//!
//! * `E020x` — inference proper: what the equations settled, and what they
//!   could not.
//! * `E021x` — `when`, the only construct that takes a choice apart.
//! * `E022x` — names, calls and construction: what a declaration is for,
//!   and what filling one in requires.
//! * `E023x` — pipelines.
//! * `E024x` — writes: what is a place, and what may write into one.
//! * `E025x` — reads that cross a placement boundary they cannot.
//! * `E026x` — the view: what an element takes.
//! * `E027x` — the JavaScript boundary: what a `foreign` may promise, and
//!   where its result may be spent.
//! * `E028x` — a function's own shape.
//! * `E029x` — events.
//!
//! # What is not covered yet
//!
//! Every rule `infer.rs` enforces is here. Two things are deliberately
//! not, and both are `TypeError`s with `code: None`.
//!
//! **`routing.rs`.** The URL bijection, route-parameter enumerability,
//! `Link` written as a literal URL, and the immutability of a signal
//! initialised from `address`. They are a family of their own — rules
//! about URLs rather than about types — and they want their own decade
//! and their own prose rather than to be filed under a type code that
//! nearly fits. They are the stated remainder of #148 rather than an
//! oversight, and five of them are the messages
//! `scripts/check-message-budget.py` waives by name, which is where a
//! reader will next trip over the gap.
//!
//! **The prelude's own incompleteness.** One message in `infer.rs` says
//! `contains` was given a type whose library function the standard library
//! did not declare. That is a defect in the compiler rather than in the
//! file being compiled, so there is no rule to state and no repair to
//! write: a code would send a reader to a page telling them to fix
//! something they did not write.
//!
//! Each code here is scanned out of this file by `zdc-diagnostics`'s
//! coverage gate, which fails when a code has no explanation and when an
//! explanation names a code nothing produces.

/// A value's type is not the one the position it sits in requires.
///
/// The commonest type error there is, and the one the issue quotes.
pub const TYPE_MISMATCH: &str = "E0201";

/// An operator or built-in was applied to a type outside the set it
/// accepts.
pub const OPERAND_SET: &str = "E0202";

/// A type that would have to contain itself — the occurs check.
pub const INFINITE_TYPE: &str = "E0203";

/// Nothing in the program fixes this type, so the checker cannot pick one.
pub const NOT_DETERMINED: &str = "E0204";

/// `when` was given a value that has no variants to take apart.
pub const NOT_A_CHOICE: &str = "E0210";

/// A `when` does not write an arm for every variant (§14G.1.6).
pub const MISSING_ARM: &str = "E0211";

/// An arm names something other than one distinct variant, with the
/// binders its fields call for.
pub const ARM_SHAPE: &str = "E0212";

/// A call does not fill the parameters the declaration names.
pub const CALL_ARGUMENTS: &str = "E0220";

/// A record or a variant was not built by naming every field once.
pub const FIELDS_GIVEN: &str = "E0221";

/// A declaration — a function, a record, a choice, a component — was
/// written where a value goes.
pub const NOT_A_VALUE: &str = "E0222";

/// A field was read from a value that does not carry it.
pub const NO_SUCH_FIELD: &str = "E0223";

/// A pipeline clause with no collection in front of it to walk.
pub const PIPELINE_SOURCE: &str = "E0230";

/// Something was written to that is not a place.
pub const NOT_A_PLACE: &str = "E0240";

/// A two-way element was not given state it can write back to.
pub const TWO_WAY_TARGET: &str = "E0241";

/// State was read from a context that cannot reach it (§14G.1.4).
pub const UNREACHABLE_READ: &str = "E0250";

/// An element was given arguments that are not the ones it takes.
pub const ELEMENT_ARGUMENTS: &str = "E0260";

/// A `foreign` declaration promises something the JavaScript boundary
/// cannot carry.
pub const FOREIGN_CONTRACT: &str = "E0270";

/// A `foreign … gives view` was used as a value, or given children.
pub const VIEW_FOREIGN_USE: &str = "E0271";

/// `do` was given a call that gives a value.
pub const DO_GIVES_NOTHING: &str = "E0272";

/// A function does not reach a `give` on every path.
pub const NOT_TOTAL: &str = "E0280";

/// An `on` handler names an event, a key, or a payload the browser does
/// not report.
pub const NO_SUCH_EVENT: &str = "E0290";
