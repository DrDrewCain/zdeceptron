use crate::raw::{tokenize_raw, RawToken};
use crate::{Span, Token, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

/// Convert source text into a token stream with explicit layout.
///
/// Indentation is spaces only; tabs are rejected outright to avoid the
/// tab/space ambiguity class of bug entirely.
pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    if let Some(width) = leading_indentation(src) {
        return Err(LexError {
            message: "The first line of a file begins at the left margin. Indentation opens a \
                      block inside the line above it, and here there is no line above."
                .to_string(),
            span: Span::new(0, width),
        });
    }

    // Before `logos` sees the source, not after: the scan this bounds is
    // the one that would have aborted (see `raw::MAX_TOKEN_CHARS`).
    if let Some((span, length)) = crate::raw::over_long_run(src) {
        return Err(LexError {
            message: format!(
                "This runs {length} characters without a break. The longest word ZDeceptron reads \
                 is {}, and something this long is a corrupted or truncated file rather than a \
                 name.",
                crate::raw::MAX_TOKEN_CHARS
            ),
            span,
        });
    }

    let raw = tokenize_raw(src);
    let mut out: Vec<Token> = Vec::new();
    let mut levels: Vec<u32> = vec![0];
    let eof = Span::new(src.len() as u32, src.len() as u32);

    let mut i = 0;
    while i < raw.len() {
        let (tok, span) = raw[i].clone();
        i += 1;

        match tok {
            RawToken::Error => return Err(invalid_character(src, span)),

            RawToken::LineStart(width) => {
                // A line containing only whitespace carries no indentation
                // information; skip to the next line-start.
                let line_is_blank = matches!(raw.get(i), Some((RawToken::LineStart(_), _)) | None);
                if line_is_blank {
                    continue;
                }

                // A Newline terminates a preceding line; if nothing has
                // been emitted yet, there is no line to terminate (this
                // also covers a file that starts with a comment, since
                // comments are skipped by logos before layout ever sees
                // them).
                if !out.is_empty() {
                    out.push(Token::new(TokenKind::Newline, span));
                }

                let current = *levels.last().expect("level stack is never empty");
                if width > current {
                    levels.push(width);
                    out.push(Token::new(TokenKind::Indent, span));
                } else if width < current {
                    while *levels.last().expect("level stack is never empty") > width {
                        levels.pop();
                        out.push(Token::new(TokenKind::Dedent, span));
                    }
                    if *levels.last().expect("level stack is never empty") != width {
                        return Err(LexError {
                            message: format!(
                                "This line is indented {width} spaces, which does not match any enclosing block."
                            ),
                            span,
                        });
                    }
                }
            }

            RawToken::Kw(kind) => out.push(Token::new(kind, span)),
        }
    }

    // Close the file: one Newline, then one Dedent per open level.
    if !out.is_empty() {
        out.push(Token::new(TokenKind::Newline, eof));
    }
    while levels.len() > 1 {
        levels.pop();
        out.push(Token::new(TokenKind::Dedent, eof));
    }
    out.push(Token::new(TokenKind::Eof, eof));

    Ok(out)
}

/// The indentation of the first line, when that line has content.
///
/// Spaces at offset 0 follow no line break, so no `LineStart` is produced
/// and the indentation is skipped rather than measured: a file whose
/// first line was indented *further* than its second parsed happily,
/// with the structure silently reinterpreted. In a language whose whole
/// claim is that indentation is the structure, a mis-indented file has to
/// be reported. A first line that holds nothing but spaces expresses no
/// structure, so it is left alone.
fn leading_indentation(src: &str) -> Option<u32> {
    let width = src.len() - src.trim_start_matches(' ').len();
    if width == 0 {
        return None;
    }
    let rest = &src[width..];
    if rest.is_empty() || rest.starts_with('\n') {
        return None;
    }
    Some(width as u32)
}

/// Report a character the language does not admit.
///
/// The characters that reach a source file by accident rather than by
/// typing are named outright, because "`\r` is not valid ZDeceptron" does
/// not tell anyone their editor saved the file with Windows line endings.
///
/// Anything else is escaped before it is quoted. A diagnostic is printed
/// to a terminal, and a terminal acts on the bytes it is given: a raw
/// carriage return reflows the line the message was on, U+202E reverses
/// everything printed after it, U+0007 rings the bell, and a byte order
/// mark shows as nothing at all. None of those may come from the
/// compiler.
fn invalid_character(src: &str, span: Span) -> LexError {
    let text = &src[span.start as usize..span.end as usize];
    let message = match text.chars().next() {
        Some('\t') => "Tabs are not valid indentation. ZDeceptron uses spaces only.".to_string(),
        Some('\r') => "This file uses Windows line endings. ZDeceptron files end a line with \
                       `\\n` alone, not with a carriage return and a newline."
            .to_string(),
        Some('\u{feff}') => "This file contains a byte order mark (U+FEFF). ZDeceptron files are \
                             plain UTF-8 and need no mark; it may be removed."
            .to_string(),
        Some('\u{a0}') => "This is a non-breaking space (U+00A0), not an ordinary space. \
                           ZDeceptron indentation is ordinary spaces."
            .to_string(),
        // The block literal is the one error here that is a *layout*
        // mistake rather than a stray character, and the three ways to
        // make it are all invisible in a diff, so they are named.
        _ if text.starts_with("\"\"\"") => {
            if text[3..].find("\"\"\"").is_none() {
                "This block text literal is never closed. A `\"\"\"` opens one and a `\"\"\"` of \
                 its own on a later line closes it."
                    .to_string()
            } else {
                "A block text literal is written with `\"\"\"` alone at the end of its opening \
                 line, the text on the lines after it, and `\"\"\"` alone on the closing line. \
                 The closing `\"\"\"`'s indentation is removed from every line, so no line may \
                 be indented less than it is."
                    .to_string()
            }
        }
        _ => format!("`{}` is not valid ZDeceptron.", text.escape_debug()),
    };
    LexError { message, span }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenKind::*;

    fn kinds(src: &str) -> Vec<crate::TokenKind> {
        tokenize(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn nesting_emits_indent_and_dedent() {
        assert_eq!(
            kinds("view\n    Column\n        Row\nstate"),
            vec![
                View,
                Newline,
                Indent,
                Ident("Column".into()),
                Newline,
                Indent,
                Ident("Row".into()),
                Newline,
                Dedent,
                Dedent,
                State,
                Newline,
                Eof,
            ]
        );
    }

    #[test]
    fn trailing_indentation_is_closed_at_eof() {
        assert_eq!(
            kinds("view\n    Column"),
            vec![
                View,
                Newline,
                Indent,
                Ident("Column".into()),
                Newline,
                Dedent,
                Eof
            ]
        );
    }

    #[test]
    fn blank_lines_do_not_affect_indentation() {
        assert_eq!(
            kinds("view\n\n    Column\n\n        Row"),
            vec![
                View,
                Newline,
                Indent,
                Ident("Column".into()),
                Newline,
                Indent,
                Ident("Row".into()),
                Newline,
                Dedent,
                Dedent,
                Eof,
            ]
        );
    }

    #[test]
    fn misaligned_dedent_is_an_error() {
        let err = tokenize("view\n        Column\n    Row").unwrap_err();
        assert!(
            err.message.contains("does not match any enclosing"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn tab_indentation_is_an_error() {
        let err = tokenize("view\n\tColumn").unwrap_err();
        assert!(err.message.contains("Tabs"), "got: {}", err.message);
    }

    #[test]
    fn leading_blank_line_does_not_emit_a_newline() {
        assert_eq!(kinds("\nview"), vec![View, Newline, Eof]);
        assert_eq!(kinds("\n\nview"), vec![View, Newline, Eof]);
    }

    #[test]
    fn leading_comment_does_not_emit_a_newline() {
        assert_eq!(kinds("# hello\nview"), vec![View, Newline, Eof]);
    }

    #[test]
    fn multi_level_dedent_emits_one_dedent_per_level() {
        assert_eq!(
            kinds("a\n    b\n        c\nd"),
            vec![
                Ident("a".into()),
                Newline,
                Indent,
                Ident("b".into()),
                Newline,
                Indent,
                Ident("c".into()),
                Newline,
                Dedent,
                Dedent,
                Ident("d".into()),
                Newline,
                Eof,
            ]
        );
    }

    /// The message a user is shown must be printable text and nothing
    /// else. Anything a terminal would act on rather than display is a
    /// defect regardless of which character it is.
    fn message_for(src: &str) -> String {
        let message = tokenize(src).unwrap_err().message;
        assert!(
            !message.chars().any(|c| c.is_control()),
            "a diagnostic must not contain a raw control character: {:?}",
            message.as_bytes()
        );
        assert!(
            !message
                .chars()
                .any(|c| matches!(c, '\u{feff}' | '\u{202e}')),
            "a diagnostic must not contain an invisible or direction-changing \
             character: {message:?}"
        );
        message
    }

    /// The defect this guards: `view` indented further than the `Column`
    /// below it used to parse, because indentation at offset 0 follows no
    /// line break and so was never measured. The file was accepted and
    /// its structure quietly rearranged.
    #[test]
    fn an_indented_first_line_is_rejected() {
        let err = tokenize("        view\n    Column\n").unwrap_err();
        assert!(err.message.contains("left margin"), "got: {}", err.message);
        assert_eq!(
            err.span,
            Span::new(0, 8),
            "the span must cover the indentation"
        );
    }

    #[test]
    fn an_indented_first_line_is_rejected_even_when_it_is_a_comment() {
        let err = tokenize("    # a note\nview\n").unwrap_err();
        assert!(err.message.contains("left margin"), "got: {}", err.message);
    }

    /// A first line holding nothing but spaces states no structure, and
    /// files often begin with a blank line.
    #[test]
    fn a_first_line_of_only_spaces_is_allowed() {
        assert_eq!(kinds("    \nview"), vec![View, Newline, Eof]);
        assert_eq!(kinds("   "), vec![Eof]);
    }

    #[test]
    fn windows_line_endings_are_named() {
        let message = message_for("view\r\n    Column\r\n");
        assert!(message.contains("Windows line endings"), "got: {message}");
    }

    #[test]
    fn a_byte_order_mark_is_named() {
        let message = message_for("\u{feff}view\n    Column\n");
        assert!(message.contains("byte order mark"), "got: {message}");
        assert!(message.contains("removed"), "got: {message}");
    }

    #[test]
    fn a_non_breaking_space_is_named() {
        let message = message_for("view\n\u{a0}\u{a0}\u{a0}\u{a0}Column\n");
        assert!(message.contains("non-breaking space"), "got: {message}");
        assert!(message.contains("ordinary spaces"), "got: {message}");
    }

    #[test]
    fn control_characters_are_escaped_before_they_are_quoted() {
        let message = message_for("view\n    Col\u{7}umn\n");
        assert!(
            message.contains("\\u{7}"),
            "expected the character to be quoted in escaped form: {message}"
        );
    }

    #[test]
    fn a_direction_override_is_escaped_before_it_is_quoted() {
        let message = message_for("view\n    Col\u{202e}umn\n");
        assert!(
            message.contains("\\u{202e}"),
            "expected the character to be quoted in escaped form: {message}"
        );
    }

    #[test]
    fn tabs_are_still_named_before_the_generic_message() {
        let message = message_for("view\n\tColumn");
        assert!(message.contains("Tabs"), "got: {message}");
    }

    #[test]
    fn non_tab_lex_errors_get_the_generic_message() {
        let err = tokenize("view\n    $Column").unwrap_err();
        assert!(!err.message.starts_with("Tabs"), "got: {}", err.message);
        assert!(
            err.message.contains("is not valid ZDeceptron"),
            "got: {}",
            err.message
        );
    }
}
