//! Where a name was declared.
//!
//! The resolver already answered this: every reference in the HIR is a
//! `Res`, and a `Res` is an arena index whose target carries the span of
//! the identifier that declared it. So this is a lookup rather than a
//! search, and it cannot disagree with what the compiler compiled.

use zdc_hir::Res;
use zdc_lexer::Span;

use crate::analysis::Analysis;
use crate::symbols::SymbolKind;

/// The span of the declaration of whatever is at this byte offset.
///
/// A name the language provides rather than the program — a view element,
/// a built-in type — has no declaration to go to, and says so by returning
/// nothing rather than by jumping somewhere arbitrary.
pub fn definition(analysis: &Analysis, offset: u32) -> Option<Span> {
    let symbol = analysis.symbols().at(offset)?;
    let hir = analysis.hir()?;

    match &symbol.kind {
        SymbolKind::Use { res, .. } => match res {
            Some(Res::Def(def)) => Some(hir.defs[*def].span),
            Some(Res::Local(local)) => Some(hir.locals[*local].span),
            Some(Res::Builtin(_)) | None => None,
        },
        // Asked for the definition of a definition, the answer is itself.
        // Editors use this to confirm they are already there.
        SymbolKind::Signal { .. }
        | SymbolKind::Function { .. }
        | SymbolKind::Binding { .. }
        | SymbolKind::View => Some(symbol.span),
        SymbolKind::Element
        | SymbolKind::Variant
        | SymbolKind::TypeName { .. }
        | SymbolKind::Label
        | SymbolKind::Field
        | SymbolKind::Event
        | SymbolKind::Is(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jump(src: &str, from: &str) -> Option<&'static str> {
        let analysis = Analysis::of(src);
        let offset = src.find(from).expect("the needle is in the source") as u32;
        let span = definition(&analysis, offset)?;
        // Leaked deliberately: the tests below compare against literals,
        // and a borrowed slice of `src` would outlive nothing useful.
        let text = src
            .get(span.start as usize..span.end as usize)
            .expect("the span lies within the source")
            .to_string();
        Some(Box::leak(text.into_boxed_str()))
    }

    #[test]
    fn a_reference_jumps_to_the_signal_that_declared_it() {
        let src = "state count is client Whole starting 0\nview\n    Text count\n";
        let analysis = Analysis::of(src);
        let use_site = src.rfind("count").expect("the reference") as u32;
        let span = definition(&analysis, use_site).expect("a definition");
        assert_eq!(span.start as usize, src.find("count").expect("the name"));
    }

    /// Declaration order does not matter, so the jump must not depend on
    /// the definition preceding the use.
    #[test]
    fn a_forward_reference_still_finds_its_declaration() {
        let src = "state doubled is client Whole from count * 2\n\
                   state count is client Whole starting 0\n";
        let analysis = Analysis::of(src);
        let use_site = src.find("count * 2").expect("the reference") as u32;
        let span = definition(&analysis, use_site).expect("a definition");
        assert_eq!(
            span.start as usize,
            src.rfind("count is").expect("the declaration")
        );
    }

    #[test]
    fn a_call_jumps_to_the_function() {
        let src = "function twice with n\n    give n * 2\n\
                   state four is client Whole from twice with n is 2\n";
        let analysis = Analysis::of(src);
        let call = src.find("twice with n is").expect("the call") as u32;
        let span = definition(&analysis, call).expect("a definition");
        assert_eq!(
            span.start as usize,
            src.find("twice with n\n").expect("the declaration")
        );
    }

    #[test]
    fn a_local_jumps_to_where_it_was_bound() {
        let src = "function twice with n\n    give n * 2\n\
                   state four is client Whole from twice with n is 2\n";
        assert_eq!(jump(src, "n * 2"), Some("n"));

        let analysis = Analysis::of(src);
        let use_site = src.find("n * 2").expect("the use") as u32;
        let span = definition(&analysis, use_site).expect("a definition");
        assert_eq!(
            span.start as usize,
            src.find("with n\n").expect("the parameter") + 5
        );
    }

    #[test]
    fn a_loop_variable_jumps_to_the_each_that_bound_it() {
        let src = "state items is client List of Text starting empty\n\
                   view\n    each item in items\n        Text item\n";
        let analysis = Analysis::of(src);
        let use_site = src.rfind("Text item").expect("the use") as u32 + 5;
        let span = definition(&analysis, use_site).expect("a definition");
        assert_eq!(
            span.start as usize,
            src.find("each item").expect("the binder") + 5
        );
    }

    #[test]
    fn a_mutation_target_jumps_to_its_signal() {
        let src = "state count is client Whole starting 0\n\
                   view\n    Button \"go\"\n        on click\n            add 1 to count\n";
        let analysis = Analysis::of(src);
        let target = src.rfind("count").expect("the target") as u32;
        let span = definition(&analysis, target).expect("a definition");
        assert_eq!(span.start as usize, src.find("count").expect("the name"));
    }

    #[test]
    fn a_builtin_has_no_declaration_to_jump_to() {
        let src = "view\n    Column\n";
        let analysis = Analysis::of(src);
        let element = src.find("Column").expect("the element") as u32;
        assert_eq!(definition(&analysis, element), None);
    }

    #[test]
    fn a_file_that_does_not_resolve_answers_nothing_rather_than_guessing() {
        let src = "state a is client Whole from missing\n";
        let analysis = Analysis::of(src);
        let use_site = src.find("missing").expect("the reference") as u32;
        assert_eq!(definition(&analysis, use_site), None);
    }

    #[test]
    fn asking_anywhere_in_a_broken_file_never_panics() {
        let sources = ["", "state", "{\"json\": true}", "view\n    Text (1 + 2\n"];
        for src in sources {
            let analysis = Analysis::of(src);
            for offset in 0..=src.len() as u32 + 4 {
                let _ = definition(&analysis, offset);
            }
        }
    }
}
