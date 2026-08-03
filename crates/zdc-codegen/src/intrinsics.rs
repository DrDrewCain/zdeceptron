//! The JavaScript behind the prelude's primitive layer, per §17.4.7.
//!
//! §17.4.10 named seventeen operations as unwritable in ZDeceptron —
//! inspecting a `Text`, building a collection whose length the source does
//! not know, f64 formatting, Unicode case tables, the clock — and the
//! prelude declares what is left of that list `foreign … from "zd:…"`,
//! alongside the ones §17.4.10 did not name: `newline`, because the
//! lexer's string rule admits no escapes.
//! This is the other half of those declarations.
//!
//! **One of them returns a collection, and only one.** "Returns a
//! collection" stopped being a reason when `append item to list` landed,
//! so `reverse`, `rest` and `values` became ordinary ZDeceptron folds, and
//! `keys` followed once `mapKeyAt` gave a fold over a map something to
//! walk. Everything else below inspects a value or reads the platform and
//! hands back a number, a character, a `Text` or an `Option`.
//! The two helpers that made that possible — `$append` and `$force` — are
//! not primitives and are named by no `foreign`: they are the emission of
//! a language construct, which is checked, rather than of a declaration,
//! which §14E.4 only asserts.
//!
//! `$split` is the exception, and it is kept on a measurement rather than
//! on principle. It *can* be written in ZDeceptron — as a fold that
//! matches its separator with `slice` at each index — and it was. But
//! every operation a content site runs over a whole document goes through
//! it, and written that way it costs one interpreted loop iteration per
//! character instead of one platform call: the delimiter family over a
//! ten-thousand character document went from milliseconds to 416 seconds,
//! against a twenty-second budget, and
//! `zdc-codegen/tests/library.rs::the_delimiter_family_survives_a_ten_thousand_character_document`
//! is where that is enforced. §14F.2 asks for operations in the language
//! where the language can express them; it does not ask for a library that
//! cannot hold a document. So `split` stays a platform call, and it is the
//! one place in this table where the reason is a number rather than an
//! argument.
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
    ("zd:map", "length", JsForm::Field("size")),
    ("zd:map", "at", JsForm::Helper("$mapAt")),
    ("zd:map", "keyAt", JsForm::Helper("$mapKeyAt")),
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

/// The helpers one helper's source calls, and therefore cannot be emitted
/// without.
///
/// Declared rather than inferred: a helper's source is a string, and a
/// grep over it for `$` would be a second, weaker spelling of the same
/// fact. [`Emitter::use_helper`] follows these edges, so asking for
/// `$listAt` brings `$force` with it and nothing else has to remember to.
pub fn requires(name: &str) -> &'static [&'static str] {
    match name {
        // Both walk a list, and a list may be an append chain.
        "$listAt" | "$append" => &["$force"],
        _other => &[],
    }
}

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
        // `using is ""` is the characters of `s`, one piece each.
        // JavaScript's own `split('')` divides a `Text` into UTF-16 units
        // and hands back half a `🎉`; this divides it the way `$textAt`
        // indexes it, which is the decision §5.4 makes everywhere else.
        "$split" => (
            "const $split = (s, using) => using === '' ? [...s] : s.split(using);\n",
            false,
        ),
        "$listAt" => (
            "const $listAt = (xs, i) => {\n  \
             const $a = $force(xs);\n  \
             return i >= 0 && i < $a.length ? variant('Some', $a[i]) : variant('None');\n\
             };\n",
            true,
        ),
        // `append item to list`, and the reason it is not `[...xs, v]`.
        //
        // A JavaScript array cannot share a prefix with a longer array —
        // `length` is storage, not a view — so an append that hands back
        // a plain array must copy, every element, every time, and a list
        // built one element at a time costs O(n²). That is the same trap
        // `rest of` set for folds, and it is not one a language should
        // set for the only way it has to build a collection.
        //
        // So an appended list is a link in a chain: O(1) to make, and
        // flattened to a real array the first time anything looks at it.
        // Building n elements is n links and one flatten, which is O(n)
        // in total. The old list is untouched by both, so the value stays
        // immutable in the way §14B.2's `remove` comment describes.
        //
        // The flatten is iterative rather than recursive because the
        // chain is as long as the list, and it caches only on the node it
        // was asked about: caching on every node would make each cache
        // the wrong length.
        "$force" => (
            "class $Ap {\n  \
             constructor(base, item) { this.base = base; this.item = item; this.flat = null; }\n  \
             get length() { return $force(this).length; }\n  \
             [Symbol.iterator]() { return $force(this)[Symbol.iterator](); }\n  \
             toJSON() { return $force(this); }\n\
             }\n\
             const $force = (xs) => {\n  \
             if (!(xs instanceof $Ap)) return xs;\n  \
             if (xs.flat) return xs.flat;\n  \
             const added = [];\n  \
             let node = xs;\n  \
             while (node instanceof $Ap && !node.flat) { added.push(node.item); node = node.base; }\n  \
             const out = $force(node).slice();\n  \
             for (let i = added.length - 1; i >= 0; i -= 1) out.push(added[i]);\n  \
             xs.flat = out;\n  \
             return out;\n\
             };\n",
            false,
        ),
        "$append" => ("const $append = (xs, v) => new $Ap(xs, v);\n", false),
        "$mapAt" => (
            "const $mapAt = (m, k) => (m.has(k) ? variant('Some', m.get(k)) : variant('None'));\n",
            true,
        ),
        "$uppercase" => ("const $uppercase = (s) => s.toUpperCase();\n", false),
        "$lowercase" => ("const $lowercase = (s) => s.toLowerCase();\n", false),
        "$trim" => ("const $trim = (s) => s.trim();\n", false),
        // The one character the lexer's string rule cannot contain, and
        // therefore the one `Text` constant the language cannot write for
        // itself. Exactly the reason `$trim` is here.
        "$newline" => ("const $newline = () => '\\n';\n", false),
        // No `$split`, `$reverse`, `$rest`, `$values` or `$keys`: each of
        // those returned a collection, which is why §17.4.10 called them
        // primitives, and each is now an ordinary fold in the prelude
        // built with `append`. Nothing below returns one.
        //
        // The key at a position, which is the map's `$listAt` and the one
        // thing a map could not do for itself. A `Map` has no indexed
        // access, so a position has to be resolved against an array of
        // its keys — and building that array per call would make every
        // fold over a map quadratic, which is the trap `rest of` set for
        // lists.
        //
        // So the array is built once per map and kept in a `WeakMap`.
        // That is sound rather than lucky: a ZDeceptron map is immutable
        // and every mutation emits a fresh `new Map(...)`, so a map that
        // is still reachable still has the keys it was built with, and a
        // map that is not takes its cache with it.
        //
        // The order of that array is the order `Map.prototype.keys`
        // gives, which ECMA-262 specifies as insertion order for every
        // kind of key. It is the order the map literal was written in,
        // the order the pair form serialises in, and the order a map
        // rebuilt from those pairs enumerates in.
        "$mapKeyAt" => (
            "const $mapKeys = new WeakMap();\n\
             const $mapKeyAt = (m, i) => {\n  \
             let ks = $mapKeys.get(m);\n  \
             if (ks === undefined) { ks = [...m.keys()]; $mapKeys.set(m, ks); }\n  \
             return i >= 0 && i < ks.length ? variant('Some', ks[i]) : variant('None');\n\
             };\n",
            true,
        ),
        "$floor" => ("const $floor = (n) => Math.floor(n);\n", false),
        "$round" => ("const $round = (n) => Math.round(n);\n", false),
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
        assert!(!helper("$trim").expect("a source").1);
    }
}
