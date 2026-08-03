//! What a durable cell holds, and the one operation that has to look inside.

use std::fmt;

/// A durable value, carried as the JSON text the runtime exchanges.
///
/// The store is deliberately incurious about structure. A `durable` signal
/// may hold a `Whole`, a `Text`, a `List of T` or a record, and the store
/// implements five operations for all of them (§8 item 5); parsing a
/// record here would buy nothing and would put a second, weaker copy of
/// the type system in the persistence layer. JSON text is what
/// `JSON.stringify` produces on one side and `JSON.parse` consumes on the
/// other, so it is the narrowest thing that round-trips.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Json(String);

impl Json {
    /// Wrap text that is already JSON.
    ///
    /// Not validated: the only producers are `JSON.stringify` in the
    /// runtime and [`Number::to_json`] below, and a validator here would
    /// be a JSON parser written to reject nothing that ever arrives.
    pub fn from_text(text: impl Into<String>) -> Json {
        Json(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A number, in the one representation the language has.
///
/// §14A.3 settles this: `Whole` compiles to `f64`, because JavaScript has
/// only doubles and the language "must not claim 64-bit integers it does
/// not have". The store holds the same representation for the same reason
/// — an `incr` that accumulated in `i64` and was read back into a double
/// would disagree with the program above 2^53, which is precisely the
/// bound §14A.3 documents rather than hides.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Number(f64);

impl Number {
    pub fn new(value: f64) -> Number {
        Number(value)
    }

    pub fn as_f64(self) -> f64 {
        self.0
    }

    /// What an absent key counts as, so `incr` on a key never written is
    /// not an error. A `durable` signal always has a `starting` value, so
    /// the first `incr` before any `set` is ordinary rather than
    /// exceptional.
    pub const ZERO: Number = Number(0.0);

    /// Read a number back out of stored JSON.
    ///
    /// Returns `None` for text that is not a JSON number, which is how
    /// `incr` on a `Text` cell becomes a named error rather than a zero.
    pub fn parse(text: &str) -> Option<Number> {
        let trimmed = text.trim();
        // `f64::from_str` accepts `inf`, `NaN` and a leading `+`, none of
        // which are JSON numbers and all of which would survive a
        // round-trip through the store as invalid JSON.
        if trimmed.is_empty() {
            return None;
        }
        if !trimmed
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
        {
            return None;
        }
        match trimmed.parse::<f64>() {
            Ok(value) if value.is_finite() => Some(Number(value)),
            Ok(_) => None,
            Err(_) => None,
        }
    }

    /// Sum, or `None` if the result is not representable as JSON.
    ///
    /// Overflow to infinity is the only way this fails, and it fails
    /// loudly: writing `Infinity` would store text that `JSON.parse`
    /// rejects, turning one bad increment into a permanently unreadable
    /// key.
    ///
    /// Named `plus` rather than `add` because it is fallible and
    /// `std::ops::Add` is not — a method that looks like the trait but
    /// returns an `Option` is the ambiguity `clippy` names here.
    pub fn plus(self, delta: Number) -> Option<Number> {
        let sum = self.0 + delta.0;
        if sum.is_finite() {
            Some(Number(sum))
        } else {
            None
        }
    }

    /// Render as JSON, the way `JSON.stringify` renders a double.
    ///
    /// Rust's `Display` for `f64` already prints `1` for `1.0` and the
    /// shortest round-tripping form otherwise, which is what JavaScript
    /// does. The two disagree only in exponent formatting far outside
    /// 2^53 — both forms are valid JSON and both parse to the same
    /// double, so nothing downstream can observe the difference.
    pub fn to_json(self) -> Json {
        Json(self.0.to_string())
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_round_trips_without_growing_a_decimal_point() {
        // The emitted client compares rendered text, so `visits` reading
        // back as `1.0` would put "1.0" on screen where the program says
        // the value is 1.
        assert_eq!(Number::new(1.0).to_json().as_str(), "1");
        assert_eq!(Number::new(-42.0).to_json().as_str(), "-42");
    }

    #[test]
    fn a_decimal_keeps_its_fraction() {
        assert_eq!(Number::new(0.5).to_json().as_str(), "0.5");
    }

    #[test]
    fn parsing_accepts_the_forms_json_stringify_emits() {
        assert_eq!(Number::parse("0"), Some(Number::new(0.0)));
        assert_eq!(Number::parse("-7"), Some(Number::new(-7.0)));
        assert_eq!(Number::parse("1.5"), Some(Number::new(1.5)));
        assert_eq!(Number::parse("1e3"), Some(Number::new(1000.0)));
        assert_eq!(Number::parse("  4  "), Some(Number::new(4.0)));
    }

    #[test]
    fn parsing_rejects_everything_that_is_not_a_json_number() {
        // `f64::from_str` would accept all four of these, and each would
        // store text that `JSON.parse` then refuses.
        assert_eq!(Number::parse("inf"), None);
        assert_eq!(Number::parse("NaN"), None);
        assert_eq!(Number::parse("\"7\""), None);
        assert_eq!(Number::parse(""), None);
        assert_eq!(Number::parse("[1]"), None);
    }

    #[test]
    fn addition_that_leaves_the_representable_range_is_refused() {
        let huge = Number::new(f64::MAX);
        assert_eq!(huge.plus(huge), None, "infinity is not JSON");
        assert_eq!(
            Number::new(1.0).plus(Number::new(2.0)),
            Some(Number::new(3.0))
        );
    }

    #[test]
    fn json_text_is_carried_through_unexamined() {
        let record = Json::from_text("{\"name\":\"ada\"}");
        assert_eq!(record.as_str(), "{\"name\":\"ada\"}");
    }
}
