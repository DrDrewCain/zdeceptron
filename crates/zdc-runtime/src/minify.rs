// The published site documents private items — `docs.yml` passes
// `--document-private-items`, and the decision is argued there: nineteen
// of the twenty-one crates are compiler internals, and documenting only
// the public surface would drop more than half the prose. So a link from
// this module's docs to a private helper *does* resolve on the site, and
// the lint that fires here is answering a question this workspace has
// already decided differently.
#![allow(rustdoc::private_intra_doc_links)]

//! Minification: what a reader downloads, minus what only a reader of the
//! *source* needs — issue #135.
//!
//! # What this does, and the whole of what it does
//!
//! Comments and redundant whitespace, in JavaScript and in CSS. Nothing
//! else. **No identifier is renamed, no expression is rewritten, no
//! statement is removed, and no token is ever joined to its neighbour.**
//! Whitespace between two tokens becomes one space; whitespace at the
//! start or end of a line becomes nothing; a blank line becomes nothing;
//! a comment becomes the whitespace it was surrounded by.
//!
//! That is a deliberately small definition of the word "minify", and the
//! rest of this comment is the argument for it, because the obvious
//! objection — "a real minifier also mangles names" — is right about what
//! other tools do and wrong about what this one should.
//!
//! # Why not mangle
//!
//! Renaming identifiers requires a real JavaScript parser and a correct
//! scope analysis: `var` hoisting, closures, `catch` bindings, labels,
//! `with`, getters named the same as locals, and — the one that decides
//! it — every property name that must *not* be renamed even though it is
//! spelled like an identifier. The runtime crosses that boundary
//! constantly: `wire.js` reads `value.constructor`, `store.js` builds
//! objects whose keys are the durable key names a program declared, and
//! `dom.js` sets DOM properties by name.
//!
//! A renamer that gets one of those wrong emits a bundle that **parses
//! and misbehaves**. That is the failure mode with no gate in front of
//! it: `crates/zdc-bench` would report a smaller number, every size
//! assertion would pass, and the defect would surface as a blank page in
//! someone's browser. The measured saving from the safe subset is around
//! 70% of the runtime — `a_release_build_is_minified_and_this_is_what_it_saves`
//! pins it, and `BENCHMARKS.md` reports it per file — so mangling would be
//! trading a hard-to-test correctness risk for the remainder of a number
//! that is already most of the way down.
//!
//! **So mangling is out of scope, on purpose, and this is the record of
//! that decision rather than an omission.** If it is ever wanted, the
//! thing to add first is a JavaScript parser — not a regex that renames
//! what looks like a local.
//!
//! # Why not shell out to `esbuild` or `terser`
//!
//! Because `zdc` is one binary. `crates/zdc-codegen/src/evaluate.rs` makes
//! the argument at length: needing Node to build ZDeceptron would be the
//! first crack in the claim that a developer installs one binary and
//! nothing else. A minifier that ran as a subprocess would put that crack
//! in `zdc build` itself, which is the one command every user runs.
//!
//! # What is not minified, and why
//!
//! * **`index.html`.** Whitespace between elements is *content* — it is
//!   the space between two inline elements a browser renders — so a
//!   whitespace pass over HTML can change what a page looks like. The
//!   emitted document is a `<head>`, a `<div id=app>` and two tags; the
//!   indentation in it is about thirty bytes, and thirty bytes is not
//!   worth a rule with an exception in it.
//! * **`manifest.json`.** Already emitted without a space in it.
//! * **Server functions.** A reader never downloads one. The bytes cost
//!   nobody a page load, and an operator reading a deployed function is
//!   the one person the formatting is for.
//!
//! # The one heuristic
//!
//! Telling a regular-expression literal from a division needs to know
//! what the previous token was, and that is the single place this scanner
//! guesses rather than knows — the guess is stated at [`starts_a_regex`].
//! What backs it is that the minified runtime is *executed*:
//! `crates/zdc-runtime/tests/render.rs` runs the release build of every
//! module through the same suite as the development build, and
//! `crates/zdc-cli/tests/browser.rs` runs a built program in a real
//! browser. A scanner that mis-read a `/` would not produce a smaller
//! file; it would produce one that throws.

/// Where the scanner is inside a template literal.
///
/// A template is the one JavaScript construct whose *whitespace is
/// content* and whose contents can contain arbitrary code, so it cannot
/// be skipped whole and it cannot be treated as ordinary text. The stack
/// is what lets `` `a${`b ${c} d`}e` `` scan correctly: each frame knows
/// which half of the construct it is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// Inside the literal text. Every byte is copied exactly.
    Text,
    /// Inside a `${…}`, which is ordinary code. The count is how many
    /// `{` have been opened since, so that the `}` closing an object
    /// literal is not mistaken for the one closing the substitution.
    Substitution(usize),
}

/// Whitespace seen since the last code byte and not yet written.
///
/// `Newline` outranks `Space`: a line break that survives is what keeps
/// automatic semicolon insertion deciding what it decided in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Gap {
    None,
    Space,
    Newline,
}

/// The keywords a `/` may follow and still open a regular expression.
///
/// `return /x/` is a regex; `total /x/` is two divisions. Nothing but the
/// word before the slash tells them apart.
const REGEX_KEYWORDS: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "do",
    "else",
    "case",
    "yield",
    "await",
    "throw",
];

/// The length in bytes of the character starting at `at`.
///
/// Every copy out of the source goes through this rather than taking one
/// byte, because slicing a `&str` in the middle of a multi-byte character
/// panics — and a template literal or a stylesheet may hold any character
/// at all.
fn char_len(source: &str, at: usize) -> usize {
    source[at..].chars().next().map(char::len_utf8).unwrap_or(1)
}

/// Whether a byte can appear inside an identifier.
///
/// Bytes above 126 are included because a JavaScript identifier may
/// contain any UTF-8 letter, and every byte of a multi-byte sequence is
/// above 126 — so treating them as identifier bytes both keeps the
/// identifier together and makes it impossible to split a character.
fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'\\') || byte > 126
}

/// Whether the `/` at this point opens a regular expression rather than
/// being a division.
///
/// **This is the guess.** It is decided from the previous code byte and,
/// when that byte is part of an identifier, from the identifier itself.
/// The rule is the one every JavaScript tokeniser uses, with two cases
/// pinned down deliberately:
///
/// * After `)` and `]` this answers *division*. `(a + b) / 2` and
///   `xs[0] / 2` are ordinary arithmetic; `if (x) /re/.test(y)` is legal
///   and is not written anywhere in this repository.
/// * After `++` and `--` it answers *division* as well, which is why the
///   byte before the previous one is passed in at all: `+` on its own
///   can precede a regex, and `i++ / n` must not be read as one.
fn starts_a_regex(previous: u8, before: u8, word: &str) -> bool {
    if is_word(previous) {
        return REGEX_KEYWORDS.contains(&word);
    }
    match previous {
        // Nothing before it: the file opens with a regex.
        0 => true,
        b'+' | b'-' => previous != before,
        b'(' | b',' | b'=' | b':' | b'[' | b'!' | b'&' | b'|' | b'?' | b'{' | b'}' | b';'
        | b'*' | b'%' | b'<' | b'>' | b'~' | b'^' => true,
        _ => false,
    }
}

/// Minify JavaScript: comments and redundant whitespace, nothing else.
///
/// The output is the input with characters removed — never reordered,
/// never rewritten, never added beyond collapsing a run of whitespace to
/// one space. `minifying_only_ever_removes_characters` is that property
/// as a test.
pub fn javascript(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut stack: Vec<Frame> = Vec::new();
    let mut gap = Gap::None;
    // The last two code bytes written, and the identifier that ended at
    // the last one, if it was one. All three exist for `starts_a_regex`.
    let mut previous = 0u8;
    let mut before = 0u8;
    let mut word = String::new();
    let mut i = 0;

    while i < bytes.len() {
        // Inside a template's text, whitespace is content and there is no
        // comment syntax, so this arm copies and the one below decides.
        if stack.last() == Some(&Frame::Text) {
            let byte = bytes[i];
            match byte {
                b'\\' if i + 1 < bytes.len() => {
                    let end = i + 1 + char_len(source, i + 1);
                    out.push_str(&source[i..end]);
                    i = end;
                }
                b'`' => {
                    out.push('`');
                    stack.pop();
                    previous = b'`';
                    i += 1;
                }
                b'$' if bytes.get(i + 1) == Some(&b'{') => {
                    out.push_str("${");
                    stack.push(Frame::Substitution(0));
                    before = b'$';
                    previous = b'{';
                    word.clear();
                    i += 2;
                }
                _ => {
                    let end = i + char_len(source, i);
                    out.push_str(&source[i..end]);
                    i = end;
                }
            }
            continue;
        }

        let byte = bytes[i];

        // --- whitespace and comments: the only things ever dropped -----
        if byte == b' ' || byte == b'\t' {
            gap = gap.max(Gap::Space);
            i += 1;
            continue;
        }
        if byte == b'\n' || byte == b'\r' {
            gap = Gap::Newline;
            i += 1;
            continue;
        }
        if byte == b'/' && bytes.get(i + 1) == Some(&b'/') {
            // To the line break, which is left for the arm above to see:
            // dropping it here would join two statements.
            i = match source[i..].find('\n') {
                Some(offset) => i + offset,
                None => bytes.len(),
            };
            continue;
        }
        if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let end = match source[i + 2..].find("*/") {
                Some(offset) => i + 2 + offset + 2,
                None => bytes.len(),
            };
            // A block comment containing a line break *is* a line break
            // as far as automatic semicolon insertion is concerned, so
            // replacing one with nothing could change what the program
            // means. It becomes the newline it stood for.
            gap = if source[i..end].contains('\n') {
                Gap::Newline
            } else {
                gap.max(Gap::Space)
            };
            i = end;
            continue;
        }

        // --- everything below is code, so the gap is settled first -----
        match gap {
            Gap::None => {}
            Gap::Space => {
                if !out.is_empty() {
                    out.push(' ');
                }
            }
            Gap::Newline => {
                if !out.is_empty() {
                    out.push('\n');
                }
            }
        }
        gap = Gap::None;

        // --- literals, copied exactly ---------------------------------
        if byte == b'"' || byte == b'\'' {
            let end = string_end(bytes, i, byte);
            out.push_str(&source[i..end]);
            before = previous;
            previous = byte;
            word.clear();
            i = end;
            continue;
        }
        if byte == b'`' {
            out.push('`');
            stack.push(Frame::Text);
            before = previous;
            previous = b'`';
            word.clear();
            i += 1;
            continue;
        }
        if byte == b'/' && starts_a_regex(previous, before, &word) {
            let end = regex_end(bytes, i);
            out.push_str(&source[i..end]);
            before = previous;
            previous = b'/';
            word.clear();
            i = end;
            continue;
        }

        // --- an identifier, kept whole so a keyword can be recognised --
        if is_word(byte) && !byte.is_ascii_digit() {
            let mut end = i;
            while end < bytes.len() && is_word(bytes[end]) {
                end += 1;
            }
            out.push_str(&source[i..end]);
            word.clear();
            word.push_str(&source[i..end]);
            before = previous;
            previous = bytes[end - 1];
            i = end;
            continue;
        }

        // --- one ordinary byte, and the template stack it may move -----
        if let Some(Frame::Substitution(depth)) = stack.last().copied() {
            if byte == b'{' {
                stack.pop();
                stack.push(Frame::Substitution(depth + 1));
            } else if byte == b'}' {
                stack.pop();
                if depth > 0 {
                    stack.push(Frame::Substitution(depth - 1));
                }
            }
        }
        let end = i + char_len(source, i);
        out.push_str(&source[i..end]);
        before = previous;
        previous = byte;
        word.clear();
        i = end;
    }

    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// One past the closing quote of the string literal starting at `open`.
///
/// An unterminated literal ends at the end of the input rather than
/// panicking: this is a text transformation, and a file that is not
/// JavaScript should come out looking like what went in rather than
/// stopping a build with a message about a scanner.
fn string_end(bytes: &[u8], open: usize, quote: u8) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            byte if byte == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// One past the last flag of the regular-expression literal at `open`.
///
/// The character class is the part a simpler scan gets wrong: `/[/]/` is
/// one literal, and a scan that stopped at the first unescaped `/` would
/// end it in the middle. `dom.js` contains exactly that shape.
fn regex_end(bytes: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    let mut in_class = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'[' => {
                in_class = true;
                i += 1;
            }
            b']' => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => {
                i += 1;
                break;
            }
            _ => i += 1,
        }
    }
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    // A trailing backslash can step the cursor past the end. Clamped
    // rather than left to the caller, because the caller slices with it.
    i.min(bytes.len())
}

/// Minify CSS: comments and redundant whitespace, nothing else.
///
/// Simpler than the JavaScript above in one way and stricter in another.
/// Simpler: CSS has no automatic semicolon insertion, so every line break
/// is redundant and the result is one line. Stricter in what is *not*
/// done — the space in `.a .b` is a descendant combinator and the space
/// in `a :hover` makes it a different selector from `a:hover`, so no
/// space is ever deleted outright, only collapsed to one.
pub fn css(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut gap = false;
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if byte.is_ascii_whitespace() {
            gap = true;
            i += 1;
            continue;
        }
        if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
            // CSS has no line comment, so this is the only form.
            i = match source[i + 2..].find("*/") {
                Some(offset) => i + 2 + offset + 2,
                None => bytes.len(),
            };
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push(' ');
        }
        gap = false;
        if byte == b'"' || byte == b'\'' {
            let end = string_end(bytes, i, byte);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }
        out.push_str(&source[i..i + char_len(source, i)]);
        i += 1;
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_go_and_the_code_around_them_stays() {
        assert_eq!(
            javascript("// a note\nconst x = 1; // trailing\n"),
            "const x = 1;\n"
        );
        assert_eq!(javascript("const /* inline */ x = 1;\n"), "const x = 1;\n");
    }

    /// A block comment with a line break in it *is* a line break to
    /// automatic semicolon insertion, so removing it outright can change
    /// what the program means.
    #[test]
    fn a_block_comment_spanning_lines_leaves_the_line_break_behind() {
        assert_eq!(javascript("a = b\n/* one\ntwo */\n(c)\n"), "a = b\n(c)\n");
    }

    /// The property that makes this a whitespace pass and not a rewrite:
    /// no two tokens are ever joined, so `let x` cannot become `letx`.
    #[test]
    fn tokens_are_never_joined() {
        assert_eq!(javascript("let    x   =   1;\n"), "let x = 1;\n");
        assert_eq!(javascript("return    x;\n"), "return x;\n");
        assert_eq!(javascript("a\n\n\n\nb\n"), "a\nb\n");
    }

    /// A comment is only a comment outside a literal.
    #[test]
    fn a_slash_inside_a_literal_is_not_a_comment() {
        for source in [
            "const u = 'https://example.com/x'; // gone\n",
            "const u = \"https://example.com/x\"; // gone\n",
            "const u = `https://example.com/x`; // gone\n",
        ] {
            let out = javascript(source);
            assert!(
                out.contains("https://example.com/x"),
                "the URL was eaten as a comment: {out}"
            );
            assert!(!out.contains("gone"), "the comment survived: {out}");
        }
    }

    /// Whitespace inside a template literal is content, not formatting.
    #[test]
    fn a_template_literal_keeps_its_own_whitespace() {
        assert_eq!(
            javascript("const s = `a  b\n  c`;   // note\n"),
            "const s = `a  b\n  c`;\n"
        );
    }

    /// The stack, exercised: code inside `${…}` is minified, the text
    /// around it is not, and a nested template does not end the outer one.
    #[test]
    fn a_substitution_is_code_and_the_text_around_it_is_not() {
        assert_eq!(javascript("`a ${  x  +  1  } b`;\n"), "`a ${ x + 1 } b`;\n");
        assert_eq!(
            javascript("`a${ { k: 1 }.k }b`; // note\n"),
            "`a${ { k: 1 }.k }b`;\n"
        );
        assert_eq!(javascript("`a${`b  c`}d`;\n"), "`a${`b  c`}d`;\n");
    }

    /// The one guess, and the shape `dom.js` actually contains.
    #[test]
    fn a_regex_literal_survives_and_a_division_is_not_mistaken_for_one() {
        assert_eq!(
            javascript("if (/[/?#]/.test(s)) return s; // note\n"),
            "if (/[/?#]/.test(s)) return s;\n"
        );
        assert_eq!(
            javascript("const r = a / b / c; // note\n"),
            "const r = a / b / c;\n"
        );
        assert_eq!(
            javascript("return /a\\/b/g.test(s);\n"),
            "return /a\\/b/g.test(s);\n"
        );
        // `i++ / n` is a division; `+` on its own can precede a regex.
        assert_eq!(javascript("const q = i++ / n;\n"), "const q = i++ / n;\n");
    }

    /// Running it twice changes nothing, which is what makes the output a
    /// fixed point rather than a stage that keeps eating.
    #[test]
    fn minifying_is_idempotent() {
        let sources = [
            "// note\nconst x = 1;\nfunction f(a) {\n  return a / 2;\n}\n",
            "const s = `a ${ b } c`;\n",
            "/* head */\nexport function g() { return /x/.test('a//b'); }\n",
        ];
        for source in sources {
            let once = javascript(source);
            assert_eq!(javascript(&once), once, "not a fixed point: {source}");
        }
    }

    /// Minification deletes; it never adds, reorders, or rewrites.
    ///
    /// Checked against the runtime itself rather than a fixture, because
    /// the runtime is what it is run on: with the whitespace taken out of
    /// both, the minified module is a subsequence of the source. A
    /// scanner that mis-read a `/` and swallowed a token would still pass
    /// this — that is what executing the result in `tests/render.rs` is
    /// for — but a scanner that *corrupted* what it copied could not.
    ///
    /// It is the same shape of claim `stripping_only_ever_removes_whole_lines`
    /// makes about #140, one level down: that one is about lines, this one
    /// is about characters.
    #[test]
    fn minifying_only_ever_removes_characters() {
        let mut compared = 0;
        for (name, source) in crate::MODULES {
            let minified = javascript(source);
            let mut written = source.chars().filter(|c| !c.is_whitespace());
            for character in minified.chars().filter(|c| !c.is_whitespace()) {
                assert!(
                    written.any(|source_character| source_character == character),
                    "{name}: minifying produced a `{character}` the source does not have, \
                     in that order — the scanner is rewriting rather than removing"
                );
                compared += 1;
            }
        }
        assert!(
            compared > 20_000,
            "only {compared} characters compared across {} modules; this has \
             stopped surveying the runtime",
            crate::MODULES.len()
        );
    }

    /// Malformed input comes back out. It does not panic, and the text
    /// does not silently vanish.
    ///
    /// This is a text transformation, not a parser, and a file that is not
    /// JavaScript is not this function's error to report — but eating it
    /// would be this function's bug.
    #[test]
    fn an_unterminated_literal_neither_panics_nor_disappears() {
        assert!(javascript("const s = 'no end\n").contains("'no end"));
        assert!(javascript("const t = `no end\n").contains("`no end"));
        assert!(javascript("const r = /no end\\").contains("/no end"));
        // A comment is the one thing that *should* vanish, terminated or not.
        assert_eq!(javascript("/* no end"), "");
        assert_eq!(css("/* no end"), "");
    }

    #[test]
    fn css_loses_its_comments_and_its_line_breaks() {
        assert_eq!(
            css("/* why */\n.a {\n  color: red;\n}\n"),
            ".a { color: red; }\n"
        );
    }

    /// The space in a descendant selector is a combinator, not layout.
    #[test]
    fn css_keeps_the_space_that_means_something() {
        assert_eq!(css(".a   .b { x: 1 }\n"), ".a .b { x: 1 }\n");
        assert_eq!(
            css("a::after { content: '  ' }\n"),
            "a::after { content: '  ' }\n"
        );
    }

    #[test]
    fn empty_input_stays_empty() {
        assert_eq!(javascript(""), "");
        assert_eq!(javascript("\n\n  \n"), "");
        assert_eq!(css(""), "");
        assert_eq!(css("/* only a comment */"), "");
    }
}
