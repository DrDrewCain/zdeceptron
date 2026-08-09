//! The words this compiler uses about a declaration, written once.
//!
//! **Why this is a module and not two copies.** The language server has
//! computed hover text from declarations since §14: it reconstructs the
//! declaration line, then says where the value lives and what a read of it
//! costs. A documentation generator needs the same two sentences about the
//! same declarations. Writing them again here would produce a file that
//! agrees with the editor on the day it is written and drifts from it
//! afterwards — and the drift is invisible, because nothing compares the
//! two. So the prose lives here, `zdc-lsp` depends on this crate, and
//! `hover.rs` calls these functions. The editor and the generated
//! documentation cannot disagree, for the same reason `zdc check` and the
//! editor cannot: there is one implementation, not two that are checked
//! against each other.
//!
//! Everything here is derived from a declaration and nothing is derived
//! from a body. That is the boundary that keeps the module honest: a
//! sentence about what a program *does* would need a reader of statements
//! and would be a guess. A sentence about where a signal lives is read off
//! the word the programmer wrote.

use zdc_ast as ast;
use zdc_types::{read_kind, ReadContext, ReadKind, SignalPlacement, Type};

/// A declaration line in the fenced block both the editor and the
/// generated pages show it in.
///
/// The language tag is the one the editors in `editors/` register, so a
/// generated page and a hover popup highlight identically.
pub fn fenced(line: &str) -> String {
    format!("```zdeceptron\n{line}\n```")
}

/// A written type, as the program would have written it.
///
/// Bounded by the parser's type-nesting limit, so this cannot recurse
/// further than the source did.
pub fn render_type(ty: &ast::TypeExpr) -> String {
    match ty {
        ast::TypeExpr::Named(name) => name.text.clone(),
        ast::TypeExpr::List(inner) => format!("List of {}", render_type(inner)),
        ast::TypeExpr::Map(key, value) => {
            format!("Map of {} to {}", render_type(key), render_type(value))
        }
        ast::TypeExpr::Pair(first, second) => {
            format!("Pair of {} to {}", render_type(first), render_type(second))
        }
        ast::TypeExpr::Option(inner) => format!("Option of {}", render_type(inner)),
        ast::TypeExpr::Remote(inner) => format!("Remote of {}", render_type(inner)),
    }
}

/// The declaration line of a signal, reconstructed from the tree.
///
/// The initialiser is elided. A `from` clause is an expression of any size
/// and the declaration's *shape* is what this line exists to show; the
/// expression itself is in the source, one click away in the editor and
/// one link away on a page.
pub fn signal_line(
    name: &str,
    placement: ast::Placement,
    ty: &str,
    secret: bool,
    is_source: bool,
) -> String {
    let secret = if secret { "secret " } else { "" };
    let init = if is_source { "starting" } else { "from" };
    format!("{secret}state {name} is {} {ty} {init} …", placement.word())
}

/// The declaration line of a function, in the one form it may be called in.
///
/// §17.4.2 gives a function exactly one calling form, so rendering `with`
/// for a function declared `of` would print a line that does not compile.
pub fn function_line(name: &str, params: &[String], form: ast::CallForm) -> String {
    match form {
        // A `with` function may have no parameters at all, and `function f
        // with` is not a thing anyone may write.
        ast::CallForm::With if params.is_empty() => format!("function {name}"),
        ast::CallForm::With => format!("function {name} with {}", params.join(", ")),
        // Exactly one parameter by construction (§14F.1). Joining is still
        // the right expression: a malformed tree prints oddly rather than
        // panicking, and nothing here is worth a panic.
        ast::CallForm::Of => format!("function {name} of {}", params.join(", ")),
    }
}

/// The declaration line of a component (§14D.1).
pub fn component_line(name: &str, params: &[String], takes_children: bool) -> String {
    let mut params = params.to_vec();
    if takes_children {
        params.push("children".to_string());
    }
    if params.is_empty() {
        format!("component {name}")
    } else {
        format!("component {name} with {}", params.join(", "))
    }
}

/// A `foreign` as it was declared: where it may be linked, what it
/// imports, what it takes and what it gives back (§14E.1, §21.9).
///
/// The parameter names are passed in rather than read out of the
/// declaration, because a `Foreign` holds `LocalId`s and the arena that
/// resolves them belongs to the caller.
pub fn foreign_line(name: &str, foreign: &zdc_hir::Foreign, param_names: &[String]) -> String {
    let gives = match &foreign.result {
        ast::ForeignResult::View => "view".to_string(),
        ast::ForeignResult::Value(ty) => match foreign.result_grant.describe() {
            Some(grant) => format!("{grant} {}", render_type(ty)),
            None => render_type(ty),
        },
    };

    let mut out = format!("foreign {name} is {}", foreign.site.describe());
    out.push_str(&format!(
        "\n    from \"{}\" as \"{}\"",
        foreign.module, foreign.export
    ));
    for ((param, ty), trusted) in param_names
        .iter()
        .zip(&foreign.param_types)
        .zip(&foreign.trusted_params)
    {
        let trusted = if *trusted { "trusted " } else { "" };
        out.push_str(&format!(
            "\n    takes {param} is {trusted}{}",
            render_type(ty)
        ));
    }
    out.push_str(&format!("\n    gives {gives}"));
    out
}

/// §5.1's table, in a sentence, with the subject left to the caller.
///
/// The subject is a parameter because the same sentence answers two
/// questions: "what is `visits`?" on a hover, and "what does `durable`
/// mean?" in a page's legend. One sentence, two subjects, no second copy.
pub fn placement_sentence(placement: ast::Placement) -> &'static str {
    match placement {
        ast::Placement::Client => {
            "lives in **browser memory**. It does not survive a reload, it may not hold secrets, \
             and the client reads it directly."
        }
        ast::Placement::Static => {
            "is computed **once at build time** and inlined into every page that reads it. It \
             costs no network request, it may not hold secrets, it may not be written, and it is \
             what a route parameter's `in` ranges over (spec §14C.3b)."
        }
        ast::Placement::Server => {
            "lives in a **serverless invocation**. It does not survive a reload, it may hold \
             secrets, and the client reaches it only through generated RPC."
        }
        ast::Placement::Durable => {
            "lives in a **persistent store**. It survives a reload, it may hold secrets, and the \
             client reaches it only through generated RPC. It is global: one value shared by \
             every visitor (spec §5.7)."
        }
    }
}

/// The same sentence about one named signal. This is the whole point of
/// the hover, and half the point of a generated page.
pub fn placement_note(name: &str, placement: ast::Placement) -> String {
    format!("`{name}` {}", placement_sentence(placement))
}

/// Which bundles a `foreign` may be linked into, in a sentence (§14E.1).
///
/// The keyword is not a sentence: `is anywhere` composed into "may be
/// linked into anywhere output", which is the kind of line that tells a
/// reader the text was assembled rather than written. Spelled out per site
/// instead, over a total match, so a fourth site is a compile error here
/// rather than a fourth ungrammatical sentence.
pub fn foreign_site_note(site: ast::ForeignSite) -> &'static str {
    match site {
        ast::ForeignSite::Client => "It may be linked into the client bundle only.",
        ast::ForeignSite::Server => "It may be linked into a server bundle only.",
        ast::ForeignSite::Anywhere => "It may be linked into either bundle.",
    }
}

pub const SECRET_NOTE: &str = "It is `secret`: no value derived from it may reach `client` state \
                               or the view (spec §5.3).";

pub const DERIVED_NOTE: &str = "It is derived with `from`, so it is recomputed when its inputs \
                                change and cannot be assigned to (spec §4.5).";

pub const FUNCTION_NOTE: &str =
    "Functions carry no placement: one runs wherever its inputs are (spec §5.1).";

pub const COMPONENT_NOTE: &str =
    "A component is written where a built-in element is, and carries no placement of its own: it \
     runs wherever its inputs are (spec §14D.1).";

pub const CROSSES_NOTE: &str =
    "This read crosses the network, so the value is not available until a `when` has eliminated \
     `Loading`, `Ready` and `Failed` — all three, in every context (spec §14G.1.6).";

/// Whether a read produced `Remote of T` from a signal whose own type is
/// not remote — that is, whether the boundary is in the type because the
/// read crossed it, rather than because the program declared it so.
pub fn crosses_a_boundary(read: &Type, declared: Option<&Type>) -> bool {
    match (read, declared) {
        (Type::Remote(_), Some(Type::Remote(_))) => false,
        (Type::Remote(_), _) => true,
        _ => false,
    }
}

/// What the browser gets when it reads this signal, in a phrase.
///
/// **This is the column no other language's documentation can have.** In a
/// language where the network is a library call, the fact that reading
/// `visits` is a round trip is in the call site, not in the declaration,
/// so a generator has nothing to print. Here the declared placement plus
/// §14G.1.4's read table answers it for every signal in the program, and
/// the answer is the type the checker will hand a reader — asked of
/// [`read_kind`] itself, so this cannot say `Text` where the checker says
/// `Remote of Text`.
///
/// `secret` is answered before the table, because it is a stronger
/// statement: the read table says what a read *would* yield, and the flow
/// pass says this read does not compile at all (§5.3).
pub fn from_the_browser(placement: ast::Placement, secret: bool, ty: &str) -> String {
    if secret {
        return "not at all — it is `secret` (§5.3)".to_string();
    }
    match read_kind(ReadContext::Client, SignalPlacement::from_ast(placement)) {
        ReadKind::Direct => format!("`{ty}`"),
        ReadKind::Remote => format!("`Remote of {ty}` — the network is here"),
        // Unreachable from the client row of §14G.1.4 as written today.
        // Written out rather than swallowed by a wildcard: a fifth
        // placement, or a change to the table, must be a decision made
        // here rather than a phrase that quietly becomes wrong.
        ReadKind::Forbidden(why) => format!("not at all — {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_derived_secret_server_signal_renders_the_line_it_was_written_as() {
        assert_eq!(
            signal_line("apiKey", ast::Placement::Server, "Text", true, false),
            "secret state apiKey is server Text from …"
        );
    }

    #[test]
    fn a_function_declared_with_of_is_never_printed_with_with() {
        assert_eq!(
            function_line("length", &["value".to_string()], ast::CallForm::Of),
            "function length of value"
        );
    }

    /// The claim `from_the_browser` exists to make.
    #[test]
    fn reading_durable_state_from_the_browser_is_remote_and_reading_client_state_is_not() {
        assert_eq!(
            from_the_browser(ast::Placement::Durable, false, "Whole"),
            "`Remote of Whole` — the network is here"
        );
        assert_eq!(
            from_the_browser(ast::Placement::Client, false, "Text"),
            "`Text`"
        );
    }

    /// A `static` signal costs no request, so calling it remote would be
    /// the one row that misleads about performance as well as about types.
    #[test]
    fn a_static_signal_is_read_directly_because_it_was_inlined() {
        assert_eq!(
            from_the_browser(ast::Placement::Static, false, "List of Text"),
            "`List of Text`"
        );
    }

    #[test]
    fn a_secret_is_not_readable_from_the_browser_at_any_type() {
        let answer = from_the_browser(ast::Placement::Server, true, "Text");
        assert!(answer.contains("secret"), "{answer}");
        assert!(!answer.contains("Remote"), "{answer}");
    }

    /// The keyword `anywhere` does not compose into a sentence, which is
    /// what this function exists to stop.
    #[test]
    fn every_foreign_site_has_a_sentence_that_reads_as_one() {
        for site in [
            ast::ForeignSite::Client,
            ast::ForeignSite::Server,
            ast::ForeignSite::Anywhere,
        ] {
            let note = foreign_site_note(site);
            assert!(note.ends_with('.'), "{note}");
            assert!(!note.contains("anywhere output"), "{note}");
        }
    }

    #[test]
    fn every_placement_has_a_sentence_and_no_two_are_the_same() {
        // Over `Placement::ALL` rather than a list written here: a fifth
        // placement must fail this test rather than slip past it.
        let mut sentences: Vec<&str> = ast::Placement::ALL
            .iter()
            .map(|p| placement_sentence(*p))
            .collect();
        assert_eq!(sentences.len(), 4);
        sentences.sort_unstable();
        sentences.dedup();
        assert_eq!(sentences.len(), 4, "two placements share a sentence");
    }
}
