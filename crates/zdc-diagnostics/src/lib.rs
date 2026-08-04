#![forbid(unsafe_code)]

//! Rendering for compiler diagnostics.
//!
//! Spec §7.3: diagnostics are a primary deliverable. Because the grammar
//! admits exactly one phrasing per construct (§4.1), every syntax error
//! must be able to name that phrasing.
//!
//! Naming it is not the same as explaining it, and the two have different
//! costs. Barik et al. measured that reading error messages consumes
//! 13–25% of a developer's fixations and that reading difficulty predicts
//! task time, so what a diagnostic says inline is budgeted: the claim, the
//! spans, and one line pointing at [`explain`]. The rule itself — why it
//! exists, and a worked repair — lives in [`explain`] and is printed on
//! request by `zdc explain <CODE>`.

pub mod explain;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use zdc_lexer::Span;

pub use explain::{explain, Explanation, INLINE_MESSAGE_BUDGET};

/// A diagnostic either points at a byte span within a known source text
/// (a parse error), or has no location at all (a file-level error: the
/// file could not be found, read, or decoded). These are deliberately
/// distinct at the type level — `Option<Span>`, not a sentinel span like
/// `Span::new(0, 0)`, which would render a caret pointing at a byte that
/// does not exist.
///
/// A diagnostic may also carry **notes**: further spans, each with its own
/// message, rendered as additional labels on the same report. Spec §7.3
/// asks the information-flow pass to "show the path along which the secret
/// would have escaped", and a path is inherently more than one span. One
/// label per step is what makes an escape readable rather than merely
/// reported (§17.2.2(d), §17.3.8).
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Option<Span>,
    /// Further spans, in the order they should be read. Rendered as
    /// secondary labels, so `ariadne` draws the whole path at once.
    pub notes: Vec<(Span, String)>,
    pub help: Option<String>,
    /// The spec code, for the diagnostics that have one. A code is what
    /// makes progressive disclosure possible: it is the handle the reader
    /// passes to `zdc explain`, and it is stable across every rewording of
    /// the message.
    pub code: Option<&'static str>,
}

impl Diagnostic {
    /// A diagnostic about a file rather than about a location within one:
    /// the file could not be read, was not found, or is not valid UTF-8.
    pub fn file_error(message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            message: message.into(),
            span: None,
            notes: Vec::new(),
            help: None,
            code: None,
        }
    }
}

impl From<zdc_parser::ParseError> for Diagnostic {
    fn from(e: zdc_parser::ParseError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            notes: Vec::new(),
            help: None,
            code: None,
        }
    }
}

impl From<zdc_resolve::ResolveError> for Diagnostic {
    fn from(e: zdc_resolve::ResolveError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            notes: Vec::new(),
            help: None,
            code: None,
        }
    }
}

/// A type error already carries its own help text, because §7.3 asks a
/// diagnostic to name what was expected, what was found, and where — and
/// for the exhaustiveness rules the "why" belongs in help rather than in
/// the message.
impl From<zdc_types::TypeError> for Diagnostic {
    fn from(e: zdc_types::TypeError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            notes: Vec::new(),
            help: e.help,
            code: None,
        }
    }
}

/// The placement and information-flow passes carry a spec code and, more
/// importantly, a **path**: §17.2.10 prints "reached: hourly → ingest →
/// name" and §17.3.8 prints the steps a secret would take to escape.
/// Neither is expressible as one span, which is why `notes` exists.
///
/// The help line is generated rather than carried. A coded diagnostic's
/// prose lives in [`explain`], in one place, so there is nowhere for the
/// inline text and the full rule to drift apart — and the inline form
/// stays inside the budget by construction rather than by review.
impl From<zdc_graph::GraphError> for Diagnostic {
    fn from(e: zdc_graph::GraphError) -> Self {
        // `render` already prefixes "Error:", so the code is bracketed
        // rather than re-spelling the word: `Error: [E-IFC-05] …`.
        Diagnostic {
            message: format!("[{}] {}", e.code, e.message),
            span: Some(e.span),
            notes: e.notes,
            help: Some(explain::inline_help(e.code)),
            code: Some(e.code),
        }
    }
}

impl From<zdc_codegen::CodegenError> for Diagnostic {
    fn from(e: zdc_codegen::CodegenError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            notes: Vec::new(),
            help: None,
            code: None,
        }
    }
}

/// Render a diagnostic as a report against the source text.
///
/// A spanless (file-level) diagnostic has no source text to snippet and no
/// byte range to point a caret at, so it is formatted directly rather than
/// forcing a fake span through `ariadne`.
pub fn render(src: &str, path: &str, diagnostic: &Diagnostic) -> String {
    let diagnostic = &Diagnostic {
        message: printable(&diagnostic.message),
        span: diagnostic.span,
        notes: diagnostic
            .notes
            .iter()
            .map(|(span, note)| (*span, printable(note)))
            .collect(),
        help: diagnostic.help.as_deref().map(printable),
        code: diagnostic.code,
    };
    let src = &printable(src);
    let path = &printable(path);

    let Some(span) = diagnostic.span else {
        return render_file_error(path, diagnostic);
    };

    let range: std::ops::Range<usize> = span.into();

    // `Span` is a byte range — the lexer produces byte offsets and every
    // pass carries them unchanged — while `ariadne` counts characters by
    // default. Left alone, a single `#` comment containing an em dash
    // slides every caret in the file, which seven of the eight checked-in
    // examples would do.
    let mut builder = Report::build(ReportKind::Error, path, range.start)
        .with_config(Config::default().with_index_type(IndexType::Byte))
        .with_message(&diagnostic.message)
        .with_label(
            Label::new((path, range))
                .with_message("here")
                .with_color(Color::Red),
        );

    // Notes are ordered: step one of an escape path must render above step
    // two. `ariadne` orders labels by their span, not by insertion, so the
    // order is restated in the message rather than left to the layout.
    for (step, (span, message)) in diagnostic.notes.iter().enumerate() {
        let range: std::ops::Range<usize> = (*span).into();
        builder = builder.with_label(
            Label::new((path, range))
                .with_message(format!("{}. {message}", step + 1))
                .with_color(Color::Yellow)
                .with_order(step as i32),
        );
    }

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

/// Every C0 control except tab and newline replaced by `?`, byte for byte.
///
/// A diagnostic quotes the program back at whoever is reading it: the
/// message interpolates the program's own names and string literals, and
/// the snippet is the source line itself. A `.zd` string literal is
/// `"[^"\n]*"`, which admits U+001B — so `state a is client Text starting
/// "\u{1b}[2J\u{1b}[H"` is a *compiler diagnostic* that clears the
/// reader's terminal, and one carrying `\u{1b}]0;…\u{7}` retitles the
/// window. A file that fails to compile is exactly the file least likely
/// to have been read first.
///
/// The substitution is one byte for one byte, and every byte it touches is
/// below 0x80, so it can never fall inside a multi-byte sequence and every
/// [`Span`] in the file still points where it did. A renderer that
/// stripped these instead would slide every caret after the first one.
fn printable(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    for byte in bytes.iter_mut() {
        let control = *byte < 0x20 || *byte == 0x7f;
        if control && *byte != b'\t' && *byte != b'\n' {
            *byte = b'?';
        }
    }
    String::from_utf8(bytes).expect("only sub-0x80 bytes were replaced, by another such byte")
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

    /// A `.zd` string literal is `"[^"\n]*"`, so it admits U+001B. The
    /// snippet a diagnostic quotes is the source line itself, and a
    /// terminal reading `\u{1b}[2J` clears itself. This is the one path
    /// where a string a program wrote reaches something that *interprets*
    /// it without ever passing through the emitter.
    #[test]
    fn a_source_file_cannot_write_escape_sequences_to_the_terminal() {
        let src = "state a is client Text starting \"\u{1b}[2J\u{1b}]0;pwned\u{7}\"\nnope\n";
        let error = zdc_parser::parse(src).unwrap_err();
        let out = render(src, "example.zd", &Diagnostic::from(error));
        let colour = out.matches('\u{1b}').count();
        assert!(colour > 0, "ariadne's own colours were stripped:\n{out}");
        assert!(
            !out.contains("\u{1b}[2J") && !out.contains("\u{1b}]0;"),
            "the program's escape sequences reached the terminal:\n{out:?}"
        );
    }

    /// The substitution is byte for byte, so a caret still lands on the
    /// token the diagnostic is about. Stripping instead would slide every
    /// span after the first control character.
    #[test]
    fn replacing_a_control_character_does_not_move_the_caret() {
        let src = "# \u{1b}[31m comment\nstate a is client Whole starting nope\n";
        let offending = src.find("nope").expect("the token is in the source") as u32;
        let d = Diagnostic {
            message: "`nope` is not defined.".to_string(),
            span: Some(zdc_lexer::Span::new(offending, offending + 4)),
            notes: Vec::new(),
            help: None,
            code: None,
        };
        let plain = strip_ansi(&render(src, "example.zd", &d));
        let caret = plain
            .lines()
            .find(|line| line.contains("here"))
            .expect("a caret line");
        assert!(caret.contains("here"), "{plain}");
        assert!(
            plain.contains("state a is client Whole starting nope"),
            "the source line moved:\n{plain}"
        );
    }

    /// A diagnostic interpolates the program's own text into its message —
    /// `environment "…"` names its key — so the message needs the same
    /// treatment the snippet gets.
    #[test]
    fn a_message_quoting_the_program_cannot_write_escape_sequences_either() {
        let d = Diagnostic {
            message: "`\u{1b}[2J` is not defined.".to_string(),
            span: Some(zdc_lexer::Span::new(0, 5)),
            notes: Vec::new(),
            help: Some("Try \u{1b}]0;pwned\u{7}.".to_string()),
            code: None,
        };
        let out = render("state votes", "example.zd", &d);
        assert!(
            !out.contains("\u{1b}[2J"),
            "message escapes leaked:\n{out:?}"
        );
        assert!(!out.contains("\u{1b}]0;"), "help escapes leaked:\n{out:?}");
    }

    #[test]
    fn help_text_is_included_when_present() {
        let d = Diagnostic {
            message: "Something went wrong.".to_string(),
            span: Some(zdc_lexer::Span::new(0, 5)),
            notes: Vec::new(),
            help: Some("Try writing `starting empty`.".to_string()),
            code: None,
        };
        let out = render("state votes", "example.zd", &d);
        assert!(out.contains("Try writing"), "missing help:\n{out}");
    }

    /// §7.3 asks a rejected program to be shown *the path* along which a
    /// value would have escaped. One span cannot draw a path, so every
    /// note gets its own numbered label on the same report.
    #[test]
    fn every_note_is_rendered_as_its_own_numbered_label() {
        let src = "secret state key is server Text from environment \"K\"\nstate leak is client Text from key\n";
        let declared = src.find("key").expect("the declaration") as u32;
        let used = src.rfind("key").expect("the use") as u32;
        let d = Diagnostic {
            message: "`leak` is not declared secret.".to_string(),
            span: Some(Span::new(used, used + 3)),
            notes: vec![
                (Span::new(declared, declared + 3), "declared secret".into()),
                (Span::new(used, used + 3), "read here".into()),
            ],
            help: None,
            code: None,
        };
        let plain = strip_ansi(&render(src, "leak.zd", &d));

        assert!(plain.contains("1. declared secret"), "{plain}");
        assert!(plain.contains("2. read here"), "{plain}");
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

    /// Spans are byte offsets. A file with any character outside ASCII —
    /// an em dash in a comment is enough — must still put the caret under
    /// the token the diagnostic is about.
    #[test]
    fn a_caret_lands_correctly_in_a_file_containing_non_ascii() {
        let src = "# an em dash — right here\nstate a is client Whole starting nope\n";
        let offending = src.find("nope").expect("the token is in the source") as u32;
        let d = Diagnostic {
            message: "`nope` is not defined.".to_string(),
            span: Some(zdc_lexer::Span::new(offending, offending + 4)),
            notes: Vec::new(),
            help: None,
            code: None,
        };
        let plain = strip_ansi(&render(src, "example.zd", &d));

        let underline = plain
            .lines()
            .find(|line| line.contains('┬'))
            .expect("a caret line");
        let source = plain
            .lines()
            .find(|line| line.contains("starting nope"))
            .expect("the offending source line");

        // Read both columns off the same rendered text: they line up only
        // if the byte range was interpreted as bytes.
        let underline_at = underline
            .chars()
            .position(|c| c == '─')
            .expect("an underline");
        let token_at = source
            .char_indices()
            .position(|(at, _)| source[at..].starts_with("nope"))
            .expect("the token on its line");

        assert_eq!(
            underline_at, token_at,
            "the underline is under the wrong characters:\n{plain}"
        );
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
