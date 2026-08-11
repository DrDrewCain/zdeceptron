//! Every place one declaration is named.
//!
//! Find-references, document highlight and rename are the same question
//! asked with three different answers wanted, so they are one traversal
//! here rather than three that could disagree. The traversal is a filter
//! over [`crate::symbols::SymbolIndex`], which already joined every span
//! to what the resolver decided was at it: two spans name the same thing
//! exactly when they carry the same `DefId` or the same `LocalId`.
//!
//! Comparing resolutions rather than spelling is what makes this correct
//! across a module boundary and inside a shadowed scope alike. A textual
//! search would find the right names in the wrong files, and the wrong
//! names in the right ones.
//!
//! # What is deliberately not renameable
//!
//! [`target`] answers `None` for anything whose occurrences this index
//! cannot enumerate in full, and the list matters more than the list it
//! does answer for. A partial rename is not a weaker feature than a
//! complete one: it edits a file into a state that no longer compiles,
//! having been asked to preserve meaning. So a name is renameable only
//! when every one of its occurrences is reachable from here.
//!
//! * A `record` or `choice` name. Types are not resolved (§14B.1 is
//!   specified and pending), so a name in type position carries no
//!   resolution and the occurrences cannot be found.
//! * A variant of a `choice`, for the same reason.
//! * A field, a named argument's label, an event name: none resolves to a
//!   declaration this program owns.
//! * A name the language provides. Its declaration is in the prelude,
//!   whose text is not in the buffer any span here indexes.

use zdc_hir::Res;
use zdc_hir::{DefId, LocalId};
use zdc_lexer::{Span, TokenKind};

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
/// `None` when there is no name there, or when it is one of the kinds
/// this module's header says cannot be enumerated.
pub fn target(analysis: &Analysis, offset: u32) -> Option<Target> {
    of_symbol(analysis, analysis.symbols().at(offset)?)
}

/// Every span naming the declaration at this offset, in source order.
///
/// The declaration's own span is included: an editor showing references
/// wants the definition in the list, and a rename has to rewrite it.
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

/// Every span a rename must overwrite, or `None` when the rename must
/// not be offered at all.
///
/// `None` rather than an empty list, and the distinction is the point: an
/// empty list is a rename that changed nothing, and a client shown one
/// reports success. A refusal has to be a refusal.
///
/// Renaming is the one editor feature that writes, and it writes into
/// files that are not on screen. So it is refused unless every occurrence
/// is known ([`target`]) and the replacement is a name the language would
/// lex as one identifier. A new name that collides with an existing
/// declaration is *not* refused here: the collision is a diagnostic the
/// compiler already gives, in words this module could only paraphrase,
/// and it appears the moment the edit lands.
pub fn rename(analysis: &Analysis, offset: u32, to: &str) -> Option<Vec<Span>> {
    if !is_identifier(to) {
        return None;
    }
    target(analysis, offset)?;
    let found = references(analysis, offset);
    // A target with no occurrences at all would mean the index and the
    // resolver disagree, which is a defect rather than an empty rename.
    (!found.is_empty()).then_some(found)
}

/// Whether the language would lex this text as exactly one identifier.
///
/// A rename is a substitution of text, so a replacement that is not one
/// identifier does not produce a program: `two words` becomes two tokens
/// and `state` becomes a keyword. Asked of the compiler's own lexer
/// rather than of a character class, so a dialect that spells its
/// keywords differently (§4.6) is checked against its own list rather
/// than against English.
pub fn is_identifier(text: &str) -> bool {
    let Ok(tokens) = zdc_lexer::tokenize(text) else {
        return false;
    };
    let mut words = tokens.iter().filter(|token| {
        !matches!(
            token.kind,
            TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
        )
    });
    let one = matches!(
        words.next().map(|token| &token.kind),
        Some(TokenKind::Ident(word)) if word == text
    );
    one && words.next().is_none()
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
        // See this module's header for why each of these is refused.
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
        // Both signals are shown, so neither draws a `W0331` for being
        // unread. The fixture is about two locals sharing a name; a
        // warning about something else in it would be noise the assertion
        // below would have to learn to ignore.
        let src = "function ahead with n\n    give n + 1\n\
                   function behind with n\n    give n + 2\n\
                   state a is client Whole from ahead with 1\n\
                   state b is client Whole from behind with 2\n\
                   \nview\n    Column\n        Text a\n        Text b\n";
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

    /// Every one of these is a rename an editor will be asked for, and
    /// every one of them would leave a file that does not compile.
    #[test]
    fn a_replacement_that_is_not_one_identifier_is_refused() {
        let src = "state count is client Whole starting 0\nview\n    Text count\n";
        let analysis = Analysis::of(src);
        let at = src.find("count").expect("the declaration") as u32;

        let refused = [
            "",
            " ",
            "two words",
            "state",
            "1",
            "1st",
            "count.other",
            "\"count\"",
        ];
        for name in refused {
            assert_eq!(rename(&analysis, at, name), None, "accepted {name:?}");
        }
        assert!(
            rename(&analysis, at, "total").is_some(),
            "an ordinary name is still accepted, or the check above proves nothing"
        );
    }

    /// A rename that cannot be completed must answer nothing rather than
    /// an empty edit, which a client reports as a rename that worked.
    #[test]
    fn a_name_with_no_findable_declaration_is_refused_rather_than_edited_emptily() {
        let src = "view\n    Column\n        Text \"hi\"\n";
        let analysis = Analysis::of(src);
        let at = src.find("Column").expect("the element") as u32;
        assert_eq!(rename(&analysis, at, "Stack"), None);
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
