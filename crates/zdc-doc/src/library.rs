//! The prelude, linked as though it were a program, so that it can be
//! documented at all.
//!
//! # Why `zdc doc prelude/list.zd` cannot work, and this exists instead
//!
//! Every entry point compiles *against* the prelude (§17.4.1): the library
//! is resolved into the same arenas first, and the program on top of it. So
//! pointing the command at `prelude/list.zd` compiles that file with the
//! library beneath it, and the very first declaration collides with itself
//! — `` `listLength` is the name of a standard-library operation `` — which
//! is the resolver working exactly as it should and is not a thing a flag
//! on the loader could fix.
//!
//! Documenting the library therefore means compiling it as the program
//! rather than as the library, which is what [`linked`] builds. Two things
//! have to be true of the result and both are the reason this is not simply
//! [`zdc_lib::load`]:
//!
//! 1. **Every span has to index the file it came from.** `zdc_lib::load`
//!    parses each source independently, so all eight files' spans start at
//!    zero and overlap; a page generator that asked which file a
//!    declaration was written in would get the first one every time. Here
//!    the sources are concatenated and each is parsed with
//!    [`zdc_resolve::modules::parse_at`] at its own offset, which is the
//!    same shift `zdc_resolve::load` performs for a program's own modules.
//! 2. **Every name has to stay visible from every file.** The library is
//!    ambient — `map.zd` calls `valueOr` from `option.zd` and `number.zd`
//!    calls `first` from `list.zd`, in a cycle, with no `use` line anywhere
//!    — and nothing here builds an import table to make that work. It falls
//!    out of [`resolve`] compiling these declarations *as the prelude*:
//!    §17.4.1's rule is that a prelude name is visible in every module
//!    rather than in the one it was collected into. `decl_module` is filled
//!    in truthfully and is simply not read on this path, and which page a
//!    declaration lands on is decided by its span against `modules`.
//!
//! What comes out is an ordinary [`Linked`], so it goes through the same
//! resolver, the same split and the same type checker a program does. The
//! library is documented by the compiler that compiles it, which is the
//! same argument §17.4.1 makes for the library being a compilation unit in
//! the first place.
//!
//! # It is still resolved *as* the prelude, and that is not a formality
//!
//! [`resolve`] hands these declarations to `Resolver::with_prelude` as the
//! prelude, with an empty program on top, rather than resolving them as an
//! ordinary program. The first attempt did the latter and the type checker
//! refused the library:
//!
//! > `` `contains` needs `textContains`, which the standard library did not
//! > provide. ``
//!
//! `a contains b` is desugared to a call to one of three library functions,
//! and `zdc_types`' lookup for them asks `Hir::is_prelude_def` — identity,
//! not name. Resolved as a program, nothing is a prelude definition, and
//! the two `contains` the library uses on itself (in `map.zd` and
//! `text.zd`) have no target. Compiled as the prelude, they resolve exactly
//! as they do in every real build, so these pages are typed by the same
//! inference that types the library when a program imports it.
//!
//! The cost is that every definition is a prelude definition, so
//! `Hir::user_defs` — which is what tells a program's declarations from the
//! library's — reports none of them. `pages.rs` reads the subject rather
//! than that flag for exactly this reason.

use std::path::PathBuf;

use zdc_hir::Hir;
use zdc_resolve::modules::parse_at;
use zdc_resolve::{Linked, Module, ResolveError};

/// The prelude's sources, linked into one program.
///
/// Panics on a parse failure, for [`zdc_lib::load`]'s reason: these sources
/// ship inside the compiler, so a failure here is a defect in this
/// workspace and there is no file a reader could be pointed at to fix.
pub fn linked() -> Linked {
    let mut modules: Vec<Module> = Vec::new();
    let mut decls = Vec::new();
    let mut decl_module = Vec::new();
    let mut combined = String::new();

    for (path, source) in zdc_lib::SOURCES {
        let offset = combined.len() as u32;
        // `parse_at` reports every error it recovered past (#151), not
        // just the first. The library is compiled into this binary and a
        // failure here is a build that should never have shipped, so the
        // panic carries all of them rather than making somebody rebuild
        // to see the second.
        let parsed = parse_at(source, offset).unwrap_or_else(|errors| {
            let reported: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            panic!("{path} does not parse: {}", reported.join("\n"))
        });
        decl_module.extend(std::iter::repeat_n(modules.len(), parsed.decls.len()));
        decls.extend(parsed.decls);
        combined.push_str(source);
        // A file that does not end in a newline would otherwise glue its
        // last token to the next file's first one. Appended to the combined
        // buffer only: the module's own text is unchanged, and a byte past
        // its end cannot be inside any of its spans.
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
        modules.push(Module {
            path: PathBuf::from(path),
            source: (*source).to_string(),
            offset,
        });
    }

    let imports = vec![Vec::new(); modules.len()];
    Linked {
        program: zdc_ast::Program { decls },
        modules,
        decl_module,
        imports,
        combined,
        // The library names no packages and could not: a `zd.toml` sits
        // beside a project's entry file, and this is the standard library
        // compiled from sources built into the binary, which has no
        // project directory to sit beside (#238).
        packages: zdc_resolve::packages::Packages::none(std::path::Path::new("prelude")),
    }
}

/// Resolve the linked library, as the prelude.
///
/// The empty program on top is the whole trick: `with_prelude` marks the
/// leading declarations as the library's, which is what `zdc_types`' lookup
/// for `textContains` and its two siblings tests. See the module note.
pub fn resolve(linked: &Linked) -> Result<Hir, Vec<ResolveError>> {
    let nothing = zdc_ast::Program { decls: Vec::new() };
    zdc_resolve::Resolver::with_prelude(&linked.program, &nothing).resolve()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claim the module makes: one program, eight files, and every
    /// span landing in the file it was written in.
    #[test]
    fn every_prelude_file_is_a_module_and_every_span_finds_its_own_file() {
        let linked = linked();
        assert_eq!(linked.modules.len(), zdc_lib::SOURCES.len());

        for module in &linked.modules {
            let (path, source, local) =
                linked.locate(zdc_lexer::Span::new(module.offset, module.offset + 1));
            assert_eq!(path, module.path, "a module's first byte is its own");
            assert_eq!(source, module.source);
            assert_eq!(local.start, 0);
        }

        // `decl_module` is not what resolution reads here, so it is only
        // worth carrying if it is true — which is what a later caller
        // passing this to `Resolver::linked` would rely on. The
        // declarations go in file order, so the vector must be one entry
        // per declaration, non-decreasing, and reach every file.
        assert_eq!(linked.decl_module.len(), linked.program.decls.len());
        assert!(linked.decl_module.windows(2).all(|pair| pair[0] <= pair[1]));
        for module in 0..linked.modules.len() {
            assert!(
                linked.decl_module.contains(&module),
                "{} declares nothing",
                linked.modules[module].path.display()
            );
        }
    }

    /// The library resolves with the cross-file calls it really makes —
    /// `map.zd` calling `valueOr`, `number.zd` calling `first` — which is
    /// what the one visibility module exists for.
    #[test]
    fn the_library_resolves_with_its_cross_file_calls_intact() {
        let linked = linked();
        let hir = resolve(&linked).expect("the prelude resolves");
        assert!(hir.defs.iter().count() > 50, "the library is not empty");
        for name in ["valueOr", "first", "textLength", "mapKeyAt"] {
            assert!(
                hir.defs.iter().any(|(_, def)| def.name == name),
                "`{name}` is missing from the resolved library"
            );
        }
    }

    /// `contains` is desugared to a library call looked up by *identity*,
    /// so the library only typechecks when it is compiled as the prelude.
    /// This is the assertion that fails if that is ever undone.
    #[test]
    fn the_library_typechecks_because_it_is_compiled_as_the_prelude() {
        let linked = linked();
        let hir = resolve(&linked).expect("the prelude resolves");
        assert!(hir.prelude_defs > 0, "the library must be the prelude");
        let split = zdc_graph::split(&hir);
        assert!(!split.has_errors(), "the prelude splits");
        zdc_types::check(&hir, &split).expect("the prelude typechecks");
    }

    /// §17.4.1 step 6 read off the declarations rather than asserted: a
    /// `state` in the library would put it into the signal graph.
    #[test]
    fn the_library_declares_no_state() {
        let linked = linked();
        let hir = resolve(&linked).expect("the prelude resolves");
        assert!(!hir
            .defs
            .iter()
            .any(|(_, def)| matches!(def.kind, zdc_hir::DefKind::Signal(_))));
    }
}
