//! The capabilities a build may ask the compiler for, and the sandbox
//! they are answered inside.
//!
//! **Why this exists at all.** §17.4.8 said build-time `foreign` works
//! "because the build host can import `marked`". It cannot: the build host
//! is `boa_engine`, embedded in the compiler, and it ships ECMAScript
//! builtins only — no `node:fs`, no npm resolution. The claim was true only
//! under the `node` approach §7 forbids, so the case §14C.3b assumed and
//! §14G.1.5 ratified — `state posts is static List of Post from readPosts
//! with directory is "content"` — had no mechanism.
//!
//! **Why capabilities rather than a module loader.** A *runtime* `foreign`
//! calls into a real host — a browser, a serverless runtime — which
//! genuinely has npm and a DOM, and §14E is right about it. A *build-time*
//! call has no host. The compiler is the host. So the honest construct is
//! not "import a module" but "ask the compiler for a capability", and four
//! properties follow that a loader could not offer:
//!
//! * `zdc` stays one binary. Nothing is fetched, resolved or installed.
//! * It is pure Rust under `#![forbid(unsafe_code)]`, so the memory-safety
//!   claim of §7.5 covers the build-time surface too.
//! * It is **sandboxable**, which matters because a build that can read
//!   arbitrary paths is a supply-chain surface. Every path below is
//!   resolved and then required to be inside the project directory.
//! * It is **deterministic**, which §17.4.7 already argues a build must be.
//!   Directory order is the obvious hazard and it is normalised here.
//!
//! **And the cost, stated plainly: the set is closed.** It grows only when
//! the compiler is released. What bounds that cost is that growing it
//! spends no keyword — the capability name is an identifier inside the
//! `build` production, so a fourth capability costs a match arm.

use std::path::{Path, PathBuf};

use zdc_hir::BuildCapability;
use zdc_runtime::{Capability, Provided};

/// The prefix a refusal carries out through the JavaScript engine.
///
/// A capability reports failure by throwing, and the engine's message is
/// the only channel back. Marking it lets [`crate::evaluate`] tell a
/// refused path from a program that threw, which are different mistakes
/// and want different diagnostics.
pub const REFUSED: &str = "E11: ";

/// Every capability, in the order [`BuildCapability::ALL`] lists them.
pub fn all() -> Vec<Capability> {
    BuildCapability::ALL
        .into_iter()
        .map(|capability| Capability {
            name: capability.name(),
            answer: match capability {
                BuildCapability::Read => read,
                BuildCapability::List => list,
                BuildCapability::Markdown => markdown,
            },
        })
        .collect()
}

/// `build read path` — one file's contents.
fn read(root: &Path, path: &str) -> Result<Provided, String> {
    let resolved = resolve(root, path)?;
    if !resolved.is_file() {
        return Err(refusal(format!(
            "`{path}` is not a file in the project directory"
        )));
    }
    match std::fs::read(&resolved) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Ok(Provided::Text(text)),
            Err(_) => Err(refusal(format!(
                "`{path}` is not UTF-8, and `Text` is. A build reads text, not bytes"
            ))),
        },
        Err(error) => Err(refusal(format!("`{path}` could not be read: {error}"))),
    }
}

/// `build list directory` — the files directly inside a directory.
///
/// Three normalisations, each of which would otherwise make a build depend
/// on something that is not the program:
///
/// 1. **Sorted by byte order.** `read_dir` yields whatever the filesystem
///    yields, which differs between machines and between two runs on one
///    machine. A build that inlines a list must inline the same list.
/// 2. **Files only.** A subdirectory is not something `build read` could
///    consume, so including one would put a value in the list that no
///    other capability accepts.
/// 3. **Relative to the project directory**, so what comes out of `list`
///    goes straight into `read` without the program doing path arithmetic
///    the sandbox would then have to re-check.
fn list(root: &Path, path: &str) -> Result<Provided, String> {
    let resolved = resolve(root, path)?;
    if !resolved.is_dir() {
        return Err(refusal(format!(
            "`{path}` is not a directory in the project directory"
        )));
    }

    let entries = std::fs::read_dir(&resolved)
        .map_err(|error| refusal(format!("`{path}` could not be listed: {error}")))?;

    let mut found = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| refusal(format!("`{path}` could not be listed: {error}")))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(refusal(format!(
                "`{path}` holds a name that is not UTF-8, so it has no `Text` to list it as"
            )));
        };
        let relative = join(path, name);
        // Resolved, not written: an entry may be a symlink, and a symlink
        // out of the project is exactly what this refuses. Refused rather
        // than skipped, because a silently shorter list is a build that
        // succeeded while doing something other than what it said.
        let target = resolve(root, &relative)?;
        if target.is_file() {
            found.push(relative);
        }
    }
    found.sort();
    Ok(Provided::List(found))
}

/// `build markdown source` — CommonMark, rendered to HTML.
///
/// No extensions are enabled. Tables, footnotes and the rest are each a
/// decision about what the language's markdown *is*, and defaulting them
/// on would make the answer depend on a crate's idea of "common" rather
/// than on a specification.
fn markdown(_root: &Path, source: &str) -> Result<Provided, String> {
    let parser = pulldown_cmark::Parser::new(source);
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    Ok(Provided::Text(html))
}

/// Resolve a path against the project directory, or refuse it.
///
/// **Checked as a resolved path, not as a written one.** The lexical pass
/// is the outer of the two and exists for its diagnostics — it is
/// [`zdc_graph::unusable_path`], the same check `static` file *emission*
/// already applies to a declared output path (E0316), so a build's reads
/// and its writes are bounded by one rule rather than by two that could
/// drift. The resolved pass is the one that is load-bearing: `canonicalize`
/// follows every symlink, so a link pointing out of the project is caught
/// where no amount of inspecting the written path would catch it.
fn resolve(root: &Path, path: &str) -> Result<PathBuf, String> {
    if let Some(reason) = zdc_graph::unusable_path(path) {
        return Err(refusal(format!("`{path}` {reason}")));
    }

    let root = root.canonicalize().map_err(|error| {
        refusal(format!(
            "the project directory `{}` could not be resolved: {error}",
            root.display()
        ))
    })?;

    let target = root.join(path);
    let target = target
        .canonicalize()
        .map_err(|error| refusal(format!("`{path}` is not in the project directory: {error}")))?;

    if !target.starts_with(&root) {
        return Err(refusal(format!(
            "`{path}` resolves to `{}`, which is outside the project directory `{}`. A build \
             reads the project it is building and nothing else",
            target.display(),
            root.display()
        )));
    }
    Ok(target)
}

/// Join a directory and a name the way the language spells a path.
///
/// `/` always, never the host separator: the path a build inlines is part
/// of the program's value, and a value that differs between Windows and
/// Linux is not a build-time constant.
fn join(directory: &str, name: &str) -> String {
    let trimmed = directory.trim_end_matches('/');
    if trimmed.is_empty() {
        return name.to_string();
    }
    format!("{trimmed}/{name}")
}

fn refusal(message: String) -> String {
    format!("{REFUSED}{message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
    }

    #[test]
    fn a_climbing_path_is_refused_before_it_is_opened() {
        let refused = read(&project(), "../Cargo.toml").expect_err("must refuse");
        assert!(refused.starts_with(REFUSED), "{refused}");
        assert!(refused.contains("climbs out of the bundle"), "{refused}");
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let refused = read(&project(), "/etc/hosts").expect_err("must refuse");
        assert!(refused.contains("is an absolute path"), "{refused}");
    }

    #[test]
    fn a_listing_is_sorted_and_relative() {
        let Provided::List(found) = list(&project(), "content").expect("lists") else {
            panic!("`list` must give a list");
        };
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(found, sorted, "a listing must not depend on the filesystem");
        assert!(
            found.iter().all(|path| path.starts_with("content/")),
            "{found:?}"
        );
    }

    #[test]
    fn markdown_is_commonmark() {
        let Provided::Text(html) = markdown(&project(), "# Title\n\ntext\n").expect("renders")
        else {
            panic!("`markdown` must give text");
        };
        assert_eq!(html, "<h1>Title</h1>\n<p>text</p>\n");
    }
}
