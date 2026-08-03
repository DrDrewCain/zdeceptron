//! Byte offsets to editor positions, and back.
//!
//! The compiler counts bytes: every `Span` in every pass is a byte range,
//! because that is what the lexer produced. The protocol counts UTF-16
//! code units within a line, because that is what LSP's default position
//! encoding is. The conversion is the whole of this module, and it is the
//! most likely place in the server for an out-of-range index to become a
//! panic — so every entry point here clamps rather than indexes.

use zdc_lexer::Span;

/// Where each line of a source text begins.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// The byte offset of the first character of each line. Always
    /// non-empty: an empty file still has one line.
    starts: Vec<u32>,
    /// The length of the text in bytes, so an offset past the end can be
    /// clamped rather than searched for.
    len: u32,
}

/// A zero-based line and a zero-based UTF-16 code unit offset within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl LineIndex {
    pub fn new(text: &str) -> LineIndex {
        let mut starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                // A file longer than `u32::MAX` cannot be lexed either —
                // spans are `u32` — so saturating here agrees with the
                // rest of the compiler rather than inventing a new limit.
                starts.push(saturating_u32(offset + 1));
            }
        }
        LineIndex {
            starts,
            len: saturating_u32(text.len()),
        }
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// The position of a byte offset, clamped into the text.
    ///
    /// An offset past the end of the text, or one that lands inside a
    /// multi-byte character, yields the nearest position rather than a
    /// panic: both arise from a span the compiler produced against a text
    /// the editor has since changed.
    pub fn position(&self, text: &str, offset: u32) -> Position {
        let offset = offset.min(self.len);
        let line = self.line_of(offset);
        let start = self.starts.get(line).copied().unwrap_or(0);
        let slice = slice(text, start, offset);
        Position {
            line: saturating_u32(line),
            character: utf16_len(slice),
        }
    }

    /// The byte offset of a position, clamped into the text.
    ///
    /// A line past the end of the file yields the end of the file, and a
    /// character past the end of a line yields the end of that line. An
    /// editor sends both routinely — a keystroke can arrive between the
    /// text change and the position that described it.
    pub fn offset(&self, text: &str, position: Position) -> u32 {
        let line = position.line as usize;
        let Some(&start) = self.starts.get(line) else {
            return self.len;
        };
        let end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.len)
            .min(self.len);
        let slice = slice(text, start, end);

        let mut consumed = 0u32;
        for (at, ch) in slice.char_indices() {
            if consumed >= position.character {
                return start.saturating_add(saturating_u32(at));
            }
            consumed = consumed.saturating_add(saturating_u32(ch.len_utf16()));
        }
        end
    }

    /// The half-open position range of a span.
    pub fn range(&self, text: &str, span: Span) -> (Position, Position) {
        (
            self.position(text, span.start),
            self.position(text, span.end.max(span.start)),
        )
    }

    /// The index of the line containing this offset.
    fn line_of(&self, offset: u32) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(line) => line,
            // `Err(0)` cannot occur: the first line starts at 0 and the
            // offset is non-negative, so the insertion point is at least
            // 1. Saturating rather than subtracting keeps that reasoning
            // from being load-bearing.
            Err(after) => after.saturating_sub(1),
        }
    }
}

/// The number of UTF-16 code units in a string.
fn utf16_len(text: &str) -> u32 {
    text.chars().fold(0u32, |total, ch| {
        total.saturating_add(ch.len_utf16() as u32)
    })
}

/// A byte range of a text, or `""` if the range is not on character
/// boundaries. Never panics, where `&text[a..b]` would.
fn slice(text: &str, start: u32, end: u32) -> &str {
    let (start, end) = (start as usize, end as usize);
    if start > end {
        return "";
    }
    text.get(start..end).unwrap_or("")
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &str, offset: u32) -> u32 {
        let index = LineIndex::new(text);
        let position = index.position(text, offset);
        index.offset(text, position)
    }

    #[test]
    fn an_empty_file_has_one_line() {
        let index = LineIndex::new("");
        assert_eq!(index.line_count(), 1);
        assert_eq!(
            index.position("", 0),
            Position {
                line: 0,
                character: 0
            }
        );
    }

    #[test]
    fn a_position_names_its_line_and_column() {
        let text = "state a\nstate b\nstate c\n";
        let index = LineIndex::new(text);
        let at = text.find("state c").expect("the third line") as u32;
        assert_eq!(
            index.position(text, at),
            Position {
                line: 2,
                character: 0
            }
        );
    }

    /// The protocol counts UTF-16 code units, so an emoji before the
    /// cursor moves it two columns and a `é` moves it one, whatever their
    /// byte lengths are.
    #[test]
    fn columns_are_counted_in_utf16_code_units_not_bytes() {
        let text = "# \u{1f600} \u{e9}x\n";
        let index = LineIndex::new(text);
        let at = text.find('x').expect("the marker") as u32;

        // Bytes: "# " is 2, the emoji is 4, " " is 1, "é" is 2 — 9 bytes.
        assert_eq!(at, 9);
        // UTF-16: "# " is 2, the emoji is 2, " " is 1, "é" is 1 — 6 units.
        assert_eq!(
            index.position(text, at),
            Position {
                line: 0,
                character: 6
            }
        );
        assert_eq!(round_trip(text, at), at);
    }

    #[test]
    fn crlf_line_endings_still_split_lines() {
        let text = "state a\r\nstate b\r\n";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 3);
        let at = text.find("state b").expect("the second line") as u32;
        assert_eq!(
            index.position(text, at),
            Position {
                line: 1,
                character: 0
            }
        );
    }

    /// Every one of these would index out of bounds if the conversion
    /// used slicing. An editor produces all of them while a file is being
    /// typed into.
    #[test]
    fn out_of_range_positions_clamp_rather_than_panic() {
        let text = "state a\n";
        let index = LineIndex::new(text);

        assert_eq!(index.position(text, 9_999).line, 1);
        assert_eq!(
            index.offset(
                text,
                Position {
                    line: 9_999,
                    character: 0
                }
            ),
            saturating_u32(text.len())
        );
        assert_eq!(
            index.offset(
                text,
                Position {
                    line: 0,
                    character: 9_999
                }
            ),
            8
        );
    }

    /// A span the compiler produced against an older revision of the file
    /// can land inside a character. It must still produce a position.
    #[test]
    fn an_offset_inside_a_character_does_not_panic() {
        let text = "\u{1f600}\n";
        let index = LineIndex::new(text);
        for offset in 0..8 {
            let _ = index.position(text, offset);
        }
    }

    #[test]
    fn every_offset_in_a_mixed_file_round_trips() {
        let text = "state \u{e9}\n# \u{1f600}\nview\n    Text \"\u{4e2d}\u{6587}\"\n";
        assert!(
            text.char_indices().count() > 30,
            "the fixture must be long enough to be worth walking"
        );
        for (offset, _) in text.char_indices() {
            let offset = offset as u32;
            assert_eq!(round_trip(text, offset), offset, "at byte {offset}");
        }
    }

    #[test]
    fn a_span_becomes_a_range_covering_it() {
        let text = "state count is client Whole starting 0\n";
        let index = LineIndex::new(text);
        let at = text.find("count").expect("the name") as u32;
        let (start, end) = index.range(text, Span::new(at, at + 5));
        assert_eq!(
            start,
            Position {
                line: 0,
                character: 6
            }
        );
        assert_eq!(
            end,
            Position {
                line: 0,
                character: 11
            }
        );
    }
}
