#![forbid(unsafe_code)]

//! Rendering for compiler diagnostics.
//!
//! Spec §7.3: diagnostics are a primary deliverable. Because the grammar
//! admits exactly one phrasing per construct (§4.1), every syntax error
//! must be able to name that phrasing.

use ariadne::{Color, Label, Report, ReportKind, Source};
use zdc_lexer::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl From<zdc_parser::ParseError> for Diagnostic {
    fn from(e: zdc_parser::ParseError) -> Self {
        Diagnostic {
            message: e.message,
            span: e.span,
            help: None,
        }
    }
}

/// Render a diagnostic as a multi-span report against the source text.
pub fn render(src: &str, path: &str, diagnostic: &Diagnostic) -> String {
    let range: std::ops::Range<usize> = diagnostic.span.into();

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

#[cfg(test)]
mod tests {
    use super::*;

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
            span: zdc_lexer::Span::new(0, 5),
            help: Some("Try writing `starting empty`.".to_string()),
        };
        let out = render("state votes", "example.zd", &d);
        assert!(out.contains("Try writing"), "missing help:\n{out}");
    }
}
