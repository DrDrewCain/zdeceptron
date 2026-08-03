//! Printing primitives: string escapes, number literals, and precedence.
//!
//! Emitted JavaScript is a compilation target, not source code (spec §14A),
//! so nothing here optimises for how the output reads. What it does optimise
//! for is being *unambiguous*: an escape that is merely usually right, or a
//! number that round-trips through a different value, is a miscompile that no
//! test in the source language can see.

/// A finished JavaScript string literal, quotes included.
///
/// The field is private and this module is the only one that can build
/// one, so the *only* way a `Quoted` comes into existence is through
/// [`string`], which escapes. That is the point: an emission site that
/// wants a string literal has to hold one of these, and a site that
/// interpolates a raw `&str` between two apostrophes no longer type-checks
/// — which is what three separate injection holes (the `import` clause,
/// the generated `class` getter, and the folded stylesheet) all had in
/// common.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quoted(String);

impl Quoted {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Quoted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A JavaScript string literal, single-quoted.
///
/// U+2028 and U+2029 are escaped because they terminate a line in
/// JavaScript source even inside a string literal, which would end the
/// literal in the middle of the program. The C0 controls are escaped
/// because a `.zd` one-line literal is `"[^"\n]*"` and admits every one of
/// them but the newline, and a raw U+001B inside emitted source is an ANSI
/// escape for whatever later reads the file. The newline is escaped for
/// the same reason and is no longer unreachable: a `"""` block literal is
/// made of them.
pub fn string(value: &str) -> Quoted {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('\'');
    Quoted(out)
}

/// A JavaScript string literal for a JSON document, double-quoted.
///
/// JSON is not JavaScript: `\'` is not an escape there, and every C0
/// control must be escaped rather than merely being unwise. `manifest.json`
/// is the one generated artefact that is parsed as JSON, and it used to
/// build its object by writing `"{name}"` around a value straight out of
/// the program.
pub fn json_string(value: &str) -> Quoted {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    Quoted(out)
}

/// One key of an object literal.
///
/// A ZDeceptron identifier is UAX#31, so it is almost always a valid
/// JavaScript `IdentifierName` and can be written bare. Almost is not
/// always — `IdentifierName` admits `$` and `_` as starters and UAX#31
/// does not admit every character JavaScript's own table does — so a name
/// that is not provably bare is quoted. Quoting is never wrong; it is only
/// noisier, and the object built here is the one a foreign is handed, so
/// the property name reaching it must be exactly the declared one.
pub fn property(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if bare {
        name.to_string()
    } else {
        string(name).as_str().to_string()
    }
}

/// A JSON document, as a JavaScript expression — §17.4.8's inlining.
///
/// JSON is very nearly a subset of JavaScript expression syntax, and the two
/// places it is not are both handled here. An object literal at the start of
/// a statement parses as a block, so one is parenthesised. `U+2028` and
/// `U+2029` are legal unescaped in JSON and were illegal in a JavaScript
/// string literal before ES2019, so they are escaped rather than trusted to
/// the host's vintage.
pub fn literal(json: &str) -> String {
    let escaped = json
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    if escaped.starts_with('{') {
        return format!("({escaped})");
    }
    escaped
}

/// A numeric literal that parses back to exactly this `f64`.
///
/// `Whole` and `Decimal` are both f64 (spec §14A.3), so there is one
/// printer. A negative literal is parenthesised: `- -1` is a syntax error
/// and `--1` is a decrement, so neither can be produced by accident.
pub fn number(value: f64) -> String {
    if !value.is_finite() {
        // A source literal cannot be non-finite, but a future constant
        // folder can produce one, and `Infinity` and `NaN` are shadowable
        // global identifiers rather than literals (spec §16.7).
        return match (value.is_nan(), value.is_sign_negative()) {
            (true, _) => "(0/0)".to_string(),
            (false, false) => "(1/0)".to_string(),
            (false, true) => "(-1/0)".to_string(),
        };
    }
    let text = plain_number(value);
    if value.is_sign_negative() && value != 0.0 {
        format!("({text})")
    } else {
        text
    }
}

/// What `String(n)` produces in JavaScript, for baking a numeric literal
/// into template markup.
pub fn number_to_text(value: f64) -> String {
    plain_number(value)
}

fn plain_number(value: f64) -> String {
    if value == 0.0 {
        // Both zeroes print as "0" in JavaScript.
        return "0".to_string();
    }
    if value.abs() >= 1e21 {
        // JavaScript switches to exponential form here; Rust does not.
        let exponential = format!("{value:e}");
        return match exponential.split_once('e') {
            Some((mantissa, exponent)) if !exponent.starts_with('-') => {
                format!("{mantissa}e+{exponent}")
            }
            _ => exponential,
        };
    }
    // Rust's `f64` Display is shortest-round-trip, as JavaScript's is.
    format!("{value}")
}

/// Escape a compile-time literal for text position inside template markup.
pub fn html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape a compile-time literal for a double-quoted attribute value.
///
/// `<` is escaped even though it does not end an attribute value, and the
/// reason is not the HTML parser. The markup this builds is a *string
/// inside `client.js`*, and `</script` inside a script element ends that
/// element wherever it appears — the tokeniser scanning script data does
/// not know it is inside an attribute, or inside a JavaScript string, or
/// inside anything. Today `client.js` is its own module file and is never
/// inlined, so nothing is exploitable; but that is a property of the page
/// shell rather than of this function, and a literal that is safe only
/// because of a decision made in another module is the shape of defect
/// this layer exists to remove. Escaping it costs one entity.
pub fn html_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// JavaScript operator precedence, high binds tighter.
///
/// Only the levels this compiler can emit. Parenthesising by table rather
/// than parenthesising everything is what keeps `count() * 2` from becoming
/// `((count()) * (2))`.
pub mod precedence {
    pub const OR: u8 = 3;
    pub const AND: u8 = 4;
    pub const EQUALITY: u8 = 8;
    pub const RELATIONAL: u8 = 9;
    pub const ADDITIVE: u8 = 11;
    pub const MULTIPLICATIVE: u8 = 12;
    pub const UNARY: u8 = 14;
    pub const MEMBER: u8 = 17;
    pub const PRIMARY: u8 = 18;
}

/// A JavaScript expression that knows how tightly it binds.
#[derive(Debug, Clone)]
pub struct Expr {
    pub text: String,
    pub precedence: u8,
}

impl Expr {
    pub fn new(text: impl Into<String>, precedence: u8) -> Expr {
        Expr {
            text: text.into(),
            precedence,
        }
    }

    pub fn primary(text: impl Into<String>) -> Expr {
        Expr::new(text, precedence::PRIMARY)
    }

    /// This expression as an operand of something binding at `needed`,
    /// parenthesised only where that changes the parse.
    pub fn operand(&self, needed: u8) -> String {
        if self.precedence < needed {
            format!("({})", self.text)
        } else {
            self.text.clone()
        }
    }

    pub fn into_text(self) -> String {
        self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_escape_what_would_end_the_literal() {
        assert_eq!(string("plain").as_str(), "'plain'");
        assert_eq!(string("it's").as_str(), "'it\\'s'");
        assert_eq!(string("a\\b").as_str(), "'a\\\\b'");
        assert_eq!(string("a\nb").as_str(), "'a\\nb'");
        assert_eq!(string("a\u{2028}b").as_str(), "'a\\u2028b'");
    }

    /// A `.zd` string literal is `"[^"\n]*"`, which admits every C0
    /// control except the newline. None of them may reach emitted source
    /// raw: U+001B is an ANSI escape for anything that later cats the file.
    #[test]
    fn strings_escape_the_control_characters_a_zd_literal_admits() {
        assert_eq!(string("a\u{1b}[31mb").as_str(), "'a\\u001b[31mb'");
        assert_eq!(string("a\u{0}b").as_str(), "'a\\u0000b'");
        assert_eq!(string("a\u{7}b").as_str(), "'a\\u0007b'");
    }

    #[test]
    fn json_strings_use_the_escapes_json_actually_has() {
        assert_eq!(json_string("plain").as_str(), "\"plain\"");
        assert_eq!(json_string("a\"b").as_str(), "\"a\\\"b\"");
        assert_eq!(json_string("a\\b").as_str(), "\"a\\\\b\"");
        assert_eq!(json_string("a\u{1b}b").as_str(), "\"a\\u001bb\"");
        assert_eq!(
            json_string("it's").as_str(),
            "\"it's\"",
            "`\\'` is not a JSON escape"
        );
    }

    #[test]
    fn numbers_print_as_javascript_would() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(2.0), "2");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(-1.0), "(-1)");
        assert_eq!(number_to_text(2.0), "2");
        assert_eq!(number_to_text(-0.0), "0", "both zeroes render as 0");
    }

    #[test]
    fn very_large_numbers_use_the_exponential_form_javascript_uses() {
        assert_eq!(number(1e21), "1e+21");
    }

    #[test]
    fn non_finite_values_avoid_shadowable_global_identifiers() {
        assert_eq!(number(f64::INFINITY), "(1/0)");
        assert_eq!(number(f64::NEG_INFINITY), "(-1/0)");
        assert_eq!(number(f64::NAN), "(0/0)");
    }

    #[test]
    fn markup_escapes_differ_by_position() {
        assert_eq!(html_text("a & b < c"), "a &amp; b &lt; c");
        assert_eq!(html_attribute("a \" b & c"), "a &quot; b &amp; c");
        assert_eq!(
            html_attribute("a > b"),
            "a > b",
            "a bare > does not end an attribute value"
        );
        assert_eq!(
            html_attribute("</script>"),
            "&lt;/script>",
            "`</script` ends a script element from inside an attribute too"
        );
    }

    #[test]
    fn operands_are_parenthesised_only_where_the_parse_would_change() {
        let sum = Expr::new("a + b", precedence::ADDITIVE);
        assert_eq!(sum.operand(precedence::MULTIPLICATIVE), "(a + b)");
        assert_eq!(sum.operand(precedence::ADDITIVE), "a + b");
        assert_eq!(
            Expr::primary("count()").operand(precedence::MEMBER),
            "count()"
        );
    }
}
