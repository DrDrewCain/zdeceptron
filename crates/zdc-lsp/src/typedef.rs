//! From a value to the declaration of its type.
//!
//! Types are inferred almost everywhere here, since a `state` writes one
//! and nothing else does, so this is the only route from a name to the
//! `record` or `choice` behind it. Go-to-definition answers where the
//! *value* was declared; this answers what the value **is**.
//!
//! Two steps, and each is a lookup rather than a search. The type comes
//! from the checker's own table, so it is the type the compiler inferred
//! and not one reconstructed here. The declaration comes from the syntax
//! tree of every file the program reaches, matched on the name the type
//! carries, which is the only handle there is, because §14B.1 is
//! specified and pending and a `Type::Named` holds a spelling rather than
//! a `DefId`. When that changes this becomes an arena lookup and the
//! matching goes away.

use zdc_ast as ast;
use zdc_hir::Res;
use zdc_lexer::Span;
use zdc_types::Type;

use crate::analysis::Analysis;
use crate::symbols::SymbolKind;

/// Where the type of whatever is at this byte offset was declared.
///
/// `None` for a value whose type the language provides rather than the
/// program: there is no `record Text` to jump to, and jumping to
/// something arbitrary would be worse than not moving.
pub fn type_definition(analysis: &Analysis, offset: u32) -> Option<Span> {
    let symbol = analysis.symbols().at(offset)?;
    let name = match &symbol.kind {
        // A name written in type position is already the type's name, so
        // no inference is involved.
        SymbolKind::TypeName { builtin: false } => symbol.name.clone(),
        SymbolKind::TypeName { builtin: true } => return None,
        _ => named(inferred(analysis, symbol)?)?,
    };
    declaration_named(analysis, &name)
}

/// The type the checker gave whatever is at this symbol.
fn inferred(analysis: &Analysis, symbol: &crate::symbols::Symbol) -> Option<Type> {
    let types = analysis.types()?;
    let found = match &symbol.kind {
        SymbolKind::Signal { def, .. } => types.def((*def)?),
        SymbolKind::Binding { local, .. } => types.local((*local)?),
        SymbolKind::Use { res, expr } => match (res, expr) {
            // The expression's own type, which is the one that carries
            // the crossing: a `server` signal read from the view is a
            // `Remote of T` and its declaration is still the `T`.
            (_, Some(expr)) => types.expr(*expr),
            (Some(Res::Def(def)), None) => types.def(*def),
            (Some(Res::Local(local)), None) => types.local(*local),
            (Some(Res::Builtin(_) | Res::BuiltinVariant(_) | Res::Variant { .. }) | None, None) => {
                None
            }
        },
        // A `component`, a `function`, the view, an element, a variant
        // spelling, a label, a field, an event, `is`: none of them is a
        // value with a type to jump to.
        SymbolKind::Function { .. }
        | SymbolKind::Component { .. }
        | SymbolKind::View
        | SymbolKind::Element { .. }
        | SymbolKind::Variant
        | SymbolKind::TypeName { .. }
        | SymbolKind::Label
        | SymbolKind::Field
        | SymbolKind::Event
        | SymbolKind::Is(_) => None,
    };
    found.cloned()
}

/// The name of the declared type inside a type expression.
///
/// A container is unwrapped to the thing it contains: asked about a `List
/// of Item`, a programmer means `Item`, because `List` is not a
/// declaration anyone can open. A `Map` yields its value type for the
/// same reason its `of K to V` reads that way, and because the value is
/// what a program indexes out of one.
///
/// Every arm is written out. A type constructor added to the language has
/// to decide here what jumping through it means, rather than inheriting
/// "nothing" from a wildcard.
fn named(ty: Type) -> Option<String> {
    match ty {
        Type::Named(name) => Some(name),
        Type::List(inner) | Type::Option(inner) | Type::Remote(inner) => named(*inner),
        Type::Map(_, value) => named(*value),
        // A function is not a value here, but its result is a type like
        // any other and is what a call site holds.
        Type::Function(_, result) => named(*result),
        Type::Text
        | Type::Markup
        | Type::Whole
        | Type::Decimal
        | Type::Truth
        | Type::Error
        | Type::Code
        | Type::Event(_)
        | Type::Var(_)
        | Type::Unknown => None,
    }
}

/// The span of the `record`, `choice` or `route` declaration of this
/// name, in whichever file declares it.
fn declaration_named(analysis: &Analysis, name: &str) -> Option<Span> {
    analysis.program().decls.iter().find_map(|decl| match decl {
        ast::Decl::Record(record) if record.name.text == name => Some(record.name.span),
        ast::Decl::Choice(choice) if choice.name.text == name => Some(choice.name.span),
        // A route *is* a choice plus a bijection onto URLs (§14G.2), so a
        // value of one has its declaration here like any other.
        ast::Decl::Route(route) if route.name.text == name => Some(route.name.span),
        ast::Decl::Record(_)
        | ast::Decl::Choice(_)
        | ast::Decl::Route(_)
        | ast::Decl::State(_)
        | ast::Decl::Function(_)
        | ast::Decl::View(_)
        | ast::Decl::Component(_)
        | ast::Decl::Use(_)
        | ast::Decl::Foreign(_)
        | ast::Decl::Release(_) => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map is unwrapped to its value type, which is what a program
    /// indexes out of one.
    #[test]
    fn a_map_reaches_the_declaration_of_what_it_holds() {
        let src = "record Item\n    id is Text\n\
                   state byId is client Map of Text to Item starting empty\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.types().is_some(),
            "the fixture must typecheck: {:?}",
            analysis.diagnostics()
        );
        let at = src.find("byId").expect("the signal") as u32;
        assert_eq!(
            type_definition(&analysis, at).map(|span| span.start as usize),
            Some(src.find("Item").expect("the record's name"))
        );
    }

    /// The container is not the declaration a programmer wants: `List` is
    /// not something anyone can open.
    #[test]
    fn a_list_reaches_the_declaration_of_what_is_in_it() {
        let src = "record Item\n    id is Text\n\
                   state items is client List of Item starting empty\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.types().is_some(),
            "the fixture must typecheck: {:?}",
            analysis.diagnostics()
        );
        let at = src.find("items").expect("the signal") as u32;
        assert_eq!(
            type_definition(&analysis, at).map(|span| span.start as usize),
            Some(src.find("Item").expect("the record's name"))
        );
    }

    #[test]
    fn a_name_written_in_type_position_reaches_its_own_declaration() {
        let src = "choice Status\n    Open\n    Shut\n\
                   state now is client Status starting empty\n";
        let analysis = Analysis::of(src);
        let at = src.rfind("Status").expect("the type as written") as u32;
        assert_eq!(
            type_definition(&analysis, at).map(|span| span.start as usize),
            Some(src.find("Status").expect("the choice's name"))
        );
    }

    #[test]
    fn a_builtin_type_has_no_declaration_to_reach() {
        let src = "state count is client Whole starting 0\nview\n    Text count\n";
        let analysis = Analysis::of(src);
        for needle in ["Whole", "count"] {
            let at = src.find(needle).expect("the needle") as u32;
            assert_eq!(type_definition(&analysis, at), None, "for {needle}");
        }
    }

    #[test]
    fn asking_anywhere_in_a_broken_file_never_panics() {
        let sources = ["", "state", "{\"json\": true}", "view\n    Text (1 + 2\n"];
        for src in sources {
            let analysis = Analysis::of(src);
            for offset in 0..=src.len() as u32 + 4 {
                let _ = type_definition(&analysis, offset);
            }
        }
    }
}
