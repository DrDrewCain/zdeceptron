#![forbid(unsafe_code)]

//! The prelude: ZDeceptron's standard library, written in ZDeceptron.
//!
//! Spec §14F found that the language defined eight types and not one
//! operation on any of them, and §14F.2 settled how that gap is closed:
//! **in the language, not by importing JavaScript.** A language whose
//! `List` operations are JavaScript calls has not defined its own
//! semantics, it has borrowed them along with every coercion rule it
//! exists to escape.
//!
//! §17.4.1 makes the library a compilation unit rather than a set of
//! compiler built-ins. The sources below are parsed and resolved into the
//! *same arenas* as the program being compiled, so a reference to
//! `valueOr` is an ordinary `Res::Def` and no pass after resolution needs
//! any rule at all for the fact that some definitions were not written by
//! the programmer. Three things fall out of that rather than needing to be
//! arranged:
//!
//! * dead-code elimination is the closure walk codegen already does, so a
//!   program that never calls `join` does not ship it;
//! * a type error in a call to a library function is reported by the same
//!   code that reports one in a call to the program's own function, at the
//!   argument's own span; and
//! * the library is checked by the compiler that compiles it, on every
//!   build, so it cannot drift from the language.
//!
//! **The placement invariant.** No prelude declaration is a `state` or
//! mentions one — the whole library is colourless. That is asserted, not
//! assumed: [`Prelude::load`] walks every declaration and panics if one
//! ever is. It is what makes a library call unable to add an edge to the
//! signal graph, and therefore unable to change any placement fact.

use std::sync::OnceLock;

/// The prelude's sources, in the order they are compiled.
///
/// Order is fixed rather than alphabetical because a reader should meet
/// the primitives before the things built on them. It has no effect on
/// meaning: top-level declarations are order-independent, which is what
/// `collect` is a separate pass for.
pub const SOURCES: &[(&str, &str)] = &[
    ("prelude/text.zd", include_str!("../prelude/text.zd")),
    ("prelude/number.zd", include_str!("../prelude/number.zd")),
    ("prelude/math.zd", include_str!("../prelude/math.zd")),
    ("prelude/option.zd", include_str!("../prelude/option.zd")),
    ("prelude/remote.zd", include_str!("../prelude/remote.zd")),
    ("prelude/list.zd", include_str!("../prelude/list.zd")),
    ("prelude/map.zd", include_str!("../prelude/map.zd")),
    ("prelude/encode.zd", include_str!("../prelude/encode.zd")),
    ("prelude/time.zd", include_str!("../prelude/time.zd")),
];

/// The parsed library, as one program.
pub struct Prelude {
    program: zdc_ast::Program,
}

impl Prelude {
    /// Every prelude declaration, as a single program to resolve against.
    pub fn program(&self) -> &zdc_ast::Program {
        &self.program
    }

    /// Every name the prelude declares, sorted.
    ///
    /// An editor offers these alongside the program's own names, and the
    /// snapshot test that pins this list is what stops an operation
    /// disappearing from the library without anybody noticing.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .program
            .decls
            .iter()
            .filter_map(declared_name)
            .collect();
        names.sort_unstable();
        names
    }
}

/// The prelude, parsed once per process.
///
/// A parse failure here is a defect in this crate's own sources, not in
/// anything a user wrote, so it aborts rather than producing a diagnostic:
/// there is no file to point at that the programmer could edit.
pub fn load() -> &'static Prelude {
    static PRELUDE: OnceLock<Prelude> = OnceLock::new();
    PRELUDE.get_or_init(|| {
        let mut decls = Vec::new();
        for (path, source) in SOURCES {
            match zdc_parser::parse(source) {
                Ok(program) => decls.extend(program.decls),
                Err(error) => panic!("{path} does not parse: {}", error.message),
            }
        }
        let program = zdc_ast::Program { decls };
        assert_colourless(&program);
        Prelude { program }
    })
}

/// §17.4.1 step 6, checked rather than assumed.
///
/// A `state` in the prelude would put the library into the signal graph,
/// and from there into placement, information flow, and every cycle check
/// that reads it. One walk over the declarations is the whole cost of
/// knowing it never happens.
fn assert_colourless(program: &zdc_ast::Program) {
    for decl in &program.decls {
        match decl {
            zdc_ast::Decl::State(state) => {
                panic!(
                    "the prelude declares `state {}`, which would give the library a placement",
                    state.name.text
                )
            }
            zdc_ast::Decl::View(_) => panic!("the prelude declares a view"),
            // The library is the thing every program's tests are written
            // *against*, so a claim inside it would be checked by every
            // `zdc test` run in the world and owned by none of them
            // (issue #169). The prelude's own behaviour is pinned by the
            // compiler's Rust tests, which is where a library's tests
            // belong.
            zdc_ast::Decl::Test(test) => panic!(
                "the prelude declares `test {:?}`, and a library's claims are not every \
                 program's to run",
                test.claim
            ),
            zdc_ast::Decl::Route(route) => panic!(
                "the prelude declares `route {}`, which would put URLs in the library",
                route.name.text
            ),
            // A release declassifies, and the prelude has nothing to
            // declassify: it has no state, so no secret can reach it.
            zdc_ast::Decl::Release(release) => panic!(
                "the prelude declares `release {}`, and the library has no secrets to release",
                release.name.text
            ),
            // A request is a client signal that leaves the machine, so it
            // has a placement for the reason `state` does — and the
            // library must have none.
            zdc_ast::Decl::Request(request) => panic!(
                "the prelude declares `request {}`, which would give the library a placement",
                request.name.text
            ),
            // Spelled out rather than wildcarded so that a new kind of
            // declaration has to be ruled on here rather than silently
            // admitted into the library.
            zdc_ast::Decl::Function(_)
            | zdc_ast::Decl::Foreign(_)
            | zdc_ast::Decl::Record(_)
            | zdc_ast::Decl::Choice(_)
            | zdc_ast::Decl::Component(_)
            | zdc_ast::Decl::Use(_) => {}
        }
    }
}

fn declared_name(decl: &zdc_ast::Decl) -> Option<&str> {
    Some(match decl {
        zdc_ast::Decl::Function(function) => &function.name.text,
        zdc_ast::Decl::Foreign(foreign) => &foreign.name.text,
        zdc_ast::Decl::Record(record) => &record.name.text,
        zdc_ast::Decl::Choice(choice) => &choice.name.text,
        // The prelude declares none of these — `assert_colourless` rejects
        // a `state` and a `view` outright, and the library neither defines
        // components nor imports anything — so none contributes a name.
        zdc_ast::Decl::State(_)
        | zdc_ast::Decl::View(_)
        | zdc_ast::Decl::Route(_)
        | zdc_ast::Decl::Component(_)
        | zdc_ast::Decl::Release(_)
        | zdc_ast::Decl::Request(_)
        | zdc_ast::Decl::Use(_)
        | zdc_ast::Decl::Test(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The library's whole surface, pinned.
    ///
    /// Not a formality: an operation that silently stops being declared
    /// takes every program that used it with it, and the failure would
    /// otherwise show up as "`join` is not defined" in somebody else's
    /// file rather than here.
    ///
    /// It runs the other way too, and that is the more expensive
    /// direction. A prelude name is ambient in every module (§17.4.1) and
    /// lives in the ordinary namespace, so a program that declares one is
    /// refused for shadowing a library name it cannot see. Adding a name
    /// here therefore takes a word away from every program that will ever
    /// be written, which is the same accounting §14G.7.7 applies to a
    /// keyword and the reason the date layer added below is seven names
    /// and not the fourteen a `yearOf`/`monthOf`/`dayOfMonthOf` surface
    /// would have cost.
    ///
    /// `CivilDate` and `CivilTime` are the first types the library
    /// declares. They are spelled with the "civil" of the calendar
    /// literature rather than as `Date` and `Time`, which are the two
    /// nouns a program is most likely to want for a record of its own.
    #[test]
    fn the_prelude_declares_exactly_these_operations() {
        assert_eq!(
            load().names(),
            [
                "CivilDate",
                "CivilTime",
                "abs",
                "acos",
                "added",
                "after",
                "afterLast",
                "allFrom",
                "allOf",
                "angleBetween",
                "angleDelta",
                "anyFrom",
                "anyOf",
                "applied",
                "asin",
                "atOr",
                "atan",
                "atan2",
                "axis",
                "base64Encoded",
                "bearing",
                "before",
                "beforeLast",
                "bitAnd",
                "bitOr",
                "bitXor",
                "cbrt",
                "civilDateOf",
                "civilTimeOf",
                "clamp",
                "clamped",
                "clamped01",
                "clock",
                "columnCount",
                "columnOf",
                "copyFrom",
                "cos",
                "countOf",
                "cross2",
                "cross3",
                "dayOf",
                "decimalOf",
                "decimalOr",
                "degrees",
                "distance",
                "dot",
                "dropFirst",
                "easeIn",
                "easeOut",
                "emptyRow",
                "endsWith",
                "entries",
                "entriesFrom",
                "equalsIgnoringCase",
                "eulerNumber",
                "exactWhole",
                "exp",
                "filled",
                "filledFrom",
                "first",
                "fixedText",
                "flatten",
                "flattenOption",
                "flattenRemote",
                "floor",
                "fromAngle",
                "groupFrom",
                "groupedDigits",
                "groupedText",
                "hyperbolicTangent",
                "hypotenuse",
                "indexOf",
                "indices",
                "indicesFrom",
                "insertAt",
                "isBlank",
                "isEmpty",
                "isNone",
                "isReady",
                "isSome",
                "join",
                "joinAllButLast",
                "joinFrom",
                "joinUntil",
                "jsonEncoded",
                "keyOfFrom",
                "keyOfOr",
                "keys",
                "keysFrom",
                "largerOf",
                "last",
                "leadingGroup",
                "leakyRectified",
                "lines",
                "listAt",
                "listContains",
                "listContainsFrom",
                "listDrop",
                "listLength",
                "listTake",
                "listTakeFrom",
                "ln",
                "log10",
                "log2",
                "lowercase",
                "magnitude",
                "magnitudeSquared",
                "mapAt",
                "mapContains",
                "mapKeyAt",
                "mapLength",
                "mapMerge",
                "mapMergeFrom",
                "mapOf",
                "mapOfFrom",
                "mapRemove",
                "mapRemoveFrom",
                "mapValues",
                "matrixAdded",
                "matrixProduct",
                "matrixScaled",
                "max",
                "maxOf",
                "mean",
                "min",
                "minOf",
                "mix",
                "mixA",
                "mixB",
                "mixC",
                "mod",
                "momentOf",
                "moneyText",
                "newline",
                "nextSeed",
                "normalized",
                "normalizedExponentials",
                "numberText",
                "overlap",
                "padEnd",
                "padStart",
                "parseDecimal",
                "parseWhole",
                "pi",
                "power",
                "progress",
                "projected",
                "queryFrom",
                "queryPart",
                "queryText",
                "quotient",
                "radians",
                "randomBelow",
                "randomBits",
                "randomDecimal",
                "range",
                "rangeFrom",
                "readyOr",
                "rectified",
                "reflected",
                "removeAt",
                "repeat",
                "repeatFrom",
                "replace",
                "rest",
                "reverse",
                "reverseFrom",
                "rotated2",
                "round",
                "rowCount",
                "rowOf",
                "scaled",
                "setAt",
                "shiftLeft",
                "shiftRight",
                "shiftedExponentials",
                "sigmoid",
                "sin",
                "slice",
                "sliceStep",
                "smallerOf",
                "smoothStep",
                "smootherStep",
                "softmax",
                "split",
                "sqrt",
                "squaredDeviations",
                "standardDeviation",
                "startsWith",
                "subtracted",
                "sumOf",
                "tan",
                "tau",
                "textAt",
                "textContains",
                "textLength",
                "toUnsigned32",
                "transposed",
                "trim",
                "unlines",
                "uppercase",
                "urlEncoded",
                "valueOr",
                "values",
                "valuesFrom",
                "variance",
                "weekdayOf",
                "wholeOr",
                "withoutDuplicates",
                "withoutDuplicatesFrom",
                "withoutPrefix",
                "withoutSuffix",
                "wrapAngle",
                "wrappingProduct",
                "zip",
                "zipFrom",
            ]
        );
    }

    /// §14F.2's requirement, counted. Every operation that *can* be
    /// written in ZDeceptron *is*, and the platform is reached for only
    /// where §17.4.10 shows the language cannot express the operation at
    /// all. If this ratio moves the wrong way, something was taken to the
    /// FFI to save effort.
    #[test]
    fn most_of_the_library_is_written_in_zdeceptron() {
        let prelude = load();
        let foreign = prelude
            .program()
            .decls
            .iter()
            .filter(|decl| matches!(decl, zdc_ast::Decl::Foreign(_)))
            .count();
        let written = prelude
            .program()
            .decls
            .iter()
            .filter(|decl| matches!(decl, zdc_ast::Decl::Function(_)))
            .count();
        // Twenty-eight. Twenty-seven are here for a reason that is a fact
        // about the language rather than an inconvenience:
        //
        //   textLength, textAt   there is no way to inspect a `Text` from
        //                        inside the language, so nothing can take
        //                        one apart
        //   uppercase, lowercase Unicode case mapping is a table, not a
        //                        rule
        //   trim                 Unicode's whitespace set is a table
        //                        too, and `trim` has to name every
        //                        character in it; a literal spells only
        //                        the ones a source file can hold
        //   listLength, listAt,  all O(1) on the platform, and all
        //   mapLength            writable now and deliberately not
        //                        written: `listAt` is what §17.4.3
        //                        dispatches `at` to, and `mapLength` as a
        //                        walk to the end would turn every
        //                        `length of` a map linear to remove one
        //                        `foreign`
        //   mapAt, mapKeyAt      a map cannot be taken apart from inside
        //                        the language; `mapAt` answers about a key
        //                        you have, `mapKeyAt` gives a fold
        //                        something to walk
        //   floor, round,        statements about the f64 representation
        //   decimalOf            §14A.3 chose, which the language gives no
        //                        way to observe
        //   parseDecimal         the same statement read backwards: a
        //                        parse has to weigh the digits of a `Text`
        //                        into that representation, and the
        //                        language can observe neither. `textAt`
        //                        gives back a one character `Text`, not
        //                        the number that character is, so nothing
        //                        in the language can even start; a
        //                        definition that got there by comparing
        //                        against ten literals would be a second,
        //                        differently rounded answer to a question
        //                        the platform's parser already answers.
        //                        `parseWhole`, `wholeOr` and `decimalOr`
        //                        are written above it, so the library has
        //                        one parser rather than two
        //   sqrt                 the platform's square root is correctly
        //                        rounded; a Newton iteration in ZDeceptron
        //                        would be a second answer to a question
        //                        that already has one, differing in the
        //                        last bit from every other tool that
        //                        touches the same data
        //   power                unwritable rather than merely worse:
        //                        repeated multiplication reaches
        //                        `exponent is 10` and says nothing about
        //                        `exponent is 0.5`, which is a root, and a
        //                        root needs the exponential and the
        //                        logarithm the language does not have
        //   fixedText            `text of` is the platform's shortest
        //                        round-tripping printer and gives no
        //                        control over digits; a fixed-point
        //                        printer written in ZDeceptron would have
        //                        to read the digits of an f64, which is
        //                        the observation §14A.3 denies. Note what
        //                        is *not* here: no `Intl`. A prelude
        //                        primitive is `is anywhere`, and §17.4.8
        //                        runs the build root in an engine with no
        //                        `Intl` in it, so that claim would be
        //                        false at one of the three roots.
        //                        Grouping and currency are folds over what
        //                        this returns, written in ZDeceptron
        //   bitAnd, bitOr,       the same test, one level down: a `Whole`
        //   bitXor, shiftLeft,   is an f64 and the language gives no way
        //   shiftRight,          to observe its bits. A ZDeceptron
        //   wrappingProduct      definition would have to take a number
        //                        apart through `mod` at thirty-two frames
        //                        per operation *and* would still not
        //                        reproduce 32-bit wraparound, which is not
        //                        a cost but an impossibility
        //   urlEncoded,          one reason for all three: each is a
        //   jsonEncoded,         statement about the *bytes* of a `Text`,
        //   base64Encoded        and the language cannot observe a byte or
        //                        even a code point's number. `textAt`
        //                        gives back a one character `Text`, not
        //                        the number that character is, so
        //                        percent-encoding cannot reach the UTF-8
        //                        encoding of a character, base64 cannot
        //                        reach its bytes six at a time, and JSON's
        //                        escape for a control character cannot
        //                        reach its code point. §17.4.10 named
        //                        "inspecting a `Text`" already; this is
        //                        that finding one level down. `queryPart`
        //                        and `queryText` are written above them
        //   clock                reads the platform
        //
        // The twenty-eighth is `split`, and it is the only one whose reason
        // is a number. Read `prelude/text.zd` and `zdc-codegen/intrinsics.rs`
        // for it in full; in short, it *can* be written in ZDeceptron and
        // was, and the delimiter family over a ten-thousand character
        // document went from milliseconds to 416 seconds against a
        // twenty-second budget, because every document-scale text
        // operation goes through it. It is a platform call again.
        //
        // It was eighteen, and the three that left — `reverse`, `keys` and
        // `values` — were all here for one reason: each returns a
        // collection, and the language could not build one. `append item
        // to list` is that construct, so "returns a collection" stopped
        // being sufficient on its own.
        //
        // §17.4.10 predicted eight would move and named these among them.
        // It was right about them and wrong about the cause: it expected
        // local bindings and `rest of` to be enough, and they were not,
        // because both take a list apart. What was missing was a way to
        // put one together.
        //
        // `keys` needed one thing more: out went a primitive handing back
        // a whole `List of K`, in came `mapKeyAt`, handing back one
        // `Option of K`. So the net of that trade is minus two, not minus
        // three, and a program that visits every entry of a map is written
        // in ZDeceptron rather than routed through the FFI.
        //
        // **`entries`, `mapOf`, `mapRemove`, `mapMerge`, `mapValues` and
        // `zip` arrived without moving this number at all**, which is the
        // claim worth checking rather than the count. Every one of them
        // hands back a collection, and "returns a collection" was the
        // reason each of them was unwritable; what closed the gap was two
        // language forms and no platform call. `set key to value in
        // table` is the map's `append`, and `Pair of K to V` is the
        // return type `zip` and `entries` had no way to name. §17.7
        // records that wall, for `bothOf` rather than for these, and it
        // was one missing type rather than three missing functions. A
        // `foreign pairOf` and a `foreign mapSet` would have bought the
        // same six functions for two more primitives; a form the language
        // owns is checked by the type checker and folded by the emitter,
        // and a `foreign` is neither.
        //
        // What is left is the honest form of a claim this layer nearly
        // got to make. **Exactly one primitive returns a collection**, it
        // is `split`, and it is kept by measurement rather than by
        // argument — which is a weaker statement than "none does" and the
        // only one the tests support.
        //
        // `newline` was the one that had moved for neither reason: it
        // built nothing, it was simply unspellable, and §17.4.10(e) named
        // the lexer's `"[^"\n]*"` as the debt behind it. The `"""` block
        // literal paid that debt — a block takes its lines from the
        // source, so a body of two empty lines is one line break — and
        // `newline` became an ordinary ZDeceptron function. It is the
        // twenty-second primitive leaving, and it left for a third
        // reason: the thing it could not spell became spellable.
        //
        // String escapes landed afterwards (#16), which is the other
        // repair §17.4.10(e) costed, and they change this count by
        // nothing: `newline` had already left, and it stays a name for a
        // reason its declaration in `prelude/text.zd` records. What they
        // change is its body, which is now `"\n"`.
        //
        // Six bitwise and not seven: `bitNot` is
        // `bitXor with left is x, right is 4294967295`, and §4.1 refuses a
        // second spelling of one operation.
        //
        // Nothing else in the numeric library is a primitive, and that is
        // the load-bearing part of this count. `quotient` and `mod` are
        // `floor of (value / divisor)` and its remainder; `nextSeed`,
        // `randomBits`, `randomBelow` and `randomDecimal` are mulberry32
        // written out in ZDeceptron. The language acquired randomness
        // without acquiring a source of entropy, so §17.4.7's argument
        // against a random seed never has to be reopened.
        //
        // The date layer (#118) added none, which is worth recording
        // because a date library is where a standard library usually
        // reaches for the platform. `civilDateOf` and `momentOf` are
        // Howard Hinnant's civil-calendar arithmetic over `quotient` and
        // `mod`, so no `Date` is constructed anywhere and no answer here
        // depends on the host's locale or time zone. `clock` was already
        // counted, and it stayed one primitive rather than becoming a
        // family: it reports a moment and every question about that
        // moment is answered in ZDeceptron.
        //
        // ### Randomness, and where it is allowed
        //
        // The rule, which is about entropy rather than about a function:
        // **ZDeceptron has no unseeded source of randomness and the
        // library may not give it one.** Every random value is a pure
        // function of a seed the program owns. That is what lets the
        // generator be written in ZDeceptron at all, what keeps a `static`
        // value the same in two builds (§17.4.8), and what leaves
        // §17.4.7's argument against a random seed closed. Where the seed
        // comes from is the program's own decision, and the only entropy
        // the language offers is `clock`, whose placement is where a
        // freshness decision gets written down (`prelude/time.zd`).
        //
        // The alternative, and why it was refused: declare
        // `foreign random … as "random"` over `Math.random`, and confine
        // it the way `clock` is said to be confined, since an unseeded
        // read from a derived signal has exactly `clock`'s staleness
        // shape. Refused on two grounds.
        //
        // The first is that a seeded generator makes the confinement
        // unnecessary rather than merely unenforced. Purity here is a
        // property the declaration states and the flow pass already reads
        // (`ForeignGrant::Pure`, `zdc-graph/src/integrity.rs`), so nothing
        // new has to be checked and no new region rule has to exist.
        //
        // The second is measured rather than argued: **that confinement
        // does not exist.** Checked on this tree, not inherited: a program
        // whose ordinary function body computes `clock + offset` passes
        // `zdc check` with exit 0, and so does one that writes
        // `set stamped to clock` inside an `on click` handler. §17.4.9's
        // sentence, which `prelude/time.zd` repeats, is a statement about
        // the specification; no pass in this compiler enforces it.
        // "Confine it the way `clock` is confined" would have confined it
        // the way nothing is confined.
        //
        // What does enforce the rule is
        // `the_prelude_declares_exactly_one_impure_primitive` below. An
        // entropy source is impure, an impure `foreign` is one that omits
        // `gives pure`, and the library is allowed exactly one of those.
        //
        // ### What belongs above the primitives
        //
        // Each of the twenty-one above carries its own reason, and the
        // layer written in ZDeceptron on top of them carried none, while
        // the tracker collected about twenty proposed additions and nine
        // of them landed at once. Without a rule the library grows by
        // whoever asks. The rule is four questions, in this order, and a
        // proposal has to answer all four.
        //
        // **1. Can the language already say it?** §4.1 admits one phrasing
        // per construct, and that binds the library exactly as hard as it
        // binds the grammar. `bitNot` is refused because it is `bitXor`
        // against 4294967295. A `filterBy` taking a predicate is refused
        // because `keep each` is that phrase already. This question is
        // first because it disposes of most proposals.
        //
        // **2. Can it be written here at all**, in ZDeceptron, over the
        // primitives, with constructs the language has? If not, the
        // proposal is a language change wearing a library issue's clothes,
        // and it belongs on the language board until the construct exists.
        // #103 and #104 are the standing example: `map` over an `Option`
        // needs a function as a value, and §17.2.5 keeps the reachability
        // graph exact by not having one.
        //
        // **3. Does it decide something the caller has to decide?** A
        // partial operation gives an `Option` and stops there.
        // `randomBelow` hands back `mod`'s `Option` rather than choosing a
        // number for an empty range; `minOf` gives `None` rather than a
        // sentinel. A library function that picks the fallback has made
        // the program's decision silently, which is the failure §5.4
        // exists to stop.
        //
        // **4. Does it cost what the hand-written form costs?** Written
        // here it must be no worse asymptotically than the obvious program
        // a user would write instead. This is the question that keeps
        // `listLength` and `listAt` primitive, that keeps every fold in
        // `list.zd` on an index and an accumulator rather than on
        // `rest of`, and that sent `split` back to the platform after a
        // measurement rather than after an argument.
        //
        // One thing that is deliberately not a question: **whether anybody
        // asked.** A name that answers all four is admitted with or
        // without an issue behind it; a name that fails one is refused
        // however many times it is proposed.
        //
        // Two consequences, stated because they are already visible above:
        //
        // * **The library has no privacy.** `minFrom` and `smallerOf` are
        //   as public as `minOf`, because a declaration is a declaration.
        //   A proposal needing three helpers adds four names to
        //   `the_prelude_declares_exactly_these_operations`, and that is
        //   part of what it costs.
        // * **The library is colourless**, and `assert_colourless` refuses
        //   a `state`, a `view`, a `route` and a `release` to keep it so.
        //   That is not a rule about what belongs; it is the reason a call
        //   into the library can never change a placement fact.
        assert_eq!(foreign, 42, "the primitive layer changed size");
        assert!(
            written > foreign,
            "{written} written in ZDeceptron against {foreign} primitives"
        );
    }

    /// Every primitive names the language's own layer rather than a
    /// package. A `foreign` reaching a real module would make the library
    /// depend on the platform's package manager, which §14F.2 rules out.
    #[test]
    fn every_primitive_is_part_of_the_language() {
        // Counted: the assertion is inside the loop, so a prelude that
        // declared no primitive at all would pass this over nothing.
        let mut scanned = 0;
        for decl in &load().program().decls {
            let zdc_ast::Decl::Foreign(foreign) = decl else {
                continue;
            };
            scanned += 1;
            assert_eq!(
                foreign.module().map(|module| module.starts_with("zd:")),
                Some(true),
                "`{}` comes from `{:?}`, which is not part of the language",
                foreign.name.text,
                foreign.module()
            );
        }
        assert_eq!(scanned, 42, "the primitive layer changed size");
    }

    /// **The randomness rule, enforced.** Exactly one prelude primitive is
    /// impure, and it is `clock`.
    ///
    /// `gives pure T` is a human's word about JavaScript the compiler
    /// cannot read (`zdc-ast`'s [`zdc_ast::ForeignGrant`]), so this test
    /// cannot catch a declaration that lies. What it catches is the honest
    /// case, which is the one that would actually happen: a source of
    /// entropy added to the library has to omit the marker, because
    /// claiming `pure` for it would be false and the flow pass would then
    /// let a `release` body reach it. So `foreign random … as "random"`
    /// fails here on the line it is written.
    ///
    /// That is the whole of what "the language acquired randomness without
    /// acquiring a source of entropy" is worth as a rule: nothing stops a
    /// program declaring its own `foreign`, and nothing here claims
    /// otherwise. This is about what the *library* is allowed to hand
    /// every program without being asked.
    #[test]
    fn the_prelude_declares_exactly_one_impure_primitive() {
        let impure: Vec<&str> = load()
            .program()
            .decls
            .iter()
            .filter_map(|decl| match decl {
                zdc_ast::Decl::Foreign(foreign)
                    if foreign.result_grant == zdc_ast::ForeignGrant::Opaque =>
                {
                    Some(foreign.name.text.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            impure,
            ["clock"],
            "the library's one impure primitive is `clock`; anything else here \
             is a source of entropy the language decided not to have"
        );
    }
}
