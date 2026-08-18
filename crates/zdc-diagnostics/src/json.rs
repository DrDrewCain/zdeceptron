//! The machine-readable form of a diagnostic.
//!
//! Everything else in this crate renders for a person: `ariadne` draws a
//! caret under a source line and colours it. Nothing could read that back.
//! An editor that is not the language server, a CI job that wants to turn
//! a rejection into a file annotation, and any script that wants to *count*
//! errors all need the same thing, and all of them were reduced to parsing
//! box-drawing characters.
//!
//! # The format is line-delimited, one diagnostic per line
//!
//! Not one document containing an array. Three reasons, in order of how
//! much they matter:
//!
//! * **A run that dies half way through still emitted valid output.** The
//!   compiler prints each finding as it reaches it. A single document has
//!   to be closed to be parseable, so a compiler killed by a timeout — or
//!   by the operating system, which is when a consumer most wants to know
//!   what it had found — would produce a file no parser accepts. Every
//!   complete line of this format is a complete diagnostic.
//! * **It streams.** A consumer reads a line and acts on it, rather than
//!   buffering until the compiler exits.
//! * **The line-oriented tools work.** `wc -l` counts diagnostics, `grep`
//!   filters them, and `jq -c` reads the stream without `--slurp`.
//!
//! The cost is that the output is not itself a JSON value, which a
//! consumer that wanted `JSON.parse(whole_file)` has to know. That is why
//! it is written down here and in `--help`, rather than left to be
//! discovered.
//!
//! # The shape
//!
//! Every key is present on every line, with `null` where the compiler has
//! nothing — a consumer's `d.code` is then never a missing-property error,
//! only a null one. Keys are never removed, and a new key is only ever
//! added.
//!
//! ```json
//! {
//!   "level": "error",              // "error" | "warning"
//!   "code": "E-IFC-05",            // string | null
//!   "message": "…",                // string
//!   "path": "src/app.zd",          // string
//!   "span": {                      // object | null
//!     "start": 120,                //   byte offset, inclusive
//!     "end": 131,                  //   byte offset, exclusive
//!     "line": 7,                   //   1-based, of `start`
//!     "column": 5                  //   1-based, in characters
//!   },
//!   "label": "…",                  // string | null, what the caret says
//!   "notes": [                     // array, possibly empty
//!     {"span": {…}, "message": "…"}
//!   ],
//!   "help": "…",                   // string | null
//!   "suggestion": {                // object | null
//!     "span": {…},
//!     "replacement": "client "
//!   }
//! }
//! ```
//!
//! Byte offsets *and* line/column, because the two audiences want
//! different ones: a span is a byte range everywhere inside this compiler,
//! and every editor and CI annotation format outside it counts lines. A
//! consumer given only bytes has to re-derive the line, which means
//! re-reading the file and agreeing with the compiler about what a line
//! is.
//!
//! # The text is the same text the terminal form prints
//!
//! A `.zd` string literal admits U+001B, so a message can quote one, and
//! JSON output is read by machines but also `cat`ed by people. Every
//! string here goes through `crate::printable` before it is escaped, so
//! the control characters are gone rather than escaped as `\u001b` and
//! decoded back into a terminal by whatever prints them.

use zdc_lexer::Span;

use crate::{printable, Diagnostic};

/// One diagnostic as a single line of JSON, newline included.
pub fn line(src: &str, path: &str, diagnostic: &Diagnostic) -> String {
    let mut out = String::new();
    out.push('{');

    field(&mut out, "level", &string(diagnostic.level.as_str()));
    out.push(',');
    field(
        &mut out,
        "code",
        &match diagnostic.code {
            Some(code) => string(code),
            None => "null".to_string(),
        },
    );
    out.push(',');
    field(
        &mut out,
        "message",
        &string(&printable(&diagnostic.message)),
    );
    out.push(',');
    field(&mut out, "path", &string(&printable(path)));
    out.push(',');
    field(&mut out, "span", &span_of(src, diagnostic.span));
    out.push(',');
    field(
        &mut out,
        "label",
        &match &diagnostic.label {
            Some(label) => string(&printable(label)),
            None => "null".to_string(),
        },
    );
    out.push(',');

    let notes: Vec<String> = diagnostic
        .notes
        .iter()
        .map(|(at, message)| {
            let mut note = String::from("{");
            field(&mut note, "span", &span_of(src, Some(*at)));
            note.push(',');
            field(&mut note, "message", &string(&printable(message)));
            note.push('}');
            note
        })
        .collect();
    field(&mut out, "notes", &format!("[{}]", notes.join(",")));
    out.push(',');

    field(
        &mut out,
        "help",
        &match &diagnostic.help {
            Some(help) => string(&printable(help)),
            None => "null".to_string(),
        },
    );
    out.push(',');
    field(
        &mut out,
        "suggestion",
        &match &diagnostic.suggestion {
            Some(suggestion) => {
                let mut value = String::from("{");
                field(&mut value, "span", &span_of(src, Some(suggestion.span)));
                value.push(',');
                field(
                    &mut value,
                    "replacement",
                    &string(&printable(&suggestion.replacement)),
                );
                value.push('}');
                value
            }
            None => "null".to_string(),
        },
    );

    out.push('}');
    out.push('\n');
    out
}

/// `"name":value`, with the name escaped like any other string.
fn field(out: &mut String, name: &str, value: &str) {
    out.push_str(&string(name));
    out.push(':');
    out.push_str(value);
}

/// A span as bytes and as a place a person can be sent to.
///
/// The line and column are derived from `src` rather than carried, because
/// a span is a byte range everywhere inside the compiler and deriving them
/// twice is how the two answers come to disagree. A span that does not fall
/// inside `src` — which is what a spanless diagnostic rendered against an
/// empty source is — still reports its bytes, and reports line 1 column 1,
/// because refusing to say anything would lose the byte range too.
fn span_of(src: &str, span: Option<Span>) -> String {
    let Some(span) = span else {
        return "null".to_string();
    };
    let start = span.start as usize;
    let (line, column) = position(src, start);
    let mut out = String::from("{");
    field(&mut out, "start", &span.start.to_string());
    out.push(',');
    field(&mut out, "end", &span.end.to_string());
    out.push(',');
    field(&mut out, "line", &line.to_string());
    out.push(',');
    field(&mut out, "column", &column.to_string());
    out.push('}');
    out
}

/// The 1-based line and column of a byte offset.
///
/// The column counts *characters*, not bytes, because that is what every
/// consumer of a line and column means by one — an editor placing a cursor
/// counts characters, and a byte column would sit in the middle of an em
/// dash. The byte offset is still in the same object for anyone who wants
/// the other answer.
fn position(src: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(src.len());
    // Not a char boundary means the span did not come from this text; the
    // largest boundary at or below it is the honest reading.
    let mut at = offset;
    while at > 0 && !src.is_char_boundary(at) {
        at -= 1;
    }
    let before = &src[..at];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let column = src[line_start..at].chars().count() + 1;
    (line, column)
}

/// A Rust string as a JSON string, quotes included.
///
/// Written here rather than taken from a JSON crate: this crate is
/// published, `zdc-diagnostics` is on the path of every other crate in the
/// workspace, and one function of escaping is a smaller thing to own than
/// a dependency edge. The escapes are RFC 8259's: the two mandatory ones,
/// the five short forms it names, and `\u00XX` for every other control
/// character.
fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
