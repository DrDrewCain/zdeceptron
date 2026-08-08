//! The one rule that bounds every path a program can make the build open.
//!
//! A ZDeceptron program writes paths, and the build acts on them: `use`
//! opens a module, and the build-time capabilities of §14C.3b (`static`
//! emission, and the `build read` / `build list` forms designed alongside
//! it) open or create files of their own. Every one of those is the same
//! hazard wearing a different keyword — a path written in a source file
//! decides which bytes on the build host enter the compilation.
//!
//! So the containment decision lives here, once, rather than beside each
//! keyword. [`refuse`] takes both the specifier as written and the path it
//! resolved to, and no caller can adopt one half of the check without the
//! other, because there is only the one entry point.
//!
//! # Two layers, and why neither is sufficient alone
//!
//! **The syntactic layer** reads the specifier as written and rejects the
//! forms that could not name a file inside *any* project — an absolute
//! path, a drive letter or URL scheme, a bare directory, the empty string.
//! These are refused on sight because no filesystem lookup could make them
//! acceptable, and refusing them early keeps the layer below from having
//! to reason about what `C:` or `""` would even resolve to.
//!
//! **The resolved layer** canonicalises the project root and the target
//! and requires the second to lie under the first. This is the layer that
//! actually holds, and it is what makes the rule trustworthy rather than
//! merely discouraging. Canonicalisation resolves symbolic links, so a
//! file planted *inside* the project that points outside it is caught
//! here and is invisible to the layer above: such a specifier contains no
//! `..` and no leading `/`, and reads as an ordinary sibling module.
//!
//! The division of labour matters. The syntactic layer is deliberately
//! *not* the place where escaping is decided — see [`malformed`] — so the
//! rule cannot be satisfied by string inspection alone, and a call site
//! that somehow skipped canonicalisation would be visibly skipping the
//! half that does the work.
//!
//! # What this rule is not
//!
//! It is deliberately *not* a check on the shape of the written specifier
//! beyond escaping. Different call sites legitimately spell paths
//! differently: a module specifier is written `"./model"` and the leading
//! `./` is the idiom §14D.2 requires, whereas an emitted file's path is
//! written `"rss.xml"` relative to the bundle root and a leading `./` is
//! meaningless there. Folding those surface conventions into this rule
//! would make it reject one call site's ordinary usage in the name of
//! another's. The invariant they share is containment, and containment is
//! all this decides; a call site that also wants a house style for its
//! specifiers enforces that separately, on top.

use std::path::{Path, PathBuf};

/// Why a path a program asked for may not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The specifier as written could never name a file inside any
    /// project. Carries a phrase describing the fault.
    Syntax(&'static str),
    /// The specifier climbed out of the project with `..`.
    Climbs,
    /// The specifier named an ordinary file inside the project, but that
    /// file resolves to somewhere outside it — a symbolic link.
    Links,
}

impl Refusal {
    /// The fault as a phrase completing "… names a file that …".
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::Syntax(reason) => reason,
            Refusal::Climbs => "climbs out of the project",
            Refusal::Links => "points outside the project",
        }
    }
}

/// The project root for a build entered at `entry`.
///
/// **The root is the entry file's parent directory**, fixed once when the
/// build starts and constant for every module the build goes on to reach.
///
/// The alternatives were each worse in a specific way. Taking the process's
/// working directory would mean the same program compiles or fails
/// depending on where `zdc` was run from, and running it from `/` would
/// switch the sandbox off entirely. Requiring a manifest would settle the
/// question, but v1 has no manifest and inventing a file format is a
/// language decision, not a fix to this one.
///
/// The part this corrects is subtler than the choice itself. The boundary
/// used to be recomputed per file — every module resolved its imports
/// against *its own* parent — which is not a boundary at all, because each
/// hop re-based it and a chain of modules could walk anywhere one `..` at
/// a time. One root for the whole build is what makes containment mean
/// something transitively: §14D.2's "paths are relative to the importing
/// file" still decides what a specifier *names*, while this decides what
/// the build may *open*, and the second no longer moves.
pub fn project_root(entry: &Path) -> PathBuf {
    let directory = entry.parent().unwrap_or(Path::new("."));
    let directory = if directory.as_os_str().is_empty() {
        Path::new(".")
    } else {
        directory
    };
    directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf())
}

/// Both layers of the rule. `None` means the build may open `target`.
///
/// `specifier` is the path exactly as the program wrote it; `target` is
/// what the caller resolved it to. Callers must run this *before* opening
/// the file — the point of the rule is that a refused path is never read,
/// so its bytes never reach the compilation whether or not anything later
/// reports an error.
pub fn refuse(root: &Path, specifier: &str, target: &Path) -> Option<Refusal> {
    if let Some(reason) = malformed(specifier) {
        return Some(Refusal::Syntax(reason));
    }
    if escapes(root, target) {
        // Both are the same fault — the build was asked for a file it may
        // not open — but they are worth telling apart, because the fix
        // differs and one of them is not visible in the specifier at all.
        return Some(if climbs(specifier) {
            Refusal::Climbs
        } else {
            Refusal::Links
        });
    }
    None
}

/// The syntactic layer: faults visible in the specifier alone.
///
/// Note what is *not* here. A `..` segment is not refused on sight, even
/// though it is how the obvious attack is written, because it is also how
/// a module in a subdirectory legitimately reaches a sibling of its own
/// parent — `use "../model"` from `views/list.zd` names a file inside the
/// project and has no business being an error. Whether a `..` leaves the
/// project is a question about where it lands, so it is [`escapes`] that
/// answers it. Refusing the segment here instead would trade a real
/// capability for a check that the layer below already performs.
fn malformed(specifier: &str) -> Option<&'static str> {
    if specifier.is_empty() {
        return Some("is empty");
    }
    if specifier.starts_with('/') || specifier.starts_with('\\') {
        return Some("is an absolute path");
    }
    // A drive letter or a URL scheme. Neither can name a file inside the
    // project, and `C:relative` on Windows is resolved against a
    // per-drive directory the build does not control.
    if specifier.contains(':') {
        return Some("names a drive or a scheme");
    }
    if specifier.ends_with('/') || specifier.ends_with('\\') {
        return Some("names a directory rather than a file");
    }
    None
}

/// Did the specifier ask to go up at all?
fn climbs(specifier: &str) -> bool {
    specifier.split(['/', '\\']).any(|segment| segment == "..")
}

/// The resolved layer: does `target` lie outside `root` once both are
/// canonical?
///
/// Canonicalising the target is what resolves symbolic links, and the
/// final component has to be resolved too — a module file that is itself a
/// link to somewhere outside the project is the case the syntactic layer
/// cannot see.
///
/// When the target does not exist there is nothing to canonicalise, so the
/// deepest existing ancestor is canonicalised instead and the remainder
/// appended. This is what keeps a *missing* file from escaping the check:
/// canonicalising the existing ancestor collapses any `..` above it, so
/// `../../nope` still resolves to a location outside the root and is
/// refused there rather than being waved through as merely absent.
///
/// A target with no existing ancestor at all resolves to nothing and is
/// allowed past — it names no file, so there is nothing to read, and the
/// caller's own "could not read" diagnostic is the right answer.
/// Whether `target` resolves outside `root`.
///
/// Public because containment is decided in one place on purpose, and a
/// second caller needed it: asset collection walks a directory rather than
/// a path a program wrote, so [`refuse`] — which also checks the shape of
/// a written specifier — is the wrong entry point, but the boundary must
/// be the same one (#188).
pub fn escapes(root: &Path, target: &Path) -> bool {
    let Ok(root) = root.canonicalize() else {
        // A root that cannot be canonicalised cannot be a boundary. Refuse
        // rather than treat the absence of a root as permission.
        return true;
    };

    match resolved(target) {
        Some(target) => !target.starts_with(&root),
        None => false,
    }
}

/// `target` with every symbolic link resolved, as far as the filesystem
/// can answer.
fn resolved(target: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = target.canonicalize() {
        return Some(canonical);
    }

    let mut trailing = Vec::new();
    let mut ancestor = target;
    loop {
        let parent = ancestor.parent()?;
        trailing.push(ancestor.file_name()?.to_os_string());
        if let Ok(canonical) = parent.canonicalize() {
            let mut full = canonical;
            for name in trailing.iter().rev() {
                full.push(name);
            }
            return Some(full);
        }
        ancestor = parent;
    }
}
