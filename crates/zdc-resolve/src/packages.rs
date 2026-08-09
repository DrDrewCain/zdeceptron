//! The project's package mapping: `zd.toml` beside the entry file (#238).
//!
//! A bare specifier — `from "three"` — names a package rather than a file,
//! and nothing in a build resolves one by itself. Before this existed the
//! compiler emitted `import … from 'three'` anyway, shipped nothing, and
//! wrote no import map, so the page failed on its first import before any
//! program code ran. That is the one outcome a compiler must not have: it
//! compiled and it could not load.
//!
//! The answer is not to guess. `zdc-codegen/src/lib.rs:163` records the
//! rule — a bare specifier "names a package the target resolves, not a file
//! this build owns, so there is nothing to copy and refusing to guess is
//! the whole of the handling" — and that rule is right and stays. What was
//! missing was somewhere for the *project* to say, so that refusing to
//! guess left the question answered rather than unanswerable:
//!
//! ```toml
//! # zd.toml, beside the entry file
//! [packages]
//! three   = "https://esm.sh/three@0.180.0"
//! slugify = "./vendor/slugify.js"
//! ```
//!
//! Project-level rather than per-declaration because which build of
//! three.js a project pins is a fact about the project. A `resolves to`
//! clause on each declaration would restate the URL at every import —
//! `examples/tree/` would state one version three times — and put a
//! deployment concern in the middle of a type signature. It is the same
//! division `assets/` already uses: a convention about where things live,
//! not something each declaration repeats.
//!
//! # Why the parser is written here rather than taken from crates.io
//!
//! The grammar this reads is four productions wide — a table header, a
//! comment, a blank line, and `key = "value"` — and every one of them has
//! to produce a diagnostic in this compiler's voice, naming the line and
//! the repair. A general TOML parser answers a different question (is this
//! valid TOML?) and its errors are in its own words. The workspace also
//! carries three non-`zdc` dependencies in total, each with a paragraph
//! saying why; "so that forty lines did not have to be written" is not one.
//!
//! What is deliberately *not* supported is therefore everything else TOML
//! has: arrays, inline tables, multi-line strings, integers, dates. Each is
//! refused by name rather than misread, so a file using one is told so
//! rather than silently losing a mapping.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file name, fixed. It sits beside the entry file, not beside
/// whichever module is doing the importing: a project has one set of
/// dependencies, and a per-directory mapping would let two files in one
/// build disagree about what `three` means.
pub const MANIFEST: &str = "zd.toml";

/// The one table this build understands.
const PACKAGES: &str = "packages";

/// What the project said about one bare specifier.
///
/// A closed answer rather than `Option<&str>`, because there are three
/// outcomes and the third one — mapped twice, to different things — must
/// not be able to arrive as a value the caller can use. Last-writer-wins
/// there would make which build of a library a page loads depend on the
/// order of two lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mapping<'a> {
    /// The project did not mention this specifier.
    Missing,
    /// Exactly one target.
    Mapped(&'a str),
    /// Two lines, two targets. Both are carried so the diagnostic can name
    /// them: the reader has to pick one, and cannot without seeing both.
    Conflicting { first: &'a str, second: &'a str },
}

/// A `zd.toml` that could not be read as a package mapping.
///
/// The line number rather than a byte span, because the file is not part
/// of the compilation unit whose spans index the combined `.zd` source.
/// Naming the line is what a reader needs to make the repair.
#[derive(Debug, Clone)]
pub struct PackagesError {
    pub line: usize,
    pub message: String,
}

/// Every mapping the project declared, and where it declared them.
#[derive(Debug, Clone)]
pub struct Packages {
    /// The mapping file — the one that exists, or the one that would have
    /// to. A diagnostic naming a repair has to name a path either way, and
    /// "create this file" is as actionable as "add a line to it".
    file: PathBuf,
    /// The directory a path this build is asked to open must lie under —
    /// `zdc_hir::sandbox::project_root` of the entry file.
    ///
    /// `None` when there is no project to bound: a source string held in
    /// memory has no directory, and a root of `.` there would mean the
    /// answer depended on where the process was started from. A caller
    /// with no root opens no files either, so there is nothing to bound.
    root: Option<PathBuf>,
    /// Specifier to the targets it was given, in the order the file gave
    /// them. A `Vec` rather than one target, because a duplicate key is a
    /// refusal and a map would have thrown away the evidence for it.
    entries: BTreeMap<String, Vec<String>>,
}

impl Packages {
    /// The mapping for a build whose entry file is `entry`, read from disk
    /// if the file is there.
    ///
    /// A missing `zd.toml` is not an error — most programs import no
    /// packages at all, and `hello.zd` should not need a config file to
    /// compile. It becomes an error only at the point some declaration
    /// needs a mapping out of it.
    pub fn read(entry: &Path) -> Result<Packages, PackagesError> {
        let file = beside(entry);
        // The same root `use` and the build-time capabilities are bounded
        // by, taken from the same function, so a mapping cannot reach a
        // file the rest of the compiler would refuse to open.
        let root = Some(zdc_hir::sandbox::project_root(entry));
        let Ok(text) = std::fs::read_to_string(&file) else {
            // Unreadable and absent are one case on purpose. A file this
            // process cannot open contributes no mappings, which is what
            // an absent one contributes, and the diagnostic a reader gets
            // then names the specifier that needed one — which is more use
            // than a permissions error on a file they may not have known
            // was consulted.
            return Ok(Packages {
                file,
                root,
                entries: BTreeMap::new(),
            });
        };
        let entries = parse(&text)?;
        Ok(Packages {
            file,
            root,
            entries,
        })
    }

    /// A build with no project directory to read: a source string held in
    /// memory, which is what every test and every in-process caller
    /// compiles.
    ///
    /// It maps nothing, so a bare specifier in one of those is refused for
    /// the same reason a bare specifier in an unmapped project is.
    pub fn none(entry: &Path) -> Packages {
        Packages {
            file: beside(entry),
            root: None,
            entries: BTreeMap::new(),
        }
    }

    /// The mapping supplied directly, for a caller that has one without a
    /// file to read it from.
    ///
    /// Test-only, and marked so rather than left `pub`: every shipping
    /// caller reaches a mapping through [`Packages::read`], which is also
    /// the only constructor that establishes a project root. A public
    /// constructor that skipped the root would be a way to obtain a
    /// `Packages` whose targets nothing bounds.
    #[cfg(test)]
    fn inline<K, V, I>(entry: &Path, pairs: I) -> Packages
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, target) in pairs {
            entries.entry(name.into()).or_default().push(target.into());
        }
        Packages {
            file: beside(entry),
            root: None,
            entries,
        }
    }

    /// Where a repair goes.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The directory every path this build opens must lie under, or `None`
    /// when this build has no project directory at all.
    ///
    /// `None` is not "unbounded": a caller with no root is compiling a
    /// string in memory and opens nothing, so there is no path for a rule
    /// to apply to. The build that *does* open files — `zdc build`, `zdc
    /// dev`, `zdc check` — always reaches its mapping through
    /// [`Packages::read`], which always has one.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// What the project said about `specifier`.
    pub fn mapping(&self, specifier: &str) -> Mapping<'_> {
        let Some(targets) = self.entries.get(specifier) else {
            return Mapping::Missing;
        };
        match targets.as_slice() {
            [] => Mapping::Missing,
            [only] => Mapping::Mapped(only),
            // The first two are enough: a reader repairing three duplicate
            // lines starts by deleting one, and listing all of them makes
            // the message longer without making it more actionable.
            [first, second, ..] => Mapping::Conflicting { first, second },
        }
    }
}

/// `zd.toml` in the entry file's own directory.
fn beside(entry: &Path) -> PathBuf {
    match entry.parent() {
        // The path is left exactly as deep as the entry the caller named,
        // uncanonicalised, so the diagnostic quotes a path the reader can
        // paste back — `examples/tree/zd.toml` and not a resolved absolute
        // one that may cross a symlink they never typed.
        Some(directory) if !directory.as_os_str().is_empty() => directory.join(MANIFEST),
        Some(_) | None => PathBuf::from(MANIFEST),
    }
}

/// The `[packages]` table, or the first line that is not one of the four
/// things this grammar has.
fn parse(text: &str) -> Result<BTreeMap<String, Vec<String>>, PackagesError> {
    let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut in_packages = false;

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(header) = trimmed.strip_prefix('[') {
            let Some(name) = header.strip_suffix(']') else {
                return Err(PackagesError {
                    line,
                    message: format!(
                        "`{MANIFEST}` line {line} opens a table and never closes it. Write \
                         `[{PACKAGES}]`."
                    ),
                });
            };
            let name = name.trim();
            if name != PACKAGES {
                return Err(PackagesError {
                    line,
                    message: format!(
                        "`{MANIFEST}` line {line} declares `[{name}]`, and this build reads one \
                         table. Write `[{PACKAGES}]`."
                    ),
                });
            }
            in_packages = true;
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            return Err(PackagesError {
                line,
                message: format!(
                    "`{MANIFEST}` line {line} is neither a table nor a mapping. Write one \
                     package per line, as in `three = \"https://esm.sh/three@0.180.0\"`."
                ),
            });
        };
        if !in_packages {
            return Err(PackagesError {
                line,
                message: format!(
                    "`{MANIFEST}` line {line} maps a package before any table opens. Write \
                     `[{PACKAGES}]` above it."
                ),
            });
        }
        let name = unquote_key(key.trim(), line)?;
        let target = unquote_value(value.trim(), line)?;
        entries.entry(name).or_default().push(target);
    }

    Ok(entries)
}

/// A package name, bare or double-quoted.
///
/// A bare key is the ordinary spelling and the quoted one exists because
/// npm scopes contain a slash: `"@scope/pkg" = "…"` cannot be written
/// without quotes.
fn unquote_key(key: &str, line: usize) -> Result<String, PackagesError> {
    if let Some(quoted) = key.strip_prefix('"') {
        let Some(name) = quoted.strip_suffix('"') else {
            return Err(PackagesError {
                line,
                message: format!(
                    "`{MANIFEST}` line {line} opens a quoted package name and never closes it. \
                     Write `\"@scope/pkg\" = \"https://esm.sh/@scope/pkg@1.0.0\"`."
                ),
            });
        };
        return Ok(name.to_string());
    }
    if key.is_empty() {
        return Err(PackagesError {
            line,
            message: format!(
                "`{MANIFEST}` line {line} maps a package with no name. Write the name to the \
                 left of the `=`, as in `three = \"https://esm.sh/three@0.180.0\"`."
            ),
        });
    }
    Ok(key.to_string())
}

/// A target, which is a double-quoted string and nothing else.
///
/// Everything else TOML can hold on the right of an `=` is refused here
/// rather than misread. A mapping to an array or an integer is not a
/// mapping this build can act on, and reading `1` as the module `"1"` is
/// worse than saying so.
fn unquote_value(value: &str, line: usize) -> Result<String, PackagesError> {
    // Anything after the closing quote is dropped only if it is a comment;
    // a trailing token means the line was not understood.
    let Some(rest) = value.strip_prefix('"') else {
        return Err(PackagesError {
            line,
            message: format!(
                "`{MANIFEST}` line {line} maps a package to something that is not a quoted \
                 module. Write `three = \"https://esm.sh/three@0.180.0\"`."
            ),
        });
    };
    let Some(end) = rest.find('"') else {
        return Err(PackagesError {
            line,
            message: format!(
                "`{MANIFEST}` line {line} opens a target and never closes it. Write `three = \
                 \"https://esm.sh/three@0.180.0\"`."
            ),
        });
    };
    let (target, tail) = rest.split_at(end);
    let tail = tail[1..].trim();
    if !tail.is_empty() && !tail.starts_with('#') {
        return Err(PackagesError {
            line,
            message: format!(
                "`{MANIFEST}` line {line} carries `{tail}` after the target. Write one package \
                 per line, as in `three = \"https://esm.sh/three@0.180.0\"`."
            ),
        });
    }
    Ok(target.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(text: &str) -> BTreeMap<String, Vec<String>> {
        parse(text).expect("the mapping parses")
    }

    #[test]
    fn a_table_of_packages_is_read() {
        let entries = mapping(
            "# what this project pins\n\
             [packages]\n\
             three   = \"https://esm.sh/three@0.180.0\"\n\
             \n\
             slugify = \"./vendor/slugify.js\"  # vendored\n",
        );
        assert_eq!(
            entries.get("three").map(Vec::as_slice),
            Some(["https://esm.sh/three@0.180.0".to_string()].as_slice())
        );
        assert_eq!(
            entries.get("slugify").map(Vec::as_slice),
            Some(["./vendor/slugify.js".to_string()].as_slice())
        );
    }

    /// A scoped package cannot be written without quotes, so the quoted
    /// form is not decoration.
    #[test]
    fn a_quoted_key_carries_a_scope() {
        let entries = mapping("[packages]\n\"@scope/pkg\" = \"https://esm.sh/@scope/pkg@1.0.0\"\n");
        assert!(entries.contains_key("@scope/pkg"));
    }

    /// Both lines survive parsing, because the refusal is the caller's to
    /// make and it needs both targets to name them.
    #[test]
    fn a_duplicate_key_keeps_both_targets() {
        let entries = mapping("[packages]\nthree = \"https://a.test/x\"\nthree = \"./b.js\"\n");
        assert_eq!(entries["three"].len(), 2);
    }

    #[test]
    fn a_line_that_is_not_a_mapping_names_its_line() {
        let error = parse("[packages]\nthree\n").expect_err("`three` maps nothing");
        assert_eq!(error.line, 2);
        assert!(error.message.contains("line 2"), "got {}", error.message);
    }

    #[test]
    fn a_target_that_is_not_a_string_is_refused_rather_than_misread() {
        for text in [
            "[packages]\nthree = 1\n",
            "[packages]\nthree = [\"a\", \"b\"]\n",
            "[packages]\nthree = \"a\" \"b\"\n",
        ] {
            let error = parse(text).expect_err("only a quoted module is a target");
            assert_eq!(error.line, 2, "for {text:?}");
        }
    }

    #[test]
    fn a_table_this_build_does_not_read_is_named() {
        let error = parse("[dependencies]\nthree = \"x\"\n").expect_err("one table is read");
        assert!(
            error.message.contains("[dependencies]"),
            "got {}",
            error.message
        );
        assert!(
            error.message.contains("[packages]"),
            "got {}",
            error.message
        );
    }

    #[test]
    fn a_mapping_outside_any_table_is_refused() {
        let error = parse("three = \"x\"\n").expect_err("a bare mapping belongs to no table");
        assert!(
            error.message.contains("[packages]"),
            "got {}",
            error.message
        );
    }

    #[test]
    fn a_missing_file_maps_nothing_and_is_not_an_error() {
        let entry = std::env::temp_dir()
            .join(format!("zdc-packages-absent-{}", std::process::id()))
            .join("app.zd");
        let packages = Packages::read(&entry).expect("an absent mapping is not a failure");
        assert_eq!(packages.mapping("three"), Mapping::Missing);
        assert!(packages.file().ends_with(MANIFEST));
    }

    #[test]
    fn the_three_answers_are_told_apart() {
        let entry = Path::new("app.zd");
        let packages = Packages::inline(
            entry,
            [
                ("three", "https://a.test/x"),
                ("marked", "./m.js"),
                ("three", "https://b.test/y"),
            ],
        );
        assert_eq!(packages.mapping("marked"), Mapping::Mapped("./m.js"));
        assert_eq!(
            packages.mapping("three"),
            Mapping::Conflicting {
                first: "https://a.test/x",
                second: "https://b.test/y",
            }
        );
        assert_eq!(packages.mapping("absent"), Mapping::Missing);
    }
}
