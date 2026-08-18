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
/// [`string`] or [`json_string`], which escape. That is the point: an
/// emission site that wants a string literal has to hold one of these, and
/// a site that interpolates a raw `&str` between two apostrophes no longer
/// type-checks — which is what three separate injection holes (the
/// `import` clause, the generated `class` getter, and the folded
/// stylesheet) all had in common.
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
/// **Not** [`string`]: JSON has no single-quoted form, `\'` is not an
/// escape there, and every C0 control must be escaped rather than merely
/// being unwise. `manifest.json` is the one generated artefact that is
/// read by `JSON.parse` rather than by an evaluator, and it used to build
/// its object by writing `"{name}"` around a value straight out of the
/// program.
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

/// A JSON string literal for a document written *inside* an HTML
/// `<script>` element, such as the import map in the head (#238).
///
/// [`json_string`] is not enough there and the difference is not cosmetic.
/// A `<script>` element's content is raw text: the HTML parser does not
/// decode entities inside it, so escaping the content as HTML would corrupt
/// it, and it ends the element at the first `</script`, so a target
/// containing that sequence would close the map early and put the rest of
/// it into the document as markup. `<` is therefore escaped to its
/// six-character JSON form, which `JSON.parse` reads back as the same
/// string and which leaves no `<` for the HTML parser to find. That also
/// disposes of `<!--`, which starts a comment in the same position.
///
/// The values here come from the project's `zd.toml` rather than from a
/// stranger, which is a reason to keep the file honest and not a reason to
/// skip the escape: the three injection holes this module was written for
/// were all in positions nobody expected an attacker to reach either.
pub fn script_json(value: &str) -> Quoted {
    let escaped = json_string(value).0.replace('<', "\\u003c");
    Quoted(escaped)
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

/// A JavaScript identifier, which is the one thing that cannot be escaped.
///
/// An `import { X as $f0 } from …` clause needs `X` as *syntax*, so there
/// is no escape that makes an arbitrary string safe there. The answer is
/// therefore a validating constructor rather than an escaping one: a site
/// that needs a bare name must prove it has one, and `None` is a refusal
/// the caller has to handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident(String);

impl std::fmt::Display for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `name` as a bare JavaScript identifier, or `None` if it is not one.
///
/// The rule itself is [`zdc_ast::is_javascript_identifier`], not a copy of
/// it. The parser refuses a `foreign`'s export against that same function,
/// so by the time anything reaches here the answer is already settled —
/// and this gate is kept anyway, because it guards the *emission* site
/// rather than one construct's syntax. It is what a future emitter writing
/// a name from somewhere else has to get past.
pub fn ident(name: &str) -> Option<Ident> {
    zdc_ast::is_javascript_identifier(name).then(|| Ident(name.to_string()))
}

/// A JSON document, as a JavaScript expression — §17.4.8's inlining.
///
/// JSON is very nearly a subset of JavaScript expression syntax, and the
/// three places it is not are all handled here. An object literal at the
/// start of a statement parses as a block, so one is parenthesised.
/// `U+2028` and `U+2029` are legal unescaped in JSON and were illegal in a
/// JavaScript string literal before ES2019, so they are escaped rather
/// than trusted to the host's vintage.
///
/// The third is the minus sign, and it is the reason this returns
/// something a caller may treat as primary. JSON's `-5` is *not* a
/// JavaScript literal — it is unary minus applied to `5`, which binds
/// looser than a member access and, under another minus, is a decrement:
/// a `static Whole` of `-5` read as `-n` inlined to `--5`, and a bundle
/// that does not parse is a build that succeeded and a page that is
/// blank. [`number`] parenthesises a negative for exactly this reason and
/// this is the same rule one layer out, so an inlined `static` is primary
/// in fact and not only by assertion.
pub fn literal(json: &str) -> String {
    let escaped = json
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    if escaped.starts_with('{') || escaped.starts_with('-') {
        return format!("({escaped})");
    }
    escaped
}

/// A value expression placed as the concise body of an arrow function.
///
/// [`literal`]'s rule, one layer out: after `=>` a leading `{` starts a
/// *block* rather than an object literal, so an expression that begins
/// with one is parenthesised. A record literal is the only value form this
/// emitter produces that begins with a brace: a collection is `[…]` and a
/// map is `new Map(…)`, so this is the whole of the exposure.
///
/// Both ways it went wrong were silent at build time, which is why the
/// rule lives in one function rather than at each site. With two or more
/// fields, `(n) => { x: n, y: n }` is a `SyntaxError` and the bundle does
/// not parse; with one, `(n) => { x: n }` is a block holding a labelled
/// statement, so the arrow returns `undefined` for every element and
/// nothing anywhere says so. `zdc check` and `zdc build` exit 0 for both
/// (#194).
///
/// Conditional rather than unconditional parentheses, for the reason the
/// [`precedence`] table exists: `(n) => (n.x)` is noise in every bundle to
/// guard a case that only a brace can reach.
///
/// **Precedence does not decide this one.** An object literal binds as
/// tightly as anything can and still cannot open a concise body, so the
/// hazard is the leading `{` and nothing about how the expression binds.
/// A record literal and a pair literal are both objects, which is why one
/// function is called at every site that writes `() => …` rather than the
/// rule being restated per site.
pub fn arrow_body(text: &str) -> String {
    if text.starts_with('{') {
        return format!("({text})");
    }
    text.to_string()
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

/// A finished HTML attribute value, double quotes included.
///
/// The same bargain [`Quoted`] makes, for the other language this
/// compiler emits. The quotes belong to the escaper because that is what
/// makes "is this value escaped?" a question the type answers: a template
/// writing its own `"{}"` around a raw `&str` has opted out of the rule,
/// and after this it does not type-check either. That is strictly more
/// than `check-emitted-strings.sh` can promise: the script looks for a
/// quote *beside a placeholder*, so a site that pushed `"=\""` and the
/// escaped value as two separate statements passed it — which is exactly
/// what `print_markup` used to do.
///
/// It is a separate type from [`Quoted`] on purpose. An HTML attribute and
/// a JavaScript string literal are escaped against different terminators —
/// `&quot;` means nothing to a JavaScript parser and `\'` means nothing to
/// an HTML one — so a value escaped for one is *not* safe in the other,
/// and one type for both would let a site swap them silently. That
/// swap is exactly the defect this pair was introduced to fix: the
/// generated `<script type="module">` block escaped its module specifier
/// with the HTML rule, which leaves an apostrophe untouched, so a path
/// containing one closed the JavaScript string it sat in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute(String);

impl std::fmt::Display for Attribute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Escape a compile-time literal as a double-quoted attribute value.
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
pub fn html_attribute(value: &str) -> Attribute {
    Attribute(format!(
        "\"{}\"",
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
    ))
}

/// JavaScript operator precedence, high binds tighter.
///
/// Only the levels this compiler can emit. Parenthesising by table rather
/// than parenthesising everything is what keeps `count() * 2` from becoming
/// `((count()) * (2))`.
pub mod precedence {
    /// `x.p = v`, emitted for `set Handle as "p"` and nowhere else.
    ///
    /// Below every operator *and* below `CONDITIONAL`, which is where
    /// JavaScript puts it: `a = b ? c : d` parses as `a = (b ? c : d)`,
    /// so an assignment used as an operand needs brackets everywhere a
    /// conditional would and in one place more. Both arrived at once
    /// wanting level 2; this is the one that has to be looser.
    ///
    /// Below every operator, which is what makes it need parentheses
    /// wherever it is an operand of one. It never is today — a write
    /// `gives nothing`, so the only position it can reach is a `do`
    /// statement — and it carries its real level anyway, because a level
    /// that is right only because of a rule enforced in another crate is
    /// the kind of coupling this table exists to avoid.
    pub const ASSIGNMENT: u8 = 1;
    /// JavaScript's `?:`, which binds looser than every operator here.
    /// Two rather than one so an emitted conditional is still an operand
    /// of nothing without brackets — and one rather than zero because a
    /// comma expression, which this emitter never writes, is looser
    /// still.
    pub const CONDITIONAL: u8 = 2;
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

    /// The one value form that begins with a brace is parenthesised and
    /// nothing else is. `(n) => { x: n }` is a block holding a labelled
    /// statement, which returns `undefined` and parses, so no static check
    /// downstream can catch what this prevents.
    #[test]
    fn an_arrow_body_is_parenthesised_only_where_a_brace_would_start_a_block() {
        assert_eq!(arrow_body("{ x: n }"), "({ x: n })");
        assert_eq!(arrow_body("{ x: n, y: n }"), "({ x: n, y: n })");
        assert_eq!(arrow_body("{ x: n }.x"), "({ x: n }.x)");
        assert_eq!(arrow_body("[1, 2]"), "[1, 2]");
        assert_eq!(arrow_body("new Map([])"), "new Map([])");
        assert_eq!(arrow_body("n.x + 1"), "n.x + 1");
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
    fn an_identifier_is_validated_rather_than_escaped() {
        assert_eq!(
            ident("mount").map(|i| i.to_string()).as_deref(),
            Some("mount")
        );
        assert_eq!(
            ident("$_a0").map(|i| i.to_string()).as_deref(),
            Some("$_a0")
        );
        assert_eq!(ident(""), None);
        assert_eq!(ident("0a"), None);
        assert_eq!(ident("a b"), None);
        assert_eq!(ident("m } from 'evil'; //"), None);
        assert_eq!(ident("a\u{2028}b"), None);
    }

    #[test]
    fn markup_escapes_differ_by_position() {
        assert_eq!(html_text("a & b < c"), "a &amp; b &lt; c");
        assert_eq!(
            html_attribute("a \" b & c").to_string(),
            "\"a &quot; b &amp; c\""
        );
        assert_eq!(
            html_attribute("a > b").to_string(),
            "\"a > b\"",
            "a bare > does not end an attribute value"
        );
        assert_eq!(
            html_attribute("</script>").to_string(),
            "\"&lt;/script>\"",
            "`</script` ends a script element from inside an attribute too"
        );
    }

    /// The quotes belong to the escaper, so a caller cannot hold an
    /// escaped attribute value without them and cannot supply its own.
    #[test]
    fn an_attribute_carries_the_quotes_that_bound_it() {
        assert_eq!(html_attribute("plain").to_string(), "\"plain\"");
        assert_eq!(
            html_attribute("a\" onload=\"x").to_string(),
            "\"a&quot; onload=&quot;x\"",
            "a value cannot end its own attribute and open another"
        );
    }

    /// An inlined `static` is put straight into the surrounding
    /// expression, so what comes back has to *be* primary rather than
    /// merely be treated as it. JSON's `-5` is unary minus, not a literal.
    #[test]
    fn an_inlined_static_is_a_primary_expression() {
        assert_eq!(literal("5"), "5");
        assert_eq!(literal("\"a\""), "\"a\"");
        assert_eq!(literal("[1,2]"), "[1,2]");
        assert_eq!(literal("{\"a\":1}"), "({\"a\":1})");
        assert_eq!(literal("-5"), "(-5)", "`- -5` and `--5` are not `-(-5)`");
        assert_eq!(literal("-0.5"), "(-0.5)");
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
