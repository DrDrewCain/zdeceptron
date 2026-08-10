//! The one canonical layout for ZDeceptron source.
//!
//! Memory safety is a mechanically verified property of this compiler,
//! not a claim: no crate in this workspace may contain `unsafe`.
//!
//! # Why this rewrites text and not a tree
//!
//! The obvious formatter parses to a tree and prints the tree. That
//! formatter cannot be written here, and the reason is worth stating
//! before anyone tries again.
//!
//! **Comments do not survive lexing.** `zdc-lexer`'s `raw.rs` matches
//! `#[^\n\r]*` with `logos::skip`, so a comment is discarded before the
//! layout pass runs, before a token is emitted, and long before the
//! parser sees anything. `zdc-ast` has no comment node and no trivia
//! field — the string "comment" does not appear in that crate or in
//! `zdc-parser` at all. A formatter that round-tripped through the tree
//! would therefore delete every comment in the repository, and this
//! repository's `.zd` files are more comment than code: the comments are
//! where the reasoning is recorded. That is not a bug to be fixed later,
//! it is a reason not to build that formatter.
//!
//! Nor is a token-stream-to-token-stream formatter available, for the
//! same reason one step out: the token stream has no comments in it
//! either, and it has also already thrown away the digits a number was
//! written with and the exact bytes of a `Text` literal.
//!
//! So this works on the **source text**. Every line is emitted from the
//! bytes the author wrote; the only thing that changes is the whitespace
//! at the front of a line, the whitespace at the end of one, and which
//! blank lines are kept. The token stream is used only to *decide* those
//! things — it is where the block structure and the extent of a block
//! text literal come from. A comment cannot be lost by construction,
//! because no comment is ever reconstructed.
//!
//! # What is canonical, and what is left alone
//!
//! Canonical: one level of nesting is [`INDENT`] spaces; no trailing
//! whitespace; exactly one line break at the end of the file; no leading
//! blank lines and no run of two blank lines; a comment line sits at the
//! indentation of the line it introduces; a block text literal's closing
//! delimiter sits one level inside the line that opens it, with its
//! interior carried along.
//!
//! **Deliberately left alone: the spacing *within* a line.** The house
//! style aligns declarations into columns —
//!
//! ```text
//! state count   is client Whole starting 0
//! state doubled is client Whole from count * 2
//! ```
//!
//! — and `examples/` does this in six files, for state declarations,
//! record fields and `when` arms. Collapsing those runs to one space is
//! a layout opinion this formatter does not hold, and *re*-deriving them
//! means deciding which adjacent lines form a group, which is a second
//! opinion on top of the first. §4.1's bargain is about the grammar
//! admitting one phrasing; two spellings of the same token sequence that
//! differ only in how many spaces separate two words are not two
//! phrasings. So intra-line spacing is out of scope, and it is out of
//! scope on purpose rather than by omission.

#![forbid(unsafe_code)]

use zdc_lexer::{Span, TokenKind};
use zdc_parser::ParseError;

/// One level of nesting, in spaces.
///
/// Read off `examples/`, which is unanimous: every indent width in every
/// file is a multiple of four and every nesting step is exactly four.
/// This formatter codifies the house style rather than proposing one.
pub const INDENT: usize = 4;

/// Why a file could not be laid out.
#[derive(Debug, Clone, PartialEq)]
pub enum FmtError {
    /// The file is not ZDeceptron. A formatter that rewrites a file it
    /// cannot read is a formatter that destroys work, so this is refused
    /// before anything is written.
    ///
    /// The bar is **parsing**, not lexing, and the difference is a real
    /// file rather than a hypothetical one: `# a header` followed by an
    /// indented `view` lexes — the comment is skipped, so the layout pass
    /// sees the indentation as opening a block — and then fails to parse
    /// with E0104. Laid out from the token stream alone it would come
    /// back as a *differently* broken file. So the gate is the one the
    /// compiler itself uses.
    Unreadable(ParseError),
    /// Code shares a line with the interior of a block text literal.
    ///
    /// A `"""` block is one token spanning many lines, so a second
    /// literal may be opened on the line that closes the first. That line
    /// is then simultaneously inside a literal — where its indentation is
    /// part of a value — and a line of code — where its indentation is
    /// the block structure. There is no single indentation that is right
    /// for it, so it is refused rather than guessed at. Nothing in
    /// `examples/` writes this shape.
    Entangled(Span),
}

impl From<ParseError> for FmtError {
    fn from(error: ParseError) -> Self {
        FmtError::Unreadable(error)
    }
}

impl FmtError {
    /// What went wrong, as a sentence for a terminal.
    pub fn message(&self) -> String {
        match self {
            FmtError::Unreadable(error) => error.message.clone(),
            FmtError::Entangled(_) => "This line both closes a block text literal and carries \
                                       code, so its indentation is part of a value and part of \
                                       the block structure at once. `zdc fmt` cannot lay it out; \
                                       give the second literal a line of its own."
                .to_string(),
        }
    }

    /// Where in the source it went wrong.
    pub fn span(&self) -> Span {
        match self {
            FmtError::Unreadable(error) => error.span,
            FmtError::Entangled(span) => *span,
        }
    }
}

/// What a physical line of source turned out to be.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Role {
    /// Nothing but spaces.
    Blank,
    /// Nothing the lexer kept: a comment, since a comment is the only
    /// thing `logos` skips that is not whitespace.
    Comment,
    /// A line the parser sees, nested this many blocks deep.
    Code { depth: usize },
    /// The interior or the closing delimiter of a block text literal
    /// opened by the code line at this index.
    Literal { owner: usize },
}

/// The canonical layout of `src`.
///
/// Refuses rather than guesses: a source the *compiler* will not read is
/// handed straight back. The bar is parsing and not lexing, because a
/// file can lex into a block structure the parser then rejects — see
/// [`FmtError::Unreadable`] — and laying that out would produce a
/// differently broken file rather than a repaired one.
pub fn format(src: &str) -> Result<String, FmtError> {
    // The tree is discarded on the spot. It holds no comments, so nothing
    // can be emitted from it; what is wanted is only the verdict that this
    // file is one the compiler reads.
    zdc_parser::parse(src)?;
    // Cannot fail: `parse` lexes first and hands a lex error back as a
    // parse error. Written as a conversion rather than an `expect` anyway,
    // because a formatter has no business panicking on a file.
    let tokens = zdc_lexer::tokenize(src).map_err(|error| {
        FmtError::Unreadable(ParseError {
            message: error.message,
            span: error.span,
            label: None,
            suggestion: None,
            code: zdc_parser::codes::ONE_VALID_FORM,
        })
    })?;
    let lines = split_lines(src);
    if lines.is_empty() {
        return Ok(String::new());
    }

    let mut roles = vec![Role::Blank; lines.len()];
    let mut depth: usize = 0;

    for token in &tokens {
        match token.kind {
            TokenKind::Indent => {
                depth += 1;
                continue;
            }
            // `saturating_sub` rather than `-`: the layout pass balances
            // its own `Indent`/`Dedent` pairs, so this cannot go below
            // zero — and a formatter is not the place to find out by
            // panicking on somebody's file.
            TokenKind::Dedent => {
                depth = depth.saturating_sub(1);
                continue;
            }
            TokenKind::Newline | TokenKind::Eof => continue,
            _ => {}
        }

        let first = line_of(&lines, token.span.start);
        if let Role::Literal { .. } = roles[first] {
            return Err(FmtError::Entangled(token.span));
        }
        if !matches!(roles[first], Role::Code { .. }) {
            roles[first] = Role::Code { depth };
        }

        // `end` is exclusive, so the last byte of the token is `end - 1`.
        // Only a `"""` block spans lines; every other token leaves this
        // loop empty.
        let last = line_of(&lines, token.span.end.saturating_sub(1));
        for role in roles.iter_mut().take(last + 1).skip(first + 1) {
            *role = Role::Literal { owner: first };
        }
    }

    for (index, line) in lines.iter().enumerate() {
        if matches!(roles[index], Role::Blank) && !line.text.trim().is_empty() {
            roles[index] = Role::Comment;
        }
    }

    let mut out: Vec<String> = Vec::new();
    let mut previous_blank = true;
    for (index, line) in lines.iter().enumerate() {
        match roles[index] {
            // A leading blank line, and the second of a pair, are the two
            // blank lines that say nothing the first one did not.
            Role::Blank => {
                if previous_blank {
                    continue;
                }
                previous_blank = true;
                out.push(String::new());
            }
            Role::Comment => {
                previous_blank = false;
                out.push(indented(comment_indent(&roles, index), line.text.trim()));
            }
            Role::Code { depth } => {
                previous_blank = false;
                out.push(indented(depth * INDENT, line.text.trim()));
            }
            Role::Literal { owner } => {
                previous_blank = false;
                out.push(shifted(line.text, literal_shift(&lines, &roles, owner)));
            }
        }
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    if out.is_empty() {
        return Ok(String::new());
    }

    // Windows is a supported platform (#242) and Git there rewrites LF to
    // CRLF on checkout. Rewriting every line ending to LF would make this
    // formatter fight the next `git pull` on that platform, and the lexer
    // reads both identically, so there is nothing to canonicalise.
    let terminator = if src.contains("\r\n") { "\r\n" } else { "\n" };
    let mut text = out.join(terminator);
    text.push_str(terminator);
    Ok(text)
}

/// One physical line: where it starts, and its bytes without the line
/// terminator.
struct Line<'a> {
    start: usize,
    text: &'a str,
}

/// The lines of a source, with `\r\n` and `\n` treated alike.
///
/// The carriage return is stripped from the text but counted in the
/// offsets, so a span still lands on the line it was taken from.
fn split_lines(src: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for piece in src.split('\n') {
        lines.push(Line {
            start,
            text: piece.strip_suffix('\r').unwrap_or(piece),
        });
        start += piece.len() + 1;
    }
    lines
}

/// The index of the line a byte offset falls on.
fn line_of(lines: &[Line<'_>], offset: u32) -> usize {
    let offset = offset as usize;
    // `partition_point` gives the count of lines beginning at or before
    // the offset; the last of those is the one it is on. It cannot be
    // zero: the first line begins at zero.
    lines
        .partition_point(|line| line.start <= offset)
        .saturating_sub(1)
}

/// Where a comment line goes.
///
/// A comment introduces the line below it far more often than it trails
/// the line above — every header in `examples/` is of that shape, and so
/// is every note inside a `view`. So a comment takes the indentation of
/// the next line the parser sees.
///
/// A comment with no such line below it has nothing to introduce. It is
/// an epilogue to the *file*, and goes to the left margin.
///
/// **This rule was chosen against the evidence and then corrected by it.**
/// The first version took the indentation of the line above, on the
/// reasoning that a note at the end of a block belongs to that block.
/// `examples/components.zd` ends with a nine-line comment recording four
/// spec defects the file found, written at the left margin, and the line
/// above it is `Text "nothing here yet"` four levels in. The rule as
/// first written moved that block sixteen columns right, which is how it
/// was found. It is the only trailing comment in `examples/`, so it is
/// the whole evidence base, and it is unambiguous.
///
/// The sharp edge, stated rather than hidden: a note deliberately written
/// at the end of a deeply nested block, with nothing after it, comes back
/// to the margin. Only whitespace on a comment moves, so no program
/// changes — but a reader who wanted that note attached to the block will
/// have to put a line of code after it or accept the move.
fn comment_indent(roles: &[Role], index: usize) -> usize {
    let depth_at = |role: &Role| match role {
        Role::Code { depth } => Some(*depth),
        Role::Blank | Role::Comment | Role::Literal { .. } => None,
    };
    roles[index + 1..]
        .iter()
        .find_map(depth_at)
        .unwrap_or(0)
        .saturating_mul(INDENT)
}

/// How far the interior of the block text literal opened on `owner`
/// moves.
///
/// **The value is preserved for any shift, provided it is the same one
/// for every line.** `raw::dedent_block` takes the value to be each
/// interior line with the *closing* delimiter's indentation removed as a
/// prefix, so the value depends only on the offsets within the literal.
/// Move the delimiter and every line under it by the same amount and
/// every difference is unchanged — which is exactly the property §17.4.10
/// gave the block form for, so that a literal could be moved a level
/// deeper without its text changing.
///
/// Canonically the closing delimiter sits one level inside the line that
/// opens it, which is how `examples/terminal-help.zd` writes all three of
/// its literals.
fn literal_shift(lines: &[Line<'_>], roles: &[Role], owner: usize) -> isize {
    let Role::Code { depth } = roles[owner] else {
        // Unreachable: `Role::Literal { owner }` is only ever written
        // beside `Role::Code` on `owner`. Returning "do not move" rather
        // than panicking, because the cost of being wrong here is a file
        // that is merely not canonical.
        return 0;
    };
    let closing = roles
        .iter()
        .enumerate()
        .skip(owner + 1)
        .take_while(|(_, role)| matches!(role, Role::Literal { owner: at } if *at == owner))
        .map(|(index, _)| index)
        .last();
    let Some(closing) = closing else { return 0 };
    let wanted = (depth + 1) * INDENT;
    wanted as isize - leading_spaces(lines[closing].text) as isize
}

/// How many spaces a line begins with.
fn leading_spaces(text: &str) -> usize {
    text.len() - text.trim_start_matches(' ').len()
}

/// A line of a block text literal, moved sideways.
///
/// Nothing but the leading spaces is touched — **trailing spaces inside a
/// literal are part of the value**, since `dedent_block` strips the
/// margin and keeps the rest of the line, so the one thing this formatter
/// removes everywhere else it must not remove here.
///
/// A shift that would take a line past the left margin can only happen on
/// a line of nothing but spaces that is shorter than the margin — every
/// line with text on it has at least the margin's worth, or the lexer
/// would have refused the literal. Such a line reads as blank before the
/// clamp and after it, so clamping cannot change the value.
fn shifted(text: &str, by: isize) -> String {
    let indent = leading_spaces(text);
    let moved = (indent as isize + by).max(0) as usize;
    let mut out = " ".repeat(moved);
    out.push_str(&text[indent..]);
    out
}

/// A line at a given indentation.
fn indented(indent: usize, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = " ".repeat(indent);
    out.push_str(text);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formatted(src: &str) -> String {
        format(src).expect("this source is readable ZDeceptron")
    }

    #[test]
    fn a_block_is_reindented_to_four_spaces_a_level() {
        assert_eq!(
            formatted("view\n  Column\n      Text \"hi\"\n"),
            "view\n    Column\n        Text \"hi\"\n"
        );
    }

    #[test]
    fn over_deep_indentation_is_brought_back_in() {
        assert_eq!(
            formatted("view\n        Column\n                Text \"hi\"\n"),
            "view\n    Column\n        Text \"hi\"\n"
        );
    }

    #[test]
    fn trailing_whitespace_goes() {
        assert_eq!(formatted("view   \n    Column  \n"), "view\n    Column\n");
    }

    #[test]
    fn a_missing_final_newline_is_supplied_and_a_doubled_one_removed() {
        assert_eq!(formatted("view\n    Column"), "view\n    Column\n");
        assert_eq!(formatted("view\n    Column\n\n\n"), "view\n    Column\n");
    }

    #[test]
    fn a_run_of_blank_lines_collapses_to_one() {
        assert_eq!(
            formatted("state a is client Whole starting 1\n\n\n\nview\n    Column\n"),
            "state a is client Whole starting 1\n\nview\n    Column\n"
        );
    }

    #[test]
    fn a_leading_blank_line_goes() {
        assert_eq!(formatted("\n\nview\n    Column\n"), "view\n    Column\n");
    }

    /// The whole point of the exercise. `#` never reaches the parser —
    /// `logos::skip` eats it in `zdc-lexer`'s `raw.rs` — so a formatter
    /// that went through the tree would delete every comment in the
    /// repository. This one rewrites the source text, and the test that
    /// says so is this one.
    #[test]
    fn comments_survive_verbatim() {
        let src = "# a header note\nview\n    Column\n        # why this Text is here\n        Text \"hi\"\n";
        assert_eq!(formatted(src), src);
    }

    #[test]
    fn a_comment_takes_the_indentation_of_the_line_it_introduces() {
        assert_eq!(
            formatted("view\n    Column\n# about the Text\n        Text \"hi\"\n"),
            "view\n    Column\n        # about the Text\n        Text \"hi\"\n"
        );
    }

    /// A comment with nothing after it has no line to introduce: it is an
    /// epilogue to the file, and goes to the left margin.
    ///
    /// `examples/components.zd` is why. It ends with a nine-line comment
    /// recording four spec defects, written at the margin, under a `Text`
    /// four levels in. The first version of this rule took the
    /// indentation of the line above and moved that block sixteen columns
    /// right; `fmt_examples` caught it.
    #[test]
    fn a_trailing_comment_goes_to_the_left_margin() {
        assert_eq!(
            formatted("view\n    Column\n        Text \"hi\"\n        # a closing note\n"),
            "view\n    Column\n        Text \"hi\"\n# a closing note\n"
        );
    }

    /// A header comment introduces the first declaration, which is at the
    /// left margin, so "the next code line's indentation" already puts it
    /// there. Every file in `examples/` opens this way.
    #[test]
    fn a_header_comment_sits_at_the_left_margin() {
        let src = "# a header\n#\n# and more of it\nview\n    Column\n";
        assert_eq!(formatted(src), src);
    }

    /// **`zdc fmt` cannot repair an indented first line**, and this
    /// records that rather than hiding it. `layout::leading_indentation`
    /// refuses the file outright — including when the first line is a
    /// comment — so there is no token stream, no block structure, and
    /// nothing to lay out. Dedenting the line and re-lexing would be a
    /// safe repair and it is deliberately not done here: a formatter that
    /// edits a file the compiler has refused is a formatter that has an
    /// opinion about what the author meant.
    #[test]
    fn an_indented_first_line_is_refused_rather_than_repaired() {
        let error = format("   # a header\nview\n").expect_err("the lexer refuses this file");
        assert!(
            error.message().contains("left margin"),
            "the lexer's own message must come through: {}",
            error.message()
        );
    }

    #[test]
    fn a_comment_after_a_line_is_left_alone() {
        assert_eq!(
            formatted("view\n  Column   # why\n"),
            "view\n    Column   # why\n"
        );
    }

    /// A block literal's value is the interior lines with the *closing*
    /// delimiter's indentation removed from each, so the value depends on
    /// the offsets within the literal and not on where the literal sits.
    /// Canonically the closing delimiter is one level inside the line that
    /// opens it, and every interior line moves with it.
    #[test]
    fn a_block_literal_is_shifted_whole() {
        let src =
            "state s is client Text starting \"\"\"\n        one\n          two\n        \"\"\"\n";
        assert_eq!(
            formatted(src),
            "state s is client Text starting \"\"\"\n    one\n      two\n    \"\"\"\n"
        );
    }

    #[test]
    fn a_block_literal_keeps_its_value_when_the_code_around_it_moves() {
        let src = "view\n  Column\n      Text \"\"\"\n      one\n        two\n      \"\"\"\n";
        let out = formatted(src);
        assert_eq!(
            out,
            "view\n    Column\n        Text \"\"\"\n            one\n              two\n            \"\"\"\n"
        );
        assert_eq!(
            text_literals(&out),
            text_literals(src),
            "the value of the literal must not depend on where it was written"
        );
    }

    /// Trailing spaces inside a block literal are part of the value —
    /// `dedent_block` strips the margin and keeps the rest of the line —
    /// so the one thing this formatter strips everywhere else it must not
    /// strip here.
    #[test]
    fn trailing_spaces_inside_a_block_literal_are_kept() {
        let src = "state s is client Text starting \"\"\"\n    one  \n    \"\"\"\n";
        assert_eq!(formatted(src), src);
        assert_eq!(text_literals(&formatted(src)), text_literals(src));
    }

    /// Two empty lines between the delimiters are a literal whose whole
    /// value is one line break — `examples/terminal-help.zd`'s `br`. The
    /// blank-line collapsing must not reach inside.
    #[test]
    fn blank_lines_inside_a_block_literal_are_not_collapsed() {
        let src = "state br is client Text starting \"\"\"\n\n\n    \"\"\"\n";
        assert_eq!(formatted(src), src);
        assert_eq!(text_literals(&formatted(src)), ["\n".to_string()]);
    }

    /// A file the compiler refuses is a file this must not rewrite.
    /// Laying out a half-typed file by guessing where the blocks were is
    /// how a formatter loses somebody's work.
    #[test]
    fn a_file_that_does_not_lex_is_refused() {
        let error = format("view\n\tColumn\n").expect_err("a tab is not indentation");
        let FmtError::Unreadable(reported) = error else {
            panic!("expected the lex error to be handed back")
        };
        assert!(
            reported.message.contains("Tabs"),
            "got: {}",
            reported.message
        );
    }

    /// The case that made the gate `parse` and not `tokenize`. A comment
    /// at the margin followed by an indented declaration *lexes* — the
    /// comment is skipped, so the layout pass reads the indentation as
    /// opening a block — and fails to parse with E0104. Laying it out
    /// from the tokens alone produced a differently broken file.
    #[test]
    fn a_file_that_lexes_but_does_not_parse_is_refused() {
        let error =
            format("# a header\n        view\n            Column\n").expect_err("E0104 refuses");
        let FmtError::Unreadable(reported) = error else {
            panic!("expected the parse error to be handed back")
        };
        assert!(
            reported.message.contains("indented block"),
            "got: {}",
            reported.message
        );
    }

    /// Code after the closing delimiter of a block literal, on the same
    /// line, is legal and cannot be laid out: that line is inside one
    /// literal and opens another, so no single indentation is right for
    /// it. Refusing is the honest answer, and it is the answer for a shape
    /// no file in this repository writes.
    #[test]
    fn code_entangled_with_a_block_literal_is_refused() {
        let src = "state s is client Text from join with a is \"\"\"\n    x\n    \"\"\", b is \"\"\"\n    y\n    \"\"\"\n";
        let error = format(src).expect_err("a line cannot be both literal and code");
        assert!(
            matches!(error, FmtError::Entangled(_)),
            "expected an entanglement report, got {error:?}"
        );
    }

    #[test]
    fn an_empty_file_is_left_empty() {
        assert_eq!(formatted(""), "");
        assert_eq!(formatted("\n\n   \n"), "");
    }

    /// Windows is a supported platform (#242) and Git there rewrites LF to
    /// CRLF on checkout. A formatter that "canonicalised" the line ending
    /// would rewrite every line of every file on that platform and fight
    /// the checkout on the next pull, so the file's own ending is kept.
    #[test]
    fn windows_line_endings_are_kept() {
        assert_eq!(
            formatted("view\r\n  Column\r\n"),
            "view\r\n    Column\r\n",
            "a CRLF file stays a CRLF file"
        );
    }

    /// Formatting twice is formatting once. Asserted here on the shapes
    /// the unit tests above cover; `zdc-cli`'s `fmt_examples` asserts it
    /// over every file in `examples/` and over mangled copies of them.
    #[test]
    fn formatting_is_idempotent() {
        let sources = [
            "view\n  Column\n      Text \"hi\"\n",
            "# note\nview\n    Column\n",
            "state s is client Text starting \"\"\"\n        one\n        \"\"\"\n",
            "\n\nview\n\n\n    Column   \n",
            "view\r\n  Column\r\n",
        ];
        for src in sources {
            let once = formatted(src);
            let twice = formatted(&once);
            assert_eq!(once, twice, "not idempotent on {src:?}");
        }
    }

    /// The values of every `Text` token in a source, in order.
    ///
    /// A block literal's value is what the formatter is most able to
    /// damage and least able to see, so the tests that move one compare
    /// this rather than the source text.
    fn text_literals(src: &str) -> Vec<String> {
        zdc_lexer::tokenize(src)
            .expect("readable source")
            .into_iter()
            .filter_map(|token| match token.kind {
                TokenKind::Text(value) => Some(value),
                _ => None,
            })
            .collect()
    }
}
