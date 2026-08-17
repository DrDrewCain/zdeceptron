//! A program compiles to the same bundle whichever way its lines end.
//!
//! #242. The lexer refused a carriage return, which was a defensible rule
//! about invisible characters and made Windows a platform the language did
//! not run on: Git there rewrites LF to CRLF on checkout, so a Windows
//! clone of this repository got a working `zdc` and a tree of `.zd` files
//! that same binary rejected — every example.
//!
//! Found by the first run of the release workflow. The Windows job built
//! the compiler, printed `zdc 0.1.1`, and failed its own smoke test on
//! `examples/hello.zd`.
//!
//! What is asserted here is stronger than "does not error". Indentation is
//! the block structure in this language, so a carriage return counted as
//! an indent column would not fail — it would silently reshape the
//! program. The emitted JavaScript has to be *identical*, and that is what
//! catches an off-by-one in the indent width.

mod support;

use support::compile_source;

/// The same program, twice.
fn both_ways(source: &str) -> (String, String) {
    let dos = source.replace('\n', "\r\n");
    (
        compile_source(source).client_js,
        compile_source(&dos).client_js,
    )
}

#[test]
fn indentation_is_the_same_width_with_either_line_ending() {
    // Nested blocks, because the failure this guards against is an indent
    // one column too wide — which only changes the answer where the
    // structure depends on the column.
    let (unix, dos) = both_ways(
        "state n is client Whole starting 0\n\
         \n\
         function step of value\n\
         \x20   if value > 3\n\
         \x20       give value\n\
         \x20   give step of (value + 1)\n\
         \n\
         state answer is client Whole from step of n\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       Text answer\n",
    );
    assert_eq!(
        unix, dos,
        "a CRLF program must emit exactly what its LF twin emits"
    );
}

/// A comment runs to the end of the line, and on Windows that line ends
/// with two bytes. If the carriage return were left outside the comment it
/// would reach the layout pass as a stray character.
#[test]
fn a_comment_ends_at_a_windows_line_ending() {
    let (unix, dos) = both_ways(
        "# a comment\n\
         state n is client Whole starting 1   # and a trailing one\n\
         \n\
         view\n\
         \x20   Text n\n",
    );
    assert_eq!(unix, dos);
}

/// A block literal spans lines, so every one of them carries the carriage
/// return — including the two delimiter lines, which are required to hold
/// nothing but spaces. Left in, the literal is refused rather than
/// mangled.
#[test]
fn a_block_literal_holds_the_same_text_either_way() {
    let (unix, dos) = both_ways(
        "state note is client Text starting \"\"\"\n\
         \x20   first line\n\
         \x20   second line\n\
         \x20   \"\"\"\n\
         \n\
         view\n\
         \x20   Text note\n",
    );
    assert_eq!(unix, dos);
    assert!(unix.contains("first line"), "{unix}");
    // And the carriage returns are not smuggled into the value.
    assert!(
        !dos.contains('\r'),
        "no carriage return may reach the output"
    );
}
