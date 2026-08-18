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
//! which §14E.4 only asserts. `$mapSet` and `$mapForce` are the map's
//! pair of the same two, and are here for the same reason.
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

/// The preamble helper turning a `Truth` into the word for it.
///
/// Named rather than spelled at each use because it now has two callers
/// that must agree: `text of` a `Truth` (§17.4.3's dispatch table) and a
/// text slot showing one (#297).
pub const TEXT_OF_TRUTH: &str = "$textOfTruth";

/// This language's word for a truth.
///
/// The compiler needs the words on the Rust side as well as inside
/// `$textOfTruth`, because a *written* `yes` is known at compile time and
/// is folded into the template rather than converted at run time. This is
/// the Rust half; the helper's source is the JavaScript half; and
/// `the_two_halves_of_a_truths_word_agree` pins them together, because a
/// pair of copies nothing pins is how one of them gets corrected alone.
pub fn truth_word(truth: bool) -> &'static str {
    if truth {
        "yes"
    } else {
        "no"
    }
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
    // The transcendental family (`prelude/math.zd`). Every one is the
    // platform's, for the reason `sqrt` is: the platform's answer is the
    // correctly-rounded one, and a series expansion written in the
    // language would be a second answer differing in the last bit.
    ("zd:number", "sin", JsForm::Helper("$sin")),
    ("zd:number", "cos", JsForm::Helper("$cos")),
    ("zd:number", "tan", JsForm::Helper("$tan")),
    ("zd:number", "asin", JsForm::Helper("$asin")),
    ("zd:number", "acos", JsForm::Helper("$acos")),
    ("zd:number", "atan", JsForm::Helper("$atan")),
    ("zd:number", "atan2", JsForm::Helper("$atan2")),
    ("zd:number", "exp", JsForm::Helper("$exp")),
    ("zd:number", "ln", JsForm::Helper("$ln")),
    ("zd:number", "log10", JsForm::Helper("$log10")),
    ("zd:number", "log2", JsForm::Helper("$log2")),
    ("zd:number", "hypotenuse", JsForm::Helper("$hypotenuse")),
    ("zd:number", "cbrt", JsForm::Helper("$cbrt")),
    ("zd:number", "hyperbolicTangent", JsForm::Helper("$tanh")),
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
/// fact. [`crate::expr::Emitter::use_helper`] follows these edges, so
/// asking for `$listAt` brings `$force` with it and nothing else has to
/// remember to.
pub fn requires(name: &str) -> &'static [&'static str] {
    match name {
        // Both walk a list, and a list may be an append chain.
        "$listAt" | "$append" => &["$force"],
        // The same edge one structure over: a map may be a write chain,
        // so the two readers force before they look and the writer needs
        // the class the chain is made of (#233).
        "$mapAt" | "$mapKeyAt" | "$mapSet" => &["$mapForce"],
        // Both answer "or nothing" with the same finiteness test, and
        // sharing it is what keeps a program that uses both from carrying
        // two copies of one line.
        "$sqrt" | "$power" | "$sin" | "$cos" | "$tan" | "$asin" | "$acos" | "$atan" | "$atan2"
        | "$exp" | "$ln" | "$log10" | "$log2" | "$hypotenuse" | "$cbrt" | "$tanh" => &["$finite"],
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
        // Forced first, and then read twice off the same real `Map`:
        // `m.has(k)` on a write chain would flatten it and `m.get(k)`
        // would flatten it again, which is one wasted walk per lookup on
        // the very path this helper is O(1) for.
        "$mapAt" => (
            "const $mapAt = (m, k) => {\n  \
             const $m = $mapForce(m);\n  \
             return $m.has(k) ? variant('Some', $m.get(k)) : variant('None');\n\
             };\n",
            true,
        ),
        // `set key to value in table`, and the reason it links where it
        // used to copy (#233).
        //
        // **The argument this replaces, and what was wrong with it.** A
        // list's append chain works because the shorter list is a
        // *prefix* of the longer one; a map has no such relation, since
        // `set k to 1 in (set k to 2 in m)` and `set k to 2 in (set k to
        // 1 in m)` differ only in which write wins. That much is true,
        // and the conclusion drawn from it — that a map must therefore
        // copy — was not. It proves a map cannot reuse `$Ap`'s *shape*,
        // in which every link is an addition and order is decided by
        // position. It says nothing against a chain of *writes* replayed
        // in the order they were made, which is what a copy is anyway,
        // one entry at a time.
        //
        // The old cost was not a small constant. A fold writing n keys
        // called `new Map(m)` n times, so it wrote n(n+1)/2 entries into
        // a `Map` to end up holding n. Driving the emitted helpers and
        // counting those writes: **1,000 copies / 500,500 entry writes**
        // for a thousand keys, **10,000 copies / 50,005,000** for ten
        // thousand — ten times the input for a hundred times the work.
        // Every prelude function that hands a map back is such a fold
        // (`mapOf`, `mapRemove`, `mapMerge`, `mapValues`), so the
        // quadratic was the whole of the map-building half of the
        // library.
        //
        // So a written map is a link in a chain, exactly as an appended
        // list is: O(1) to make, and flattened to a real `Map` the first
        // time anything reads it. The same fold measured the same way is
        // **1 copy / 1,000 entry writes**, **1 / 10,000** and **1 /
        // 100,000** — one flatten at any size, and one entry write per
        // key. The map the write was given is untouched, so the value
        // stays immutable in the way §14B.2's `remove` comment
        // describes.
        //
        // **What this does not buy, which is the half worth being exact
        // about.** The claim above is about a fold that writes n times
        // and reads at the end. A fold that reads *between* its writes —
        // a visited set, `examples/graph-traversal.zd` — flattens after
        // every read, so the link it writes next sits on a base that is
        // already a real `Map` and flattening copies it entire. That is
        // one copy per write, which is exactly what `new Map(m)` was
        // doing; the chain moves the copy from the write to the read and
        // does not remove it. Measured the same way, that shape is
        // **1,000 copies / 500,500** and **10,000 / 50,005,000** both
        // before and after: unchanged, and still quadratic.
        //
        // So the honest statement of what landed is *a build that
        // batches its reads became linear*, and not *insert became
        // O(1)*. Insert is O(1) to perform and the flatten it defers is
        // O(size of the map) charged to the next reader, which
        // amortises to O(1) per insert over a run of writes and to
        // nothing at all when every write is read. Removing that second
        // case needs a structure with no flatten in it — a HAMT — which
        // is #233's own stated fallback and is not what this is.
        // `depth.rs` measures both shapes so the difference cannot be
        // claimed away.
        "$mapSet" => ("const $mapSet = (m, k, v) => new $MapSet(m, k, v);\n", false),
        // The map's `$force`: flatten a chain of writes once, and cache
        // the real `Map` on the node that was asked for it.
        //
        // **Order is the whole of the correctness argument.** `keys`,
        // `values` and `mapKeyAt` are defined over a real `Map`'s
        // iteration order, and ECMA-262 makes that insertion order for
        // every kind of key. So the flatten has to reproduce, exactly,
        // what a run of `new Map(m).set(k, v)` produced: copy the base
        // in its own order, then apply the chained writes **oldest
        // first**. `set` on a key the copy already holds replaces the
        // value and leaves the key where it was; a key the copy does not
        // hold goes on the end. Applying newest-first would be the same
        // *map* and a different *order* — the newest write to a fresh
        // key would arrive before older ones — which is why the loop
        // below counts down through a stack collected on the way up
        // rather than writing as it walks.
        //
        // Iterative rather than recursive, because the chain is as long
        // as the number of writes; and it caches only on the node it was
        // asked about, because caching on every node would make each
        // cache a different map. Both are `$force`'s reasoning, and the
        // two functions are deliberately the same shape.
        //
        // `$MapSet` is a class rather than a tagged object literal for
        // the reason `$Bounce` is: no ZDeceptron value is ever an
        // instance of a class the emitter declares, so `instanceof`
        // cannot collide with a program's data.
        //
        // The five members are the whole of what a map is read through
        // outside `$mapAt` and `$mapKeyAt`, which force for themselves.
        // `size` is `length of` a map (§17.4.7 emits it as a field, not
        // a call). `get` and `has` are what a forced reader would reach
        // for. The iterator is `remove key from table`'s `[...m]` and
        // anything else that spreads a map. `toJSON` is the wire trip:
        // #204 is the same bug for `$Ap`, where a link reached
        // `wire.js`'s `encode` and was walked structurally into
        // `{"base":…,"item":…,"flat":null}`, and it is fixed here in
        // advance by the same means — `encode` consults `toJSON` first,
        // and a real `Map` is what this hands it.
        //
        // **`base` is dropped once `flat` exists, and that is a fix
        // rather than a tidy-up.** A forced node answers every question
        // out of `flat` and never reads `base` again, but holding the
        // field keeps the whole chain behind it alive — and every node in
        // that chain that was itself forced is holding a whole `Map`. A
        // program that reads a map between writes forces every link, so
        // the retained chain is n maps of average size n/2: measured on
        // the harness this branch's numbers come from, writing 20,000
        // keys with a read between each **exhausted a 6 GB heap**, where
        // the copying `$mapSet` — which returned a fresh `Map` referring
        // to nothing — peaked at **53 MB**. Nulling the field puts that
        // back to one live map, and it is sound because it is not a
        // mutation anyone can observe: the base *value* is untouched, and
        // what is dropped is this node's pointer at it.
        //
        // `$force` has the same shape and the same retention for lists,
        // and it is deliberately not changed here: it is a defect that
        // predates this branch and fixing it would rewrite the bundle of
        // every program that appends.
        "$mapForce" => (
            "class $MapSet {\n  \
             constructor(base, key, value) {\n    \
             this.base = base; this.key = key; this.value = value; this.flat = null;\n  \
             }\n  \
             get size() { return $mapForce(this).size; }\n  \
             get(k) { return $mapForce(this).get(k); }\n  \
             has(k) { return $mapForce(this).has(k); }\n  \
             [Symbol.iterator]() { return $mapForce(this)[Symbol.iterator](); }\n  \
             toJSON() { return $mapForce(this); }\n\
             }\n\
             const $mapForce = (m) => {\n  \
             if (!(m instanceof $MapSet)) return m;\n  \
             if (m.flat) return m.flat;\n  \
             const written = [];\n  \
             let node = m;\n  \
             while (node instanceof $MapSet && !node.flat) { written.push(node); node = node.base; }\n  \
             const out = new Map($mapForce(node));\n  \
             for (let i = written.length - 1; i >= 0; i -= 1) out.set(written[i].key, written[i].value);\n  \
             m.flat = out;\n  \
             m.base = null;\n  \
             return out;\n\
             };\n",
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
        // and every mutation builds a fresh `Map` rather than writing
        // into an old one, so a map that is still reachable still has
        // the keys it was built with, and a map that is not takes its
        // cache with it.
        //
        // Keyed on the **forced** map and not on whatever was passed in,
        // which is what keeps that soundness true now that `$mapSet`
        // hands back a write chain (#233). A chain node is mutable in
        // exactly one way — it fills in its `flat` the first time it is
        // read — so caching a key array against the node would be
        // caching against a value whose identity is not yet the map's.
        // `$mapForce` gives the same `Map` object every time, so the
        // cache is keyed on the thing whose order it describes.
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
             const $m = $mapForce(m);\n  \
             let ks = $mapKeys.get($m);\n  \
             if (ks === undefined) { ks = [...$m.keys()]; $mapKeys.set($m, ks); }\n  \
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
        // One shape, thirteen times. Each is the platform's function
        // behind the same finiteness gate `sqrt` and `power` sit behind,
        // so `None` means exactly "the answer is not a finite number" and
        // means it identically across the family.
        "$sin" => ("const $sin = (n) => $finite(Math.sin(n));\n", false),
        "$cos" => ("const $cos = (n) => $finite(Math.cos(n));\n", false),
        "$tan" => ("const $tan = (n) => $finite(Math.tan(n));\n", false),
        "$asin" => ("const $asin = (n) => $finite(Math.asin(n));\n", false),
        "$acos" => ("const $acos = (n) => $finite(Math.acos(n));\n", false),
        "$atan" => ("const $atan = (n) => $finite(Math.atan(n));\n", false),
        "$atan2" => (
            "const $atan2 = (y, x) => $finite(Math.atan2(y, x));\n",
            false,
        ),
        "$exp" => ("const $exp = (n) => $finite(Math.exp(n));\n", false),
        "$ln" => ("const $ln = (n) => $finite(Math.log(n));\n", false),
        "$log10" => ("const $log10 = (n) => $finite(Math.log10(n));\n", false),
        "$log2" => ("const $log2 = (n) => $finite(Math.log2(n));\n", false),
        "$hypotenuse" => (
            "const $hypotenuse = (a, b) => $finite(Math.hypot(a, b));\n",
            false,
        ),
        "$cbrt" => ("const $cbrt = (n) => $finite(Math.cbrt(n));\n", false),
        "$tanh" => ("const $tanh = (n) => $finite(Math.tanh(n));\n", false),
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
        //
        // It serves a text slot as well as `text of` (#297), which is what
        // makes `Text flag` and `Text (text of flag)` write the same word.
        // The two words are also `truth_word`'s, and a test below pins
        // this string against it so the Rust half and the JavaScript half
        // cannot answer differently.
        TEXT_OF_TRUTH => ("const $textOfTruth = (v) => (v ? 'yes' : 'no');\n", false),
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
        // --- the modal (#53) -------------------------------------------
        //
        // `Dialog showing`, both ways. Here rather than in `dom.js` for
        // the reason the two fields above are: the shipped runtime is
        // against `zdc-bench`'s size gate, and a program that writes no
        // modal must not carry the machinery for one.
        //
        // **`n.open`, never a remembered flag.** `showModal()` throws
        // `InvalidStateError` on a dialog that is already open and
        // `close()` on a closed one is a no-op, so the only sound question
        // is what the DOM is doing. A cached copy of the last write is
        // wrong the instant Escape closes the dialog without asking, which
        // is the case this element exists for.
        //
        // **The `close` listener is the other half of that.** Escape, the
        // browser's own close request and a `close()` this effect caused
        // all arrive as one `close` event. Writing `false` back is what
        // keeps the program and the DOM agreeing; without it the signal
        // stays `true`, the effect sees no change on the next click, and
        // the button that opened the dialog stops working. The write is
        // idempotent by construction — `signal` compares with `Object.is`
        // and returns early — so the close this effect *caused* costs one
        // comparison and reaches nothing.
        //
        // **The deferral is not caution, it is the load path.** Every
        // binding runs while the tree is still a clone of a `<template>`,
        // before `mount` or an `ifInto` insertion connects it, and
        // `showModal()` throws on a node that is not in the document. So a
        // dialog whose signal *starts* true would throw at load and stop
        // module evaluation before the view is ever attached — #205's
        // failure shape. `queueMicrotask` runs after the synchronous task
        // that did the inserting, and the callback asks again rather than
        // trusting what was true when it was queued: the signal may have
        // been written back to false in between.
        //
        // Focus is absent from this helper, and that is the design.
        // Moving focus in, trapping it, making the rest of the page inert
        // and **returning focus to whatever opened it** are all
        // `showModal()`'s own behaviour. `zdc-cli/tests/browser.rs` asks a
        // real browser whether it still does them.
        "$modal" => (
            "const $modal = (n, get, set) => {\n  \
             effect(() => {\n    \
             if (Boolean(get()) === n.open) return;\n    \
             if (n.open) { n.close(); return; }\n    \
             if (n.isConnected) n.showModal();\n    \
             else queueMicrotask(() => { if (get() && !n.open && n.isConnected) n.showModal(); });\n  \
             });\n  \
             on(n, 'close', () => set(false));\n\
             };\n",
            false,
        ),
        // --- the file picker (#47) -------------------------------------
        //
        // Here rather than in `dom.js` for the reason the two above are,
        // and with the same consequence: `elements.js` states both rules
        // again in its own words, and the two copies are pinned by
        // `element_parity.rs` on the shape and `vocabulary.rs` on the
        // behaviour.
        //
        // The name of the file a reader chose, out of the `FileList` the
        // browser puts on the input. **`?.` and `??` are deliberately not
        // used**: `zdc-bench` measures the emitted preamble, the guard is
        // one comparison either way, and every other helper in this file
        // is written to the same plain subset.
        //
        // A `FileList` is empty in exactly the cases that mean nothing
        // was chosen: before the first pick, and after the program
        // cleared the control. A cancelled dialog fires no `change` at
        // all, so a reader who opens the picker and thinks better of it
        // leaves the previous choice standing — which is what the browser
        // does and what a `cancel` event, were this element to grow one,
        // would be needed to observe.
        "$chosenName" => (
            "const $chosenName = (files) =>\n  \
             files && files.length > 0 ? variant('Some', files[0].name) : variant('None');\n",
            true,
        ),
        // The other direction, which is a *clear* and not a write.
        //
        // **No script may put a file into a file picker.** The DOM refuses
        // any assignment to `value` but the empty string, so this is the
        // whole of what the write half of the binding can do: `None`
        // empties the control, and a `Some` leaves it alone because there
        // is no way to make the control show a file the reader did not
        // choose.
        //
        // What that buys is the one disagreement a program can cause. A
        // handler that writes `None` after an upload — the ordinary way a
        // form resets — would otherwise leave last week's file named in
        // the control under a program that believes nothing is chosen.
        // What it does not buy is the reverse: a `Some` the program
        // invented names a file the picker has never held, and no
        // diagnostic anywhere says so. `elements.rs`'s `Slot::Chosen`
        // records that as the limitation it is.
        //
        // The guard is on the control's own emptiness rather than on the
        // name, because the two are not comparable: the control's `value`
        // is a fake path (`C:\\fakepath\\report.csv`) that browsers have
        // reported for twenty years, and testing it against a name would
        // clear a control that agreed with the signal.
        "$fileField" => (
            "const $fileField = (n, get) =>\n  \
             effect(() => {\n    \
             if (get().tag === 'None' && n.value !== '') n.value = '';\n  \
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

    use crate::js;

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
            let module = foreign.module().unwrap_or_else(|| {
                panic!(
                    "`{}` is declared as a method, and the primitive layer has none",
                    foreign.name.text
                )
            });
            let form = intrinsic(module, foreign.export.as_str()).unwrap_or_else(|| {
                panic!(
                    "`{}` comes from `{}` as `{}`, which has no JavaScript form",
                    foreign.name.text, module, foreign.export
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
        // 28 before `prelude/math.zd`, which added fourteen: the circular
        // family and its inverses, `exp` and three logarithms, `cbrt`,
        // `hypotenuse` and `hyperbolicTangent`. Every one is the platform's
        // behind the same finiteness gate `sqrt` and `power` sit behind.
        assert_eq!(
            scanned, 42,
            "the primitive layer changed size; every one needs a JavaScript form"
        );
    }

    /// The words a `Truth` is shown in are spelled twice — once in Rust,
    /// for a truth written down and folded into the markup, and once in
    /// JavaScript, for one that is not known until the page runs (#297).
    /// Two copies of one rule is how one of them gets corrected alone, so
    /// this is the pin that makes them one.
    #[test]
    fn the_two_halves_of_a_truths_word_agree() {
        let source = helper(TEXT_OF_TRUTH).expect("a source").0;
        // Quoted by `js::string` rather than by writing an apostrophe
        // beside a placeholder here. `check-emitted-strings.sh` refuses
        // that shape anywhere in an emitter source and does not care that
        // this one is an assertion rather than an emission — nor should
        // it, since it cannot tell them apart and the whole point is that
        // the compiler owns its quoting in one place. Asking the same
        // function the emitter uses also makes this a stronger pin: a
        // change to how a string is quoted moves both sides together.
        let yes = js::string(truth_word(true));
        let no = js::string(truth_word(false));
        assert!(
            source.contains(yes.as_str()),
            "the helper does not say `{}`: {source}",
            truth_word(true)
        );
        assert!(
            source.contains(no.as_str()),
            "the helper does not say `{}`: {source}",
            truth_word(false)
        );
        // The order matters as much as the pair: a helper reading
        // `(v ? 'no' : 'yes')` would satisfy both assertions above and
        // answer every question backwards.
        assert!(
            source.contains(&format!("{yes} : {no}")),
            "the helper's two arms are the wrong way round: {source}"
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
