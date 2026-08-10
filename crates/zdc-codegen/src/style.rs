//! The value grammar of every style argument, per spec §16.3.11.
//!
//! # Why a grammar per argument, and not one string check
//!
//! `padding` and `weight` were the whole styling surface, and `weight`
//! took any text that [`crate::elements::style_value_is_permitted`]
//! admitted. That check is an allowlist of *characters*: it says a value
//! cannot end the declaration it is printed into, which is the security
//! property, and it says nothing at all about whether the value means
//! anything. `weight is "8px"` and `weight is "reddish"` both pass it and
//! both emit a declaration a browser drops on the floor.
//!
//! One check of that shape does not scale to a styling vocabulary. A
//! colour, a length and an alignment are three different sets, and the
//! only way to widen the surface without widening the injection surface
//! with it is to say, per argument, what the argument admits. So each
//! entry in [`crate::elements::STYLE_ARGUMENTS`] carries a [`Grammar`],
//! and a value the grammar does not admit is a diagnostic naming what the
//! argument *is* rather than what the value was not.
//!
//! The alternative that was rejected is the obvious one: accept a CSS
//! declaration value and escape it. There is no escape. An escaped `;` is
//! not a semicolon in a length, it is a parse error, and a value that
//! survives escaping unchanged is a value that was never dangerous.
//! §16.3.5 reaches the opposite conclusion about markup for exactly the
//! reason it does not apply here: HTML has an escape that preserves
//! meaning and CSS does not.
//!
//! # What is checked, and what is a constant
//!
//! Only the *program's* text is checked by a grammar. The right-hand sides
//! a grammar maps a keyword onto are `&'static str` in the compiler, so
//! they are not values a program can choose. [`printable`] is the
//! belt-and-braces gate over everything either half produces, so a grammar
//! that admitted something it should not is a refusal rather than a
//! defacement.

/// When a declaration applies.
///
/// A generated class is no longer one flat declaration set. Modelling the
/// circumstance on the declaration is what keeps *one class per distinct
/// set* true: the set becomes a set of conditioned declarations rather
/// than a set of plain ones, and two elements that transition the same way
/// still share one class.
///
/// The order of the variants is the order the rules are printed in, and
/// that is load-bearing: every rule the compiler generates carries one
/// class of specificity, so later wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Condition {
    /// Always.
    Always,
    /// While the pointer is over the element.
    Hover,
    /// While the element has keyboard focus and the browser judges a ring
    /// worth showing.
    Focus,
    /// While the element is being pressed.
    Active,
    /// While the element is disabled.
    Disabled,
    /// Below the one breakpoint.
    Narrow,
    /// At or above the one breakpoint.
    Wide,
    /// Only where the system asks for a dark colour scheme.
    Dark,
    /// Only where the reader has not asked for less motion.
    ///
    /// Every transition the language can express is inside this, which is
    /// what "respected by construction" means: a program cannot write a
    /// transition that ignores the preference, because there is no
    /// argument that produces one.
    Motion,
}

/// The one breakpoint.
///
/// In `rem` rather than `px`, so that it moves with the reader's own font
/// size: a breakpoint in pixels puts a reader who enlarged their text on
/// the wrong side of it.
///
/// One breakpoint and not a scale of them, and that is the decision #101
/// really carries. Two named widths are the smallest thing that makes a
/// layout respond; every extra one is a word a reader of the program has
/// to hold in their head, and none of the examples needed a third.
/// Adding one later means adding a `Condition` variant, which every match
/// in the compiler is then forced to consider. That is the cost being
/// deferred rather than avoided.
///
/// What is deliberately not here is CSS's media grammar. `narrowDisplay
/// is "none"` says what it means; `@media (max-width: 47.99rem) and
/// (orientation: portrait)` is a second language inside the first, and a
/// program that could write one could write `@media` blocks into the
/// sheet, which is the injection surface everything else here closes.
pub const BREAKPOINT: &str = "48rem";

impl Condition {
    /// The selector suffix this condition adds, and the at-rule it sits
    /// inside.
    ///
    /// Written out rather than defaulted, so a variant added later is a
    /// compile error here rather than a rule that silently applies always.
    pub fn wrapping(self) -> (&'static str, Option<String>) {
        match self {
            Condition::Always => ("", None),
            Condition::Hover => (":hover", None),
            // `:focus-visible`, not `:focus`. A mouse click focuses a
            // button, and a ring drawn then is exactly the ring designers
            // remove focus styling altogether to avoid, which is how
            // keyboard navigation ends up invisible. The browser already
            // knows which focus deserves a ring; this asks it.
            Condition::Focus => (":focus-visible", None),
            Condition::Active => (":active", None),
            // Both ways a control in this language can be unavailable.
            //
            // `:disabled` alone matched nothing a program could write.
            // The vocabulary has no `disabled` *attribute* — HTML's takes
            // the control out of the tab order and its announcement with
            // it — so until `disabled is yes` reached `aria-disabled`
            // there was no element this rule could apply to, and
            // `disabledColor` was a style nobody could ever see. Matching
            // both makes `disabled is yes, disabledColor is "grey"` one
            // sentence rather than two halves that never meet.
            //
            // `:is(…)`, so this stays a *suffix* on the generated class
            // and the folded rule stays one rule.
            Condition::Disabled => (r#":is(:disabled,[aria-disabled="true"])"#, None),
            // The range syntax, `(width < 48rem)`, rather than
            // `(max-width: 47.99rem)`. The subtracted hundredth is a
            // workaround for a comparison the old syntax could not
            // express, it leaves a hairline of widths in neither
            // condition, and the number it subtracts is a guess about the
            // reader's font size once the unit is `rem`. The range form
            // says exactly what is meant and has been in every engine
            // since 2022.
            Condition::Narrow => ("", Some(format!("@media (width < {BREAKPOINT})"))),
            Condition::Wide => ("", Some(format!("@media (width >= {BREAKPOINT})"))),
            Condition::Dark => ("", Some("@media (prefers-color-scheme: dark)".to_string())),
            Condition::Motion => (
                "",
                Some("@media (prefers-reduced-motion: no-preference)".to_string()),
            ),
        }
    }
}

/// One declaration in a generated class.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Declaration {
    pub condition: Condition,
    pub property: String,
    pub value: String,
}

/// The emitter builds declarations from a `StyleArgument`, which carries
/// its own condition, so this shorthand has no caller outside the tests
/// that exercise the interner directly.
#[cfg(test)]
impl Declaration {
    pub fn always(property: impl Into<String>, value: impl Into<String>) -> Self {
        Declaration {
            condition: Condition::Always,
            property: property.into(),
            value: value.into(),
        }
    }
}

/// What one style argument admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grammar {
    /// One to four lengths, each a number of CSS pixels.
    Lengths,
    /// A bare number, with no unit and therefore nothing a unit could
    /// hide in.
    Number,
    /// A whole number, positive or negative.
    Whole,
    /// A number from 0 to 100, printed as the fraction CSS wants.
    Percent,
    /// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, or one of [`COLOURS`].
    Colour,
    /// A URL, filtered exactly as an `Image`'s source is, and then again
    /// for the delimiters of the `url(…)` it is printed inside.
    Url,
    /// One word from a closed list, mapped to what CSS calls it.
    ///
    /// The mapping is the point. A keyword argument is where CSS's own
    /// vocabulary is translated, so a program writes `decoration is
    /// "struck"` rather than `text-decoration-line is "line-through"`,
    /// and a program that had to spell the CSS anyway would be a program
    /// that could have written the CSS.
    Keyword(&'static [(&'static str, &'static str)]),
    /// An angle in degrees, positive or negative, printed as `Ndeg`.
    ///
    /// Degrees and not radians, and not a bare number. A bare number is
    /// what CSS refuses here — `rotate: 30` is invalid and silently drops
    /// the declaration — and radians would make every program that wants
    /// a quarter turn write a constant it cannot check. Fractions are
    /// admitted because a tree's branch angles are not whole numbers.
    Angle,
    /// Anything [`crate::elements::style_value_is_permitted`] admits.
    /// `weight` alone, which predates this module: narrowing it would
    /// refuse programs that compile today, and no issue asked for that.
    Free,
}

/// The plain colour words.
///
/// Twenty-one of CSS's named colours and not all 148: the long tail is
/// `lightgoldenrodyellow` and `mediumspringgreen`, which nobody writes on
/// purpose and which a reader cannot tell apart from a typo. A colour
/// outside this list is written as a hex triple, which is unambiguous.
///
/// What is deliberately *not* admitted is a function call. `rgb(1, 2, 3)`
/// and `color-mix(…)` are the CSS spellings of a colour, and both put a
/// parenthesis in a printed declaration; a value that can open one can
/// leave one open, and an unclosed parenthesis swallows the brace that
/// ends the rule. Hex says the same thing with no delimiter in it.
pub const COLOURS: &[&str] = &[
    "black",
    "white",
    "grey",
    "gray",
    "silver",
    "red",
    "maroon",
    "orange",
    "yellow",
    "olive",
    "lime",
    "green",
    "teal",
    "aqua",
    "blue",
    "navy",
    "purple",
    "fuchsia",
    "brown",
    "pink",
    "transparent",
];

/// The theme tokens: custom properties `base.css` declares once for the
/// light scheme and again under `prefers-color-scheme: dark`.
///
/// This is the layer #102 asked for, and the argument for it is not
/// tidiness. A hex triple written at a use site is a colour that is right
/// in one scheme, so a program built out of hex triples has one scheme and
/// cannot have two: `background is "surface"` is one declaration that
/// means two colours, and which one a reader gets is decided by their
/// system rather than by anything the program has to notice. That is what
/// "dark mode follows the system preference" has to mean if it is to cost
/// a program nothing.
///
/// Nine names, in pairs where a pair is needed: a surface has ink on it,
/// an accent has `onAccent` on it, a danger has `onDanger` on it. A token
/// for a background with no token for the text that sits on it is a token
/// that is half a decision.
///
/// The names are the *role*, not the colour. `ink` rather than `black`,
/// because in the dark scheme it is not black, and a token called
/// `black` that is white in half the world's browsers is worse than no
/// token at all.
pub const TOKENS: &[&str] = &[
    "ink", "muted", "surface", "raised", "line", "accent", "onAccent", "danger", "onDanger",
];

/// Whether a value the compiler is about to print into a rule can end it.
///
/// The last gate before `styles.css`, over both halves of a declaration:
/// the part a grammar built from the program's text, and the part that is
/// a constant in the compiler. Neither is trusted here, because checking a
/// constant costs nothing and a grammar that admitted one character too
/// many costs the whole page.
///
/// What is refused is everything that can *leave* a declaration: `;` and
/// `}` end it, `{` begins a block, `/*` swallows the rest of the sheet, a
/// backslash is a CSS escape, and a control character is not a value
/// anybody typed. Parentheses are permitted but must balance, because
/// values the compiler builds later will have them and a value ending
/// inside one swallows the rule's closing brace.
pub fn printable(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("/*")
        && !value
            .chars()
            .any(|c| matches!(c, ';' | '{' | '}' | '\\') || c.is_control())
        && balanced(value)
}

/// Whether every parenthesis in `value` is closed inside it.
fn balanced(value: &str) -> bool {
    let mut depth: i32 = 0;
    for c in value.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// A colour, or `None`.
///
/// A token wins over a plain colour word, and the two sets are disjoint
/// by construction: no token is named for a colour, because a token names
/// the role rather than the hue.
fn colour(value: &str) -> Option<String> {
    if let Some(digits) = value.strip_prefix('#') {
        let hex =
            matches!(digits.len(), 3 | 4 | 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit());
        return hex.then(|| value.to_string());
    }
    if TOKENS.contains(&value) {
        return Some(format!("var(--zd-{value})"));
    }
    COLOURS.contains(&value).then(|| value.to_string())
}

/// A number, or `None`.
///
/// Written by hand rather than through `f64::from_str`, which accepts
/// `inf`, `NaN`, `1e9` and a leading `+`. None of those is a length
/// anybody writes and the first two are not lengths at all.
fn number(value: &str) -> Option<&str> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() {
        return None;
    }
    let mut parts = digits.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or("0");
    if parts.next().is_some() {
        return None;
    }
    let digits_only = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    (digits_only(whole) && digits_only(fraction)).then_some(value)
}

/// The sentence a refusal uses to say what an argument admits.
pub fn expectation(grammar: Grammar) -> String {
    match grammar {
        Grammar::Lengths => "a length in pixels, or up to four of them separated by spaces".into(),
        Grammar::Number => "a number, with no unit".into(),
        Grammar::Whole => "a whole number".into(),
        Grammar::Percent => "a number from 0 to 100".into(),
        Grammar::Colour => format!(
            "a colour: `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`, one of {}, or one of the theme \
             tokens {}, which follow the reader's colour scheme",
            list(COLOURS),
            list(TOKENS)
        ),
        Grammar::Url => "a URL: relative, or absolute with a scheme that is not script, and \
                         spelled with the characters a URL is spelled with"
            .into(),
        Grammar::Keyword(words) => format!(
            "one of {}",
            list(&words.iter().map(|(word, _)| *word).collect::<Vec<_>>())
        ),
        Grammar::Angle => "an angle in degrees, which may be negative or fractional".into(),
        Grammar::Free => "a length, a keyword, a colour or a comma-separated list of those".into(),
    }
}

/// The characters a URL printed into `url("…")` may use.
///
/// A second filter, and not a duplicate of the first. `url_is_safe`
/// decides whether the *destination* executes script, which is the
/// question `Link` and `Image` ask. This one decides whether the text can
/// leave the parentheses and quotes it is about to be printed between,
/// which is a different question with a different answer: `/a.png"),
/// url(https://evil.example/x` names no scheme at all and passes the
/// first check cleanly.
///
/// Percent-encoding is what a URL with any other character in it is for,
/// and it is already the only spelling a browser resolves the same way
/// twice.
fn url_character(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '/' | '.' | '-' | '_' | '~' | '?' | '=' | '&' | '%' | '+' | ':' | '#' | '@' | ','
        )
}

/// `` `a` ``, `` `b` `` and `` `c` ``, the phrasing every list in a
/// diagnostic uses.
fn list(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("`{item}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// The CSS value `text` means under `grammar`, or `None` if the grammar
/// does not admit it.
pub fn value(grammar: Grammar, text: &str) -> Option<String> {
    let built = match grammar {
        Grammar::Lengths => {
            let parts: Vec<&str> = text.split(' ').collect();
            if parts.is_empty() || parts.len() > 4 {
                return None;
            }
            let mut out = Vec::with_capacity(parts.len());
            for part in parts {
                out.push(format!("{}px", number(part)?));
            }
            out.join(" ")
        }
        Grammar::Number => number(text)?.to_string(),
        // `number` admits a leading `-` and rules out `inf`, `NaN` and the
        // exponent forms, so what is printed here is a plain decimal with
        // a unit stuck to it and nothing that could end the declaration.
        Grammar::Angle => format!("{}deg", number(text)?),
        Grammar::Whole => {
            let digits = text.strip_prefix('-').unwrap_or(text);
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            text.to_string()
        }
        Grammar::Percent => {
            // `parse` after `number` has already ruled out `inf`, `NaN`
            // and the exponent forms, so what reaches it is a plain
            // decimal and the only thing left to decide is the range.
            let parsed: f64 = number(text)?.parse().ok()?;
            if !(0.0..=100.0).contains(&parsed) {
                return None;
            }
            // Printed as a fraction rather than as `N%`. Both are legal
            // CSS for `opacity`, and the fraction is what every stylesheet
            // in the world writes, so a reader of the generated sheet sees
            // what they expect.
            let fraction = format!("{:.4}", parsed / 100.0);
            let trimmed = fraction.trim_end_matches('0').trim_end_matches('.');
            if trimmed.is_empty() {
                "0".to_string()
            } else {
                trimmed.to_string()
            }
        }
        Grammar::Colour => colour(text)?,
        Grammar::Url => {
            if !crate::elements::url_is_permitted(text) || !text.chars().all(url_character) {
                return None;
            }
            // Unquoted, deliberately. CSS's `url-token` admits every code
            // point except a quote, a parenthesis, a backslash, whitespace
            // and the non-printables, which is a superset of what
            // `url_character` just admitted, so the quotes buy nothing
            // here and writing them would put a quote character next to a
            // placeholder, which is the adjacency
            // `scripts/check-emitted-strings.sh` forbids and which was the
            // shape of all three injection holes this compiler has had.
            format!("url({text})")
        }
        Grammar::Keyword(words) => words
            .iter()
            .find(|(word, _)| *word == text)
            .map(|(_, css)| (*css).to_string())?,
        Grammar::Free => {
            if !crate::elements::style_value_is_permitted(text) {
                return None;
            }
            text.to_string()
        }
    };
    printable(&built).then_some(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_a_hex_triple_or_a_plain_word() {
        assert_eq!(value(Grammar::Colour, "red").as_deref(), Some("red"));
        assert_eq!(
            value(Grammar::Colour, "#b3151c").as_deref(),
            Some("#b3151c")
        );
        assert_eq!(value(Grammar::Colour, "#abc").as_deref(), Some("#abc"));
        assert_eq!(value(Grammar::Colour, "#abcd").as_deref(), Some("#abcd"));
        assert_eq!(
            value(Grammar::Colour, "#aabbccdd").as_deref(),
            Some("#aabbccdd")
        );
        assert_eq!(
            value(Grammar::Colour, "transparent").as_deref(),
            Some("transparent")
        );
    }

    #[test]
    fn a_colour_that_is_not_one_is_refused() {
        for refused in [
            "reddish",
            "#ab",
            "#abcde",
            "#gggggg",
            "rgb(1,2,3)",
            "red; } body { display: none } x {",
            "red;",
            "var(--anything)",
            "",
            "url(https://example.com/x)",
            "RED",
        ] {
            assert!(
                value(Grammar::Colour, refused).is_none(),
                "`{refused}` is not a colour"
            );
        }
    }

    #[test]
    fn a_length_is_one_to_four_numbers_of_pixels() {
        assert_eq!(value(Grammar::Lengths, "8").as_deref(), Some("8px"));
        assert_eq!(value(Grammar::Lengths, "0.5").as_deref(), Some("0.5px"));
        assert_eq!(
            value(Grammar::Lengths, "8 0 4 0").as_deref(),
            Some("8px 0px 4px 0px")
        );
        for refused in ["8 0 4 0 2", "8px", "", " 8", "8  0", "eight", "8;"] {
            assert!(
                value(Grammar::Lengths, refused).is_none(),
                "`{refused}` is not a length"
            );
        }
    }

    #[test]
    fn a_value_that_could_end_its_declaration_is_not_printable() {
        assert!(printable("red"));
        assert!(printable("0 1px 2px"));
        assert!(printable("var(--zd-ink)"));
        assert!(!printable(""));
        assert!(!printable("red;"));
        assert!(!printable("red } body { display: none"));
        assert!(!printable("red /* swallow"));
        assert!(!printable("\\3c script"));
        assert!(!printable("red\nblue"));
        assert!(
            !printable("url(\"/a.png"),
            "an unclosed parenthesis swallows the rule's closing brace"
        );
    }

    #[test]
    fn a_number_is_a_number_and_not_an_infinity() {
        assert_eq!(number("8"), Some("8"));
        assert_eq!(number("0.5"), Some("0.5"));
        assert_eq!(number("-2"), Some("-2"));
        for refused in ["inf", "NaN", "1e9", "+1", "8px", "", ".", "1.", "1.2.3"] {
            assert_eq!(number(refused), None, "`{refused}` is not a number");
        }
    }
}
