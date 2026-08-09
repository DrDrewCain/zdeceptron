//! Doc comments, harvested from source text because they are nowhere else.
//!
//! # Comments do not survive lexing, and this is what that costs
//!
//! `zdc-lexer`'s `raw.rs` matches a comment with
//! `#[regex(r"#[^\n\r]*", logos::skip)]`, and `tokenize_raw` `continue`s on
//! the one it produces. A comment is therefore gone before the layout
//! algorithm runs, before the parser sees a token, and long before name
//! resolution builds the HIR every other part of this crate reads. There is
//! no trivia list, no attached-comment field on an AST node, and nothing to
//! recover downstream: **by the time a program is a `Hir`, its comments do
//! not exist.**
//!
//! That matters more here than anywhere else in the compiler. The reasoning
//! in this codebase lives in `#` comments — `guestbook.zd` is more comment
//! than code, and the prelude is mostly comment — so a documentation
//! generator that could not read them would print the shape of a program
//! and none of its meaning.
//!
//! ## What it would take to do this properly
//!
//! Preserving comments is a lexer change with a cost, which is why this
//! module exists instead. It would take, in order:
//!
//! 1. `Lexeme::Comment` stops being `logos::skip` and `tokenize_raw` emits
//!    it with its span, into a side channel rather than into the token
//!    stream — `layout.rs` counts tokens to decide indentation, and a
//!    comment token in the stream would change the layout algorithm's
//!    input. `raw.rs`'s own test `comment_at_end_of_line_does_not_eat_
//!    following_indentation` is the shape of what breaks if it is not a
//!    side channel.
//! 2. `zdc_parser::parse` carries that side channel into `ast::Program`,
//!    and every declaration gains a `doc: Option<String>` filled by the
//!    same "the block directly above, with no blank line" rule this module
//!    implements against text.
//! 3. `zdc-resolve` copies it onto `hir::Def`, which is one more field to
//!    keep through lowering, and `Def`'s `PartialEq` starts comparing
//!    documentation — a difference no pass should be sensitive to.
//!
//! The payoff would be that the *language server* could show a
//! declaration's comment on hover, which it cannot today and which this
//! module does not give it, since a hover has a buffer rather than a file
//! set. That is the argument for doing it, and it is a language change
//! rather than a documentation feature, so it is recorded here rather than
//! attempted.
//!
//! ## Why reading the text is sound enough to ship
//!
//! The risk in scraping `#` lines is mistaking text for a comment: a `#`
//! inside a string literal is not a comment, and a block literal can hold
//! whole lines of them. So this does not decide from the line's first
//! character alone — it asks the **real lexer** which byte ranges are
//! tokens. Comments are skipped, so a `#` that no token span covers is a
//! comment, and a `#` inside a `"""…"""` literal is inside that literal's
//! `Text` token and is not. The rule is decided by the same code that
//! decides it for the compiler, which is the only version of this that
//! cannot drift.

use std::collections::BTreeSet;

use zdc_lexer::raw::tokenize_raw;

/// Which lines of one source file are comment lines.
///
/// Built once per module and asked once per declaration, because
/// tokenizing is linear in the file and a program has more declarations
/// than files.
pub struct Comments<'a> {
    source: &'a str,
    /// The byte offset each line starts at, in order.
    line_starts: Vec<usize>,
    /// Indices into `line_starts` whose line is a comment.
    comment_lines: BTreeSet<usize>,
}

impl<'a> Comments<'a> {
    pub fn of(source: &'a str) -> Comments<'a> {
        let mut line_starts = vec![0usize];
        for (offset, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }

        // Every byte range the lexer calls a token. Comments are not among
        // them — they are skipped — so a `#` covered by one of these is a
        // `#` inside something else, which in practice means inside a text
        // literal.
        let mut spans: Vec<(usize, usize)> = tokenize_raw(source)
            .into_iter()
            .map(|(_, span)| (span.start as usize, span.end as usize))
            .collect();
        spans.sort_unstable();

        let mut comment_lines = BTreeSet::new();
        for (index, start) in line_starts.iter().enumerate() {
            let line = line_at(source, *start);
            let Some(hash) = line.find('#') else {
                continue;
            };
            // Only a line whose *first* content is `#`. A trailing comment
            // after code documents nothing above it, and treating it as a
            // doc comment would attach a note about one line to the
            // declaration on the next.
            if !line[..hash].trim().is_empty() {
                continue;
            }
            let offset = start + hash;
            if spans
                .iter()
                .any(|(from, to)| *from <= offset && offset < *to)
            {
                continue;
            }
            comment_lines.insert(index);
        }

        Comments {
            source,
            line_starts,
            comment_lines,
        }
    }

    /// The comment block directly above the declaration at `offset`, as
    /// Markdown, or `None` when there is none.
    ///
    /// "Directly above" means with no blank line between: a blank line is
    /// how a file header is told from the first declaration's own note,
    /// and `guestbook.zd` relies on exactly that — its five-line header is
    /// separated from `apiKey`'s three-line note by one empty line.
    pub fn above(&self, offset: u32) -> Option<String> {
        let line = self.line_of(offset);
        let first = self.block_start(line)?;
        Some(self.render(first..line))
    }

    /// The file's own header: the comment block at the top, when it is not
    /// already the first declaration's.
    ///
    /// A file that opens with a comment and then declares something on the
    /// very next line has documented that declaration, not the file.
    pub fn header(&self) -> Option<String> {
        if !self.comment_lines.contains(&0) {
            return None;
        }
        let mut end = 0;
        while self.comment_lines.contains(&(end + 1)) {
            end += 1;
        }
        // The line after the block. If it holds code, the block belongs to
        // that code and `above` will return it there.
        let next = end + 1;
        if next < self.line_starts.len() && !self.line(next).trim().is_empty() {
            return None;
        }
        Some(self.render(0..next))
    }

    /// Which line a byte offset falls on.
    fn line_of(&self, offset: u32) -> usize {
        let offset = offset as usize;
        match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            // `binary_search` returns where the offset would be inserted,
            // which is one past the line that contains it.
            Err(index) => index.saturating_sub(1),
        }
    }

    /// The first line of the contiguous comment block ending just above
    /// `line`, or `None` when the line above is not a comment.
    fn block_start(&self, line: usize) -> Option<usize> {
        let mut first = line.checked_sub(1)?;
        if !self.comment_lines.contains(&first) {
            return None;
        }
        while let Some(previous) = first.checked_sub(1) {
            if !self.comment_lines.contains(&previous) {
                break;
            }
            first = previous;
        }
        Some(first)
    }

    fn line(&self, index: usize) -> &str {
        line_at(self.source, self.line_starts[index])
    }

    /// A run of comment lines as Markdown.
    ///
    /// The `#` and one following space are removed — the space is a
    /// convention every comment in this repository follows, and leaving it
    /// on would indent every line by one, which in Markdown is harmless
    /// until four of them make a code block.
    ///
    /// A line that is bare `#` becomes an empty line, which is how a
    /// comment's own paragraph breaks survive into the page.
    fn render(&self, lines: std::ops::Range<usize>) -> String {
        let mut out = String::new();
        for index in lines {
            let line = self.line(index).trim_start();
            let body = line.strip_prefix('#').unwrap_or(line);
            let body = body.strip_prefix(' ').unwrap_or(body);
            out.push_str(body.trim_end());
            out.push('\n');
        }
        out.trim_end().to_string()
    }
}

/// The text of the line starting at `start`, without its newline.
fn line_at(source: &str, start: usize) -> &str {
    let rest = &source[start..];
    match rest.find('\n') {
        Some(end) => rest[..end].trim_end_matches('\r'),
        None => rest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_directly_above_a_declaration_is_its_documentation() {
        let source = "# The visitor count.\n# Shared by everyone.\nstate visits is durable Whole \
                      starting 0\n";
        let comments = Comments::of(source);
        let offset = source.find("state visits").expect("the declaration") as u32;
        assert_eq!(
            comments.above(offset).as_deref(),
            Some("The visitor count.\nShared by everyone.")
        );
    }

    #[test]
    fn a_blank_line_ends_the_block() {
        let source = "# About the file.\n\n# About the signal.\nstate x is client Whole starting \
                      0\n";
        let comments = Comments::of(source);
        let offset = source.find("state x").expect("the declaration") as u32;
        assert_eq!(comments.above(offset).as_deref(), Some("About the signal."));
        assert_eq!(comments.header().as_deref(), Some("About the file."));
    }

    /// The case that makes the lexer worth asking. Without the token
    /// spans, this line reads as a comment and lands in the wrong page.
    #[test]
    fn a_hash_inside_a_text_literal_is_not_a_comment() {
        let source =
            "state banner is client Text starting \"\"\"\n# a heading\n\"\"\"\nstate n is client \
             Whole starting 0\n";
        let comments = Comments::of(source);
        let offset = source.find("state n").expect("the declaration") as u32;
        assert_eq!(comments.above(offset), None);
        assert_eq!(comments.header(), None);
    }

    #[test]
    fn a_trailing_comment_documents_nothing() {
        let source = "state x is client Whole starting 0 # counts\nstate y is client Whole \
                      starting 0\n";
        let comments = Comments::of(source);
        let offset = source.find("state y").expect("the declaration") as u32;
        assert_eq!(comments.above(offset), None);
    }

    #[test]
    fn a_header_attached_to_the_first_declaration_belongs_to_it() {
        let source = "# About the signal.\nstate x is client Whole starting 0\n";
        let comments = Comments::of(source);
        assert_eq!(comments.header(), None);
        let offset = source.find("state x").expect("the declaration") as u32;
        assert_eq!(comments.above(offset).as_deref(), Some("About the signal."));
    }

    #[test]
    fn a_bare_hash_becomes_a_paragraph_break() {
        let source = "# One.\n#\n# Two.\nstate x is client Whole starting 0\n";
        let comments = Comments::of(source);
        let offset = source.find("state x").expect("the declaration") as u32;
        assert_eq!(comments.above(offset).as_deref(), Some("One.\n\nTwo."));
    }
}
