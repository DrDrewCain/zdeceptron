#![forbid(unsafe_code)]

//! A language server for ZDeceptron, which asks the compiler rather than
//! guessing.
//!
//! `editors/vscode/README.md` sets out why this exists. The TextMate
//! grammar there classifies tokens and stops, because a regular expression
//! cannot resolve the language's structure: `is` does three different jobs,
//! a capitalised name is either a type or a view element depending on name
//! resolution, indentation is syntax, and dialects (§4.6) would need one
//! grammar copy each. Everything in this crate is answered by running the
//! real passes, so the editor and the compiler cannot disagree.
//!
//! Four things are surfaced, in the order they earn their place:
//!
//! * **Diagnostics.** §7.3 already makes them a primary deliverable — the
//!   parser names the single valid phrasing for every syntax error and the
//!   checker names what was expected and found. This puts them inline.
//! * **Hover**, giving the inferred type and *where the value lives*. The
//!   second is the interesting one: a `server` signal read from the view is
//!   a `Remote of T` because the network is between them (§5.2), and saying
//!   so at the cursor puts the boundary in the editor.
//! * **Go to definition**, which is a lookup in the resolver's output.
//! * **Semantic tokens**, which is where `is` and the capitalised names
//!   finally get told apart, and where a reference carries the placement of
//!   the signal it names.
//!
//! Completion is offered too, and is the one feature that reads position
//! from tokens rather than from a tree.
//!
//! Nothing here may panic. A crashed language server takes the editor's
//! diagnostics and highlighting down and says nothing about why, so a file
//! that is mid-keystroke — usually not a valid program, sometimes not a
//! program at all — comes back as diagnostics instead.

mod analysis;
mod complete;
mod goto;
mod hover;
mod lines;
mod outline;
mod refs;
mod server;
mod symbols;
mod tokens;

pub use analysis::{Analysis, Located};
pub use complete::{complete, Completion, CompletionKind};
pub use goto::definition;
pub use hover::hover;
pub use lines::{LineIndex, Position};
pub use outline::{declarations, document_declarations, Declaration, DeclarationKind};
pub use refs::{references, Target};
pub use server::run;
pub use symbols::{IsRole, Symbol, SymbolIndex, SymbolKind};
pub use tokens::{encode, highlights, Highlight, TOKEN_MODIFIERS, TOKEN_TYPES};
