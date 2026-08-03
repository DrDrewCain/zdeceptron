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
            _ => {}
        }
    }
}

fn declared_name(decl: &zdc_ast::Decl) -> Option<&str> {
    Some(match decl {
        zdc_ast::Decl::Function(function) => &function.name.text,
        zdc_ast::Decl::Foreign(foreign) => &foreign.name.text,
        zdc_ast::Decl::Record(record) => &record.name.text,
        zdc_ast::Decl::Choice(choice) => &choice.name.text,
        zdc_ast::Decl::State(_) | zdc_ast::Decl::View(_) => return None,
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
                "atOr",
                "charactersFrom",
                "clamp",
                "clock",
                "containsFrom",
                "copyFrom",
                "decimalOf",
                "dropFirst",
                "endsWith",
                "first",
                "floor",
                "isBlank",
                "isEmpty",
                "isNone",
                "isReady",
                "isSome",
                "join",
                "joinFrom",
                "keyOfFrom",
                "keyOfOr",
                "keys",
                "keysFrom",
                "last",
                "listAt",
                "listContains",
                "listContainsFrom",
                "listLength",
                "lowercase",
                "mapAt",
                "mapContains",
                "mapKeyAt",
                "mapLength",
                "max",
                "min",
                "readyOr",
                "rest",
                "reverse",
                "reverseFrom",
                "round",
                "slice",
                "sliceStep",
                "split",
                "splitFrom",
                "startsWith",
                "sumFrom",
                "sumOf",
                "textAt",
                "textContains",
                "textLength",
                "trim",
                "uppercase",
                "valueOr",
                "values",
                "valuesFrom",
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
        // Fourteen. It was eighteen, and the four that left — `split`,
        // `reverse`, `rest` and `values` — were all here for one reason:
        // each returns a collection, and the language could not build one.
        // `append item to list` is that construct, so "returns a
        // collection" is no longer a reason for anything to be `foreign`.
        //
        // §17.4.10 predicted eight would move and named these four among
        // them. It was right about the four and wrong about the cause: it
        // expected local bindings and `rest of` to be enough, and they
        // were not, because both take a list apart. What was missing was a
        // way to put one together.
        //
        // `keys` left too, and its replacement kept the count the same:
        // out went a primitive that hands back a whole `List of K`, in
        // came `mapKeyAt`, which hands back one `Option of K`. The number
        // is unchanged and the layer is not — **no primitive returns a
        // collection any more**, so nothing the platform builds is taken
        // on trust past its own boundary, and any program that has to
        // visit every entry of a map is now written in ZDeceptron rather
        // than routed through the FFI.
        //
        // The other four §17.4.10 named do not move, and each says
        // something different. `listLength`, `listAt` and `mapLength`
        // *could* be written now and must not be: all three are O(1) on
        // the platform, `listAt` is what §17.4.3 dispatches `at` to, and
        // `mapLength` written as a walk to the end would turn every
        // `length of` a map linear to remove one `foreign`. `join` and
        // `listContains`, the last two on that list, were already written
        // in ZDeceptron before any of this.
        assert_eq!(foreign, 14, "the primitive layer changed size");
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
        for decl in &load().program().decls {
            let zdc_ast::Decl::Foreign(foreign) = decl else {
                continue;
            };
            assert!(
                foreign.module.starts_with("zd:"),
                "`{}` comes from `{}`, which is not part of the language",
                foreign.name.text,
                foreign.module
            );
        }
    }
}
