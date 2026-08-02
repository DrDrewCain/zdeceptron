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
    let raw = tokenize_raw(src);
    let mut out: Vec<Token> = Vec::new();
    let mut levels: Vec<u32> = vec![0];
    let eof = Span::new(src.len() as u32, src.len() as u32);

    let mut i = 0;
    while i < raw.len() {
        let (tok, span) = raw[i].clone();
        i += 1;

        match tok {
            RawToken::Error => {
                let text = &src[span.start as usize..span.end as usize];
                let message = if text.contains('\t') {
                    "Tabs are not valid indentation. ZDeceptron uses spaces only.".to_string()
                } else {
                    format!("`{text}` is not valid ZDeceptron.")
                };
                return Err(LexError { message, span });
            }

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
