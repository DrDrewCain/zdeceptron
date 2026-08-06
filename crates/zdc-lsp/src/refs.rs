//! Every place one declaration is named.
//!
//! Find-references and document highlight are the same question asked
//! with two different answers wanted, so they are one traversal here
//! rather than two that could disagree. The traversal is a filter
//! over [`crate::symbols::SymbolIndex`], which already joined every span
//! to what the resolver decided was at it: two spans name the same thing
//! exactly when they carry the same `DefId` or the same `LocalId`.
//!
//! Comparing resolutions rather than spelling is what makes this correct
//! across a module boundary and inside a shadowed scope alike. A textual
//! search would find the right names in the wrong files, and the wrong
//! names in the right ones.
//!

use zdc_hir::Res;
use zdc_hir::{DefId, LocalId};
use zdc_lexer::Span;

use crate::analysis::Analysis;
use crate::symbols::{Symbol, SymbolKind};

/// The declaration two occurrences of a name have in common.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A top-level `state`, `function`, `component` or `release`.
    Def(DefId),
    /// A parameter, a loop variable, or a pattern's binder.
    Local(LocalId),
}

/// What the name at this byte offset declares or refers to.
///
/// `None` when there is no name there, or when it names something whose
/// occurrences this index cannot enumerate in full: a `record` or
/// `choice` name and a variant of one, because types are not resolved
/// (§14B.1 is specified and pending); a field, a label or an event name,
/// because none of them resolves to a declaration this program owns; and
/// a name the language provides, because its declaration is in the
/// prelude, whose text is not in the buffer any span here indexes.
pub fn target(analysis: &Analysis, offset: u32) -> Option<Target> {
    of_symbol(analysis, analysis.symbols().at(offset)?)
}

/// Every span naming the declaration at this offset, in source order.
///
/// The declaration's own span is included: an editor showing references
/// wants the definition in the list.
pub fn references(analysis: &Analysis, offset: u32) -> Vec<Span> {
    let Some(target) = target(analysis, offset) else {
        return Vec::new();
    };
    let mut found: Vec<Span> = analysis
        .symbols()
        .iter()
        .filter(|symbol| of_symbol(analysis, symbol) == Some(target))
        .map(|symbol| symbol.span)
        .collect();

    // The name as written on a `use` line, which is in no index because a
    // `use` is not a declaration and lowers to nothing.
    if let Some(declaration) = declaration(analysis, target) {
        let name = analysis
            .symbols()
            .at(declaration.start)
            .map(|symbol| symbol.name.clone())
            .unwrap_or_default();
        found.extend(analysis.import_sites(&name, declaration));
    }

    found.sort_by_key(|span| (span.start, span.end));
    found.dedup();
    found
}

/// Where the declaration itself is, which is the span a rename anchors on.
pub fn declaration(analysis: &Analysis, target: Target) -> Option<Span> {
    let hir = analysis.hir()?;
    match target {
        Target::Def(def) => Some(hir.defs[def].span),
        Target::Local(local) => Some(hir.locals[local].span),
    }
}

/// The declaration a symbol names, if it names one that can be found in
/// full.
fn of_symbol(analysis: &Analysis, symbol: &Symbol) -> Option<Target> {
    let target = match &symbol.kind {
        SymbolKind::Signal { def, .. }
        | SymbolKind::Function { def }
        | SymbolKind::Component { def } => Target::Def((*def)?),
        SymbolKind::Binding { local, .. } => Target::Local((*local)?),
        SymbolKind::Use { res, .. } | SymbolKind::Element { res } => match res {
            Some(Res::Def(def)) => Target::Def(*def),
            Some(Res::Local(local)) => Target::Local(*local),
            // A variant names its choice, and renaming the choice at a
            // variant's spelling would rewrite the wrong word. The two
            // built-in arms name declarations in the prelude.
            Some(Res::Variant { .. } | Res::Builtin(_) | Res::BuiltinVariant(_)) | None => {
                return None
            }
        },
        // See `target` for why each of these is refused.
        SymbolKind::View
        | SymbolKind::Variant
        | SymbolKind::TypeName { .. }
        | SymbolKind::Label
        | SymbolKind::Field
        | SymbolKind::Event
        | SymbolKind::Is(_) => return None,
    };

    // A prelude declaration's spans index the library's own source files
    // and not the buffer this analysis was built from, so listing them
    // would point an editor at offsets in a file it does not have.
    let hir = analysis.hir()?;
    let prelude = match target {
        Target::Def(def) => hir.is_prelude_def(def),
        Target::Local(local) => hir.is_prelude_local(local),
    };
    (!prelude).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spelled(analysis: &Analysis, spans: &[Span]) -> Vec<String> {
        spans
            .iter()
            .map(|span| {
                let found = analysis.locate(*span);
                found
                    .text
                    .get(found.span.start as usize..found.span.end as usize)
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_signal_is_found_at_its_declaration_and_at_every_read() {
        let src = "state count is client Whole starting 0\n\
                   state doubled is client Whole from count + count\n\
                   view\n    Text count\n";
        let analysis = Analysis::of(src);
        let at = src.rfind("count").expect("the view reference") as u32;
        let found = references(&analysis, at);

        assert_eq!(
            found.len(),
            4,
            "one declaration and three reads: {:?}",
            spelled(&analysis, &found)
        );
        assert!(spelled(&analysis, &found)
            .iter()
            .all(|text| text == "count"));
        assert_eq!(
            found[0].start as usize,
            src.find("count").expect("the declaration")
        );
    }

    /// Two locals with the same name in different functions are two
    /// declarations, and a textual search would merge them.
    #[test]
    fn a_local_is_not_confused_with_a_same_named_local_elsewhere() {
        let src = "function ahead with n\n    give n + 1\n\
                   function behind with n\n    give n + 2\n\
                   state a is client Whole from ahead with 1\n\
                   state b is client Whole from behind with 2\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let at = src.find("n + 1").expect("the first body") as u32;
        let found = references(&analysis, at);

        assert_eq!(
            found.len(),
            2,
            "the parameter and its one read: {:?}",
            spelled(&analysis, &found)
        );
        let boundary = src.find("function behind").expect("the second function");
        assert!(found.iter().all(|span| (span.end as usize) <= boundary));
    }

    #[test]
    fn a_built_in_element_has_no_declaration_to_enumerate() {
        let src = "view\n    Column\n        Text \"hi\"\n";
        let analysis = Analysis::of(src);
        let at = src.find("Column").expect("the element") as u32;
        assert_eq!(target(&analysis, at), None);
        assert!(references(&analysis, at).is_empty());
    }

    /// A prelude name resolves to a definition whose span indexes a file
    /// this analysis does not hold, so it must be refused rather than
    /// listed at whatever offsets happen to match.
    #[test]
    fn a_library_name_is_refused_rather_than_pointed_somewhere() {
        let src = "state items is client List of Text starting empty\n\
                   state n is client Whole from length of items\n\
                   view\n    Text n\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let at = src.find("length of").expect("the library call") as u32;
        assert_eq!(target(&analysis, at), None);
    }

    #[test]
    fn asking_anywhere_in_a_broken_file_never_panics() {
        let sources = ["", "state", "{\"json\": true}", "view\n    Text (1 + 2\n"];
        for src in sources {
            let analysis = Analysis::of(src);
            for offset in 0..=src.len() as u32 + 4 {
                let _ = references(&analysis, offset);
            }
        }
    }
}
