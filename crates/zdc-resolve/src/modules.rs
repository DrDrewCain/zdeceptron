//! Modules: every `.zd` file is one, and `use` is the only way in.
//!
//! Spec §14D.2. All top-level declarations are importable; imports are
//! explicit and name what they bring in; paths are relative to the
//! importing file and the `.zd` ending is implied. Cycles are an error
//! reported with the full import path.
//!
//! **Linking happens before resolution, and therefore before placement**,
//! because a `durable` signal may be declared in one file and read in
//! another (§14D.3). The product is one program whose declarations came
//! from several files, plus the record of which file each came from — a
//! module is a unit of naming, never of deployment, so nothing downstream
//! sees the seams.
//!
//! # Spans across files
//!
//! A [`Span`] is a byte range and carries no file. Rather than thread a
//! file id through every node of every pass, the loader concatenates the
//! sources it read into one buffer and shifts each module's tokens by that
//! module's offset before parsing. Every span downstream indexes the
//! combined text, and [`Linked::locate`] turns one back into the file that
//! owns it. Diagnostics stay exactly as precise as they were for one file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zdc_ast::{Decl, Program};
use zdc_lexer::{Span, Token};

use crate::collect::ResolveError;

/// One `.zd` file: where it came from, what it said, and where its text
/// begins in the combined buffer.
#[derive(Debug, Clone)]
pub struct Module {
    pub path: PathBuf,
    pub source: String,
    pub offset: u32,
}

/// One name a module borrowed from another.
#[derive(Debug, Clone)]
pub struct Import {
    pub name: String,
    pub span: Span,
    /// The module the name was asked of.
    pub from: usize,
}

/// Every module reachable from the entry file, linked into one program.
#[derive(Debug, Clone)]
pub struct Linked {
    pub program: Program,
    pub modules: Vec<Module>,
    /// The module each declaration of `program` came from, by index.
    pub decl_module: Vec<usize>,
    /// What each module imported, by module index.
    pub imports: Vec<Vec<Import>>,
    /// Every module's source, concatenated. Spans index this.
    pub combined: String,
}

impl Linked {
    /// A program with no imports, for the callers that hold source rather
    /// than a path: the language server, the dev server's in-memory
    /// rebuild, and every test that writes a program inline.
    pub fn single(path: impl Into<PathBuf>, source: String, program: Program) -> Linked {
        let decl_module = vec![0; program.decls.len()];
        Linked {
            combined: source.clone(),
            modules: vec![Module {
                path: path.into(),
                source,
                offset: 0,
            }],
            decl_module,
            imports: vec![Vec::new()],
            program,
        }
    }

    /// The file a span belongs to, and the span within that file.
    ///
    /// A diagnostic is rendered against one file's text, so this is what
    /// turns a linked span back into something a reader can look at.
    pub fn locate(&self, span: Span) -> (&Path, &str, Span) {
        let module = self
            .modules
            .iter()
            .rev()
            .find(|module| span.start >= module.offset)
            .unwrap_or(&self.modules[0]);
        let start = span.start.saturating_sub(module.offset);
        let end = span.end.saturating_sub(module.offset);
        let limit = module.source.len() as u32;
        (
            &module.path,
            &module.source,
            Span::new(start.min(limit), end.min(limit)),
        )
    }
}

/// Read the entry file and every file it reaches, and link them.
///
/// A parse error in any of them is reported against that file, which is
/// why the loader returns [`ResolveError`] rather than the parser's own
/// error type: by the time several files are in play, "the error" needs a
/// span that means something across all of them.
pub fn load(entry: &Path) -> Result<Linked, Vec<ResolveError>> {
    Loader::default().load(entry, None)
}

/// The same, with the entry file's text supplied rather than read.
///
/// An editor holds a buffer that has not been saved, so reading the entry
/// off disk would analyse the last save rather than what is on screen. The
/// files it *imports* are still read from disk, which is the best available
/// answer: an unsaved import is not visible to anything but its own editor
/// window.
pub fn load_with_entry(entry: &Path, source: String) -> Result<Linked, Vec<ResolveError>> {
    Loader::default().load(entry, Some(source))
}

#[derive(Default)]
struct Loader {
    /// The directory every module this build opens must lie inside
    /// ([`crate::sandbox::project_root`]). Fixed from the entry file
    /// before anything is read, and never recomputed.
    root: PathBuf,
    modules: Vec<Module>,
    parsed: Vec<Program>,
    /// Canonical path to module index, so a file reached by two routes is
    /// one module and not two.
    by_path: HashMap<PathBuf, usize>,
    /// Per module, the `use` lines it wrote, resolved to module indices.
    edges: Vec<Vec<(usize, Vec<zdc_ast::Ident>)>>,
    combined: String,
    errors: Vec<ResolveError>,
}

impl Loader {
    fn load(mut self, entry: &Path, text: Option<String>) -> Result<Linked, Vec<ResolveError>> {
        // The boundary is fixed before the first byte is read, and from the
        // entry file rather than from whichever module is doing the
        // importing, so that it cannot be re-based one hop at a time.
        self.root = crate::sandbox::project_root(entry);

        // A file that cannot be read at all has no span to report against,
        // so the entry is the one case that fails outright.
        let root = match self.read_source(entry, None, text) {
            Some(index) => index,
            None => {
                return Err(std::mem::take(&mut self.errors));
            }
        };

        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        if let Some(cycle) = self.find_cycle(root) {
            return Err(vec![cycle]);
        }

        // Imports first, so a declaration is always linked after everything
        // it can refer to. The order is not what makes forward references
        // work — collection already does that — but it keeps the linked
        // program readable when it is dumped.
        let order = self.postorder(root);

        let mut program = Program { decls: Vec::new() };
        let mut decl_module = Vec::new();
        let mut position: HashMap<usize, usize> = HashMap::new();
        for (rank, module) in order.iter().enumerate() {
            position.insert(*module, rank);
        }

        for module in &order {
            for decl in &self.parsed[*module].decls {
                if matches!(decl, Decl::Use(_)) {
                    continue;
                }
                program.decls.push(decl.clone());
                decl_module.push(*module);
            }
        }

        let imports = (0..self.modules.len())
            .map(|module| {
                self.edges
                    .get(module)
                    .map(|edges| {
                        edges
                            .iter()
                            .flat_map(|(target, names)| {
                                names.iter().map(|name| Import {
                                    name: name.text.clone(),
                                    span: name.span,
                                    from: *target,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();

        Ok(Linked {
            program,
            modules: self.modules,
            decl_module,
            imports,
            combined: self.combined,
        })
    }

    /// Read, parse and register one file, then everything it imports.
    ///
    /// Returns the module's index, or `None` when the file could not be
    /// read or parsed. `blame` is the `use` line that asked for it, so a
    /// missing file is reported at the import rather than nowhere.
    fn read(&mut self, path: &Path, blame: Option<Span>) -> Option<usize> {
        self.read_source(path, blame, None)
    }

    fn read_source(
        &mut self,
        path: &Path,
        blame: Option<Span>,
        supplied: Option<String>,
    ) -> Option<usize> {
        // A file that does not exist cannot be canonicalised, and that is
        // the case whose diagnostic matters most, so the raw path stands in
        // for it rather than the read being skipped.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(index) = self.by_path.get(&canonical) {
            return Some(*index);
        }

        let source = match supplied
            .ok_or(())
            .or_else(|()| std::fs::read_to_string(path))
        {
            Ok(source) => source,
            Err(error) => {
                let message = format!("Could not read `{}`: {error}.", path.display());
                match blame {
                    Some(span) => self.errors.push(ResolveError { message, span }),
                    None => self.errors.push(ResolveError {
                        message,
                        span: Span::new(0, 0),
                    }),
                }
                return None;
            }
        };

        let offset = self.combined.len() as u32;
        self.combined.push_str(&source);
        // A file that does not end in a newline would run its last line
        // into the next file's first one, both in the combined text and in
        // the line numbering a diagnostic computes from it.
        if !source.ends_with('\n') {
            self.combined.push('\n');
        }

        let index = self.modules.len();
        self.modules.push(Module {
            path: path.to_path_buf(),
            source,
            offset,
        });
        self.parsed.push(Program { decls: Vec::new() });
        self.edges.push(Vec::new());
        self.by_path.insert(canonical, index);

        let program = match parse_at(&self.modules[index].source, offset) {
            Ok(program) => program,
            Err(error) => {
                self.errors.push(error);
                return Some(index);
            }
        };

        let directory = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        for decl in &program.decls {
            let Decl::Use(import) = decl else { continue };
            let target = directory.join(format!("{}.zd", import.path));

            // Checked before the read, not after it: a refused module's
            // text never reaches `combined`, so nothing downstream can
            // have seen it even though the build carries on to collect
            // whatever other errors the program has.
            if let Some(refusal) = crate::sandbox::refuse(&self.root, &import.path, &target) {
                self.errors.push(ResolveError {
                    message: format!(
                        "`use \"{}\"` names a file that {}. A module is read from inside the \
                         project, and the project is the directory holding the file this build \
                         started from — `use` reaches files under it and nowhere else. Move the \
                         file into the project, or start the build from a directory that \
                         contains both.",
                        import.path,
                        refusal.reason()
                    ),
                    span: import.path_span,
                });
                continue;
            }

            if let Some(target) = self.read(&target, Some(import.path_span)) {
                self.edges[index].push((target, import.names.clone()));
            }
        }

        self.parsed[index] = program;
        Some(index)
    }

    /// The first import cycle reachable from `root`, named in full.
    ///
    /// Depth-first with the current path on the stack: when an edge points
    /// back into the path, everything from that point on is the cycle, in
    /// the order it was written.
    fn find_cycle(&self, root: usize) -> Option<ResolveError> {
        let mut path: Vec<usize> = Vec::new();
        let mut done = vec![false; self.modules.len()];
        self.walk_for_cycle(root, &mut path, &mut done)
    }

    fn walk_for_cycle(
        &self,
        module: usize,
        path: &mut Vec<usize>,
        done: &mut [bool],
    ) -> Option<ResolveError> {
        if let Some(at) = path.iter().position(|seen| *seen == module) {
            let mut names: Vec<String> = path[at..]
                .iter()
                .map(|module| self.describe(*module))
                .collect();
            names.push(self.describe(module));
            let span = self
                .edges
                .get(*path.last().expect("a cycle has a previous module"))
                .and_then(|edges| edges.iter().find(|(target, _)| *target == module))
                .and_then(|(_, names)| names.first().map(|name| name.span))
                .unwrap_or(Span::new(0, 0));
            return Some(ResolveError {
                message: format!(
                    "These files import each other in a circle: {}. A module is read before the \
                     files that import it, so a circle has no place to start. Move what they \
                     share into a file of its own.",
                    names.join(" → ")
                ),
                span,
            });
        }
        if done[module] {
            return None;
        }
        path.push(module);
        for (target, _) in &self.edges[module] {
            if let Some(cycle) = self.walk_for_cycle(*target, path, done) {
                return Some(cycle);
            }
        }
        path.pop();
        done[module] = true;
        None
    }

    /// Modules in dependency order: everything a module imports comes
    /// before it.
    fn postorder(&self, root: usize) -> Vec<usize> {
        let mut order = Vec::new();
        let mut seen = vec![false; self.modules.len()];
        let mut stack = vec![(root, false)];
        while let Some((module, expanded)) = stack.pop() {
            if expanded {
                order.push(module);
                continue;
            }
            if seen[module] {
                continue;
            }
            seen[module] = true;
            stack.push((module, true));
            for (target, _) in self.edges[module].iter().rev() {
                stack.push((*target, false));
            }
        }
        order
    }

    fn describe(&self, module: usize) -> String {
        self.modules[module]
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.modules[module].path.display().to_string())
    }
}

/// Parse one file whose text begins at `offset` in the combined buffer.
///
/// The shift happens on the token stream rather than on the syntax tree:
/// every span a later pass sees comes from a token, so moving them once
/// here is total, whereas walking the tree would need an arm per node and
/// would silently miss a new one.
pub fn parse_at(source: &str, offset: u32) -> Result<Program, ResolveError> {
    let mut tokens: Vec<Token> = zdc_lexer::tokenize(source).map_err(|error| ResolveError {
        message: error.message,
        span: shift(error.span, offset),
    })?;
    for token in &mut tokens {
        token.span = shift(token.span, offset);
    }
    zdc_parser::Parser::new(tokens)
        .program()
        .map_err(|error| ResolveError {
            message: error.message,
            span: error.span,
        })
}

fn shift(span: Span, offset: u32) -> Span {
    Span::new(span.start + offset, span.end + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Files {
        root: PathBuf,
    }

    impl Files {
        fn new(name: &str) -> Files {
            let root =
                std::env::temp_dir().join(format!("zdc-modules-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("a temporary directory");
            Files { root }
        }

        fn write(&self, name: &str, source: &str) -> PathBuf {
            let path = self.root.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory for the module");
            }
            std::fs::write(&path, source).expect("writing a test module");
            path
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn an_import_brings_the_named_declarations_into_the_program() {
        let files = Files::new("import");
        files.write("model.zd", "record Item\n    id is Text\n");
        let app = files.write(
            "app.zd",
            "use \"./model\" for Item\nstate a is client Whole starting 1\n",
        );

        let linked = load(&app).expect("links");
        assert_eq!(linked.modules.len(), 2);
        assert_eq!(
            linked.program.decls.len(),
            2,
            "the `use` line is not a decl"
        );
        // The imported module is linked first.
        assert!(matches!(linked.program.decls[0], Decl::Record(_)));
        assert_eq!(linked.decl_module, vec![1, 0]);
    }

    #[test]
    fn a_span_from_an_imported_file_locates_to_that_file() {
        let files = Files::new("locate");
        let model = files.write("model.zd", "record Item\n    id is Text\n");
        let app = files.write("app.zd", "use \"./model\" for Item\nview\n    Column\n");

        let linked = load(&app).expect("links");
        let Decl::Record(record) = &linked.program.decls[0] else {
            panic!("expected the imported record first")
        };
        let (path, source, span) = linked.locate(record.name.span);
        assert_eq!(path, model);
        assert_eq!(&source[std::ops::Range::<usize>::from(span)], "Item");
    }

    #[test]
    fn a_cycle_names_every_file_on_the_path() {
        let files = Files::new("cycle");
        files.write("a.zd", "use \"./b\" for B\nrecord A\n    x is Text\n");
        files.write("b.zd", "use \"./a\" for A\nrecord B\n    x is Text\n");
        let entry = files.root.join("a.zd");

        let errors = load(&entry).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("a.zd"),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("b.zd"),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains('→'),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn a_module_reached_twice_is_read_once() {
        let files = Files::new("diamond");
        files.write("model.zd", "record Item\n    id is Text\n");
        files.write(
            "left.zd",
            "use \"./model\" for Item\nrecord L\n    x is Text\n",
        );
        files.write(
            "right.zd",
            "use \"./model\" for Item\nrecord R\n    x is Text\n",
        );
        let app = files.write(
            "app.zd",
            "use \"./left\" for L\nuse \"./right\" for R\nview\n    Column\n",
        );

        let linked = load(&app).expect("links");
        assert_eq!(linked.modules.len(), 4);
    }

    fn resolve(entry: &Path) -> Result<zdc_hir::Hir, Vec<ResolveError>> {
        let linked = load(entry)?;
        crate::Resolver::linked(&linked).resolve()
    }

    /// Imports are explicit, so being linked into the same program is not
    /// the same as being visible (§14D.2). Without this, `use` would be a
    /// comment.
    #[test]
    fn a_name_that_was_not_imported_is_not_visible() {
        let files = Files::new("visibility");
        files.write(
            "model.zd",
            "record Item\n    id is Text\nrecord Hidden\n    id is Text\n",
        );
        let app = files.write(
            "app.zd",
            "use \"./model\" for Item\nstate a is client Hidden starting empty\nview\n    Column\n",
        );

        let errors = resolve(&app).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("does not import"),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("use \"./that-file\" for Hidden"),
            "the message must show the fix: {}",
            errors[0].message
        );
    }

    #[test]
    fn an_imported_name_resolves_across_the_boundary() {
        let files = Files::new("across");
        files.write("model.zd", "function double with n\n    give n + n\n");
        let app = files.write(
            "app.zd",
            "use \"./model\" for double\nstate a is client Whole from double with 2\nview\n    Text a\n",
        );
        resolve(&app).expect("an imported function is callable");
    }

    /// A component crosses a module boundary like anything else: it is a
    /// top-level declaration, and every top-level declaration is
    /// importable.
    #[test]
    fn a_component_may_be_imported() {
        let files = Files::new("component");
        files.write(
            "cards.zd",
            "component Card with title\n    Row\n        Text title\n",
        );
        let app = files.write(
            "app.zd",
            "use \"./cards\" for Card\nview\n    Column\n        Card \"hello\"\n",
        );
        resolve(&app).expect("an imported component is usable as an element");
    }

    /// Module resolution runs before placement, because a `durable` signal
    /// may be declared in one file and read in another (§14D.3).
    #[test]
    fn a_durable_signal_declared_in_one_file_is_read_in_another() {
        let files = Files::new("placement");
        files.write("store.zd", "state visits is durable Whole starting 0\n");
        let app = files.write(
            "app.zd",
            "use \"./store\" for visits\n\
             view\n\
             \x20   when visits\n\
             \x20       Loading show Spinner\n\
             \x20       Failed with e show Spinner\n\
             \x20       Ready with total show Text total\n",
        );
        let hir = resolve(&app).expect("resolves");
        // The split first, in §17.1.2's order: the crossing is what makes
        // this read a `Remote of Whole`, so the checker cannot be asked
        // about it without the pass that decided it.
        let split = zdc_graph::split(&hir);
        assert!(
            !split.has_errors(),
            "the placement pass rejected a legal cross-module read"
        );
        zdc_types::check(&hir, &split)
            .expect("the read still crosses a boundary and still typechecks");
    }

    #[test]
    fn a_name_asked_of_the_wrong_file_is_reported() {
        let files = Files::new("wrongfile");
        files.write("empty.zd", "record Nothing\n    id is Text\n");
        let app = files.write("app.zd", "use \"./empty\" for Missing\nview\n    Column\n");
        let errors = resolve(&app).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("not declared in the file"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn a_missing_file_is_reported_at_the_import_that_asked_for_it() {
        let files = Files::new("missing");
        let app = files.write("app.zd", "use \"./nope\" for Thing\nview\n    Column\n");

        let errors = load(&app).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("nope.zd"),
            "got: {}",
            errors[0].message
        );
        let span = errors[0].span;
        assert_eq!(span.start, 4, "the span points at the quoted path");
    }

    // --- the sandbox (spec §14D.2a) ---

    /// A file outside the project, whose text is recognisable wherever it
    /// turns up. If any of these tests finds this string in the combined
    /// buffer, the build read a file it was supposed to refuse.
    const SECRET: &str = "# zdc-test-secret-4f1c9\nrecord Secret\n    id is Text\n";
    const MARKER: &str = "zdc-test-secret-4f1c9";

    /// Drive the loader directly so the test can look at what it read.
    ///
    /// `load` returns `Err` on a refusal and therefore hands back nothing
    /// to inspect — but "the error was reported" is a weaker claim than
    /// the one that matters, which is that the refused file's bytes never
    /// entered the compilation at all. The combined buffer is where they
    /// would be.
    fn read_into_loader(entry: &Path) -> Loader {
        let mut loader = Loader {
            root: crate::sandbox::project_root(entry),
            ..Default::default()
        };
        loader.read_source(entry, None, None);
        loader
    }

    #[test]
    fn a_use_that_climbs_out_of_the_project_is_refused() {
        let files = Files::new("climb");
        files.write("secrets.zd", SECRET);
        let app = files.write(
            "project/nested/app.zd",
            "use \"./../../secrets\" for Secret\nview\n    Column\n",
        );

        let errors = load(&app).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("./../../secrets"),
            "the diagnostic shows the specifier as written: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("climbs out of the project"),
            "the diagnostic names the rule: {}",
            errors[0].message
        );
    }

    /// The acceptance criterion that matters most: a refusal is not an
    /// error raised after the fact, it is a file that was never opened.
    #[test]
    fn a_module_that_climbs_out_is_never_read() {
        let files = Files::new("climb-unread");
        files.write("secrets.zd", SECRET);
        let app = files.write(
            "project/nested/app.zd",
            "use \"./../../secrets\" for Secret\nview\n    Column\n",
        );

        let loader = read_into_loader(&app);
        assert!(
            !loader.combined.contains(MARKER),
            "the refused file's text reached the compilation"
        );
        assert_eq!(
            loader.modules.len(),
            1,
            "only the entry file became a module"
        );
    }

    /// Canonicalisation is the layer that earns its place here. This
    /// specifier has no `..` and no leading `/` — nothing in the program's
    /// text says it leaves the project — so a check on the written path
    /// alone would admit it.
    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_project_is_refused() {
        let files = Files::new("symlink");
        let outside = files.write("outside.zd", SECRET);
        files.write("project/app.zd", "");
        let link = files.root.join("project/lib.zd");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&outside, &link).expect("planting the symlink");

        let app = files.write(
            "project/app.zd",
            "use \"./lib\" for Secret\nview\n    Column\n",
        );

        let errors = load(&app).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("./lib"),
            "the diagnostic shows the specifier: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("points outside the project"),
            "the diagnostic names the rule: {}",
            errors[0].message
        );

        let loader = read_into_loader(&app);
        assert!(
            !loader.combined.contains(MARKER),
            "the linked-to file's text reached the compilation"
        );
        assert_eq!(loader.modules.len(), 1);
    }

    #[test]
    fn an_absolute_module_path_is_refused() {
        let files = Files::new("absolute");
        let app = files.write("app.zd", "use \"/etc/hosts\" for Thing\nview\n    Column\n");

        let errors = load(&app).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("is an absolute path"),
            "got: {}",
            errors[0].message
        );
    }

    /// The boundary is the project, not the importing file's directory, so
    /// `..` is only an error when it lands outside. A module in a
    /// subdirectory reaching a sibling of its own parent is ordinary, and
    /// refusing it would trade a real capability for nothing.
    #[test]
    fn a_relative_import_that_stays_inside_the_project_still_works() {
        let files = Files::new("inside");
        files.write("shared.zd", "record Item\n    id is Text\n");
        files.write(
            "views/list.zd",
            "use \"../shared\" for Item\nrecord L\n    x is Text\n",
        );
        let app = files.write("app.zd", "use \"./views/list\" for L\nview\n    Column\n");

        let linked = load(&app).expect("an import that stays inside the project links");
        assert_eq!(linked.modules.len(), 3);
    }

    /// `use` is transitive, so the boundary has to be too: an imported
    /// module must not be able to reach further out than the file that
    /// imported it could. The root is fixed from the entry file precisely
    /// so that it cannot be re-based one hop at a time.
    #[test]
    fn an_imported_module_cannot_climb_out_on_its_own() {
        let files = Files::new("transitive");
        files.write("secrets.zd", SECRET);
        files.write(
            "project/deep/lib.zd",
            "use \"../../secrets\" for Secret\nrecord L\n    x is Text\n",
        );
        let app = files.write(
            "project/app.zd",
            "use \"./deep/lib\" for L\nview\n    Column\n",
        );

        let errors = load(&app).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("climbs out of the project")),
            "got: {:?}",
            errors.iter().map(|e| &e.message).collect::<Vec<_>>()
        );

        let loader = read_into_loader(&app);
        assert!(
            !loader.combined.contains(MARKER),
            "a dependency read a file the entry file could not have"
        );
    }
}
