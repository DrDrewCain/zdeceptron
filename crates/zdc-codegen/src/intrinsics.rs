//! The JavaScript behind the prelude's primitive layer, per §17.4.7.
//!
//! §17.4.10 named seventeen operations as unwritable in ZDeceptron —
//! inspecting a `Text`, building a collection whose length the source does
//! not know, f64 formatting, Unicode case tables, the clock — and the
//! prelude declares what is left of that list `foreign … from "zd:…"`.
//! This is the other half of those declarations.
//!
//! `newline` was here too, and §17.4.10 had not named it: the lexer's
//! string rule admitted no escapes, so the line separator was a `Text`
//! constant the language could not write. The `"""` block literal made it
//! writable — a body of two empty lines is one line break — and the
//! prelude spells it out now, so there is no `$newline` below.
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
    ("zd:number", "parseDecimal", JsForm::Helper("$parseDecimal")),
    ("zd:number", "sqrt", JsForm::Helper("$sqrt")),
    ("zd:number", "power", JsForm::Helper("$power")),
    ("zd:number", "fixed", JsForm::Helper("$fixed")),
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
    // Encoding, and all three are about the *bytes* of a `Text`, which the
    // language can observe no more than it can observe an f64's digits.
    ("zd:encode", "url", JsForm::Helper("$urlEncoded")),
    ("zd:encode", "json", JsForm::Helper("$jsonEncoded")),
    ("zd:encode", "base64", JsForm::Helper("$base64Encoded")),
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
        // Both answer "or nothing" with the same finiteness test, and
        // sharing it is what keeps a program that uses both from carrying
        // two copies of one line.
        "$sqrt" | "$power" => &["$finite"],
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
        // `Number.isInteger` first, and it is load-bearing rather than
        // belt-and-braces. `i >= 0 && i < length` already rejects `NaN`
        // and both infinities by accident of IEEE comparison — every
        // comparison against `NaN` is false, and no length exceeds
        // `Infinity` — but it *admits* a finite fraction. §14A.3 makes
        // `Whole` an f64 and `/` is emitted as JavaScript's `/`, so
        // `xs at (3 / 2)` is a well-typed program whose index is `1.5`,
        // which passes the range test and then reads a property that is
        // not there. Without the kind check `at` returns `Some(undefined)`:
        // a `None`-shaped failure wearing a `Some`, an `Option of T`
        // inhabited by a value of no type, which `when` then unwraps and
        // hands on. §14A.3's ruling that a `Whole` is integral makes that
        // unreachable through the type system, and unreachable is not
        // impossible, so the sink is checked as well as the source. O(1)
        // still: one intrinsic predicate, no allocation, and `at` keeps
        // the cost §5.4 promises. `$mapAt` below needs no equivalent
        // because `m.has(k)` is already total over every `k`.
        "$textAt" => (
            "const $textAt = (s, i) => {\n  \
             const points = [...s];\n  \
             return Number.isInteger(i) && i >= 0 && i < points.length\n    \
             ? variant('Some', points[i])\n    \
             : variant('None');\n\
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
             return Number.isInteger(i) && i >= 0 && i < $a.length\n    \
             ? variant('Some', $a[i])\n    \
             : variant('None');\n\
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
        // The trampoline two mutually tail-recursive functions run on
        // (#198), and the reason it is a class rather than a plain object.
        //
        // A cycle of functions that give the result of calling one another
        // is a loop, exactly as a function that calls itself is. The
        // self-call becomes `continue $tail` and costs nothing; a call
        // that crosses to another member of the cycle has nowhere to jump
        // to, so it returns a marker saying who to call next and the
        // wrapper drives it.
        //
        // `$Bounce` is a class so `instanceof` decides. A tagged object
        // literal would have to test a property name, and a program can
        // build a record with any field it likes — including that one. No
        // ZDeceptron value is ever an instance of a class the emitter
        // declares, so `instanceof` cannot collide with a program's data
        // the way a duck-typed tag can.
        //
        // One allocation per cross-call, paid only by the functions in a
        // cycle. Everything else in the program is emitted exactly as it
        // was, which is what makes the trampoline affordable: the note on
        // `TailSelfCall` used to argue against it on the grounds that
        // every call would pay, and that is true of a trampoline applied
        // to the whole program rather than to the cycles.
        "$bounce" => (
            "class $Bounce {\n  \
             constructor(step, args) { this.step = step; this.args = args; }\n\
             }\n\
             const $bounce = (r) => {\n  \
             while (r instanceof $Bounce) r = r.step(...r.args);\n  \
             return r;\n\
             };\n",
            false,
        ),
        "$mapAt" => (
            "const $mapAt = (m, k) => (m.has(k) ? variant('Some', m.get(k)) : variant('None'));\n",
            true,
        ),
        // `set key to value in table`, and the reason it copies where
        // `$append` links.
        //
        // A list's append chain works because the shorter list is a
        // *prefix* of the longer one and nothing can change that. A map
        // has no such relation: `set k to 1 in (set k to 2 in m)` and
        // `set k to 2 in (set k to 1 in m)` differ only in which write
        // wins, so a chain would have to be walked from the newest end
        // to answer `at`, and `$mapKeyAt`'s cache is keyed on a real
        // `Map` object. Copying keeps every existing reader unchanged.
        //
        // O(n) per call, which costs the map nothing it was not already
        // paying: a ZDeceptron map is immutable, so any construction
        // copies, and that is also why one form is enough here where a
        // list needed a chain. `prelude/map.zd` records what is written
        // above it.
        //
        // `new Map(m)` preserves iteration order, and `set` on a key the
        // copy already holds replaces the value while leaving the key
        // where it was. Both are ECMA-262, and together they are the
        // order promise `map.zd` documents.
        "$mapSet" => (
            "const $mapSet = (m, k, v) => new Map(m).set(k, v);\n",
            false,
        ),
        "$uppercase" => ("const $uppercase = (s) => s.toUpperCase();\n", false),
        "$lowercase" => ("const $lowercase = (s) => s.toLowerCase();\n", false),
        "$trim" => ("const $trim = (s) => s.trim();\n", false),
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
        //
        // `Number.isInteger` for the same reason `$listAt` and `$textAt`
        // carry it: this helper was written after that guard and against
        // the older bounds test, and `ks[1.5]` is `undefined`, so without
        // it the map's `at` could hand back a `Some` wrapping nothing
        // where the list's and the text's could not.
        "$mapKeyAt" => (
            "const $mapKeys = new WeakMap();\n\
             const $mapKeyAt = (m, i) => {\n  \
             let ks = $mapKeys.get(m);\n  \
             if (ks === undefined) { ks = [...m.keys()]; $mapKeys.set(m, ks); }\n  \
             return Number.isInteger(i) && i >= 0 && i < ks.length\n    \
             ? variant('Some', ks[i])\n    \
             : variant('None');\n\
             };\n",
            true,
        ),
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
        // The way into a number, and the reason it is a regular expression
        // rather than a call to `Number`. `Number` is total over every
        // `Text` and answers for four of them in ways a form field must
        // not inherit: `""` is 0, `"0x1f"` is 31, `"Infinity"` is an
        // infinity, and `parseFloat` would take the numeric prefix of
        // `"12abc"`. The pattern is the language's own numeric literal
        // plus a leading sign, so the set this accepts is a statement
        // about ZDeceptron rather than about JavaScript, and `Number` is
        // reached only after the text has been agreed to be a number.
        //
        // `Number.isFinite` on top of that, because a literal can still
        // overflow: `"1e400"` matches the pattern and weighs to
        // `Infinity`. §14A.3 makes an infinity a legal `Decimal`, so this
        // is a decision and `prelude/number.zd` records it.
        // The rule `sqrt` and `power` share: an answer that is a finite
        // number, or nothing. Written once here rather than twice below,
        // because a program that computes a distance usually squares
        // something first and would otherwise ship the test twice.
        //
        // `Number.isFinite` and not the global `isFinite`, for the reason
        // `$floor` gives: the global coerces its argument first, and a
        // coercion is what this guard exists to refuse.
        "$finite" => (
            "const $finite = (n) => (Number.isFinite(n) ? variant('Some', n) : variant('None'));\n",
            true,
        ),
        // A root and an exponent, both from `Math`. Neither is writable in
        // ZDeceptron: a Newton iteration would round differently from the
        // platform's correctly rounded `sqrt`, and repeated multiplication
        // says nothing at all about a fractional exponent.
        //
        // The finiteness test is doing real work in both. `Math.sqrt(-1)`
        // is `NaN`, `Math.sqrt(Infinity)` is `Infinity`, `Math.pow(10,
        // 400)` overflows to `Infinity`, and `Math.pow(0, -1)` is the
        // division by zero `quotient` already refuses. All four are `None`.
        // Fixed-point text, and the whole of what the prelude takes from
        // the platform for formatting. `Intl` is deliberately not reached:
        // a prelude primitive is `is anywhere`, and the sandbox §17.4.8
        // runs the build root in has no `Intl` at all, so the claim would
        // be false at one of the three roots.
        //
        // Three guards, and each closes a way `toFixed` would break the
        // promise `groupedText` reads it under — a sign, digits and at
        // most one point. A count outside `0 … 100` throws a `RangeError`;
        // a non-finite value renders as the word `Infinity`; and at or
        // above 1e21 the platform gives exponential notation instead of
        // digits.
        "$fixed" => (
            "const $fixed = (n, d) =>\n  \
             Number.isFinite(n) && Math.abs(n) < 1e21 && Number.isInteger(d) && d >= 0 && d <= 100\n    \
             ? variant('Some', n.toFixed(d))\n    \
             : variant('None');\n",
            true,
        ),
        "$sqrt" => ("const $sqrt = (n) => $finite(Math.sqrt(n));\n", false),
        "$power" => (
            "const $power = (a, b) => $finite(Math.pow(a, b));\n",
            false,
        ),
        "$parseDecimal" => (
            "const $parseDecimal = (s) => {\n  \
             const t = s.trim();\n  \
             if (!/^[+-]?([0-9]+(\\.[0-9]*)?|\\.[0-9]+)([eE][+-]?[0-9]+)?$/.test(t)) \
             return variant('None');\n  \
             const n = Number(t);\n  \
             return Number.isFinite(n) ? variant('Some', n) : variant('None');\n\
             };\n",
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
        // The component form, which escapes `/`, `?`, `#`, `&` and `=`
        // because inside a path segment or a parameter each of those is
        // data rather than syntax. `encodeURI` escapes less and is not
        // offered: a program builds a URL out of parts, and a second
        // spelling of one operation is what §4.1 refuses.
        "$urlEncoded" => (
            "const $urlEncoded = (s) => encodeURIComponent(s);\n",
            false,
        ),
        // `JSON.stringify` of a string is the JSON *value*, quotes
        // included, which is the form a program concatenates into a body.
        "$jsonEncoded" => ("const $jsonEncoded = (s) => JSON.stringify(s);\n", false),
        // Base64 over the UTF-8 bytes, written out rather than delegated.
        //
        // `btoa` is the obvious call and it is wrong twice: it reads its
        // argument as one byte per UTF-16 unit and *throws* above U+00FF,
        // so it cannot encode `é`; and it is a Web API rather than
        // ECMA-262, so it is absent from the engine §17.4.8 runs the build
        // root in, which would make a `static` base64 fail in a build
        // while the same expression worked in a browser. `TextEncoder` has
        // the second problem alone. `encodeURIComponent` is core, and its
        // output is the UTF-8 bytes already: everything it did not escape
        // is ASCII and therefore its own byte, and everything it did is a
        // `%` and two hexadecimal digits.
        "$base64Encoded" => (
            "const $base64Encoded = (s) => {\n  \
             const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';\n  \
             const escaped = encodeURIComponent(s);\n  \
             const bytes = [];\n  \
             for (let i = 0; i < escaped.length; i += 1) {\n    \
             if (escaped[i] === '%') {\n      \
             bytes.push(parseInt(escaped.slice(i + 1, i + 3), 16));\n      \
             i += 2;\n    \
             } else {\n      \
             bytes.push(escaped.charCodeAt(i));\n    \
             }\n  \
             }\n  \
             let out = '';\n  \
             for (let i = 0; i < bytes.length; i += 3) {\n    \
             const n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);\n    \
             out += alphabet[(n >> 18) & 63] + alphabet[(n >> 12) & 63];\n    \
             out += i + 1 < bytes.length ? alphabet[(n >> 6) & 63] : '=';\n    \
             out += i + 2 < bytes.length ? alphabet[n & 63] : '=';\n  \
             }\n  \
             return out;\n\
             };\n",
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
        // --- the two typed fields (#45, #48) ---------------------------
        //
        // These two are not prelude primitives, and they are here rather
        // than in `dom.js` for a measured reason: **the shipped runtime
        // has no room left.** `zdc-bench`'s null-program gate asserts that
        // `signal.js` plus `dom.js` plus one program's emission is under a
        // third of Swift's 73 kB, and on the commit this was written
        // against that leaves *five bytes*. A helper in `dom.js` is paid
        // for by every program; a helper here is paid for only by a
        // program that writes one of these elements, which is the right
        // place for the cost of one element to sit anyway.
        //
        // `elements.js` states both rules again, because it is a
        // *reference implementation* rather than a shipped module and so
        // has to build the same node without importing either of these.
        // Its copy is pinned by `element_parity.rs` on the shape and by
        // `vocabulary.rs` on the behaviour.
        //
        // The `Option` a numeric field's `valueAsNumber` stands for.
        // `Number.isNaN` and not the coercing global: `isNaN('')` is
        // `false`, so an empty box would read as `Some 0`.
        "$optionalNumber" => (
            "const $optionalNumber = (v) =>\n  \
             Number.isNaN(v) ? variant('None') : variant('Some', v);\n",
            true,
        ),
        // The other direction: the signal, into the box.
        //
        // **`valueAsNumber` and not `value`, and that is not a detail.**
        // A number field runs HTML's value sanitisation, so while a
        // reader is part way through `1.` or `-` its `value` is the empty
        // string; comparing text would rewrite the box on every keystroke
        // and a decimal point could never be typed at all. Comparing the
        // number leaves the box alone while the two agree.
        //
        // It is also what serves `DateInput` with no calendar of its own:
        // HTML defines a date field's `valueAsNumber` as the moment at
        // midnight UTC on the chosen day, so the browser renders
        // `YYYY-MM-DD` from the number `prelude/time.zd` already speaks.
        // `zdc-cli/tests/browser.rs` asks a real browser for both claims.
        //
        // `NaN` empties the box, which is what `None` looks like, and is
        // also where a non-finite number has to go: the setter throws on
        // an infinity, and no box can show one.
        "$numberField" => (
            "const $numberField = (n, get) =>\n  \
             effect(() => {\n    \
             const held = get();\n    \
             const shown =\n      \
             held.tag === 'Some' && Number.isFinite(held.fields[0]) ? held.fields[0] : NaN;\n    \
             if (!Object.is(n.valueAsNumber, shown)) n.valueAsNumber = shown;\n  \
             });\n",
            false,
        ),
        _ => return None,
    })
}

/// `variant`, for a root that cannot import it.
///
/// Spelled to construct the same object `runtime/dom.js` does, because
/// `when` dispatches on that shape and a second spelling is a second
/// thing that can get it wrong. `tests/non_importing_roots.rs` runs both
/// and compares, so the two cannot drift apart quietly.
const VARIANT: &str = "const variant = (tag, ...fields) => ({ tag, fields });\n";

/// The definitions a root that **imports nothing** has to declare itself.
///
/// The client bundle imports `variant` from `dom.js` and declares the `$`
/// helpers in its preamble. The other two roots can do neither: §17.4.8
/// runs the build root inside the compiler's own sandbox, which has no
/// `dom.js` in it, and §8.2's server root gets `$env` and `$store`
/// injected by a platform adapter and nothing else. So a build root
/// holding a `static` variant printed `variant('Busy')` against nothing,
/// and stopped with E10; a server root printed the same call and would
/// have thrown at request time.
///
/// `variant` is emitted first because the three `at` helpers call it.
pub fn preamble(used: &crate::view::RuntimeImports) -> String {
    let mut out = String::new();
    for name in &used.dom {
        match *name {
            "variant" => out.push_str(VARIANT),
            // unreached: an internal guard. Every other `dom.js` name is
            // inserted while lowering a `view`, and neither of these roots
            // has one.
            other => unreachable!(
                "`{other}` is a `dom.js` name in a root that imports nothing; only `variant` \
                 can reach one"
            ),
        }
    }
    for name in &used.helpers {
        let (source, _) =
            helper(name).unwrap_or_else(|| unreachable!("`{name}` was used, so it has a source"));
        out.push_str(source);
    }
    out
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
        // Counted: every assertion below is inside the loop, so a prelude
        // that stopped declaring primitives — or a `load()` that returned
        // nothing — would pass this over zero declarations.
        let mut scanned = 0;
        for decl in &zdc_lib::load().program().decls {
            let zdc_ast::Decl::Foreign(foreign) = decl else {
                continue;
            };
            let form = intrinsic(&foreign.module, foreign.export.as_str()).unwrap_or_else(|| {
                panic!(
                    "`{}` comes from `{}` as `{}`, which has no JavaScript form",
                    foreign.name.text, foreign.module, foreign.export
                )
            });
            scanned += 1;
            if let JsForm::Helper(name) = form {
                assert!(
                    helper(name).is_some(),
                    "`{name}` is named by a form but has no source"
                );
            }
        }
        assert_eq!(
            scanned, 28,
            "the primitive layer changed size; every one needs a JavaScript form"
        );
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
