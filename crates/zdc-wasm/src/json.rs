//! Just enough JSON to describe one compilation.
//!
//! Written rather than depended on, because the alternative is `serde` and
//! `serde_json` — two crates, a derive macro and a build of `syn` — to
//! print an object with six keys whose shapes this module already knows.
//! The workspace's only other JSON producer, `zdc-codegen`'s `js.rs`, made
//! the same call for the same reason.
//!
//! Only the encoder exists. Nothing here parses JSON: the answer travels
//! one way, and the host's `JSON.parse` is the other end.

/// A JSON string literal, quotes included.
///
/// Escaping is by the specification's list and then by codepoint, which is
/// what makes this safe to hand a compiler diagnostic. A diagnostic is
/// rendered text containing box-drawing characters, tabs, and the user's
/// own program spliced into it — so it can contain anything a `<textarea>`
/// can hold, and a quote or a backslash that escaped the encoder would
/// truncate the whole answer at the host's `JSON.parse`.
///
/// Control characters below 0x20 must be escaped or the JSON is invalid;
/// `\u00XX` covers every one of them that has no shorter spelling. Above
/// 0x1f nothing needs escaping except the quote and the backslash — UTF-8
/// travels as itself, because the host decodes these bytes as UTF-8 before
/// parsing them.
pub fn string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            control if control < '\u{20}' => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// An object from already-encoded values: `{"key":value,...}`.
///
/// The values arrive encoded rather than as strings, so the same helper
/// builds `{"ok":true}` and `{"client_js":"..."}` without a second
/// function and without a way to forget to encode one of them.
pub fn object(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(key, value)| format!("{}:{}", string(key), value))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// An array from already-encoded values.
pub fn array(values: &[String]) -> String {
    format!("[{}]", values.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The characters that would truncate the answer if they got through.
    ///
    /// A diagnostic carries the user's source text, so every one of these
    /// is reachable from a `<textarea>`.
    #[test]
    fn a_string_survives_what_a_diagnostic_contains() {
        assert_eq!(string(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(string(r"C:\path"), r#""C:\\path""#);
        assert_eq!(string("line\nnext"), r#""line\nnext""#);
        assert_eq!(string("a\tb"), r#""a\tb""#);
        // The caret row of every rendered diagnostic is box drawing, and
        // it must arrive as itself rather than as an escape.
        assert_eq!(string("╭─▶"), "\"╭─▶\"");
        // A control character with no short spelling still has to go.
        assert_eq!(string("\u{1}"), r#""\u0001""#);
    }

    #[test]
    fn an_object_nests_encoded_values() {
        let inner = object(&[("ok", "true".to_string())]);
        assert_eq!(
            object(&[("a", string("x")), ("b", inner), ("c", array(&[]))]),
            r#"{"a":"x","b":{"ok":true},"c":[]}"#
        );
    }
}
