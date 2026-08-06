//! Where a block starts and stops, for folding.
//!
//! Indentation is syntax here, so the fold structure is not a second
//! analysis of the file: it is what the lexer's layout pass already
//! computed. `Indent` and `Dedent` are tokens, emitted where a block opens
//! and closes, and a fold is the pair.
//!
//! Read off the layout tokens rather than re-measured from the text, so a
//! fold cannot disagree with the block the parser saw. A file that does
//! not parse still folds, because the layout pass runs before the parser
//! and its tokens survive a parse error (`crate::analysis`).

use zdc_lexer::TokenKind;

use crate::analysis::Analysis;

/// One foldable block: the line that opens it, and the last line inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    /// The line the block hangs from: the `view`, the `function`, the
    /// `each`. It stays visible when the block is folded, which is what
    /// makes a fold readable.
    pub start_line: u32,
    pub end_line: u32,
}

/// Every foldable block in the document, outermost first.
pub fn folds(analysis: &Analysis) -> Vec<Fold> {
    let text = analysis.text();
    let lines = analysis.lines();
    let last_line = last_line_with_text(text);

    let mut open: Vec<u32> = Vec::new();
    let mut out: Vec<Fold> = Vec::new();
    for token in analysis.tokens() {
        // A layout token carries the span of the line break that produced
        // it, and a line break belongs to the line it ends. So an
        // `Indent`'s line is the header the block hangs from, and a
        // `Dedent`'s is the last line of the block being closed.
        let line = lines.position(text, token.span.start).line;
        match token.kind {
            TokenKind::Indent => open.push(line),
            TokenKind::Dedent => {
                let Some(start_line) = open.pop() else {
                    // More dedents than indents cannot come out of the
                    // layout pass, which balances them. Ignoring one
                    // rather than trusting the invariant costs nothing.
                    continue;
                };
                // The dedents that close a file are emitted at the end of
                // the text, which is past the last line that has anything
                // on it when the file ends in a newline.
                let end_line = line.min(last_line);
                if end_line > start_line {
                    out.push(Fold {
                        start_line,
                        end_line,
                    });
                }
            }
            _ => {}
        }
    }

    // Outermost first, and within one nesting level in source order, which
    // is the order an editor's fold gutter is drawn in.
    out.sort_by_key(|fold| (fold.start_line, std::cmp::Reverse(fold.end_line)));
    out
}

/// The last line of the text that has anything but whitespace on it.
fn last_line_with_text(text: &str) -> u32 {
    let mut found = 0;
    for (number, line) in text.lines().enumerate() {
        if !line.trim().is_empty() {
            found = u32::try_from(number).unwrap_or(u32::MAX);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_and_the_blocks_inside_it_each_fold() {
        let src = "state count is client Whole starting 0\n\
                   view\n\
                   \x20   Column\n\
                   \x20       Text count\n\
                   \x20       Button \"go\"\n\
                   \x20           on click\n\
                   \x20               add 1 to count\n";
        let analysis = Analysis::of(src);
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );

        assert_eq!(
            folds(&analysis),
            [
                // `view`, lines 1 to 6.
                Fold {
                    start_line: 1,
                    end_line: 6
                },
                // `Column`, lines 2 to 6.
                Fold {
                    start_line: 2,
                    end_line: 6
                },
                // `Button`, lines 4 to 6.
                Fold {
                    start_line: 4,
                    end_line: 6
                },
                // `on click`, lines 5 to 6.
                Fold {
                    start_line: 5,
                    end_line: 6
                },
            ]
        );
    }

    /// A file that ends in a newline must not fold onto the empty line
    /// after it, which is where the closing dedents are emitted.
    #[test]
    fn a_fold_stops_at_the_last_line_that_has_anything_on_it() {
        let src = "function twice with n\n    give n + n\n\n\n";
        let analysis = Analysis::of(src);
        assert_eq!(
            folds(&analysis),
            [Fold {
                start_line: 0,
                end_line: 1
            }]
        );
    }

    /// A file with no blocks folds nowhere, rather than folding the whole
    /// file into one range that means nothing.
    #[test]
    fn a_file_with_no_indentation_has_no_folds() {
        let src = "state a is client Whole starting 0\nstate b is client Whole starting 1\n";
        let analysis = Analysis::of(src);
        assert!(folds(&analysis).is_empty());
    }

    /// The layout pass runs before the parser, so folding survives a file
    /// that does not parse, which is what it is like while being typed.
    #[test]
    fn a_file_that_does_not_parse_still_folds() {
        let src = "view\n    Text (1 + 2\n    Text \"after\"\n";
        let analysis = Analysis::of(src);
        assert!(
            !analysis.diagnostics().is_empty(),
            "the fixture must be one that does not parse"
        );
        assert_eq!(
            folds(&analysis),
            [Fold {
                start_line: 0,
                end_line: 2
            }]
        );
    }

    #[test]
    fn asking_of_a_broken_file_never_panics() {
        let sources = ["", " ", "\t\tstate", "{\"json\": true}", "((((((("];
        for src in sources {
            let _ = folds(&Analysis::of(src));
        }
    }
}
