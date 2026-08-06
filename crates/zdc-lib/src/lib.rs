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
    ("prelude/option.zd", include_str!("../prelude/option.zd")),
    ("prelude/remote.zd", include_str!("../prelude/remote.zd")),
    ("prelude/list.zd", include_str!("../prelude/list.zd")),
    ("prelude/map.zd", include_str!("../prelude/map.zd")),
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
        | zdc_ast::Decl::Use(_) => return None,
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
    #[test]
    fn the_prelude_declares_exactly_these_operations() {
        assert_eq!(
            load().names(),
            [
                "abs",
                "after",
                "afterLast",
                "allFrom",
                "allOf",
                "anyFrom",
                "anyOf",
                "atOr",
                "before",
                "beforeLast",
                "bitAnd",
                "bitOr",
                "bitXor",
                "clamp",
                "clock",
                "copyFrom",
                "countFrom",
                "countOf",
                "decimalOf",
                "decimalOr",
                "dropFirst",
                "endsWith",
                "equalsIgnoringCase",
                "exactWhole",
                "filled",
                "filledFrom",
                "first",
                "flatten",
                "flattenFrom",
                "floor",
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
                "keyOfFrom",
                "keyOfOr",
                "keys",
                "keysFrom",
                "largerOf",
                "last",
                "lines",
                "listAt",
                "listContains",
                "listContainsFrom",
                "listDrop",
                "listLength",
                "listTake",
                "listTakeFrom",
                "lowercase",
                "mapAt",
                "mapContains",
                "mapKeyAt",
                "mapLength",
                "max",
                "maxFrom",
                "maxOf",
                "min",
                "minFrom",
                "minOf",
                "mixA",
                "mixB",
                "mixC",
                "mod",
                "newline",
                "nextSeed",
                "padEnd",
                "padStart",
                "parseDecimal",
                "parseWhole",
                "quotient",
                "randomBelow",
                "randomBits",
                "randomDecimal",
                "range",
                "rangeFrom",
                "readyOr",
                "removeAt",
                "repeat",
                "repeatFrom",
                "replace",
                "rest",
                "reverse",
                "reverseFrom",
                "round",
                "shiftLeft",
                "shiftRight",
                "slice",
                "sliceStep",
                "smallerOf",
                "split",
                "startsWith",
                "sumFrom",
                "sumOf",
                "textAt",
                "textContains",
                "textLength",
                "toUnsigned32",
                "trim",
                "unlines",
                "uppercase",
                "valueOr",
                "values",
                "valuesFrom",
                "wholeOr",
                "withoutDuplicates",
                "withoutDuplicatesFrom",
                "withoutPrefix",
                "withoutSuffix",
                "wrappingProduct",
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
        // Twenty-two. Twenty-one are here for a reason that is a fact
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
        //   bitAnd, bitOr,       the same test, one level down: a `Whole`
        //   bitXor, shiftLeft,   is an f64 and the language gives no way
        //   shiftRight,          to observe its bits. A ZDeceptron
        //   wrappingProduct      definition would have to take a number
        //                        apart through `mod` at thirty-two frames
        //                        per operation *and* would still not
        //                        reproduce 32-bit wraparound, which is not
        //                        a cost but an impossibility
        //   clock                reads the platform
        //
        // The twenty-second is `split`, and it is the only one whose reason
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
        // What is left is the honest form of a claim this layer nearly
        // got to make. **Exactly one primitive returns a collection**, it
        // is `split`, and it is kept by measurement rather than by
        // argument — which is a weaker statement than "none does" and the
        // only one the tests support.
        //
        // `newline` was the one that had moved for neither reason: it
        // built nothing, it was simply unspellable, and §17.4.10(e) named
        // the lexer's `"[^"\n]*"` as the debt behind it. The `"""` block
        // literal pays that debt without the string escapes §17.4.10(e)
        // costed as the alternative — a block takes its lines from the
        // source, so a body of two empty lines is one line break — and
        // `newline` is an ordinary ZDeceptron function now. It is the
        // twenty-second primitive leaving, and it left for a third
        // reason: the thing it could not spell became spellable.
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
        assert_eq!(foreign, 22, "the primitive layer changed size");
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
            assert!(
                foreign.module.starts_with("zd:"),
                "`{}` comes from `{}`, which is not part of the language",
                foreign.name.text,
                foreign.module
            );
        }
        assert_eq!(scanned, 22, "the primitive layer changed size");
    }
}
