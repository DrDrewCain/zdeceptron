//! The types the file does not write down.
//!
//! A `state` declaration names its type; nothing else does. Every
//! parameter, every loop variable, every `with` binding and every pattern
//! binder has a type the checker inferred and the reader has to
//! reconstruct. Putting it at the binder is the whole of this feature.
//!
//! It shows placement too, without a second mechanism, because placement
//! *is* in the type: a `server` signal read from the view is a `Remote of
//! T` (§5.2, §14G.1.4), and a binding that holds one says so. That is the
//! network boundary, at the name that crosses it. What it does not show
//! is the region a piece of code was assigned to, which is the split
//! pass's answer and is not among the products this crate keeps.
//!
//! Every hint comes from `zdc_types::TypeTable`, so it is the type the
//! compiler inferred rather than one reconstructed here. A binding whose
//! type was never solved gets no hint at all: an editor showing `?` in
//! grey is worse than an editor showing nothing, because the reader
//! cannot tell it apart from a type that really is unknown.

use zdc_lexer::Span;

use crate::analysis::Analysis;
use crate::symbols::SymbolKind;

/// One inferred type, and where to draw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Hint {
    /// The end of the name being annotated, which is where the label goes.
    pub at: u32,
    /// The label as it should be drawn, `is` and all: the language spells
    /// a declared type `name is client Whole`, so a hint that read `:
    /// Whole` would be teaching a syntax this language does not have.
    pub label: String,
    /// The binder the hint describes, for a client that highlights it.
    pub span: Span,
}

/// Every inferred type in the document, in source order.
///
/// Bounded to lines `from` to `to` inclusive, which is how a client asks
/// for what is on screen.
pub fn hints(analysis: &Analysis, from: u32, to: u32) -> Vec<Hint> {
    let Some(types) = analysis.types() else {
        return Vec::new();
    };
    let text = analysis.text();
    let lines = analysis.lines();

    let mut out: Vec<Hint> = Vec::new();
    for symbol in analysis.symbols().iter() {
        // The imported modules' binders belong to their own windows.
        if !analysis.in_document(symbol.span) {
            continue;
        }
        let line = lines.position(text, symbol.span.start).line;
        if line < from || line > to {
            continue;
        }
        // Only a binder. A *use* of a name already has a hover, and a
        // hint on every occurrence would bury the line it annotates.
        let SymbolKind::Binding { local, .. } = &symbol.kind else {
            continue;
        };
        let Some(ty) = local.and_then(|local| types.local(local)) else {
            continue;
        };
        out.push(Hint {
            at: symbol.span.end,
            label: format!("is {ty}"),
            span: symbol.span,
        });
    }
    out.sort_by_key(|hint| hint.at);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(src: &str) -> Vec<String> {
        let analysis = Analysis::of(src);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        hints(&analysis, 0, u32::MAX)
            .into_iter()
            .map(|hint| hint.label)
            .collect()
    }

    #[test]
    fn a_parameter_and_a_loop_variable_each_get_the_type_that_was_inferred() {
        let src = "state names is client List of Text starting empty\n\
                   function shout with word\n    give word + \"!\"\n\
                   view\n    each name in names\n        Text (shout with name)\n";
        assert_eq!(labels(src), ["is Text", "is Text"]);
    }

    /// The one that earns the feature: the type says where the value
    /// lives, because a read across the network is a `Remote of T`.
    #[test]
    fn a_binding_that_crosses_the_network_says_so_in_its_type() {
        let src = "state visits is durable Whole starting 0\n\
                   view\n\
                   \x20   when visits\n\
                   \x20       Loading show Spinner\n\
                   \x20       Failed with problem show Spinner\n\
                   \x20       Ready with total show Text total\n";
        let found = labels(src);
        assert_eq!(
            found,
            ["is Error", "is Whole"],
            "the `Failed` payload and the value inside `Ready`"
        );
    }

    /// A file that does not typecheck has no inferred types to show, and
    /// must show none rather than a placeholder.
    #[test]
    fn a_file_that_does_not_typecheck_offers_no_hints() {
        let src = "state a is client Whole starting \"text\"\n\
                   function twice with n\n    give n + n\n";
        let analysis = Analysis::of(src);
        assert!(
            !analysis.diagnostics().is_empty(),
            "the fixture must be one that does not typecheck"
        );
        assert!(hints(&analysis, 0, u32::MAX).is_empty());
    }

    #[test]
    fn hints_outside_the_requested_lines_are_not_computed() {
        let src = "state names is client List of Text starting empty\n\
                   function shout with word\n    give word + \"!\"\n\
                   view\n    each name in names\n        Text (shout with name)\n";
        let analysis = Analysis::of(src);
        let all = hints(&analysis, 0, u32::MAX);
        assert_eq!(all.len(), 2, "the fixture must offer two");
        let first = hints(&analysis, 0, 1);
        assert_eq!(first.len(), 1, "only `word`, on line 1: {first:?}");
        assert_eq!(first[0].label, "is Text");
    }
}
