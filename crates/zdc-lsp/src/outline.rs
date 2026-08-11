//! The declarations a file makes, in the order it makes them.
//!
//! This answers two requests at once: a document's outline, and a search
//! across every file a program reaches. They differ only in which spans
//! are kept, so they are one traversal rather than two that could report
//! different declarations for the same file.
//!
//! Read off the syntax tree rather than the HIR. Every top-level
//! declaration has a name and a span there, including the `record` and
//! `choice` declarations that resolution turns into types no name in the
//! index points at, and a tree exists for a file that does not resolve,
//! which is most of the time while one is being written. An outline that
//! blinked out mid-edit would be missing at the moment it is most used.

use zdc_ast as ast;
use zdc_lexer::Span;

use crate::analysis::Analysis;

/// One top-level declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    /// The name itself, which is where a jump should land.
    pub selection: Span,
    /// The whole declaration, which is what an outline collapses and what
    /// a breadcrumb bar names the cursor as being inside.
    pub span: Span,
    pub kind: DeclarationKind,
}

/// What kind of declaration it is, in the language's own vocabulary.
///
/// Written out per `ast::Decl` variant rather than collapsed into the
/// protocol's `SymbolKind`, so adding a declaration form to the language
/// is a compile error here rather than a declaration that silently stops
/// appearing in outlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Signal(ast::Placement),
    /// A `test` declaration — issue #169. Listed in the outline because
    /// the claims a file makes are the part of it a reader most often
    /// wants a list of.
    Test,
    Function,
    View,
    Record,
    Choice,
    Component,
    Foreign,
    Release,
    Route,
}

/// Every declaration of every file this analysis covers, in source order.
///
/// Source order across the combined buffer, which is the order the files
/// were read: a module's declarations stay together and in the order they
/// were written.
pub fn declarations(analysis: &Analysis) -> Vec<Declaration> {
    let mut found: Vec<Declaration> = analysis
        .program()
        .decls
        .iter()
        .filter_map(declaration)
        .collect();
    found.sort_by_key(|declaration| (declaration.span.start, declaration.span.end));
    found
}

/// The declarations written in the open document itself.
pub fn document_declarations(analysis: &Analysis) -> Vec<Declaration> {
    declarations(analysis)
        .into_iter()
        .filter(|declaration| analysis.in_document(declaration.span))
        .collect()
}

fn declaration(decl: &ast::Decl) -> Option<Declaration> {
    let (name, selection, span, kind) = match decl {
        ast::Decl::State(state) => (
            &state.name,
            state.name.span,
            state.span,
            DeclarationKind::Signal(state.placement),
        ),
        ast::Decl::Function(function) => (
            &function.name,
            function.name.span,
            function.span,
            DeclarationKind::Function,
        ),
        // A view has no name of its own, so the keyword stands in for
        // one: it is what the programmer clicks in an outline, and
        // `symbols.rs` already treats it as the declaration's name.
        ast::Decl::View(view) => {
            return Some(Declaration {
                name: "view".to_string(),
                selection: Span::new(view.span.start, view.span.start.saturating_add(4)),
                span: view.span,
                kind: DeclarationKind::View,
            })
        }
        ast::Decl::Record(record) => (
            &record.name,
            record.name.span,
            record.span,
            DeclarationKind::Record,
        ),
        ast::Decl::Choice(choice) => (
            &choice.name,
            choice.name.span,
            choice.span,
            DeclarationKind::Choice,
        ),
        ast::Decl::Component(component) => (
            &component.name,
            component.name.span,
            component.span,
            DeclarationKind::Component,
        ),
        ast::Decl::Foreign(foreign) => (
            &foreign.name,
            foreign.name.span,
            foreign.span,
            DeclarationKind::Foreign,
        ),
        ast::Decl::Release(release) => (
            &release.name,
            release.name.span,
            release.span,
            DeclarationKind::Release,
        ),
        ast::Decl::Route(route) => (
            &route.name,
            route.name.span,
            route.span,
            DeclarationKind::Route,
        ),
        // A test's name is its claim, which is a `Text` literal and not an
        // `Ident`, so it takes the same early return the view does. The
        // selection is the quoted claim: an outline entry should land the
        // reader on the sentence, which is the part they are looking for.
        ast::Decl::Test(test) => {
            return Some(Declaration {
                name: test.claim.clone(),
                selection: test.claim_span,
                span: test.span,
                kind: DeclarationKind::Test,
            })
        }
        // A `use` names declarations another file made. Listing them here
        // would report one declaration twice, once where it was written
        // and once where it was borrowed.
        ast::Decl::Use(_) => return None,
    };
    Some(Declaration {
        name: name.text.clone(),
        selection,
        span,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declaration_form_the_language_has_appears_in_the_outline() {
        let src = "record Item\n    id is Text\n\
                   choice Status\n    Open\n    Shut\n\
                   component Card with title\n    Text title\n\
                   function twice with n\n    give n + n\n\
                   state count is client Whole starting 0\n\
                   view\n    Text count\n";
        let analysis = Analysis::of(src);
        let found = declarations(&analysis);

        let names: Vec<&str> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["Item", "Status", "Card", "twice", "count", "view"],
            "in the order they were written"
        );
        let kinds: Vec<DeclarationKind> = found.iter().map(|d| d.kind).collect();
        assert_eq!(
            kinds,
            [
                DeclarationKind::Record,
                DeclarationKind::Choice,
                DeclarationKind::Component,
                DeclarationKind::Function,
                DeclarationKind::Signal(ast::Placement::Client),
                DeclarationKind::View,
            ]
        );
    }

    /// The name's span has to lie inside the declaration's, or an editor
    /// draws a breadcrumb that does not contain the thing it names.
    #[test]
    fn a_declaration_encloses_its_own_name() {
        let src = "state count is client Whole starting 0\n\
                   function twice with n\n    give n + n\n\
                   view\n    Text count\n";
        let analysis = Analysis::of(src);
        let found = declarations(&analysis);
        assert_eq!(found.len(), 3, "{found:?}");
        for declaration in &found {
            assert!(
                declaration.span.start <= declaration.selection.start
                    && declaration.selection.end <= declaration.span.end,
                "{declaration:?}"
            );
            assert_eq!(
                &src[declaration.selection.start as usize..declaration.selection.end as usize],
                declaration.name
            );
        }
    }

    /// A file that does not resolve still has an outline, because the
    /// tree is what it is read from.
    #[test]
    fn an_unresolved_file_still_has_an_outline() {
        let src = "state a is client Whole from nowhere\nview\n    Text a\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.hir().is_none(),
            "the fixture must be one that does not resolve"
        );
        let names: Vec<String> = declarations(&analysis)
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, ["a", "view"]);
    }
}
