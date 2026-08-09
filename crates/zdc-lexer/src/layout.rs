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

    if let Some(error) = exponent_literal(&out) {
        return Err(error);
    }

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

/// Report a whole-number literal the value cannot hold, naming the
/// nearest one it can (#183).
///
/// A run of digits always matches the number rule, so the only way one
/// reaches here is `raw::number` having refused it, and the only reason
/// it refuses is this. The check is repeated anyway rather than assumed:
/// a message this specific must not be reachable by anything else.
///
/// `Whole` is an integer type, and the narrowing operations `floor of`
/// and `round of` give an `Option` precisely so that a `Whole` cannot
/// quietly stop being one. A literal the value cannot hold is the same
/// promise broken one step earlier, and it is the step where refusing
/// costs nothing at run time.
fn unrepresentable_whole(text: &str) -> Option<String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value = text.parse::<f64>().ok()?;
    if crate::raw::exactly_holds(text, value) {
        return None;
    }
    Some(format!(
        "`{text}` is not a whole number this language holds exactly. The nearest one it holds \
         is `{value}`. A `Whole` is an integer up to 9007199254740992, so write a number inside \
         that or hold this as `Text`."
    ))
}

/// Report an escape a one-line `Text` literal does not have, naming the
/// four it does (#16).
///
/// The list is read off `raw::ESCAPES` rather than written again, so a
/// fifth escape would be offered here in the same edit that admits it.
/// Saying which ones exist is the point: a reader who wrote `\r` needs
/// what to write instead, and a message that only said "no" would leave
/// them to guess (§7.3).
fn unknown_escape(text: &str) -> Option<String> {
    let body = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            continue;
        }
        let written = chars.next()?;
        if crate::raw::ESCAPES
            .iter()
            .any(|(escape, _)| *escape == written)
        {
            continue;
        }
        let offered = crate::raw::ESCAPES
            .iter()
            .map(|(escape, _)| format!("`\\{escape}`"))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(format!(
            "`\\{}` is not an escape ZDeceptron has. A `Text` literal writes {offered}, and \
             nothing else; for text with line breaks laid out on the page, write a `\"\"\"` \
             block instead.",
            written.escape_debug()
        ));
    }
    None
}

/// The exponent literal the language does not have, reported where it was
/// written (#184).
///
/// `1e10` lexes as `1` and then the name `e10`, so the failure used to
/// surface two tokens later as *"Expected a line break after the
/// declaration"* — a rule the writer had not broken, about a construct
/// they had not written. The tokens are adjacent, which is what makes the
/// intent unambiguous: `2 each` has a space and is two things.
///
/// **This does not decide whether the language should have them.** That is
/// a grammar addition, and §4.2 keeps the grammar deliberately small, so
/// it is the owner's call. What is fixed here is the reporting, which was
/// wrong whichever way that decision goes.
fn exponent_literal(tokens: &[Token]) -> Option<LexError> {
    for pair in tokens.windows(2) {
        let [number, next] = pair else { continue };
        if !matches!(number.kind, TokenKind::Number(_)) {
            continue;
        }
        // Adjacency is the whole test. A space means two tokens the writer
        // meant as two.
        if next.span.start != number.span.end {
            continue;
        }
        let TokenKind::Ident(name) = &next.kind else {
            continue;
        };
        let mut characters = name.chars();
        if !matches!(characters.next(), Some('e' | 'E')) {
            continue;
        }
        // `1e10` is the whole suffix; `1e-10` lexes the sign separately and
        // leaves a bare `e`, so both shapes are recognised here rather than
        // only the one that happens to be one token.
        if !characters.all(|c| c.is_ascii_digit()) {
            continue;
        }
        return Some(LexError {
            message: format!(
                "`{}{name}` is not a number this language can write. There are no exponent \
                 literals: write the digits out, or divide by a power of ten.",
                number_text(number)
            ),
            span: Span::new(number.span.start, next.span.end),
        });
    }
    None
}

/// The digits a number token was written with, for quoting it back.
fn number_text(token: &Token) -> String {
    match token.kind {
        TokenKind::Number(value) if value.fract() == 0.0 => format!("{value:.0}"),
        TokenKind::Number(value) => format!("{value}"),
        _ => String::new(),
    }
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
    if let Some(message) = unrepresentable_whole(text) {
        return LexError { message, span };
    }
    if let Some(message) = unknown_escape(text) {
        return LexError { message, span };
    }
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

    /// #184. `1e10` lexed as `1` and then `e10`, so the failure surfaced
    /// two tokens later as *"Expected a line break after the
    /// declaration"* — a rule the writer had not broken, about a
    /// construct they had not written.
    ///
    /// The language has no exponent literal. That is a grammar decision
    /// and this does not take it; what it fixes is the reporting, which
    /// was wrong whichever way the decision goes.
    #[test]
    fn an_exponent_literal_is_refused_at_the_literal() {
        let error = tokenize("state d is client Decimal starting 1e10\n")
            .expect_err("`1e10` is not a number this language can write");

        assert!(
            error.message.contains("exponent"),
            "the message must name what was written: {}",
            error.message
        );
        // The span covers `1e10`, not the line break after it. `1e10`
        // begins at byte 35 of that line.
        assert_eq!(
            (error.span.start, error.span.end),
            (35, 39),
            "the caret must land on the literal: {:?}",
            error.span
        );
    }

    /// The forms that *are* numbers keep lexing, including one that ends
    /// in a name beginning with `e` — `2 each` is two tokens and not a
    /// malformed exponent.
    #[test]
    fn ordinary_numbers_and_a_following_name_are_untouched() {
        assert_eq!(kinds("1\n"), [Number(1.0), Newline, Eof]);
        assert_eq!(kinds("2.5\n"), [Number(2.5), Newline, Eof]);
        assert!(matches!(
            kinds("2 each\n").as_slice(),
            [Number(_), Each, ..]
        ));
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

    /// **Windows is a supported platform (#242).** A carriage return
    /// before a line feed is a line ending, not a stray character.
    ///
    /// Git on Windows rewrites LF to CRLF on checkout, so before this the
    /// compiler could not read its own examples there: the release
    /// workflow's Windows job built `zdc 0.1.0` and then failed on
    /// `examples/hello.zd`. Refusing was a defensible rule and it made the
    /// language unusable on a platform, which is a worse trade than
    /// accepting two bytes where one was expected.
    ///
    /// A CRLF file must tokenise **identically** to the same file with LF,
    /// which is stronger than "does not error": indentation is significant
    /// here, so a `\r` counted as an indent column would change the block
    /// structure rather than fail.
    #[test]
    fn windows_line_endings_are_a_line_ending() {
        assert_eq!(
            kinds("view\r\n    Column\r\n"),
            kinds("view\n    Column\n"),
            "a CRLF file must tokenise exactly as the LF file does"
        );
    }

    /// A lone carriage return is still not a line ending, and still says
    /// so. `\r` alone has not been a line terminator since Mac OS 9, and a
    /// file containing one is far more likely to be damaged than intended.
    #[test]
    fn a_lone_carriage_return_is_still_named() {
        let message = message_for("view\r    Column\n");
        assert!(message.contains("carriage return"), "got: {message}");
    }

    /// A block literal, written on a Windows machine (#242). Splitting on
    /// `\n` alone would leave a carriage return on the end of every line
    /// — including the opening and closing delimiter lines, which are
    /// required to hold nothing but spaces, so the literal would be
    /// refused rather than mangled. The value must be the same either way.
    #[test]
    fn a_block_literal_reads_the_same_with_windows_line_endings() {
        let unix = kinds("state s is client Text starting \"\"\"\n    one\n    two\n    \"\"\"\n");
        let dos =
            kinds("state s is client Text starting \"\"\"\r\n    one\r\n    two\r\n    \"\"\"\r\n");
        assert_eq!(
            unix, dos,
            "a block literal must not depend on the line ending"
        );
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
