//! What is under the cursor, in words.
//!
//! Two things are worth saying about a name: its inferred type, and **where
//! its value lives**. The second is the one this language exists for. A
//! `server` signal read from the view is not a `List of Item`, it is a
//! `Remote of List of Item`, because the network is between the two — and
//! the type checker already knows that, having applied §14G.1.4's read
//! table to decide it. Surfacing it on hover puts the network boundary in
//! the editor, at the moment the read is being written.

use std::fmt::Write as _;

use zdc_ast as ast;
use zdc_hir::{DefId, DefKind, Hir, LocalId, Res};
use zdc_lexer::Span;
use zdc_types::{Type, TypeTable};

use crate::analysis::Analysis;
use crate::symbols::{IsRole, Symbol, SymbolKind};

/// Markdown describing what is at a byte offset, and the span it covers.
pub fn hover(analysis: &Analysis, offset: u32) -> Option<(Span, String)> {
    let symbol = analysis.symbols().at(offset)?;
    let text = describe(analysis, symbol)?;
    Some((symbol.span, text))
}

fn describe(analysis: &Analysis, symbol: &Symbol) -> Option<String> {
    let hir = analysis.hir();
    let types = analysis.types();
    let name = &symbol.name;

    Some(match &symbol.kind {
        SymbolKind::Signal {
            def,
            placement,
            secret,
            source,
        } => {
            let mut out = signature_of_signal(hir, *def, name, *placement, *secret, *source);
            let _ = write!(out, "\n{}", placement_note(name, *placement));
            if *secret {
                out.push_str(
                    "\n\nIt is `secret`: no value derived from it may reach `client` state or the \
                     view (spec §5.3).",
                );
            }
            if !*source {
                out.push_str(
                    "\n\nIt is derived with `from`, so it is recomputed when its inputs change \
                     and cannot be assigned to (spec §4.5).",
                );
            }
            out
        }

        SymbolKind::Function { def } => function_signature(hir, *def, name),

        SymbolKind::Component { def } => component_signature(hir, *def, name),

        SymbolKind::View => "```zdeceptron\nview\n```\n\nThe program's view. Everything under it \
                             runs in client context (spec §5.6)."
            .to_string(),

        SymbolKind::Binding { local, parameter } => {
            let what = if *parameter { "parameter" } else { "name" };
            let mut out = format!("```zdeceptron\n{name}\n```\n\nA {what} bound here");
            match local.and_then(|id| types?.local(id)) {
                Some(ty) => {
                    let _ = write!(out, ", of type `{ty}`.");
                }
                None => out.push('.'),
            }
            out
        }

        SymbolKind::Use { res, expr } => {
            let hir = hir?;
            match res {
                Some(Res::Def(def)) => use_of_definition(hir, types, *def, *expr),
                Some(Res::Local(local)) => use_of_local(hir, types, *local),
                Some(Res::Builtin(builtin)) => match builtin {
                    zdc_hir::Builtin::Element(_) => element_note(name),
                    zdc_hir::Builtin::Type => {
                        format!("```zdeceptron\n{name}\n```\n\nA type the language provides.")
                    }
                },
                Some(Res::Variant { choice, .. }) => format!(
                    "```zdeceptron\n{name}\n```\n\nA variant of `{}`.",
                    hir.defs[*choice].name
                ),
                Some(Res::BuiltinVariant(variant)) => format!(
                    "```zdeceptron\n{name}\n```\n\nA variant of `{}`, which the language \
                     provides.",
                    match variant {
                        zdc_hir::BuiltinVariant::Some | zdc_hir::BuiltinVariant::None => "Option",
                        _ => "Remote",
                    }
                ),
                None => return None,
            }
        }

        SymbolKind::Element => element_note(name),

        SymbolKind::Variant => variant_note(name),

        SymbolKind::TypeName { builtin } => {
            if *builtin {
                format!("```zdeceptron\n{name}\n```\n\nA type the language provides (spec §5.4).")
            } else {
                format!(
                    "```zdeceptron\n{name}\n```\n\nA type this program names. `record` and \
                     `choice` are specified (§14B.1) and not yet implemented, so `{name}` is \
                     treated as opaque: it is never interchangeable with another named type."
                )
            }
        }

        SymbolKind::Label => format!(
            "```zdeceptron\n{name} is …\n```\n\nA named argument. The `is` that follows binds \
             this name to a value; it is not equality."
        ),

        SymbolKind::Field => {
            format!(
                "```zdeceptron\n.{name}\n```\n\nSelects the `{name}` field of the value \
                     before the dot."
            )
        }

        SymbolKind::Event => format!(
            "```zdeceptron\non {name}\n```\n\nThe block below runs when this browser event fires. \
             Event handlers are legal only in client context (spec §5.6)."
        ),

        SymbolKind::Is(role) => is_note(*role),
    })
}

/// The declaration line of a signal, reconstructed from the tree.
fn signature_of_signal(
    hir: Option<&Hir>,
    def: Option<DefId>,
    name: &str,
    placement: ast::Placement,
    secret: bool,
    source: bool,
) -> String {
    let secret = if secret { "secret " } else { "" };
    let init = if source { "starting" } else { "from" };
    let ty = def
        .zip(hir)
        .and_then(|(def, hir)| match &hir.defs[def].kind {
            DefKind::Signal(signal) => Some(render(&signal.ty)),
            DefKind::Function(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_)
            | DefKind::Foreign(_) => None,
        })
        .unwrap_or_else(|| "…".to_string());
    format!(
        "```zdeceptron\n{secret}state {name} is {} {ty} {init} …\n```\n",
        describe_placement(placement)
    )
}

fn function_signature(hir: Option<&Hir>, def: Option<DefId>, name: &str) -> String {
    let params = def
        .zip(hir)
        .and_then(|(def, hir)| match &hir.defs[def].kind {
            DefKind::Function(function) => Some(
                function
                    .params
                    .iter()
                    .map(|id| hir.locals[*id].name.clone())
                    .collect::<Vec<_>>(),
            ),
            DefKind::Signal(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_)
            | DefKind::Foreign(_) => None,
        })
        .unwrap_or_default();

    let head = if params.is_empty() {
        format!("function {name}")
    } else {
        format!("function {name} with {}", params.join(", "))
    };
    format!(
        "```zdeceptron\n{head}\n```\n\nFunctions carry no placement: one runs wherever its inputs \
         are (spec §5.1)."
    )
}

/// A component's declaration line, and the one thing a reader most needs
/// to know about it: it has no placement of its own (spec §14D.1).
fn component_signature(hir: Option<&Hir>, def: Option<DefId>, name: &str) -> String {
    let mut params: Vec<String> = Vec::new();
    let mut takes_children = false;
    if let Some((def, hir)) = def.zip(hir) {
        if let DefKind::Component(component) = &hir.defs[def].kind {
            params = component
                .params
                .iter()
                .map(|id| hir.locals[*id].name.clone())
                .collect();
            takes_children = component.children.is_some();
        }
    }
    if takes_children {
        params.push("children".to_string());
    }

    let head = if params.is_empty() {
        format!("component {name}")
    } else {
        format!("component {name} with {}", params.join(", "))
    };
    format!(
        "```zdeceptron\n{head}\n```\n\nA component is written where a built-in element is, and \
         carries no placement of its own: it runs wherever its inputs are (spec §14D.1)."
    )
}

/// A use of a top-level name. This is where placement is surfaced.
fn use_of_definition(
    hir: &Hir,
    types: Option<&TypeTable>,
    def: DefId,
    expr: Option<zdc_hir::ExprId>,
) -> String {
    let name = hir.defs[def].name.clone();
    match &hir.defs[def].kind {
        DefKind::Record(record) => format!(
            "```zdeceptron\nrecord {name}\n```\n\n{} field{}.",
            record.fields.len(),
            if record.fields.len() == 1 { "" } else { "s" }
        ),
        DefKind::Choice(choice) => format!(
            "```zdeceptron\nchoice {name}\n```\n\n{} variant{}. Take it apart with `when`.",
            choice.variants.len(),
            if choice.variants.len() == 1 { "" } else { "s" }
        ),
        DefKind::Component(_) => component_signature(Some(hir), Some(def), &name),
        DefKind::Signal(signal) => {
            let declared = render(&signal.ty);
            let secret = if signal.secret { "secret " } else { "" };
            let init = if signal.is_source { "starting" } else { "from" };
            let placement = describe_placement(signal.placement);

            let mut out = format!(
                "```zdeceptron\n{secret}state {name} is {placement} {declared} {init} …\n```\n\n{}",
                placement_note(&name, signal.placement)
            );

            // The read type, which is the declared type only when the read
            // does not cross a boundary. §5.2: the network appears in the
            // type system exactly where the network is.
            let read = expr.zip(types).and_then(|(expr, types)| types.expr(expr));
            if let Some(read) = read {
                let _ = write!(out, "\n\nRead here it is `{read}`.");
                if crosses_a_boundary(read, types.and_then(|t| t.def(def))) {
                    let _ = write!(
                        out,
                        " This read crosses the network, so the value is not available until a \
                         `when` has eliminated `Loading`, `Ready` and `Failed` — all three, in \
                         every context (spec §14G.1.6)."
                    );
                }
            }

            if signal.secret {
                out.push_str(
                    "\n\nIt is `secret`: no value derived from it may reach `client` state or the \
                     view (spec §5.3).",
                );
            }
            out
        }
        DefKind::Function(_) => function_signature(Some(hir), Some(def), &name),
        DefKind::Foreign(foreign) => format!(
            "```zdeceptron\nforeign {name} is {}\n```\n\nA platform operation, from `{}` as \
             `{}`. Its types are asserted rather than inferred, because it has no ZDeceptron \
             body (spec §14E.4).",
            foreign.site.describe(),
            foreign.module,
            foreign.symbol
        ),
        DefKind::View(_) => "```zdeceptron\nview\n```\n\nThe program's view.".to_string(),
    }
}

fn use_of_local(hir: &Hir, types: Option<&TypeTable>, local: LocalId) -> String {
    let name = &hir.locals[local].name;
    match types.and_then(|types| types.local(local)) {
        Some(ty) => format!("```zdeceptron\n{name}\n```\n\nA name bound here, of type `{ty}`."),
        None => format!("```zdeceptron\n{name}\n```\n\nA name bound in this body."),
    }
}

/// Whether a read produced `Remote of T` from a signal whose own type is
/// not remote — that is, whether the boundary is in the type because the
/// read crossed it, rather than because the program declared it so.
fn crosses_a_boundary(read: &Type, declared: Option<&Type>) -> bool {
    match (read, declared) {
        (Type::Remote(_), Some(Type::Remote(_))) => false,
        (Type::Remote(_), _) => true,
        _ => false,
    }
}

fn describe_placement(placement: ast::Placement) -> &'static str {
    placement.word()
}

/// §5.1's table, in a sentence. This is the whole point of the hover.
fn placement_note(name: &str, placement: ast::Placement) -> String {
    match placement {
        ast::Placement::Client => format!(
            "`{name}` lives in **browser memory**. It does not survive a reload, it may not hold \
             secrets, and the client reads it directly."
        ),
        ast::Placement::Static => format!(
            "`{name}` is computed **once at build time** and inlined into every page that reads \
             it. It costs no network request, it may not hold secrets, it may not be written, and \
             it is what a route parameter's `in` ranges over (spec §14C.3b)."
        ),
        ast::Placement::Server => format!(
            "`{name}` lives in a **serverless invocation**. It does not survive a reload, it may \
             hold secrets, and the client reaches it only through generated RPC."
        ),
        ast::Placement::Durable => format!(
            "`{name}` lives in a **persistent store**. It survives a reload, it may hold secrets, \
             and the client reaches it only through generated RPC. It is global: one value shared \
             by every visitor (spec §5.7)."
        ),
    }
}

fn element_note(name: &str) -> String {
    let extra = match name {
        "Input" => {
            "\n\nIts first argument is a two-way binding to a `Text` signal, which must be \
             `client`-placed: a keystroke must not silently become a network write (spec §14B.5)."
        }
        "Checkbox" => {
            "\n\nIts first argument is a two-way binding to a `Truth` signal, which must be \
             `client`-placed (spec §14B.5)."
        }
        _ => "",
    };
    format!("```zdeceptron\n{name}\n```\n\nA view element the language provides.{extra}")
}

fn variant_note(name: &str) -> String {
    let owner = match name {
        "Loading" | "Ready" | "Failed" => {
            "`Remote of T`, the type a read that crosses the network has (spec §5.2). All three \
             arms must be written, including ones the compiler can prove unreachable (§14G.1.6)."
        }
        "Some" | "None" => {
            "`Option of T`, which is how absence is spelled: there is no `null` and no `undefined` \
             (spec §5.4)."
        }
        _ => "a choice type.",
    };
    format!("```zdeceptron\n{name}\n```\n\nA variant of {owner}")
}

fn is_note(role: IsRole) -> String {
    match role {
        IsRole::Declaration => "```zdeceptron\nis\n```\n\nHere `is` introduces a declaration: it \
                                gives the name its placement and its type."
            .to_string(),
        IsRole::NamedArgument => "```zdeceptron\nis\n```\n\nHere `is` binds a **named argument**: \
                                  the name before it is the argument's name, not a value being \
                                  compared."
            .to_string(),
        IsRole::Equality => "```zdeceptron\nis\n```\n\nHere `is` tests **equality**. There is one \
                             equality operator, and it never coerces (spec §5.4)."
            .to_string(),
    }
}

/// A written type, as the program would have written it.
///
/// Bounded by the parser's type-nesting limit, so this cannot recurse
/// further than the source did.
fn render(ty: &ast::TypeExpr) -> String {
    match ty {
        ast::TypeExpr::Named(name) => name.text.clone(),
        ast::TypeExpr::List(inner) => format!("List of {}", render(inner)),
        ast::TypeExpr::Map(key, value) => format!("Map of {} to {}", render(key), render(value)),
        ast::TypeExpr::Option(inner) => format!("Option of {}", render(inner)),
        ast::TypeExpr::Remote(inner) => format!("Remote of {}", render(inner)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(src: &str, needle: &str) -> String {
        let analysis = Analysis::of(src);
        let offset = src.find(needle).expect("the needle is in the source") as u32;
        hover(&analysis, offset)
            .unwrap_or_else(|| panic!("no hover at {needle:?}"))
            .1
    }

    #[test]
    fn hovering_a_signal_gives_its_type_and_where_it_lives() {
        let text = at(
            "state count is client Whole starting 0\nview\n    Text count\n",
            "count is",
        );
        assert!(text.contains("Whole"), "{text}");
        assert!(text.contains("browser memory"), "{text}");
    }

    /// The hover this crate exists for: reading `server` state from the
    /// view is a network call, and the type says so.
    #[test]
    fn hovering_a_server_read_from_the_view_names_the_network() {
        let src = "state items is server List of Text starting empty\n\
                   view\n    when items\n        Loading show Spinner\n\
                   \x20       Failed with error show ErrorBar message is error.message\n\
                   \x20       Ready with ready show Text \"ok\"\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );

        let offset = src.find("when items").expect("the read") as u32 + 5;
        let (_, text) = hover(&analysis, offset).expect("a hover on the read");

        assert!(text.contains("serverless invocation"), "{text}");
        assert!(text.contains("Remote of List of Text"), "{text}");
        assert!(text.contains("crosses the network"), "{text}");
    }

    /// The same signal read from a `server` derivation does not cross a
    /// boundary, and the hover must not claim it does (§5.2's third row).
    #[test]
    fn a_read_that_does_not_cross_a_boundary_is_not_called_remote() {
        let src = "state raw is durable Whole starting 0\n\
                   state doubled is server Whole from raw * 2\n\
                   view\n    Text \"x\"\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );

        let offset = src.find("raw * 2").expect("the read") as u32;
        let (_, text) = hover(&analysis, offset).expect("a hover");
        assert!(text.contains("persistent store"), "{text}");
        assert!(!text.contains("crosses the network"), "{text}");
    }

    #[test]
    fn hovering_a_durable_signal_says_it_survives_a_reload() {
        let text = at("state votes is durable Whole starting 0\n", "votes");
        assert!(text.contains("survives a reload"), "{text}");
        assert!(text.contains("every visitor"), "{text}");
    }

    #[test]
    fn hovering_a_secret_signal_names_the_flow_rule() {
        let text = at(
            "secret state key is server Text from environment \"K\"\n",
            "key is",
        );
        assert!(text.contains("secret"), "{text}");
        assert!(text.contains("§5.3"), "{text}");
    }

    #[test]
    fn each_job_of_is_explains_itself_differently() {
        // `is` three times over. Note that the one in the view is a
        // named argument and not a comparison, which is exactly the
        // distinction being asserted.
        let src = "state open is client Truth starting no\n\
                   state shown is client Truth from open is yes\n\
                   view\n    Checkbox open, hint is \"search\"\n";
        let analysis = Analysis::of(src);

        let declaration = src.find(" is ").expect("declaration") as u32 + 1;
        let named = src.find("hint is").expect("named") as u32 + 5;
        let equality = src.rfind("open is yes").expect("equality") as u32 + 5;

        assert!(hover(&analysis, declaration)
            .expect("hover")
            .1
            .contains("introduces a declaration"));
        assert!(hover(&analysis, named)
            .expect("hover")
            .1
            .contains("named argument"));
        assert!(hover(&analysis, equality)
            .expect("hover")
            .1
            .contains("equality"));
    }

    #[test]
    fn hovering_a_local_gives_its_inferred_type() {
        let src = "function twice with n\n    give n * 2\nview\n    Text (twice with n is 2)\n";
        let analysis = Analysis::of(src);
        let offset = src.find("n * 2").expect("the use") as u32;
        let (_, text) = hover(&analysis, offset).expect("a hover");
        assert!(text.contains("Whole"), "{text}");
    }

    #[test]
    fn hovering_a_function_shows_its_parameters() {
        let text = at(
            "function twice with n\n    give n * 2\nview\n    Text (twice with n is 2)\n",
            "twice with n\n",
        );
        assert!(text.contains("function twice with n"), "{text}");
        assert!(text.contains("no placement"), "{text}");
    }

    #[test]
    fn hovering_an_element_says_it_is_one() {
        let text = at("view\n    Column\n", "Column");
        assert!(text.contains("view element"), "{text}");
    }

    #[test]
    fn hovering_a_bound_element_names_the_binding_rule() {
        let text = at(
            "state q is client Text starting \"\"\nview\n    Input q\n",
            "Input",
        );
        assert!(text.contains("two-way"), "{text}");
        assert!(text.contains("client"), "{text}");
    }

    #[test]
    fn hovering_anywhere_in_a_broken_file_never_panics() {
        let sources = [
            "",
            "state",
            "state x is client Whole starting",
            "{\"json\": true}",
            "view\n    Text (1 + 2\n",
            "\u{1f600} state \u{4e2d}",
        ];
        for src in sources {
            let analysis = Analysis::of(src);
            for offset in 0..=src.len() as u32 + 4 {
                let _ = hover(&analysis, offset);
            }
        }
    }
}
