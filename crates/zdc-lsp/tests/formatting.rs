//! What the language server answers `textDocument/formatting` with.
//!
//! Three things are being held, and the third is the one that would be
//! expensive to get wrong:
//!
//! 1. **A document already in the canonical layout is answered with no
//!    edits.** Format-on-save fires on every save, and an edit that
//!    replaces a file by itself dirties the buffer and adds an undo step
//!    each time.
//!
//! 2. **A document the compiler cannot read is answered with nothing at
//!    all.** A formatter that rewrites a file it could not parse is a
//!    formatter that destroys work, and a save in the middle of an
//!    unfinished edit is the ordinary case rather than the exceptional one.
//!
//! 3. **Applying the edits yields exactly what `zdc fmt` writes.** The
//!    edits are minimal — a re-indented line is an edit covering its
//!    indentation and not the line — and a minimal edit list that rebuilds
//!    the wrong document is invisible in any assertion about how many edits
//!    came back. So the assertion is on the document, over every file in
//!    `examples/` and over a mangled copy of each.

use std::path::{Path, PathBuf};

use zdc_lsp::{formatting, Analysis, Edit};

/// The document an edit list describes, so a test asserts on what the
/// programmer would be left holding.
///
/// Applied last first, so that an earlier edit's offsets are still the ones
/// it was computed against. That the ranges are disjoint and ascending —
/// which the protocol requires, since a client may apply them in any order
/// — is asserted on the way past.
fn applied(before: &str, edits: &[Edit]) -> String {
    let mut out = before.to_string();
    let mut lowest = before.len();
    for edit in edits.iter().rev() {
        assert!(edit.at.start <= edit.at.end, "{edit:?} runs backwards");
        assert!(
            edit.at.end as usize <= lowest,
            "{edit:?} overlaps the edit after it"
        );
        lowest = edit.at.start as usize;
        out.replace_range(edit.at.start as usize..edit.at.end as usize, &edit.text);
    }
    out
}

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Every `.zd` file under `examples/`, at any depth — `examples/tree/` has
/// one of its own, and a formatter gate that skips a file does not cover
/// it.
fn examples() -> Vec<PathBuf> {
    fn walk(at: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(at).expect("a readable directory") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                walk(&path, into);
            } else if path.extension().and_then(|e| e.to_str()) == Some("zd") {
                into.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&examples_dir(), &mut found);
    found.sort();
    found
}

/// A file whose layout has been destroyed but whose block structure has
/// not: every indentation doubled, a space hung off the end of every line,
/// every blank line tripled and the final newline removed.
///
/// Doubling is a monotone map on indentation, so the `Indent` and `Dedent`
/// the layout pass produces are exactly the ones it produced before and the
/// file still parses — which is all this needs. It does *not* preserve the
/// value of a block text literal, and it does not have to: what is asserted
/// below is that the server's edits rebuild `zdc fmt`'s answer for **this**
/// text, not that this text means what the example meant. `zdc-cli`'s
/// `fmt_examples` is where the formatter is held to preserving the program.
fn mangled(src: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in src.split('\n') {
        if line.trim().is_empty() {
            out.push(String::new());
            out.push(String::new());
            out.push(String::new());
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        out.push(format!("{}{} ", " ".repeat(indent * 2), line.trim_end()));
    }
    let mut text = out.join("\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text
}

#[test]
fn a_canonical_document_is_answered_with_no_edits() {
    let canonical = "state count is client Whole starting 0\nview\n    Text count\n";
    assert_eq!(
        formatting(&Analysis::of(canonical)),
        Some(Vec::new()),
        "a file `zdc fmt --check` is silent about must be answered with nothing to do"
    );
}

#[test]
fn a_mangled_document_is_answered_with_the_edits_that_repair_it() {
    let source = "state count is client Whole starting 0   \n\n\nview\n  Text count\n";
    let edits = formatting(&Analysis::of(source)).expect("a readable source");

    assert_eq!(
        applied(source, &edits),
        "state count is client Whole starting 0\n\nview\n    Text count\n"
    );
    assert_eq!(
        applied(source, &edits),
        zdc_fmt::format(source).expect("a readable source"),
        "the editor and `zdc fmt` must lay one file out one way"
    );
}

/// A file the compiler will not read is not edited: not partially, not
/// hopefully, not at all.
#[test]
fn a_document_that_does_not_parse_is_answered_with_nothing() {
    for broken in [
        // A tab is not indentation, so this does not lex.
        "view\n\tColumn\n",
        // This lexes and does not parse: the comment is skipped, so the
        // layout pass reads the indentation below it as opening a block.
        "# a header\n        view\n            Column\n",
        // Half-typed, which is what a buffer usually is when a save fires.
        "state count is client Whole starting ",
    ] {
        assert_eq!(formatting(&Analysis::of(broken)), None, "{broken:?}");
    }
}

/// Every example, and a mangled copy of every example.
///
/// The examples are where the layout was read off in the first place, so
/// the server must have nothing to say about any of them; the mangled
/// copies are where the difference has real work to do, over real files
/// rather than over fixtures written by the same hand as the algorithm.
#[test]
fn every_example_is_canonical_and_a_mangled_copy_is_repaired_exactly() {
    let files = examples();
    assert!(
        files.len() > 20,
        "the scan found only {} examples, so it is not reading examples/",
        files.len()
    );

    for path in &files {
        let name = path.display();
        let src = std::fs::read_to_string(path).expect("a readable example");

        assert_eq!(
            formatting(&Analysis::of(&src)),
            Some(Vec::new()),
            "{name} is not in the canonical layout, so the server offered edits"
        );

        let mangled = mangled(&src);
        let edits = formatting(&Analysis::of(&mangled))
            .unwrap_or_else(|| panic!("{name} stopped being readable when it was mangled"));
        assert!(!edits.is_empty(), "{name}: mangling it changed nothing");
        assert_eq!(
            applied(&mangled, &edits),
            zdc_fmt::format(&mangled).expect("a readable source"),
            "{name}: the edits rebuilt a different document"
        );
    }
}
