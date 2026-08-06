//! What calls a callable, and what it calls.
//!
//! This language has no first-class functions: a `function`, a `release`
//! or a `component` can only be named in order to be called (§17.4.2). So
//! a reference to one *is* a call, and the call graph is the subset of
//! the reference graph whose targets are callable. That is why this is a
//! filter over the same index find-references reads rather than a walk of
//! its own: the two cannot come to disagree about whether one declaration
//! reaches another.
//!
//! Placement makes the answer unusually informative here. The call graph
//! is also the region graph (§17.1.2): a call from view context into a
//! `server`-rooted callable is where the network is, so reading a call
//! hierarchy is reading the boundaries of the deployment.
//!
//! Which declaration a call is *in* is decided by span containment
//! against the outline, which is exact: a declaration's span covers its
//! whole body and declarations do not nest at the top level.

use zdc_hir::{DefId, DefKind, Res};
use zdc_lexer::Span;

use crate::analysis::Analysis;
use crate::outline::{declarations, Declaration};
use crate::symbols::SymbolKind;

/// One end of a call edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Callable {
    pub def: DefId,
    pub name: String,
    /// The whole declaration, which is what an editor shows as the item.
    pub span: Span,
    /// The name within it, which is where a jump lands.
    pub selection: Span,
}

/// One edge, with every place the call is written.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub callable: Callable,
    /// The spans of the calls themselves, which an editor lists under the
    /// item so that one of several call sites can be jumped to.
    pub sites: Vec<Span>,
}

/// The callable at this offset, if there is one there.
///
/// Answers for both a call site and a declaration, because an editor asks
/// this of wherever the cursor happens to be before it will offer the
/// hierarchy at all.
pub fn callable_at(analysis: &Analysis, offset: u32) -> Option<Callable> {
    let symbol = analysis.symbols().at(offset)?;
    let def = match &symbol.kind {
        SymbolKind::Function { def } | SymbolKind::Component { def } => (*def)?,
        SymbolKind::Use {
            res: Some(Res::Def(def)),
            ..
        }
        | SymbolKind::Element {
            res: Some(Res::Def(def)),
        } => *def,
        _ => return None,
    };
    callable(analysis, &declarations(analysis), def)
}

/// Every callable that names this one.
pub fn incoming(analysis: &Analysis, def: DefId) -> Vec<Edge> {
    // Built once and passed down. Both the enclosing-declaration lookup
    // and the whole-declaration span of each callable read it, and
    // rebuilding it per call site would make this quadratic in the size
    // of the program.
    let enclosing = declarations(analysis);
    let mut found: Vec<Edge> = Vec::new();
    for (site, target) in call_sites(analysis) {
        if target != def {
            continue;
        }
        let Some(caller) = innermost(&enclosing, site) else {
            continue;
        };
        // A call written in a `state` initialiser or in the view has no
        // calling *callable*, and attributing it to one would invent an
        // edge the program does not have.
        let Some(caller) = declared_callable(analysis, &enclosing, caller) else {
            continue;
        };
        push(&mut found, caller, site);
    }
    found
}

/// Every callable this one names.
pub fn outgoing(analysis: &Analysis, def: DefId) -> Vec<Edge> {
    let enclosing = declarations(analysis);
    let Some(here) = callable(analysis, &enclosing, def) else {
        return Vec::new();
    };
    let mut found: Vec<Edge> = Vec::new();
    for (site, target) in call_sites(analysis) {
        if site.start < here.span.start || site.end > here.span.end {
            continue;
        }
        let Some(callee) = callable(analysis, &enclosing, target) else {
            continue;
        };
        push(&mut found, callee, site);
    }
    found
}

/// Every place a callable is named, as (the name's span, what it names).
fn call_sites(analysis: &Analysis) -> Vec<(Span, DefId)> {
    let Some(hir) = analysis.hir() else {
        return Vec::new();
    };
    analysis
        .symbols()
        .iter()
        .filter_map(|symbol| {
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
            // A library callable's declaration is in a file this analysis
            // has no text for, so an edge to it could not be opened.
            (!hir.is_prelude_def(def) && is_callable(analysis, def)).then_some((symbol.span, def))
        })
        .collect()
}

fn is_callable(analysis: &Analysis, def: DefId) -> bool {
    let Some(hir) = analysis.hir() else {
        return false;
    };
    match &hir.defs[def].kind {
        DefKind::Function(_)
        | DefKind::Release(_)
        | DefKind::Component(_)
        | DefKind::Foreign(_) => true,
        // Reading a signal is not calling it, and a record, a choice or
        // the view is not something a call can name at all.
        DefKind::Signal(_) | DefKind::View(_) | DefKind::Record(_) | DefKind::Choice(_) => false,
    }
}

fn callable(analysis: &Analysis, enclosing: &[Declaration], def: DefId) -> Option<Callable> {
    let hir = analysis.hir()?;
    if hir.is_prelude_def(def) || !is_callable(analysis, def) {
        return None;
    }
    let selection = hir.defs[def].span;
    // The name's span is the definition's; the whole declaration is the
    // outline's, and the two are joined on the name. A definition with no
    // declaration in the tree cannot happen, but falling back to the name
    // keeps that from being load-bearing.
    let whole = enclosing
        .iter()
        .find(|declaration| declaration.selection == selection)
        .map(|declaration| declaration.span)
        .unwrap_or(selection);
    Some(Callable {
        def,
        name: hir.defs[def].name.clone(),
        span: whole,
        selection,
    })
}

/// The callable a declaration is, if it is one. A call written inside a
/// `state` initialiser or the view has no callable caller.
fn declared_callable(
    analysis: &Analysis,
    enclosing: &[Declaration],
    declaration: &Declaration,
) -> Option<Callable> {
    let hir = analysis.hir()?;
    let def = hir
        .user_defs()
        .find(|(_, found)| found.span == declaration.selection)
        .map(|(id, _)| id)?;
    callable(analysis, enclosing, def)
}

/// The innermost declaration whose span contains a call site.
fn innermost(declarations: &[Declaration], site: Span) -> Option<&Declaration> {
    declarations
        .iter()
        .filter(|declaration| {
            declaration.span.start <= site.start && site.end <= declaration.span.end
        })
        .min_by_key(|declaration| declaration.span.len())
}

/// Add a site to an existing edge, or start a new one.
fn push(found: &mut Vec<Edge>, callable: Callable, site: Span) {
    match found
        .iter_mut()
        .find(|edge| edge.callable.def == callable.def)
    {
        Some(edge) => edge.sites.push(site),
        None => found.push(Edge {
            callable,
            sites: vec![site],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "function twice with n\n    give n + n\n\
                       function quadruple with n\n    give twice with (twice with n)\n\
                       state four is client Whole from quadruple with 1\n\
                       view\n    Text four\n";

    fn analysed() -> Analysis {
        let analysis = Analysis::of(SRC);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        analysis
    }

    #[test]
    fn the_callers_of_a_function_are_the_declarations_that_name_it() {
        let analysis = analysed();
        let at = SRC.find("twice with (").expect("a call") as u32;
        let here = callable_at(&analysis, at).expect("a callable");
        assert_eq!(here.name, "twice");

        let callers = incoming(&analysis, here.def);
        assert_eq!(callers.len(), 1, "{callers:?}");
        assert_eq!(callers[0].callable.name, "quadruple");
        assert_eq!(
            callers[0].sites.len(),
            2,
            "`quadruple` names `twice` twice, and both are call sites"
        );
    }

    /// A call written in a `state` initialiser has no callable caller,
    /// so it must be dropped rather than attributed to something.
    #[test]
    fn a_call_from_a_signal_initialiser_has_no_calling_callable() {
        let analysis = analysed();
        let at = SRC.find("quadruple with 1").expect("the call") as u32;
        let here = callable_at(&analysis, at).expect("a callable");
        assert_eq!(here.name, "quadruple");
        assert!(
            incoming(&analysis, here.def).is_empty(),
            "the only call to it is in a `state` line"
        );
    }

    #[test]
    fn what_a_function_calls_is_read_from_inside_its_own_declaration() {
        let analysis = analysed();
        let at = SRC.find("quadruple with n").expect("the declaration") as u32;
        let here = callable_at(&analysis, at).expect("a callable");

        let calls = outgoing(&analysis, here.def);
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0].callable.name, "twice");
        assert_eq!(calls[0].sites.len(), 2);

        assert!(
            outgoing(
                &analysis,
                callable_at(
                    &analysis,
                    SRC.find("twice with n").expect("the leaf") as u32
                )
                .expect("a callable")
                .def
            )
            .is_empty(),
            "`twice` calls nothing"
        );
    }

    #[test]
    fn a_signal_is_not_a_callable_and_has_no_hierarchy() {
        let analysis = analysed();
        let at = SRC.rfind("four").expect("the read in the view") as u32;
        assert_eq!(callable_at(&analysis, at), None);
    }

    #[test]
    fn asking_anywhere_in_a_broken_file_never_panics() {
        let sources = ["", "state", "{\"json\": true}", "view\n    Text (1 + 2\n"];
        for src in sources {
            let analysis = Analysis::of(src);
            for offset in 0..=src.len() as u32 + 4 {
                let _ = callable_at(&analysis, offset);
            }
        }
    }
}
