//! The parameters of the call being written.
//!
//! A call is `f with a, b`, so `with` is the exact moment the parameter
//! list becomes the thing the writer needs. Position is read off the
//! token stream rather than the syntax tree, for the same reason
//! [`crate::complete()`] does: a call being typed is usually not a call
//! yet, and a tree that does not exist cannot be walked. Everything past
//! the position comes from the compiler: which callable it is, what its
//! parameters are called, what types they were inferred to have.

use zdc_hir::{DefKind, Res};
use zdc_lexer::TokenKind;

use crate::analysis::Analysis;
use crate::symbols::SymbolKind;

/// The callable a cursor is inside the arguments of.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    /// The whole call as the language writes it: `twice with n`.
    pub label: String,
    /// Each parameter as it appears within `label`, so a client can
    /// underline the active one without re-parsing the label.
    pub parameters: Vec<String>,
    /// Which parameter the cursor is on, counted by the commas before it.
    pub active: u32,
}

/// The signature of the call the cursor is inside, if it is inside one.
pub fn signature(analysis: &Analysis, offset: u32) -> Option<Signature> {
    let (callee, active) = call_at(analysis, offset)?;
    let symbol = analysis.symbols().at(callee)?;
    let def = match &symbol.kind {
        SymbolKind::Use {
            res: Some(Res::Def(def)),
            ..
        }
        | SymbolKind::Element {
            res: Some(Res::Def(def)),
        } => *def,
        _ => return None,
    };

    let hir = analysis.hir()?;
    let parameters: Vec<String> = match &hir.defs[def].kind {
        DefKind::Function(function) => named(analysis, &function.params),
        DefKind::Release(release) => named(analysis, &release.params),
        DefKind::Component(component) => named(analysis, &component.params),
        DefKind::Foreign(foreign) => named(analysis, &foreign.params),
        // A signal, a view, a record or a choice is not something a
        // `with` can be written after, so there is no list to show.
        DefKind::Signal(_) | DefKind::View(_) | DefKind::Record(_) | DefKind::Choice(_) => {
            return None
        }
    };

    let name = &hir.defs[def].name;
    let label = if parameters.is_empty() {
        name.clone()
    } else {
        format!("{name} with {}", parameters.join(", "))
    };
    // Clamped: a call written with more arguments than the callable takes
    // is an error the checker reports, and until it is fixed the last
    // parameter is the nearest true thing to point at.
    let last = u32::try_from(parameters.len().saturating_sub(1)).unwrap_or(0);
    Some(Signature {
        label,
        parameters,
        active: active.min(last),
    })
}

/// Each parameter as `name is Type`, or as the bare name when the type
/// was not solved. The spelling is the language's own: a declaration
/// writes `name is Type` and nothing here should teach a different one.
fn named(analysis: &Analysis, params: &[zdc_hir::LocalId]) -> Vec<String> {
    let hir = analysis.hir();
    params
        .iter()
        .map(|id| {
            let name = hir
                .map(|hir| hir.locals[*id].name.clone())
                .unwrap_or_default();
            match analysis.types().and_then(|types| types.local(*id)) {
                Some(ty) => format!("{name} is {ty}"),
                None => name,
            }
        })
        .collect()
}

/// The offset of the callee's name, and how many commas separate the
/// cursor from the `with` that opened the argument list.
///
/// Read backwards from the cursor over the tokens on its own line. A call
/// does not span lines here, because a line break closes an argument
/// list the way it closes everything else, so the line is the whole of the
/// context that matters.
fn call_at(analysis: &Analysis, offset: u32) -> Option<(u32, u32)> {
    let text = analysis.text();
    let line_start = text
        .get(..offset.min(text.len() as u32) as usize)
        .and_then(|before| before.rfind('\n').map(|at| at as u32 + 1))
        .unwrap_or(0);

    let before: Vec<&zdc_lexer::Token> = analysis
        .tokens()
        .iter()
        .filter(|token| {
            token.span.start >= line_start
                && token.span.end <= offset
                && !matches!(
                    token.kind,
                    TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
                )
        })
        .collect();

    // Backwards to the nearest `with` that is not inside a parenthesised
    // sub-expression, counting the commas passed on the way. A nested
    // call is a nearer `with`, and it is the one being written.
    let mut depth = 0i32;
    let mut commas = 0u32;
    for (at, token) in before.iter().enumerate().rev() {
        match token.kind {
            TokenKind::RParen => depth += 1,
            TokenKind::LParen => {
                if depth == 0 {
                    // An unclosed `(` before the cursor opens a group the
                    // cursor is inside, so nothing before it is part of
                    // this argument list.
                    return None;
                }
                depth -= 1;
            }
            TokenKind::Comma if depth == 0 => commas += 1,
            TokenKind::With if depth == 0 => {
                let callee = before.get(at.checked_sub(1)?)?;
                return matches!(callee.kind, TokenKind::Ident(_))
                    .then_some((callee.span.start, commas));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "function area with width, height\n    give width * height\n\
                       state size is client Whole from area with 2, 3\n\
                       view\n    Text size\n";

    fn at(src: &str, needle: &str) -> u32 {
        (src.find(needle).expect("the needle is in the source") + needle.len()) as u32
    }

    #[test]
    fn a_call_shows_the_parameters_with_their_inferred_types() {
        let analysis = Analysis::of(SRC);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let found = signature(&analysis, at(SRC, "from area with ")).expect("a signature");
        assert_eq!(found.label, "area with width is Whole, height is Whole");
        assert_eq!(found.parameters, ["width is Whole", "height is Whole"]);
        assert_eq!(found.active, 0);
    }

    /// The `with` of a declaration introduces parameters rather than
    /// passing arguments, so there is no call to describe there.
    #[test]
    fn the_with_of_a_declaration_is_not_a_call() {
        let analysis = Analysis::of(SRC);
        assert_eq!(signature(&analysis, at(SRC, "function area with ")), None);
    }

    #[test]
    fn the_active_parameter_follows_the_commas_written_so_far() {
        let analysis = Analysis::of(SRC);
        assert_eq!(
            signature(&analysis, at(SRC, "area with 2, "))
                .expect("a signature")
                .active,
            1
        );
    }

    /// The nearest `with` is the call being written, not the outer one.
    #[test]
    fn a_nested_call_reports_the_inner_callable() {
        let src = "function twice with n\n    give n + n\n\
                   function plus with a, b\n    give a + b\n\
                   state s is client Whole from plus with (twice with 1), 2\n\
                   view\n    Text s\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let found = signature(&analysis, at(src, "(twice with ")).expect("a signature");
        assert_eq!(found.label, "twice with n is Whole");
        assert_eq!(found.active, 0);
    }

    #[test]
    fn a_cursor_that_is_not_inside_a_call_has_no_signature() {
        let analysis = Analysis::of(SRC);
        for needle in ["state size", "give width"] {
            assert_eq!(
                signature(&analysis, at(SRC, needle)),
                None,
                "for {needle:?}"
            );
        }
    }

    #[test]
    fn asking_anywhere_in_a_broken_file_never_panics() {
        let sources = ["", "state", "f with ", "a with (b with , ", "((((((("];
        for src in sources {
            let analysis = Analysis::of(src);
            for offset in 0..=src.len() as u32 + 4 {
                let _ = signature(&analysis, offset);
            }
        }
    }
}
