//! The JavaScript behind the prelude's primitive layer, per §17.4.7.
//!
//! §17.4.10 names the operations that cannot be written in
//! ZDeceptron — inspecting a `Text`, building a collection whose length
//! the source does not know, f64 formatting, Unicode case tables, the
//! clock — and the prelude declares each of them `foreign … from "zd:…"`.
//! This is the other half of those declarations.
//!
//! **Never an import.** §16.3.12 assertion A requires a bundle to contain
//! no import of a ZDeceptron-generated module, and inlining per bundle is
//! also what makes a primitive dead-code-eliminable: a program that never
//! counts a list does not carry `.length` machinery it will not run.
//!
//! Two forms, because the difference matters for §14A.1's shape claim. A
//! [`JsForm::Field`] compiles to a property access with no call at all;
//! a [`JsForm::Helper`] compiles to a call to a `$`-prefixed function
//! declared once in the preamble, so ten uses of `at` share one function
//! object rather than ten closures.

/// How a primitive is emitted at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsForm {
    /// `x.length` — a property of the operand.
    Field(&'static str),
    /// `$listAt(x, i)` — a call to a preamble helper.
    Helper(&'static str),
    /// The operand itself. `decimalOf` and `text of` a `Text` are both
    /// the identity, and emitting a call for either would cost a frame to
    /// compute a value the compiler already has.
    Identity,
}

/// Every `zd:` primitive, by module and symbol.
///
/// Keyed by what the prelude *wrote*, not by a Rust name, so the two
/// halves cannot drift apart without the lookup failing loudly — which
/// `every_primitive_has_a_javascript_form` turns into a test failure
/// rather than an `undefined` in somebody's bundle.
pub const INTRINSICS: &[(&str, &str, JsForm)] = &[
    ("zd:text", "length", JsForm::Helper("$textLength")),
    ("zd:text", "at", JsForm::Helper("$textAt")),
    ("zd:text", "uppercase", JsForm::Helper("$uppercase")),
    ("zd:text", "lowercase", JsForm::Helper("$lowercase")),
    ("zd:text", "trim", JsForm::Helper("$trim")),
    ("zd:text", "split", JsForm::Helper("$split")),
    ("zd:text", "newline", JsForm::Helper("$newline")),
    ("zd:list", "length", JsForm::Field("length")),
    ("zd:list", "at", JsForm::Helper("$listAt")),
    ("zd:list", "reverse", JsForm::Helper("$reverse")),
    ("zd:map", "length", JsForm::Field("size")),
    ("zd:map", "at", JsForm::Helper("$mapAt")),
    ("zd:map", "keys", JsForm::Helper("$keys")),
    ("zd:map", "values", JsForm::Helper("$values")),
    ("zd:number", "floor", JsForm::Helper("$floor")),
    ("zd:number", "round", JsForm::Helper("$round")),
    // §14A.3 makes both numeric types f64, so widening a `Whole` to a
    // `Decimal` is a statement about the type system and nothing about
    // the value.
    ("zd:number", "decimalOf", JsForm::Identity),
    // The bitwise window. Six, not seven: `bitNot` is
    // `bitXor with left is x, right is 4294967295` and a second spelling
    // of one operation is what §4.1 exists to refuse.
    ("zd:number", "bitAnd", JsForm::Helper("$bitAnd")),
    ("zd:number", "bitOr", JsForm::Helper("$bitOr")),
    ("zd:number", "bitXor", JsForm::Helper("$bitXor")),
    ("zd:number", "shiftLeft", JsForm::Helper("$shiftLeft")),
    ("zd:number", "shiftRight", JsForm::Helper("$shiftRight")),
    (
        "zd:number",
        "wrappingProduct",
        JsForm::Helper("$wrappingProduct"),
    ),
    ("zd:time", "now", JsForm::Helper("$now")),
];

/// The JavaScript form of a primitive, if it has one.
pub fn intrinsic(module: &str, symbol: &str) -> Option<JsForm> {
    INTRINSICS
        .iter()
        .find(|(m, s, _)| *m == module && *s == symbol)
        .map(|(_, _, form)| *form)
}

/// The source of one preamble helper, and whether it needs `variant` from
/// the runtime.
///
/// The three `at` helpers build an `Option`, and they build it with the
/// runtime's own `variant` rather than an object literal of their own:
/// `when` dispatches on the shape `variant` produces, and a second place
/// that knows that shape is a second place that can get it wrong.
pub fn helper(name: &str) -> Option<(&'static str, bool)> {
    Some(match name {
        // Indexed by code point, not by UTF-16 unit: §5.4 says a `Text` is
        // text, and `"🎉".length` being 2 is a JavaScript detail the
        // language exists to keep out of the source.
        "$textLength" => ("const $textLength = (s) => [...s].length;\n", false),
        "$textAt" => (
            "const $textAt = (s, i) => {\n  \
             const points = [...s];\n  \
             return i >= 0 && i < points.length ? variant('Some', points[i]) : variant('None');\n\
             };\n",
            true,
        ),
        "$listAt" => (
            "const $listAt = (xs, i) =>\n  \
             i >= 0 && i < xs.length ? variant('Some', xs[i]) : variant('None');\n",
            true,
        ),
        "$mapAt" => (
            "const $mapAt = (m, k) => (m.has(k) ? variant('Some', m.get(k)) : variant('None'));\n",
            true,
        ),
        "$uppercase" => ("const $uppercase = (s) => s.toUpperCase();\n", false),
        "$lowercase" => ("const $lowercase = (s) => s.toLowerCase();\n", false),
        "$trim" => ("const $trim = (s) => s.trim();\n", false),
        "$split" => ("const $split = (s, using) => s.split(using);\n", false),
        // The one character the lexer's string rule cannot contain, and
        // therefore the one `Text` constant the language cannot write for
        // itself. Exactly the reason `$trim` is here.
        "$newline" => ("const $newline = () => '\\n';\n", false),
        // A copy, because ZDeceptron values are not aliased: `reverse of
        // xs` gives a new list and leaves `xs` alone.
        "$reverse" => ("const $reverse = (xs) => xs.slice().reverse();\n", false),
        "$keys" => ("const $keys = (m) => [...m.keys()];\n", false),
        "$values" => ("const $values = (m) => [...m.values()];\n", false),
        // The narrowing §14A.3 made partial. A `Whole` is a *finite*
        // integral f64 and a `Decimal` is every f64, so `Infinity`,
        // `-Infinity` and `NaN` have no `Whole` to become and these say so
        // rather than handing back a value their declared type has
        // misdescribed. `Number.isFinite` and not the global `isFinite`:
        // the global coerces its argument first, and a coercion is the
        // thing this guard exists to refuse.
        "$floor" => (
            "const $floor = (n) =>\n  \
             Number.isFinite(n) ? variant('Some', Math.floor(n)) : variant('None');\n",
            true,
        ),
        "$round" => (
            "const $round = (n) =>\n  \
             Number.isFinite(n) ? variant('Some', Math.round(n)) : variant('None');\n",
            true,
        ),
        // Every one of these ends in `>>> 0`, which is `ToUint32`: the
        // window the prelude promises is unsigned, and JavaScript's `&`,
        // `|`, `^` and `<<` all give back a *signed* int32. `>>>` is
        // already unsigned, so `$shiftRight` is the one that does not
        // need it.
        "$bitAnd" => ("const $bitAnd = (a, b) => (a & b) >>> 0;\n", false),
        "$bitOr" => ("const $bitOr = (a, b) => (a | b) >>> 0;\n", false),
        "$bitXor" => ("const $bitXor = (a, b) => (a ^ b) >>> 0;\n", false),
        "$shiftLeft" => ("const $shiftLeft = (a, n) => (a << n) >>> 0;\n", false),
        "$shiftRight" => ("const $shiftRight = (a, n) => a >>> n;\n", false),
        "$wrappingProduct" => (
            "const $wrappingProduct = (a, b) => Math.imul(a, b) >>> 0;\n",
            false,
        ),
        "$now" => ("const $now = () => Date.now();\n", false),
        // `text of` a number. §14A.3 makes both numeric types f64, and
        // JavaScript's own number-to-string is the shortest form that
        // round-trips — the same algorithm §17.4.10 names `ryu-js` for.
        // Writing a second one in ZDeceptron would give one value two
        // spellings.
        "$textOf" => ("const $textOf = (n) => String(n);\n", false),
        // `text of` a `Truth`. §17.4.9 gives the ZDeceptron definition —
        // `if value / give "yes" / give "no"` — and this is it, inlined so
        // that showing a `Truth` costs no call into the library.
        "$textOfTruth" => ("const $textOfTruth = (v) => (v ? 'yes' : 'no');\n", false),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every primitive the prelude declares has a JavaScript form, and
    /// every helper a form names has a source. A gap in either direction
    /// is an `undefined` in a generated bundle, which is exactly the
    /// failure mode §16.3.1 refuses to ship.
    #[test]
    fn every_primitive_has_a_javascript_form() {
        for decl in &zdc_lib::load().program().decls {
            let zdc_ast::Decl::Foreign(foreign) = decl else {
                continue;
            };
            let form = intrinsic(&foreign.module, &foreign.symbol).unwrap_or_else(|| {
                panic!(
                    "`{}` comes from `{}` as `{}`, which has no JavaScript form",
                    foreign.name.text, foreign.module, foreign.symbol
                )
            });
            if let JsForm::Helper(name) = form {
                assert!(
                    helper(name).is_some(),
                    "`{name}` is named by a form but has no source"
                );
            }
        }
    }

    #[test]
    fn a_helper_that_builds_an_option_says_it_needs_the_runtime() {
        assert!(helper("$listAt").expect("a source").1);
        // §14A.3 made the `Decimal`-to-`Whole` narrowing partial, so these
        // two build an `Option` now and need `variant` as the three `at`
        // helpers always did. Saying `false` here would emit a bundle
        // calling a function it never declared.
        assert!(helper("$floor").expect("a source").1);
        assert!(helper("$round").expect("a source").1);
        assert!(!helper("$trim").expect("a source").1);
    }
}
