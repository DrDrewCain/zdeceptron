#![forbid(unsafe_code)]

//! Rendering for compiler diagnostics.
//!
//! Spec §7.3: diagnostics are a primary deliverable. Because the grammar
//! admits exactly one phrasing per construct (§4.1), every syntax error
//! must be able to name that phrasing.

use ariadne::{Color, Label, Report, ReportKind, Source};
use zdc_lexer::Span;

/// A diagnostic either points at a byte span within a known source text
/// (a parse error), or has no location at all (a file-level error: the
/// file could not be found, read, or decoded). These are deliberately
/// distinct at the type level — `Option<Span>`, not a sentinel span like
/// `Span::new(0, 0)`, which would render a caret pointing at a byte that
/// does not exist.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Option<Span>,
    pub help: Option<String>,
}

impl Diagnostic {
    /// A diagnostic about a file rather than about a location within one:
    /// the file could not be read, was not found, or is not valid UTF-8.
    pub fn file_error(message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            message: message.into(),
            span: None,
            help: None,
        }
    }
}

impl From<zdc_parser::ParseError> for Diagnostic {
    fn from(e: zdc_parser::ParseError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            help: None,
        }
    }
}

impl From<zdc_resolve::ResolveError> for Diagnostic {
    fn from(e: zdc_resolve::ResolveError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            help: None,
        }
    }
}

impl From<zdc_codegen::CodegenError> for Diagnostic {
    fn from(e: zdc_codegen::CodegenError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            help: None,
        }
    }
}

/// Render a diagnostic as a report against the source text.
///
/// A spanless (file-level) diagnostic has no source text to snippet and no
/// byte range to point a caret at, so it is formatted directly rather than
/// forcing a fake span through `ariadne`.
pub fn render(src: &str, path: &str, diagnostic: &Diagnostic) -> String {
    let Some(span) = diagnostic.span else {
        return render_file_error(path, diagnostic);
    };

    let range: std::ops::Range<usize> = span.into();

    let mut builder = Report::build(ReportKind::Error, path, range.start)
        .with_message(&diagnostic.message)
        .with_label(
            Label::new((path, range))
                .with_message("here")
                .with_color(Color::Red),
        );

    if let Some(help) = &diagnostic.help {
        builder = builder.with_help(help);
    }

    let mut buffer = Vec::new();
    builder
        .finish()
        .write((path, Source::from(src)), &mut buffer)
        .expect("writing to an in-memory buffer cannot fail");

    String::from_utf8(buffer).expect("ariadne emits valid UTF-8")
}

/// Render a file-level diagnostic: message and path, no snippet, no caret.
fn render_file_error(path: &str, diagnostic: &Diagnostic) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "Error: {}", diagnostic.message);
    let _ = writeln!(out, "  --> {path}");
    if let Some(help) = &diagnostic.help {
        let _ = writeln!(out, "  help: {help}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ariadne` colors the highlighted source line character-by-character,
    /// which splits multi-character substrings with ANSI escapes. Strip
    /// them so tests can assert on plain text.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn rendered_output_contains_the_source_line_and_message() {
        let src = "state votes is Map of Id to Int starting empty";
        let error = zdc_parser::parse(src).unwrap_err();
        let out = render(src, "example.zd", &Diagnostic::from(error));
        assert!(out.contains("example.zd"), "missing path:\n{out}");
        assert!(out.contains("client"), "missing the valid forms:\n{out}");
    }

    #[test]
    fn help_text_is_included_when_present() {
        let d = Diagnostic {
            message: "Something went wrong.".to_string(),
            span: Some(zdc_lexer::Span::new(0, 5)),
            help: Some("Try writing `starting empty`.".to_string()),
        };
        let out = render("state votes", "example.zd", &d);
        assert!(out.contains("Try writing"), "missing help:\n{out}");
    }

    #[test]
    fn spanned_diagnostics_still_render_the_source_snippet() {
        // Regression check: introducing the spanless case must not change
        // the normal (spanned) rendering path.
        let src = "state votes is Map of Id to Int starting empty";
        let error = zdc_parser::parse(src).unwrap_err();
        let out = render(src, "example.zd", &Diagnostic::from(error));
        let plain = strip_ansi(&out);
        assert!(
            plain.contains("Map"),
            "expected the offending source snippet to be quoted:\n{out}"
        );
        assert!(plain.contains('│'), "expected a source-line gutter:\n{out}");
    }

    #[test]
    fn spanless_diagnostics_render_message_and_path_without_a_snippet() {
        let d = Diagnostic::file_error("Could not read nope.zd: No such file or directory");
        let out = render("", "nope.zd", &d);

        assert!(out.contains("nope.zd"), "missing path:\n{out}");
        assert!(
            out.contains("No such file or directory"),
            "missing the underlying cause:\n{out}"
        );
        assert!(
            !out.contains('┬'),
            "spanless diagnostics must not draw a caret:\n{out}"
        );
        assert!(
            !out.contains('│'),
            "spanless diagnostics must not draw a source-line gutter:\n{out}"
        );
    }

    #[test]
    fn rendering_a_spanless_diagnostic_does_not_panic() {
        // Regardless of what `src` is passed (it is irrelevant for a
        // file-level error), rendering must not panic.
        let d = Diagnostic::file_error("boom");
        let _ = render("anything, or nothing at all", "path.zd", &d);
        let _ = render("", "path.zd", &d);
    }
}
