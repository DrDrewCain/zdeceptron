//! What could be written here.
//!
//! §4.1 gives the language exactly one phrasing per construct, which makes
//! completion unusually well defined: after `state count is`, the only
//! words that may follow are the three placements, and the list is not a
//! ranking of likely guesses but the complete set of legal continuations.
//!
//! Position is read off the **token stream**, not the syntax tree, because
//! the file is being typed into and usually does not parse. That is the one
//! thing here that is a judgement rather than a lookup, and it is confined
//! to [`context`] so it can be read in one place. Every list of names it
//! then offers comes from the compiler: the elements from `zdc-resolve`,
//! the variants from the same table, the base types from `zdc-types`, and
//! the declared names from whatever of the file parsed.

use zdc_ast as ast;
use zdc_hir::DefKind;
use zdc_lexer::{Token, TokenKind};

use crate::analysis::Analysis;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Placement,
    Type,
    Element,
    Variant,
    Signal,
    Function,
}

/// Where in a line the cursor is, as far as the tokens on it can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    /// Nothing has been written on this line yet.
    TopOfLine,
    /// Directly after the `is` of a `state` declaration.
    AfterDeclarationIs,
    /// Inside a declaration's type, after a placement, `of`, or `to`.
    InType,
    /// Anywhere a value may be written.
    InValue,
    /// The cursor is inside a name being typed, so the client filters.
    Anywhere,
}

/// Every legal continuation at a byte offset.
pub fn complete(analysis: &Analysis, offset: u32) -> Vec<Completion> {
    let before = tokens_before(analysis, offset);
    match context(&before) {
        Context::AfterDeclarationIs => placements(),
        Context::InType => types(analysis),
        // A line may begin a declaration, a view node, or a `when` arm,
        // and nothing written yet says which.
        Context::TopOfLine => {
            let mut out = declaration_keywords();
            out.extend(node_keywords());
            out.extend(elements());
            out.extend(variants());
            out
        }
        Context::InValue | Context::Anywhere => {
            let mut out = names(analysis);
            out.extend(value_keywords());
            out.extend(elements());
            out
        }
    }
}

/// The tokens on the cursor's line that end before it.
///
/// Layout tokens are dropped: `Newline` carries the break *and* the next
/// line's indentation, so a token stream sliced by offset alone would put
/// the cursor on the previous line whenever it sits in the indentation.
fn tokens_before(analysis: &Analysis, offset: u32) -> Vec<&Token> {
    let text = analysis.text();
    let line_start = text
        .get(..offset.min(text.len() as u32) as usize)
        .and_then(|before| before.rfind('\n').map(|at| at as u32 + 1))
        .unwrap_or(0);

    analysis
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
        .collect()
}

fn context(before: &[&Token]) -> Context {
    let Some(last) = before.last() else {
        return Context::TopOfLine;
    };
    let declaring = matches!(
        before.first().map(|token| &token.kind),
        Some(TokenKind::State | TokenKind::Secret)
    );

    match &last.kind {
        TokenKind::Is if declaring => Context::AfterDeclarationIs,
        TokenKind::Client | TokenKind::Server | TokenKind::Durable => Context::InType,
        // `of` only ever opens a type argument. `to` does not: it is the
        // second half of `Map of K to V`, and it is also `set x to 1` and
        // `add 1 to x`. Which one it is follows from how the line began.
        TokenKind::Of => Context::InType,
        TokenKind::To if declaring => Context::InType,
        // A partly typed word: the client filters the full list by it, so
        // narrowing here would only remove candidates it would have hidden
        // anyway — including, sometimes, the one being typed.
        TokenKind::Ident(_) => Context::Anywhere,
        _ => Context::InValue,
    }
}

fn declaration_keywords() -> Vec<Completion> {
    vec![
        keyword("state", "Declare a signal."),
        keyword(
            "secret",
            "Mark a `server` or `durable` signal secret (spec §5.3).",
        ),
        keyword(
            "function",
            "Declare a function. Functions carry no placement (spec §5.1).",
        ),
        keyword("view", "Declare the program's view. There is exactly one."),
    ]
}

fn node_keywords() -> Vec<Completion> {
    vec![
        keyword("each", "Repeat the nodes below for every item of a list."),
        keyword(
            "when",
            "Eliminate a choice. Every arm must be written (spec §14G.1.6).",
        ),
        keyword(
            "on",
            "Handle a browser event. Legal only in client context (spec §5.6).",
        ),
    ]
}

fn value_keywords() -> Vec<Completion> {
    vec![
        keyword("yes", "The true value of `Truth`."),
        keyword("no", "The false value of `Truth`."),
        keyword("empty", "An empty `List` or `Map`."),
        keyword("not", "Boolean negation."),
        keyword("and", "Boolean conjunction."),
        keyword("or", "Boolean disjunction."),
        keyword("at", "Index a `List` or a `Map`."),
        keyword("is", "Test equality. It never coerces (spec §5.4)."),
        keyword("is not", "Test inequality."),
        keyword(
            "environment",
            "Read a process environment variable. Legal only in `server` or `durable` context \
             (spec §5.6).",
        ),
    ]
}

/// The three placements, with §5.1's table as their detail.
fn placements() -> Vec<Completion> {
    vec![
        Completion {
            label: "client".to_string(),
            kind: CompletionKind::Placement,
            detail: "Browser memory. Gone on reload, no secrets, read directly.".to_string(),
        },
        Completion {
            label: "server".to_string(),
            kind: CompletionKind::Placement,
            detail: "A serverless invocation. Gone on reload, may hold secrets, reached by RPC."
                .to_string(),
        },
        Completion {
            label: "durable".to_string(),
            kind: CompletionKind::Placement,
            detail: "A persistent store. Survives reload, may hold secrets, reached by RPC."
                .to_string(),
        },
    ]
}

/// The base types and the four constructors, from the compiler's own
/// tables rather than from a copy of them.
fn types(analysis: &Analysis) -> Vec<Completion> {
    let mut out: Vec<Completion> = zdc_types::Type::builtin_names()
        .iter()
        .map(|name| Completion {
            label: (*name).to_string(),
            kind: CompletionKind::Type,
            detail: "A type the language provides.".to_string(),
        })
        .collect();

    for (label, detail) in [
        ("List", "`List of T` — an ordered collection."),
        ("Map", "`Map of K to V` — a keyed collection."),
        (
            "Option",
            "`Option of T` — how absence is spelled. There is no `null`.",
        ),
        (
            "Remote",
            "`Remote of T` — a value on the far side of the network (spec §5.2).",
        ),
    ] {
        out.push(Completion {
            label: label.to_string(),
            kind: CompletionKind::Type,
            detail: detail.to_string(),
        });
    }

    // Types the file already names, so a second `List of Item` completes
    // from the first.
    for name in named_types(analysis) {
        out.push(Completion {
            label: name,
            kind: CompletionKind::Type,
            detail: "A type this program names.".to_string(),
        });
    }
    out
}

fn named_types(analysis: &Analysis) -> Vec<String> {
    let Some(hir) = analysis.hir() else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for (_, def) in hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        collect_named(&signal.ty, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn collect_named(ty: &ast::TypeExpr, out: &mut Vec<String>) {
    match ty {
        ast::TypeExpr::Named(name) => {
            if !zdc_types::Type::is_builtin_name(&name.text) {
                out.push(name.text.clone());
            }
        }
        ast::TypeExpr::List(inner)
        | ast::TypeExpr::Option(inner)
        | ast::TypeExpr::Remote(inner) => collect_named(inner, out),
        ast::TypeExpr::Map(key, value) => {
            collect_named(key, out);
            collect_named(value, out);
        }
    }
}

fn elements() -> Vec<Completion> {
    zdc_resolve::BUILTIN_ELEMENTS
        .iter()
        .map(|name| Completion {
            label: (*name).to_string(),
            kind: CompletionKind::Element,
            detail: "A view element the language provides.".to_string(),
        })
        .collect()
}

fn variants() -> Vec<Completion> {
    zdc_resolve::BUILTIN_PATTERNS
        .iter()
        .map(|name| Completion {
            label: (*name).to_string(),
            kind: CompletionKind::Variant,
            detail: match *name {
                "Loading" | "Ready" | "Failed" => "A variant of `Remote of T` (spec §5.2).",
                _ => "A variant of `Option of T` (spec §5.4).",
            }
            .to_string(),
        })
        .collect()
}

/// Every top-level name the file declares.
///
/// Falls back to the token stream when the file does not parse, which is
/// most of the time completion is asked for: the line being typed is the
/// one that does not parse yet, and the declarations above it are still
/// perfectly readable as tokens.
///
/// Locals are not offered either way. The HIR records no owner for a
/// binding, so there is no way to tell which body the cursor is in without
/// a scope pass this crate does not have — and offering every local in the
/// file would suggest names that are not in scope, which is worse than
/// offering none.
fn names(analysis: &Analysis) -> Vec<Completion> {
    match analysis.hir() {
        Some(hir) => hir
            .defs
            .iter()
            .filter_map(|(_, def)| match &def.kind {
                // A declared type is offerable wherever a type is written.
                DefKind::Record(_) => Some(Completion {
                    label: def.name.clone(),
                    kind: CompletionKind::Type,
                    detail: "a record you declared".to_string(),
                }),
                DefKind::Choice(_) => Some(Completion {
                    label: def.name.clone(),
                    kind: CompletionKind::Type,
                    detail: "a choice you declared".to_string(),
                }),
                DefKind::Signal(signal) => Some(Completion {
                    label: def.name.clone(),
                    kind: CompletionKind::Signal,
                    detail: format!(
                        "`{}` state{}",
                        placement_word(signal.placement),
                        if signal.is_source {
                            ""
                        } else {
                            ", derived with `from`"
                        }
                    ),
                }),
                DefKind::Function(function) => Some(Completion {
                    label: def.name.clone(),
                    kind: CompletionKind::Function,
                    detail: format!("A function of {} argument(s).", function.params.len()),
                }),
                DefKind::Component(component) => Some(Completion {
                    label: def.name.clone(),
                    kind: CompletionKind::Element,
                    detail: format!(
                        "a component you declared, taking {} argument(s)",
                        component.params.len()
                    ),
                }),
                DefKind::View(_) => None,
            })
            .collect(),
        None => declared_in_tokens(analysis.tokens()),
    }
}

/// `state <name>` and `function <name>`, read straight off the tokens.
fn declared_in_tokens(tokens: &[Token]) -> Vec<Completion> {
    let mut out = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        let TokenKind::Ident(name) = &token.kind else {
            continue;
        };
        match tokens.get(at.wrapping_sub(1)).map(|token| &token.kind) {
            Some(TokenKind::State) => out.push(Completion {
                label: name.clone(),
                kind: CompletionKind::Signal,
                detail: match tokens.get(at + 2).map(|token| &token.kind) {
                    Some(TokenKind::Client) => "`client` state".to_string(),
                    Some(TokenKind::Server) => "`server` state".to_string(),
                    Some(TokenKind::Durable) => "`durable` state".to_string(),
                    _ => "A signal declared in this file.".to_string(),
                },
            }),
            Some(TokenKind::Function) => out.push(Completion {
                label: name.clone(),
                kind: CompletionKind::Function,
                detail: "A function declared in this file.".to_string(),
            }),
            _ => {}
        }
    }
    out
}

fn placement_word(placement: ast::Placement) -> &'static str {
    match placement {
        ast::Placement::Client => "client",
        ast::Placement::Server => "server",
        ast::Placement::Durable => "durable",
    }
}

fn keyword(label: &str, detail: &str) -> Completion {
    Completion {
        label: label.to_string(),
        kind: CompletionKind::Keyword,
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(src: &str, after: &str) -> Vec<String> {
        let analysis = Analysis::of(src);
        let offset =
            src.find(after).expect("the needle is in the source") as u32 + after.len() as u32;
        complete(&analysis, offset)
            .into_iter()
            .map(|item| item.label)
            .collect()
    }

    /// The list after `is` in a declaration is not a guess: §5.1 says
    /// there are exactly three placements and nothing else may follow.
    #[test]
    fn after_the_is_of_a_declaration_only_placements_are_offered() {
        let offered = labels("state count is ", "state count is ");
        assert_eq!(offered, vec!["client", "server", "durable"]);
    }

    #[test]
    fn after_a_placement_types_are_offered() {
        let offered = labels("state count is client ", "is client ");
        assert!(offered.contains(&"Whole".to_string()), "{offered:?}");
        assert!(offered.contains(&"List".to_string()), "{offered:?}");
        assert!(!offered.contains(&"client".to_string()), "{offered:?}");
    }

    #[test]
    fn after_of_the_element_type_is_offered() {
        let offered = labels("state xs is client List of ", "List of ");
        assert!(offered.contains(&"Text".to_string()), "{offered:?}");
    }

    /// The completion list is the compiler's table, so a new element
    /// cannot appear in one and not the other.
    #[test]
    fn every_element_the_resolver_accepts_is_offered() {
        let offered = labels(
            "state a is client Whole starting 0\nview\n    ",
            "view\n    ",
        );
        for element in zdc_resolve::BUILTIN_ELEMENTS {
            assert!(
                offered.contains(&(*element).to_string()),
                "{element} is not offered: {offered:?}"
            );
        }
    }

    #[test]
    fn declared_names_are_offered_where_a_value_goes() {
        let src = "state count is client Whole starting 0\n\
                   function twice with n\n    give n * 2\n\
                   state other is client Whole starting ";
        let offered = labels(src, "state other is client Whole starting ");
        assert!(offered.contains(&"count".to_string()), "{offered:?}");
        assert!(offered.contains(&"twice".to_string()), "{offered:?}");
        assert!(offered.contains(&"yes".to_string()), "{offered:?}");
    }

    #[test]
    fn a_top_of_line_offers_the_declaration_keywords() {
        let offered = labels("state a is client Whole starting 0\n", "starting 0\n");
        assert!(offered.contains(&"state".to_string()), "{offered:?}");
        assert!(offered.contains(&"function".to_string()), "{offered:?}");
        assert!(offered.contains(&"view".to_string()), "{offered:?}");
    }

    #[test]
    fn a_placement_completion_says_where_the_value_would_live() {
        let analysis = Analysis::of("state count is ");
        let offered = complete(&analysis, 15);
        let durable = offered
            .iter()
            .find(|item| item.label == "durable")
            .expect("durable is offered");
        assert!(durable.detail.contains("Survives reload"), "{durable:?}");
    }

    #[test]
    fn completing_anywhere_in_a_broken_file_never_panics() {
        let sources = [
            "",
            "state",
            "{\"json\": 1}",
            "view\n    Text (1 + 2\n",
            "\u{1f600}",
        ];
        for src in sources {
            let analysis = Analysis::of(src);
            for offset in 0..=src.len() as u32 + 4 {
                let _ = complete(&analysis, offset);
            }
        }
    }
}
