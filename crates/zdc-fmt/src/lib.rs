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

/// The column a line is wrapped at.
///
/// Ninety-six rather than eighty. The comments in this repository are
/// hard-wrapped near seventy-eight and are prose; code here is
/// `with name is value` triples, and eighty puts two of them on a line
/// while ninety-six usually fits three — which is the difference between
/// an argument list you scan and one you read down.
pub const WIDTH: usize = 96;

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
    /// The rest of the logical line that began at this index.
    ///
    /// A line break inside a bracket, or after a trailing comma, is not
    /// layout — the lexer emits no `Newline` for it. So the *physical*
    /// lines a wrapped argument list occupies are one *logical* line, and
    /// the formatter has to see it that way or it cannot lay it out
    /// again: re-emitting each physical line at the statement's own depth
    /// undoes the wrap's indentation, and formatting twice stops giving
    /// what formatting once gave.
    Continuation { owner: usize },
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
    // The physical line the current logical line began on.
    let mut logical: Option<usize> = None;
    let mut ended = true;

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
            TokenKind::Newline | TokenKind::Eof => {
                ended = true;
                continue;
            }
            _ => {}
        }

        let first = line_of(&lines, token.span.start);
        if let Role::Literal { .. } = roles[first] {
            return Err(FmtError::Entangled(token.span));
        }
        // No `Newline` since the last code line means this one continues
        // it: the break was inside a bracket or after a trailing comma,
        // and the lexer suspended layout across it.
        if !matches!(roles[first], Role::Code { .. } | Role::Continuation { .. }) {
            roles[first] = match logical.filter(|owner| *owner != first && !ended) {
                Some(owner) => Role::Continuation { owner },
                None => {
                    logical = Some(first);
                    Role::Code { depth }
                }
            };
        }
        if matches!(roles[first], Role::Code { .. }) {
            logical = Some(first);
        }
        ended = false;

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
                // The whole logical line, continuations and all, joined
                // back into one before it is laid out again.
                let mut whole = line.text.trim().to_string();
                for (at, role) in roles.iter().enumerate().skip(index + 1) {
                    match role {
                        Role::Continuation { owner } if *owner == index => {
                            whole.push(' ');
                            whole.push_str(lines[at].text.trim());
                        }
                        _ => {}
                    }
                    if at > index && matches!(role, Role::Code { .. }) {
                        break;
                    }
                }
                out.extend(wrapped(depth * INDENT, &whole));
            }
            // Emitted with its owner, above.
            Role::Continuation { .. } => {}
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

/// One code line, wrapped at [`WIDTH`] if it does not fit.
///
/// # Why this is possible at all now, and was not before
///
/// A line could not be broken: there is no continuation syntax and a
/// newline inside a bracket used to close the line, so an expression was
/// as long as it was and the formatter had nothing to decide. The lexer
/// now suspends layout while a bracket is open, which is what gives this
/// function somewhere to put the rest.
///
/// # Where it breaks
///
/// At the commas of the **shallowest** bracket depth that yields more
/// than one piece. That is the outermost argument list or list literal,
/// which is the group a reader is trying to scan; breaking a nested one
/// while its parent stays joined puts the pieces of two different lists
/// at the same indentation and reads as one list of the wrong length.
///
/// Each piece then goes through this function again, so a long argument
/// whose own arguments do not fit is broken in turn.
///
/// # What is left alone
///
/// A line with a trailing comment is never wrapped. The comment belongs
/// to the whole line, there is no piece it is about, and attaching it to
/// the last one would move it somewhere it does not mean. A line with no
/// comma at any depth is left alone too, because there is no break in it
/// that the grammar admits.
fn wrapped(indent: usize, text: &str) -> Vec<String> {
    if indent + text.chars().count() <= WIDTH || has_trailing_comment(text) {
        return vec![indented(indent, text)];
    }
    let Some(pieces) = split_at_shallowest_commas(text) else {
        return vec![indented(indent, text)];
    };
    let mut out: Vec<String> = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        let at = if index == 0 { indent } else { indent + INDENT };
        let mut lines = wrapped(at, piece);
        // The comma is appended to the piece's *last emitted line* rather
        // than to the piece before wrapping it. Putting it back first
        // makes the piece splittable at that same comma again, and the
        // recursion never bottoms out — which it did, once, spectacularly.
        // The opener piece is a bracket and nothing else; a comma after
        // it would be an empty first argument.
        let opens = matches!(piece.as_str().chars().last(), Some('(') | Some('['));
        if index + 1 != pieces.len() && !opens {
            if let Some(last) = lines.last_mut() {
                last.push(',');
            }
        }
        out.extend(lines);
    }
    out
}

/// The line cut at the commas of the shallowest depth that has any.
///
/// `None` when the line holds no comma outside a string, which is every
/// line that has nothing this formatter knows how to break.
fn split_at_shallowest_commas(text: &str) -> Option<Vec<String>> {
    let depths = comma_depths(text);
    let shallowest = depths.iter().map(|(_, depth)| *depth).min()?;
    let cuts: Vec<usize> = depths
        .iter()
        .filter(|(_, depth)| *depth == shallowest)
        .map(|(at, _)| *at)
        .collect();
    if cuts.is_empty() {
        return None;
    }
    // When the group being broken is inside a bracket, the break starts
    // *after the bracket* as well as at each comma. Otherwise everything
    // up to the first comma stays on line one — which for
    // `… from cardsFrom of (listTake with items is (listDrop with items is
    // mixed, count is 5)` is most of the line, and the wrap has bought
    // nothing. Breaking after the opener puts the whole group at one
    // indent and lines its elements up under each other.
    let opener = if shallowest > 0 {
        opening_bracket_before(text, cuts[0])
    } else {
        None
    };
    let mut pieces = Vec::new();
    let mut from = 0usize;
    if let Some(at) = opener {
        pieces.push(text[..=at].trim().to_string());
        from = at + 1;
    }
    for cut in cuts {
        pieces.push(text[from..cut].trim().to_string());
        from = cut + 1;
    }
    pieces.push(text[from..].trim().to_string());
    Some(pieces)
}

/// The `(` or `[` that opens the group `cut` sits directly inside.
///
/// Scanned backwards from the comma, counting closers, so a sibling
/// group already closed before it is stepped over rather than mistaken
/// for the enclosing one.
fn opening_bracket_before(text: &str, cut: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut closed: u32 = 0;
    for at in (0..cut).rev() {
        match bytes[at] {
            b')' | b']' => closed += 1,
            b'(' | b'[' if closed == 0 => return Some(at),
            b'(' | b'[' => closed -= 1,
            _ => {}
        }
    }
    None
}

/// Every comma outside a string literal, with the bracket depth it sits
/// at.
fn comma_depths(text: &str) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    let mut depth: u32 = 0;
    let mut in_text = false;
    for (at, byte) in text.bytes().enumerate() {
        match byte {
            b'"' => in_text = !in_text,
            _ if in_text => {}
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b',' => out.push((at, depth)),
            // A comment runs to the end of the line, so every comma after
            // one is inside it and none of them is a break.
            b'#' => break,
            _ => {}
        }
    }
    out
}

/// Whether the line ends with a `#` comment outside a string.
fn has_trailing_comment(text: &str) -> bool {
    let mut in_text = false;
    for byte in text.bytes() {
        match byte {
            b'"' => in_text = !in_text,
            b'#' if !in_text => return true,
            _ => {}
        }
    }
    false
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
        // A continuation is part of the line above it, so it is not the
        // line a comment below is introducing.
        Role::Blank | Role::Comment | Role::Literal { .. } | Role::Continuation { .. } => None,
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

#[cfg(test)]
mod wrapping {
    use super::*;

    /// A record of three `Text` fields, so a long argument list is long
    /// because of its *arguments* rather than because of a literal the
    /// type cannot hold.
    const SHIP: &str = "record Ship\n    a is Text\n    b is Text\n    c is Text\n\n";

    /// The line that motivated the whole thing: an argument list with no
    /// bracket around it, hundreds of characters long, and no way to
    /// break it until the lexer learned that a trailing comma continues.
    #[test]
    fn a_long_argument_list_breaks_one_argument_to_a_line() {
        let src = format!(
            "{SHIP}function make\n\
             \x20   give Ship with a is \"aaaaaaaaaaaaaaaaaaaaaaaaaaaa\", \
             b is \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\", \
             c is \"cccccccccccccccccccccccccccccccc\"\n"
        );
        let out = format(&src).expect("formats");
        // Each argument alone on its line, and the two continuations one
        // level in from the `give`. Matched on the *values* rather than
        // on `b is`, which is also how the record declares its field.
        assert!(
            out.contains("\n    give Ship with a is \"aaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\n")
                && out.contains("\n        b is \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\n")
                && out.contains("\n        c is \"cccccccccccccccccccccccccccccccc\"\n"),
            "each argument goes on its own line, one level in:\n{out}"
        );
    }

    /// And what comes out has to still be the same program. The lexer
    /// suspends layout inside a bracket and after a trailing comma, so
    /// the wrapped form lexes to the same tokens — this is the assertion
    /// that says so rather than the comment claiming it.
    #[test]
    fn a_wrapped_line_lexes_to_the_same_tokens() {
        let src = format!(
            "{SHIP}function make\n\
             \x20   give Ship with a is \"aaaaaaaaaaaaaaaaaaaaaaaaaaaa\", \
             b is \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\", \
             c is \"cccccccccccccccccccccccccccccccc\"\n"
        );
        let out = format(&src).expect("formats");
        assert!(out.lines().count() > src.lines().count(), "it must have wrapped");
        let kinds = |text: &str| {
            zdc_lexer::tokenize(text)
                .expect("lexes")
                .iter()
                .map(|t| t.kind.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds(&src), kinds(&out), "wrapping changed the token stream");
    }

    /// Formatting twice must give what formatting once gave, or the
    /// formatter has no canonical form to converge on.
    #[test]
    fn wrapping_is_idempotent() {
        let src = format!(
            "{SHIP}function make\n\
             \x20   give Ship with a is (Ship with a is \"one\", b is \"two\", c is \"three\"), \
             b is \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\", \
             c is \"cccccccccccccccccccccccccccccccccccc\"\n"
        );
        let once = format(&src).expect("formats");
        let twice = format(&once).expect("formats again");
        assert_eq!(once, twice, "formatting is not idempotent");
    }

    /// A trailing comment belongs to the whole line and there is no piece
    /// it is about, so the line is left as it is.
    #[test]
    fn a_line_with_a_trailing_comment_is_left_alone() {
        let src = format!(
            "{SHIP}function make\n\
             \x20   give Ship with a is \"aaaaaaaaaaaaaaaaaaaaaaaaaaaa\", \
             b is \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\", \
             c is \"cccccccccccccccccccccccccccccccc\"  # why\n"
        );
        let out = format(&src).expect("formats");
        assert_eq!(
            out.lines().filter(|l| l.contains("give Ship")).count(),
            1,
            "a commented line must not wrap:\n{out}"
        );
    }

    /// A short line is untouched, which is most lines.
    #[test]
    fn a_line_that_fits_is_not_wrapped() {
        let src = "state count is client Whole starting 0\n\nview\n    Text (text of count)\n";
        assert_eq!(format(src).expect("formats"), src);
    }
}
