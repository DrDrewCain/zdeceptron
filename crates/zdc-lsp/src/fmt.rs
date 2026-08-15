//! The canonical layout, as edits an editor can apply.
//!
//! `zdc-fmt` already decides what the one canonical layout of a file is,
//! and `zdc fmt` writes it to disk. This is the same answer delivered the
//! other way round: as a list of edits, so an editor can put a buffer into
//! that layout without the file having been saved first. The layout itself
//! is not re-decided here — a second opinion about where a line goes would
//! be a second formatter, and the command line and the editor would drift
//! apart the first time one of them was changed.
//!
//! # A file the compiler cannot read is not edited
//!
//! [`zdc_fmt::format`] refuses a source that does not parse, because a
//! formatter that rewrites a file it cannot read is a formatter that
//! destroys work. That refusal is passed straight through as "no edits",
//! and the document is left exactly as it is. It matters more here than on
//! the command line: format-on-save fires on every save, and a save in the
//! middle of an unfinished edit is the ordinary case rather than the
//! exceptional one. What the programmer sees is the file unchanged and the
//! syntax error already underlined, which is the only answer that cannot
//! lose a keystroke.
//!
//! # Why the client's formatting options are ignored
//!
//! `FormattingOptions` carries `tabSize` and `insertSpaces`, and neither
//! can be honoured. §4.1's bargain is one phrasing per construct, and
//! [`zdc_fmt::INDENT`] is the indentation half of it: a server that
//! indented by the editor's `tabSize` would lay one file out two ways in
//! two editors, and `zdc fmt --check` in CI would disagree with both.
//! `insertSpaces: false` is refused a level lower still — the lexer rejects
//! a tab as indentation outright — so honouring it would produce a file the
//! compiler will not read.
//!
//! # Why `textDocument/rangeFormatting` is not answered
//!
//! It is deliberately not advertised, and this is the note the next person
//! looking for it should find. Laying out one range is not something this
//! formatter can do honestly:
//!
//! * The gate is **parsing the whole file**. A selected range is almost
//!   never a program on its own — half a `view`, one arm of a `when` — so
//!   the honest answer to nearly every range request would be a refusal.
//! * Indentation is syntax here, so a line's depth comes from the block
//!   structure *above* it, which the selection does not contain.
//! * A comment takes the indentation of the next code line below it, which
//!   may be outside the selection.
//!
//! Filtering a whole-file layout down to the lines that overlap the
//! selection would answer the request without doing what it asks: the
//! result would depend on text the user did not select, and it would still
//! do nothing at all when the error that makes the file unreadable is
//! somewhere else. An unadvertised capability is a menu entry that is
//! greyed out, which is an honest report; an advertised one that usually
//! refuses is a feature that appears to be broken.
//!
//! # Why the edits are small
//!
//! One edit replacing the whole document is the easy answer and a bad one:
//! most editors restore a cursor by character offset rather than by what
//! the code says, and every selection and every mark inside a replaced
//! range is lost. The formatter only ever changes the whitespace at the
//! ends of lines and which blank lines are kept, so the difference between
//! two layouts of one file is usually a handful of lines, and saying so
//! leaves everything else untouched. The difference is taken over whole
//! lines and then narrowed within each changed line to the bytes that
//! really differ, so re-indenting a line is one edit covering its
//! indentation and nothing else.

use zdc_lexer::Span;

use crate::analysis::Analysis;

/// One replacement: the byte range of the document to overwrite, and the
/// text to put there.
///
/// An empty range is an insertion and empty text is a deletion. Ranges
/// never overlap and arrive in ascending order, which is what the protocol
/// requires of a set of edits a client may apply in any order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub at: Span,
    pub text: String,
}

/// The edits that put a document into the canonical layout.
///
/// `None` when the file cannot be laid out — it does not parse, or it puts
/// code on a line that is still inside a block text literal. An empty list
/// when the document is already canonical, which is the answer that has to
/// arrive whenever `zdc fmt --check` would be silent.
///
/// The source is parsed again here rather than reusing the analysis the
/// server already holds. `zdc_fmt::format` owns its own gate — the verdict
/// that this is a file the compiler reads — and reaching around it to ask
/// the analysis instead would put the decision to rewrite somebody's file
/// in two places. It costs one parse of one file, on a keystroke the
/// programmer typed on purpose, which is not the per-keystroke path.
pub fn formatting(analysis: &Analysis) -> Option<Vec<Edit>> {
    let before = analysis.text();
    let after = zdc_fmt::format(before).ok()?;
    Some(edits(before, &after))
}

/// The edits that turn `before` into `after`.
fn edits(before: &str, after: &str) -> Vec<Edit> {
    let old = lines(before);
    let new = lines(after);

    // The lines at each end that both texts agree about. A file that is
    // already canonical is entirely head, and a file with one line to
    // repair leaves one line between head and tail.
    let head = (0..old.len().min(new.len()))
        .take_while(|index| old[*index] == new[*index])
        .count();
    let tail = (0..old.len().min(new.len()) - head)
        .take_while(|index| old[old.len() - 1 - index] == new[new.len() - 1 - index])
        .count();

    let old_middle = &old[head..old.len() - tail];
    let new_middle = &new[head..new.len() - tail];
    let start: usize = old[..head].iter().map(|line| line.len()).sum();
    let end = start + old_middle.iter().map(|line| line.len()).sum::<usize>();

    let replacements = match script(old_middle, new_middle) {
        Some(script) => grouped(old_middle, new_middle, start, &script),
        // More of the file changed than the search is willing to look at,
        // which means most of it changed. Replacing the middle whole is
        // then both the honest answer and very nearly the minimal one.
        None => vec![Edit {
            at: span(start, end),
            text: new_middle.concat(),
        }],
    };

    replacements
        .into_iter()
        .map(|edit| narrowed(before, edit))
        // Narrowing cannot empty an edit that came out of the difference,
        // but an edit that changes nothing would be noise in a client's
        // undo history, so it is not sent on the strength of an argument.
        .filter(|edit| edit.at.start != edit.at.end || !edit.text.is_empty())
        .collect()
}

/// The lines of a text, each carrying its own line terminator.
///
/// The terminator belongs to the line so that the offsets add up: a run of
/// lines is then a byte range of the source with nothing between them to
/// account for separately. `\r\n` stays whole, which is what keeps a CRLF
/// file's edits from cutting a line ending in half.
fn lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (at, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            out.push(&text[start..=at]);
            start = at + 1;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// One step of an edit script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// A line both texts have.
    Keep,
    /// A line only the old text has.
    Drop,
    /// A line only the new text has.
    Add,
}

/// The longest edit script this will search for, in steps.
///
/// Myers' algorithm costs `(n + m) · d` time and `d²` space in the length
/// `d` of the script it finds, so it is nearly free when a file needs two
/// lines repaired and expensive when every line of it moved. Past this
/// bound the answer is one replacement of the whole changed region, which
/// is what a file that changed everywhere was going to get anyway: a
/// thousand-step script is five hundred rewritten lines, and there is no
/// cursor left to save.
const LONGEST_SCRIPT: usize = 1024;

/// The shortest script turning `old` into `new`, or `None` if every script
/// is longer than [`LONGEST_SCRIPT`].
///
/// Myers' greedy algorithm (1986). `furthest[k]` is the furthest point
/// reached on diagonal `k = x - y` after `d` steps, where a step right
/// drops a line of `old` and a step down adds a line of `new`; the run of
/// equal lines after each step is free. Every round is saved so the path
/// can be walked back once the end is reached.
fn script(old: &[&str], new: &[&str]) -> Option<Vec<Step>> {
    let n = old.len() as isize;
    let m = new.len() as isize;
    let longest = (old.len() + new.len()).min(LONGEST_SCRIPT);
    // One past the furthest diagonal any round can reach, so that `k ± 1`
    // is inside the array without being checked.
    let bound = longest + 1;
    let mut furthest = vec![0isize; 2 * bound + 1];
    let mut trace: Vec<Vec<isize>> = Vec::with_capacity(longest + 1);

    for round in 0..=longest {
        // Only the diagonals this round reads, which is also the window the
        // walk back reads. Saving the whole array each round would cost the
        // length of the file per step rather than the length of the script.
        trace.push(furthest[bound - round..=bound + round].to_vec());

        let d = round as isize;
        let mut k = -d;
        while k <= d {
            let at = (bound as isize + k) as usize;
            let mut x = if k == -d || (k != d && furthest[at - 1] < furthest[at + 1]) {
                furthest[at + 1]
            } else {
                furthest[at - 1] + 1
            };
            let mut y = x - k;
            while same(old, new, x, y) {
                x += 1;
                y += 1;
            }
            furthest[at] = x;
            // Both sequences consumed. It is exactly `(n, m)`: reaching it
            // at all takes `n + m - 2·(equal lines)` steps, so no shorter
            // round can, and no state past the end can be reached sooner.
            if x >= n && y >= m {
                return Some(walk_back(&trace, round, n, m));
            }
            k += 2;
        }
    }
    None
}

/// Whether both texts have a line at this point and the two agree.
///
/// Indexed through `get` rather than by slicing. The algorithm's own
/// invariants keep both coordinates inside their sequences, but a language
/// server may not panic on anybody's file, and a bound that is checked
/// anyway costs nothing beside a string comparison.
fn same(old: &[&str], new: &[&str], x: isize, y: isize) -> bool {
    let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
        return false;
    };
    match (old.get(x), new.get(y)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// The script the saved rounds describe, read back from `(n, m)`.
fn walk_back(trace: &[Vec<isize>], found: usize, n: isize, m: isize) -> Vec<Step> {
    let mut steps = Vec::new();
    let (mut x, mut y) = (n, m);

    for round in (0..=found).rev() {
        let window = &trace[round];
        let d = round as isize;
        // The window holds diagonals `-d ..= d`, so diagonal `k` is at
        // `k + d`. Reading past it is unreachable — the branch that would
        // need `k - 1` at `k == -d` is the branch not taken — and answering
        // zero rather than panicking keeps that reasoning off the critical
        // path.
        let value = |k: isize| -> isize {
            usize::try_from(k + d)
                .ok()
                .and_then(|index| window.get(index))
                .copied()
                .unwrap_or(0)
        };

        let k = x - y;
        let (previous_x, previous_y) = if round == 0 {
            (0, 0)
        } else {
            let previous_k = if k == -d || (k != d && value(k - 1) < value(k + 1)) {
                k + 1
            } else {
                k - 1
            };
            let previous_x = value(previous_k);
            (previous_x, previous_x - previous_k)
        };

        // The run of equal lines that ended this round, walked back first.
        while x > previous_x && y > previous_y {
            steps.push(Step::Keep);
            x -= 1;
            y -= 1;
        }
        if round > 0 {
            steps.push(if x == previous_x {
                Step::Add
            } else {
                Step::Drop
            });
            x = previous_x;
            y = previous_y;
        }
    }

    steps.reverse();
    steps
}

/// Runs of dropped and added lines, as one replacement each.
///
/// A run is grouped rather than emitted step by step because a client
/// applies each edit separately: deleting a line and inserting its
/// replacement is one rewritten line to a reader and two entries in an
/// undo history.
fn grouped(old: &[&str], new: &[&str], start: usize, script: &[Step]) -> Vec<Edit> {
    let mut out = Vec::new();
    let mut at = start;
    let (mut i, mut j, mut step) = (0usize, 0usize, 0usize);

    while step < script.len() {
        if script[step] == Step::Keep {
            at += old.get(i).map_or(0, |line| line.len());
            i += 1;
            j += 1;
            step += 1;
            continue;
        }

        let from = at;
        let mut text = String::new();
        while step < script.len() && script[step] != Step::Keep {
            match script[step] {
                Step::Drop => {
                    at += old.get(i).map_or(0, |line| line.len());
                    i += 1;
                }
                Step::Add => {
                    text.push_str(new.get(j).copied().unwrap_or(""));
                    j += 1;
                }
                Step::Keep => break,
            }
            step += 1;
        }
        out.push(Edit {
            at: span(from, at),
            text,
        });
    }
    out
}

/// An edit shrunk to the bytes that really differ.
///
/// A re-indented line arrives here as a replacement of the whole line by
/// the whole line with different spaces at the front, which as an edit
/// moves every cursor and mark on it. Trimming what the two sides already
/// agree about leaves an edit covering the indentation alone.
///
/// Both ends are pulled back to a character boundary. The bytes either side
/// of a trim are equal by construction, so the two strings agree about
/// where the boundaries are — except at the trim itself, where a character
/// whose leading bytes match and whose last byte does not is exactly the
/// case that makes the check necessary.
fn narrowed(before: &str, edit: Edit) -> Edit {
    let Some(old) = before.get(edit.at.start as usize..edit.at.end as usize) else {
        // Unreachable: the range came from adding up the lines of this very
        // text. Handing the edit back untouched rather than panicking,
        // because the cost of being wrong here is an edit that is merely
        // larger than it had to be.
        return edit;
    };
    let new = edit.text.as_str();

    let mut head = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(left, right)| left == right)
        .count();
    while head > 0 && !(old.is_char_boundary(head) && new.is_char_boundary(head)) {
        head -= 1;
    }

    let mut tail = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(old.len().min(new.len()) - head)
        .take_while(|(left, right)| left == right)
        .count();
    while tail > 0
        && !(old.is_char_boundary(old.len() - tail) && new.is_char_boundary(new.len() - tail))
    {
        tail -= 1;
    }

    Edit {
        at: span(edit.at.start as usize + head, edit.at.end as usize - tail),
        text: new[head..new.len() - tail].to_string(),
    }
}

/// A byte range as a span, saturating rather than wrapping.
///
/// Spans are `u32` throughout the compiler, and a file too long to index
/// with one is a file the lexer refused long before this ran.
fn span(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document `edits` describes, so a test asserts what an editor
    /// would end up holding rather than what the edit list looks like.
    ///
    /// Applied back to front, which is the one order that does not need the
    /// later ranges rewritten as the earlier edits change the offsets. That
    /// the ranges are disjoint and ascending is asserted on the way.
    fn applied(before: &str, edits: &[Edit]) -> String {
        let mut out = before.to_string();
        let mut last = before.len();
        for edit in edits.iter().rev() {
            assert!(edit.at.start <= edit.at.end, "{edit:?} runs backwards");
            assert!(
                edit.at.end as usize <= last,
                "{edit:?} overlaps the edit after it"
            );
            last = edit.at.start as usize;
            out.replace_range(edit.at.start as usize..edit.at.end as usize, &edit.text);
        }
        out
    }

    /// Every source the unit tests below and `zdc-fmt`'s own tests are
    /// written over, plus the shapes that only matter to a diff.
    const SOURCES: &[&str] = &[
        "",
        "\n\n   \n",
        "view\n    Column\n",
        "view\n  Column\n      Text \"hi\"\n",
        "view\n        Column\n                Text \"hi\"\n",
        "view   \n    Column  \n",
        "view\n    Column",
        "view\n    Column\n\n\n",
        "state a is client Whole starting 1\n\n\n\nview\n    Column\n",
        "\n\nview\n    Column\n",
        "# a header note\nview\n    Column\n        # why\n        Text \"hi\"\n",
        "view\n    Column\n# about the Text\n        Text \"hi\"\n",
        "view\n    Column\n        Text \"hi\"\n        # a closing note\n",
        "state s is client Text starting \"\"\"\n        one\n          two\n        \"\"\"\n",
        "view\r\n  Column\r\n",
        "state a is client Whole starting 1\nstate b is client Whole starting 2\n\
         state c is client Whole starting 3\nview\n  Column\n    Text a\n",
        "# 😀 é\nview\n  Column\n        Text \"中文\"\n",
    ];

    /// The whole contract, over every shape: applying what this returns to
    /// the document yields exactly what `zdc fmt` would have written.
    ///
    /// This is what makes the difference algorithm above safe to trust. A
    /// minimal edit list that reconstructs the wrong text is the one bug a
    /// formatter must not have, and it is invisible in an assertion about
    /// how many edits came back.
    #[test]
    fn applying_the_edits_yields_what_the_formatter_would_have_written() {
        for source in SOURCES {
            let expected = zdc_fmt::format(source).expect("a readable source");
            let found = edits(source, &expected);
            assert_eq!(
                applied(source, &found),
                expected,
                "the edits for {source:?} rebuilt the wrong document: {found:?}"
            );
        }
    }

    /// A file already in the canonical layout is answered with nothing at
    /// all — not with an edit that replaces it by itself, which would dirty
    /// the buffer and add an undo step on every save.
    #[test]
    fn an_already_canonical_file_needs_no_edits() {
        for source in SOURCES {
            let canonical = zdc_fmt::format(source).expect("a readable source");
            assert_eq!(
                edits(&canonical, &canonical),
                Vec::new(),
                "{canonical:?} is canonical and was still edited"
            );
        }
    }

    /// A misindented line is one edit, and it covers the indentation rather
    /// than the line. Everything else on that line — every cursor, every
    /// selection, every mark — is left where it was.
    #[test]
    fn one_misindented_line_is_one_edit_of_the_indentation_alone() {
        let source = "view\n    Column\n          Text \"a\"\n";
        let expected = zdc_fmt::format(source).expect("a readable source");
        assert_eq!(expected, "view\n    Column\n        Text \"a\"\n");

        let found = edits(source, &expected);
        let over_indented = source.find("          Text").expect("the mangled line") as u32;
        assert_eq!(
            found,
            [Edit {
                // The two spaces too many, and not the line they are on.
                at: Span::new(over_indented + 8, over_indented + 10),
                text: String::new(),
            }]
        );
        assert_eq!(applied(source, &found), expected);
    }

    /// Two distant repairs are two edits and not one covering everything
    /// between them. This is the case that a difference made only of a
    /// common prefix and a common suffix gets wrong, and the reason there
    /// is a real algorithm above.
    ///
    /// Over plain text rather than through the formatter: the point is what
    /// the difference does with two changes far apart, and a source mangled
    /// enough to produce them line by line is a source that does not parse,
    /// because an indentation this language does not accept is a lexical
    /// error rather than a layout to be repaired.
    #[test]
    fn two_distant_repairs_do_not_swallow_the_lines_between_them() {
        let middle: String = (0..40).map(|n| format!("        Text \"{n}\"\n")).collect();
        let before = format!("view\n  Column\n{middle}      Text \"last\"\n");
        let after = format!("view\n    Column\n{middle}        Text \"last\"\n");

        let found = edits(&before, &after);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(applied(&before, &found), after);
    }

    /// A blank line the formatter drops is a deletion of that line, not a
    /// rewrite of the lines around it.
    #[test]
    fn a_dropped_blank_line_is_a_deletion() {
        let source = "view\n    Column\n\n\n    Text \"hi\"\n";
        let expected = zdc_fmt::format(source).expect("a readable source");
        let found = edits(source, &expected);

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].text.is_empty(), "{found:?}");
        assert_eq!(applied(source, &found), expected);
    }

    /// A missing final newline is one insertion at the end of the file.
    #[test]
    fn a_missing_final_newline_is_one_insertion() {
        let source = "view\n    Column";
        let expected = zdc_fmt::format(source).expect("a readable source");
        assert_eq!(
            edits(source, &expected),
            [Edit {
                at: Span::new(source.len() as u32, source.len() as u32),
                text: "\n".to_string(),
            }]
        );
    }

    /// Narrowing an edit must not cut a character in half. The bytes of `é`
    /// and `è` agree except for the last one, so a byte-wise trim lands
    /// inside both.
    #[test]
    fn narrowing_stops_at_character_boundaries() {
        let before = "état\n";
        let edit = narrowed(
            before,
            Edit {
                at: Span::new(0, before.len() as u32),
                text: "ètat\n".to_string(),
            },
        );
        assert_eq!(before.get(..edit.at.start as usize), Some(""));
        assert!(before.is_char_boundary(edit.at.start as usize));
        assert!(before.is_char_boundary(edit.at.end as usize));

        let mut out = before.to_string();
        out.replace_range(edit.at.start as usize..edit.at.end as usize, &edit.text);
        assert_eq!(out, "ètat\n");
    }

    /// A CRLF file's edits must not split a line ending: the formatter
    /// keeps whatever ending the file has, so the two texts differ in
    /// indentation and agree about the `\r`.
    #[test]
    fn a_crlf_file_is_edited_without_touching_its_line_endings() {
        let source = "view\r\n  Column\r\n";
        let expected = zdc_fmt::format(source).expect("a readable source");
        let found = edits(source, &expected);

        assert_eq!(applied(source, &found), expected);
        assert!(
            found.iter().all(|edit| !edit.text.contains('\r')),
            "a line ending was rewritten: {found:?}"
        );
    }

    /// A difference longer than the search will look at still rebuilds the
    /// document exactly — as one replacement of the changed region, which
    /// is the fallback that must not be wrong for being simple.
    #[test]
    fn a_difference_past_the_search_bound_falls_back_and_is_still_exact() {
        let before: String = (0..LONGEST_SCRIPT).map(|n| format!("{n}\n")).collect();
        let after: String = (0..LONGEST_SCRIPT).map(|n| format!("x{n}\n")).collect();
        let found = edits(&before, &after);

        assert_eq!(found.len(), 1, "the fallback is a single replacement");
        assert_eq!(applied(&before, &found), after);
    }

    /// Every source in the whole file's worth of shapes, truncated at every
    /// character boundary. Most of these do not parse and are refused by
    /// `zdc_fmt`, so this drives the difference directly: no prefix of any
    /// file may panic or rebuild the wrong text.
    #[test]
    fn no_prefix_of_a_file_can_break_the_difference() {
        let target = "# 😀\nstate count is client Whole starting 0\nview\n    Text count\n";
        for (end, _) in target.char_indices().chain([(target.len(), ' ')]) {
            let before = &target[..end];
            for after in [target, "", "view\n"] {
                let found = edits(before, after);
                assert_eq!(applied(before, &found), after, "prefix of {end} bytes");
            }
        }
    }

    /// The public entry point refuses a file the compiler will not read.
    /// Nothing is edited, so nothing is lost.
    #[test]
    fn a_file_that_does_not_parse_is_answered_with_no_edits_at_all() {
        assert_eq!(formatting(&Analysis::of("view\n\tColumn\n")), None);
        assert_eq!(formatting(&Analysis::of("state a is client")), None);
    }

    #[test]
    fn a_readable_file_is_answered_with_the_edits_that_lay_it_out() {
        let source = "view\n  Column\n";
        let found = formatting(&Analysis::of(source)).expect("a readable source");
        assert_eq!(applied(source, &found), "view\n    Column\n");
        assert!(formatting(&Analysis::of("view\n    Column\n"))
            .expect("a readable source")
            .is_empty());
    }
}
