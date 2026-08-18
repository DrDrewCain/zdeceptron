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
//! What is surfaced, in the order the features earn their place:
//!
//! * **Diagnostics.** §7.3 already makes them a primary deliverable — the
//!   parser names the single valid phrasing for every syntax error and the
//!   checker names what was expected and found. This puts them inline, on
//!   open, on change and on save.
//! * **Hover**, giving the inferred type and *where the value lives*. The
//!   second is the interesting one: a `server` signal read from the view is
//!   a `Remote of T` because the network is between them (§5.2), and saying
//!   so at the cursor puts the boundary in the editor. **Inlay hints** put
//!   the same answer at every binder, without being asked.
//! * **Go to definition** and **go to type definition**, which are lookups
//!   in the resolver's and the checker's output.
//! * **Find references**, **document highlight** and **rename**, which are
//!   one traversal ([`references`]) with three answers wanted.
//! * **Document symbols** and **workspace symbols**, an outline of one file
//!   and a search across every file a program reaches.
//! * **Call hierarchy**, which here is also the region graph: this language
//!   has no first-class functions, so naming a callable is calling it, and
//!   a call from the view into a `server`-rooted callable is the network.
//! * **Semantic tokens**, whole-document and by range, which is where `is`
//!   and the capitalised names finally get told apart, and where a
//!   reference carries the placement of the signal it names.
//! * **Folding ranges**, which are the layout pass's own output rather than
//!   a second measurement of the indentation.
//! * **Code actions**, offering the one repair this compiler can derive
//!   rather than paraphrase: a name a reachable file declares and the
//!   `use` line did not borrow.
//! * **Formatting**, which is `zdc fmt`'s own layout delivered as edits so
//!   that format-on-save reaches a buffer that was never written to disk.
//!   The layout is not re-decided here; the range form is deliberately not
//!   offered, and `fmt.rs` says why.
//!
//! Completion and **signature help** are offered too, and are the two
//! features that read position from tokens rather than from a tree,
//! because a file being typed into usually has no tree.
//!
//! Every answer that carries a location goes through [`Analysis::locate`].
//! A span is an offset into the linker's combined buffer, so a span from
//! an imported module names a file that is not the one on screen, and a
//! feature that rendered it against the open document would point at the
//! right offset of the wrong file.
//!
//! Nothing here may panic. A crashed language server takes the editor's
//! diagnostics and highlighting down and says nothing about why, so a file
//! that is mid-keystroke — usually not a valid program, sometimes not a
//! program at all — comes back as diagnostics instead.

mod actions;
mod analysis;
mod calls;
mod complete;
mod fmt;
mod folds;
mod goto;
mod hints;
mod hover;
mod lines;
mod outline;
mod refs;
mod server;
mod signature;
mod symbols;
mod tokens;
mod typedef;

pub use actions::{actions, Action};
pub use analysis::{Analysis, Located};
pub use calls::{callable_at, incoming, outgoing, Callable, Edge};
pub use complete::{complete, Completion, CompletionKind};
pub use fmt::{formatting, Edit};
pub use folds::{folds, Fold};
pub use goto::definition;
pub use hints::{hints, Hint};
pub use hover::hover;
pub use lines::{LineIndex, Position};
pub use outline::{declarations, document_declarations, Declaration, DeclarationKind};
pub use refs::{references, Target};
pub use server::run;
pub use signature::{signature, Signature};
pub use symbols::{IsRole, Symbol, SymbolIndex, SymbolKind};
pub use tokens::{encode, highlights, highlights_within, Highlight, TOKEN_MODIFIERS, TOKEN_TYPES};
pub use typedef::type_definition;
