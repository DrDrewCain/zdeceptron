//! The fix a diagnostic already knows.
//!
//! One quick fix is offered, and it is the one this compiler can derive
//! rather than paraphrase: a name that is declared in a file this one
//! already imports from, but was not among the names the `use` line
//! borrowed. §14D.2 makes imports explicit, so being linked into the same
//! program is not the same as being visible, and the repair is to add the
//! name to the list.
//!
//! It is derived from the module graph rather than from the diagnostic's
//! text. The loader knows which file declares the name and which `use`
//! line reaches that file, so the edit is a fact about the program.
//! Reading it back out of the message would make the fix depend on the
//! wording of a sentence, which is the one thing §7.3 keeps free to
//! change.
//!
//! # What is deliberately not offered
//!
//! The larger family is not here: "insert `client` after `is`", and the
//! rest of the parse errors that name the single valid phrasing. Those
//! diagnostics carry their repair as English inside `message` and `help`
//! and as nothing else: `zdc_diagnostics::Diagnostic` has a `code`, a
//! span and prose, and no structured suggestion. A quick fix built by
//! matching on that prose would apply an edit derived from a sentence,
//! and would silently stop firing the first time the sentence was
//! reworded. Giving the diagnostics a machine-readable repair is the
//! change that unlocks them, and it belongs in the crate that produces
//! them.

use zdc_ast as ast;
use zdc_diagnostics::Diagnostic;
use zdc_lexer::Span;

use crate::analysis::Analysis;

/// One offered fix: what to call it, and the text to insert where.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub title: String,
    /// The diagnostic this repairs, so a client can attach the fix to it.
    pub diagnostic: Diagnostic,
    /// An empty range at the insertion point, and the text to put there.
    pub at: Span,
    pub insert: String,
}

/// Every fix offered for the diagnostics overlapping a range.
pub fn actions(analysis: &Analysis, range: Span) -> Vec<Action> {
    analysis
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .span
                .is_some_and(|span| span.start <= range.end && range.start <= span.end)
        })
        .filter_map(|diagnostic| import_fix(analysis, diagnostic))
        .collect()
}

/// The fix for a name that a reachable file declares and this file did
/// not borrow.
fn import_fix(analysis: &Analysis, diagnostic: &Diagnostic) -> Option<Action> {
    let span = diagnostic.span?;
    let name = analysis
        .text()
        .get(span.start as usize..span.end as usize)?
        .trim();
    if name.is_empty() {
        return None;
    }

    // Where the name really is declared, which is what makes this a fact
    // rather than a guess. Only a declaration in another file is worth
    // offering: one in this file is visible already, so a missing import
    // is not what went wrong.
    let declared = crate::outline::declarations(analysis)
        .into_iter()
        .find(|declaration| declaration.name == name && !analysis.in_document(declaration.span))?;

    let line = analysis.use_line_importing_from(declared.span)?;
    // Already borrowed, so the diagnostic is about something else.
    if line.names.iter().any(|written| written.text == name) {
        return None;
    }
    let last = line.names.last()?;

    Some(Action {
        title: format!("Import `{name}` from \"{}\"", line.path),
        diagnostic: diagnostic.clone(),
        at: Span::new(last.span.end, last.span.end),
        insert: format!(", {name}"),
    })
}

/// The `use` line of this document that borrows from the file owning a
/// span, if it has one.
///
/// Lives here rather than on `Analysis` only because nothing else needs
/// it; the module bookkeeping it reads is on `Analysis` because that is
/// where the linker's output is kept.
pub(crate) fn use_line(document: &ast::Program, within: Span) -> Option<&ast::UseDecl> {
    document.decls.iter().find_map(|decl| match decl {
        ast::Decl::Use(line) if line.span.start <= within.start && within.end <= line.span.end => {
            Some(line)
        }
        ast::Decl::Use(_)
        | ast::Decl::State(_)
        | ast::Decl::Function(_)
        | ast::Decl::View(_)
        | ast::Decl::Record(_)
        | ast::Decl::Choice(_)
        | ast::Decl::Component(_)
        | ast::Decl::Foreign(_)
        | ast::Decl::Release(_)
        | ast::Decl::Route(_)
        | ast::Decl::Test(_) => None,
    })
}
