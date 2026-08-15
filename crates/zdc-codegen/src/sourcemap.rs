//! Source Map v3: the route back from a generated line to the `.zd` line
//! that produced it (#6).
//!
//! A browser stack trace names `client.js:47:2`. Without a map that is
//! where the investigation ends, because the emitted file is a compilation
//! target and nothing in it says what it came from. The emitter already
//! knows: every statement it prints has a [`Span`] attached to it in the
//! HIR, and this module is the two halves of turning that into something
//! devtools reads — the byte offsets recorded while the text is built, and
//! the V3 document they serialise to.
//!
//! # What this map claims, exactly
//!
//! **One mapping per emitted statement, at the statement's first
//! character.** That is the whole of it, and the narrowness is deliberate.
//! A segment says "the generated position *at or after* me came from
//! here", so a map that names only statement starts answers a stack trace
//! at *any* column inside a statement with that statement's own source
//! position — which is the right answer to "which line did this come
//! from?" and an honest one, because it is the granularity the emitter
//! genuinely has.
//!
//! Mapping sub-expressions would need the expression emitter to return
//! offsets as well as text, and every one of the fifty-odd sites that
//! composes an expression from its operands to preserve them. Until that
//! exists, a map that claimed a column inside a statement would be
//! *guessing*, and a map that points at the wrong place is worse than no
//! map: it costs the reader the trip before they learn not to trust it.
//!
//! # What is not mapped, and why
//!
//! - **Event handlers and view code.** A handler's body is emitted into a
//!   string that is then trimmed, re-indented, wrapped in an arrow and
//!   interpolated into a binding, and the template emitter composes those
//!   without carrying offsets. Rebasing marks through that is exactly the
//!   guesswork above.
//! - **The prelude.** §17.4.1's library is resolved into the same arenas as
//!   the program, but its spans index the prelude's *own* sources, which
//!   are not in this map's span space. A mark taken from a prelude
//!   declaration would point at a random byte of the user's file, so
//!   `emit` never records one.
//! - **Server functions.** A server stack trace happens where the `.zd`
//!   file is, and neither Deno nor Node reads a `//#` comment without
//!   being asked. Worth doing; not this change.

use zdc_lexer::Span;

use crate::js;

/// A byte offset into a fragment of emitted text, and the source span the
/// text at that offset came from.
///
/// Byte offsets rather than line/column because a fragment is written
/// before anyone knows where it lands: a function body is built on its own
/// and then spliced under a `function` header, which is under a run of
/// imports whose length depends on what the program reached. Offsets
/// rebase by addition; line numbers would have to be recounted.
pub(crate) type Mark = (usize, Span);

/// `marks`, as they read once the fragment they index begins at `base`.
pub(crate) fn rebase(marks: &[Mark], base: usize) -> impl Iterator<Item = Mark> + '_ {
    marks
        .iter()
        .map(move |(offset, span)| (base + offset, *span))
}

/// One generated position, and the source span that produced it.
///
/// Line and column are both zero-based and the column is in UTF-16 code
/// units, which is what the format means by "column" and what a browser
/// counts in. They almost always agree with bytes here — a statement mark
/// sits after a run of ASCII indentation — but "almost always" is how a
/// map ends up off by one inside the one file that has an emoji in a
/// string literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    pub line: u32,
    pub column: u32,
    pub span: Span,
}

/// One file a map names, and where its text begins in the span space.
///
/// `offset` is what makes a multi-file program work: `zdc-resolve` links
/// every module into one string and spans index *that*, so the file a span
/// belongs to is the last one whose offset does not exceed it. The caller
/// supplies the table rather than this crate reading it, for the reason
/// `Options::stylesheets` is also data: `compile` reads no file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// The name written into `sources`, as a reader should see it.
    pub name: String,
    pub text: String,
    pub offset: u32,
}

/// Whether the map carries the text of the sources it names.
///
/// **This is a decision about publishing, not about size.** `sourcesContent`
/// is what lets devtools show the `.zd` line rather than only naming it,
/// and it works because the map contains the program's source. A deployed
/// bundle's map sits at a guessable URL, so embedding there publishes every
/// line of the program to anyone who asks — which is a choice an author
/// might well make and is not one a compiler should make for them.
///
/// So `zdc dev` embeds and `zdc build` does not. The release map still
/// names the file and the line, which is the whole of what #6 asked for:
/// "there is no route back to the `.zd` line" is answered by `app.zd:12`,
/// with or without the text beside it. A developer who wants the text runs
/// the dev server, where the source is theirs already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    /// Name the sources and do not carry them. `zdc build`.
    Omit,
    /// Carry them, so a trace is readable with no source tree present.
    /// `zdc dev`.
    Embed,
}

/// The comment that tells a browser where the map is.
///
/// A comment rather than a `SourceMap` header, because the header needs a
/// server and this bundle is meant to be openable from a static host or
/// from `file:`. It needs no Content-Security-Policy exception: the page's
/// policy governs what the *page* loads, and a map is fetched by devtools
/// on the developer's behalf and not by the document — `crates/zdc-codegen/tests/csp.rs`
/// pins the policy and nothing in it mentions the map, which is correct
/// rather than an oversight.
pub fn trailer(map_name: &str) -> String {
    format!("//# sourceMappingURL={map_name}\n")
}

/// The same text with any `//#` trailer removed.
///
/// For [`Bundle::minified`](crate::Bundle::minified), which ships no map:
/// a trailer left behind would send a browser after a file the build does
/// not write, and a 404 in the console is a worse answer than silence.
///
/// Written as a suffix strip rather than a line filter because the pragma
/// is the last line by construction — `compile` appends it — and a filter
/// would also take a `//#` a program's own `foreign` module happened to
/// contain.
pub fn without_trailer(text: &str) -> String {
    match text.rfind("//# sourceMappingURL=") {
        Some(at) if text[at..].lines().count() <= 1 => text[..at].to_string(),
        _ => text.to_string(),
    }
}

/// Every mark against `text`, as generated positions.
///
/// Sorted and deduplicated by position: the format requires a line's
/// segments in increasing column order, and two marks at one position
/// would emit two segments the decoder reads as one overriding the other.
pub(crate) fn positions(text: &str, marks: &[Mark]) -> Vec<Mapping> {
    let index = LineIndex::new(text);
    let mut mappings: Vec<Mapping> = marks
        .iter()
        .filter_map(|(offset, span)| {
            let (line, column) = index.locate(text, *offset)?;
            Some(Mapping {
                line,
                column,
                span: *span,
            })
        })
        .collect();
    mappings.sort_by_key(|mapping| (mapping.line, mapping.column));
    mappings.dedup_by_key(|mapping| (mapping.line, mapping.column));
    mappings
}

/// A Source Map v3 document.
///
/// `names` is empty and stays empty: this map does not rename anything it
/// could name, so an index into a list of original identifiers has nothing
/// to point at. An empty array is what the format asks for in that case,
/// and it is one field a decoder does not have to guess about.
pub fn render(
    file: &str,
    mappings: &[Mapping],
    sources: &[SourceFile],
    content: Content,
) -> String {
    let indexes: Vec<LineIndex> = sources
        .iter()
        .map(|source| LineIndex::new(&source.text))
        .collect();

    let names: Vec<String> = sources
        .iter()
        .map(|source| js::json_string(&source.name).as_str().to_string())
        .collect();

    let mut out = String::from("{\"version\":3,\"file\":");
    out.push_str(js::json_string(file).as_str());
    out.push_str(",\"sources\":[");
    out.push_str(&names.join(","));
    out.push(']');
    if content == Content::Embed {
        let texts: Vec<String> = sources
            .iter()
            .map(|source| js::json_string(&source.text).as_str().to_string())
            .collect();
        out.push_str(",\"sourcesContent\":[");
        out.push_str(&texts.join(","));
        out.push(']');
    }
    out.push_str(",\"names\":[],\"mappings\":\"");
    out.push_str(&encode(mappings, sources, &indexes));
    out.push_str("\"}\n");
    out
}

/// The `mappings` field: base64 VLQ, relative on every axis.
///
/// The four numbers in a segment are the generated column, the index of
/// the source, its line and its column, and all four but the first are
/// deltas against the *previous segment in the whole map* rather than
/// against the previous line. The generated column resets at each `;`
/// because the line it counts within has changed; the other three do not,
/// and an encoder that reset them anyway produces a map that decodes
/// cleanly and points at the wrong file from the second line onwards.
fn encode(mappings: &[Mapping], sources: &[SourceFile], indexes: &[LineIndex]) -> String {
    let mut out = String::new();
    let mut line = 0u32;
    let mut previous_column = 0i64;
    let mut previous_source = 0i64;
    let mut previous_source_line = 0i64;
    let mut previous_source_column = 0i64;
    let mut first_on_line = true;

    for mapping in mappings {
        let Some(source) = file_of(sources, mapping.span) else {
            continue;
        };
        let local = mapping.span.start - sources[source].offset;
        let Some((source_line, source_column)) =
            indexes[source].locate(&sources[source].text, local as usize)
        else {
            continue;
        };

        while line < mapping.line {
            out.push(';');
            line += 1;
            previous_column = 0;
            first_on_line = true;
        }
        if !first_on_line {
            out.push(',');
        }
        first_on_line = false;

        let column = i64::from(mapping.column);
        let source = source as i64;
        let source_line = i64::from(source_line);
        let source_column = i64::from(source_column);
        vlq(column - previous_column, &mut out);
        vlq(source - previous_source, &mut out);
        vlq(source_line - previous_source_line, &mut out);
        vlq(source_column - previous_source_column, &mut out);
        previous_column = column;
        previous_source = source;
        previous_source_line = source_line;
        previous_source_column = source_column;
    }
    out
}

/// The index of the file `span` indexes, or `None` if no file does.
///
/// `None` is reachable and is not an error: a span from the prelude, or
/// from a synthesised node, indexes a text this map does not name. Dropping
/// the mapping is the only sound answer — the alternative is a segment
/// pointing at whichever file happens to cover that byte.
fn file_of(sources: &[SourceFile], span: Span) -> Option<usize> {
    sources
        .iter()
        .enumerate()
        .rev()
        .find(|(_, source)| {
            span.start >= source.offset
                && (span.start - source.offset) as usize <= source.text.len()
        })
        .map(|(index, _)| index)
}

const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// One base64 VLQ number.
///
/// The sign is the *low* bit of the value, not a leading minus, and the
/// continuation is the high bit of each six-bit digit. Both are easy to
/// write the other way round and neither shows up as a parse failure —
/// which is why `sourcemap.rs`'s tests decode what this writes rather than
/// comparing it against a string somebody typed out by hand.
fn vlq(value: i64, out: &mut String) {
    let mut bits = if value < 0 {
        ((value.unsigned_abs()) << 1) | 1
    } else {
        (value as u64) << 1
    };
    loop {
        let mut digit = (bits & 0b1_1111) as usize;
        bits >>= 5;
        if bits > 0 {
            digit |= 0b10_0000;
        }
        out.push(DIGITS[digit] as char);
        if bits == 0 {
            return;
        }
    }
}

/// Where every line of a text begins, so an offset can be turned into a
/// line and a column without rescanning from the top each time.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> LineIndex {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        LineIndex { starts }
    }

    /// The zero-based line and UTF-16 column of `offset`, or `None` if it
    /// is past the end or splits a character.
    fn locate(&self, text: &str, offset: usize) -> Option<(u32, u32)> {
        if offset > text.len() || !text.is_char_boundary(offset) {
            return None;
        }
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let column = text[self.starts[line]..offset]
            .chars()
            .map(char::len_utf16)
            .sum::<usize>();
        Some((u32::try_from(line).ok()?, u32::try_from(column).ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decoder the encoder is checked against, in the test rather than
    /// beside it: nothing in the compiler reads a source map, so a decoder
    /// in `src/` would be unused code whose bugs cancelled the encoder's.
    fn unvlq(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> i64 {
        let mut result = 0i64;
        let mut shift = 0;
        loop {
            let c = chars.next().expect("a VLQ digit");
            let digit = DIGITS
                .iter()
                .position(|d| *d as char == c)
                .expect("a base64 digit") as i64;
            result |= (digit & 0b1_1111) << shift;
            shift += 5;
            if digit & 0b10_0000 == 0 {
                break;
            }
        }
        if result & 1 == 1 {
            -(result >> 1)
        } else {
            result >> 1
        }
    }

    #[test]
    fn a_vlq_number_round_trips_through_its_own_decoder() {
        for value in [
            0i64, 1, -1, 15, 16, -16, 17, 31, 32, -32, 1_000, -1_000, 65_535,
        ] {
            let mut encoded = String::new();
            vlq(value, &mut encoded);
            let mut chars = encoded.chars().peekable();
            assert_eq!(unvlq(&mut chars), value, "round trip of {value}");
            assert!(chars.next().is_none(), "{value} left digits behind");
        }
    }

    /// The sign bit is the low bit, so 0 and -0 are the same number and
    /// the first digits are the ones a hand-written table would list.
    #[test]
    fn the_first_vlq_digits_are_the_documented_ones() {
        let encoded = |value| {
            let mut out = String::new();
            vlq(value, &mut out);
            out
        };
        assert_eq!(encoded(0), "A");
        assert_eq!(encoded(1), "C");
        assert_eq!(encoded(-1), "D");
        assert_eq!(encoded(16), "gB");
    }

    #[test]
    fn a_column_is_counted_in_utf16_units() {
        let text = "a\n\u{1f600}x\n";
        let index = LineIndex::new(text);
        assert_eq!(index.locate(text, 0), Some((0, 0)));
        assert_eq!(index.locate(text, 2), Some((1, 0)));
        // The emoji is one `char` and two UTF-16 units.
        assert_eq!(index.locate(text, 6), Some((1, 2)));
        assert_eq!(index.locate(text, 3), None, "mid-character");
    }
}
