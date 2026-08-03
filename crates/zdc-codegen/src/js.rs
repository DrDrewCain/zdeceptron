//! Printing primitives: string escapes, number literals, and precedence.
//!
//! Emitted JavaScript is a compilation target, not source code (spec §14A),
//! so nothing here optimises for how the output reads. What it does optimise
//! for is being *unambiguous*: an escape that is merely usually right, or a
//! number that round-trips through a different value, is a miscompile that no
//! test in the source language can see.

/// A JavaScript string literal, single-quoted.
///
/// U+2028 and U+2029 are escaped because they terminate a line in
/// JavaScript source even inside a string literal, which would end the
/// literal in the middle of the program.
pub fn string(value: &str) -> String {
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
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// A JSON string. **Not** [`string`]: JSON has no single-quoted form, and
/// `manifest.json` is read by `JSON.parse` rather than by an evaluator.
pub fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
pub fn html_attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;")
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
        assert_eq!(string("plain"), "'plain'");
        assert_eq!(string("it's"), "'it\\'s'");
        assert_eq!(string("a\\b"), "'a\\\\b'");
        assert_eq!(string("a\nb"), "'a\\nb'");
        assert_eq!(string("a\u{2028}b"), "'a\\u2028b'");
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
