// The published site documents private items — `docs.yml` passes
// `--document-private-items` and argues why — so a link from these docs
// to a private helper resolves there. Same ruling as `minify`'s.
#![allow(rustdoc::private_intra_doc_links)]

//! The asset directory: how a real stylesheet reaches a build.
//!
//! §6.1 argues that `class is "…"` makes existing CSS frameworks work, and
//! it does — `class="zd-col prose"` emits correctly, statically and
//! dynamically. But `index.html` linked exactly one stylesheet, the one the
//! compiler generates, and there was nowhere to put a file. So the four
//! hundred utility classes a program can *name* could not be *delivered*,
//! and §6.1's claim was architecturally sound and practically empty.
//!
//! A program's assets live in `assets/` beside its entry file. Everything
//! under it is copied into the bundle unchanged, and every `.css` file is
//! linked from the document after the generated stylesheet — so a program's
//! own rules win over the base classes without an `!important`.
//!
//! This is the one part of `zdc-codegen` that touches the filesystem, and
//! it is a separate entry point for that reason: `compile` takes the result
//! as data (`Options::stylesheets`) and reads no file itself.
//!
//! # A stylesheet's name carries its content hash (#137)
//!
//! "Copied unchanged" is still true of the bytes and no longer true of the
//! name: `assets/site.css` ships as `assets/site.<hash>.css`, and the href
//! in the document is that same string. A file whose URL is a function of
//! its bytes can be cached for a year with `immutable`, and one whose URL
//! is not has to be revalidated on every load forever — that is the whole
//! of what the rename buys, and [`crate::cache`] is where the policy it
//! feeds is written down.
//!
//! Stylesheets only, and the reason is that this compiler prints the
//! `<link>` and therefore owns the reference. It does not own
//! `Image source is "/assets/desk.png"`, which is a program's own text, nor
//! `url(./Inter.woff2)` inside an author's stylesheet. So the font and the
//! image keep the names they were given, and [`rename_hashed`] carries the
//! argument in full.
//!
//! The href and the destination are **one field** ([`Asset::relative`]),
//! not two that agree. A build that computed them separately could emit a
//! document pointing at a file it did not write, and the symptom of that is
//! an unstyled page and a 404 in a network panel nobody has open.
//!
//! # Shipping a font, and what the sandbox rule for it is
//!
//! Issue #88 asked whether a font file can be part of the asset directory
//! and what that means for the build-time capability sandbox. It can, and
//! it means nothing, and the second half is the part worth writing down.
//!
//! The build-time capability sandbox (`capability.rs`) exists for paths a
//! *program* names: `build read "…"` takes a path from the source text, so
//! it has to refuse a climbing path and a symlink pointing out of the
//! project. Nothing here takes a path from a program. [`ASSET_DIR`] is a
//! compiler constant, the root is the entry file's own parent, and the
//! walk copies what it finds. So a `.woff2` under `assets/` ships exactly
//! as a `.svg` does, and the `@font-face` that names it belongs in an
//! `assets/*.css`, which is linked after the generated stylesheet.
//!
//! **That gap is closed (#188).** It used to be stated here rather than
//! glossed: `read_dir` and `Path::is_dir` both follow symlinks, so a link
//! planted under `assets/` copied whatever it pointed at into the bundle.
//! The walk was the whole policy, and a symlink is a hop a walk does not
//! notice.
//!
//! `collect` now decides containment on the **resolved** path, against
//! the project root fixed from the entry file, through the same
//! `sandbox::escapes` the capability rule uses — so the two routes into a
//! build share one boundary instead of one having a weaker version of the
//! other's. An asset that resolves outside is refused *by name* and fails
//! the build: a stylesheet that vanishes from a bundle with no
//! explanation is a worse afternoon than one that is named.
//!
//! The old note argued the risk was small because whoever can plant the
//! symlink can also edit the source. That is true and it was the wrong
//! test — the same argument excuses every path check in the compiler, and
//! §14D.2's transitive-`use` traversal was fixed on the opposite
//! principle: a boundary re-based at each hop is not a boundary.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::hash::hashed_name;

/// The directory a program's assets live in, relative to its entry file.
pub const ASSET_DIR: &str = "assets";

/// One file to copy into the bundle.
pub struct Asset {
    /// Where it goes, relative to the bundle root: `assets/site.css`, or
    /// `assets/site.<hash>.css` for a stylesheet that carries a content
    /// hash (#137).
    ///
    /// **This is the destination and the URL both.** The href in the
    /// document is this string with `./` in front of it, and the file
    /// written to disk is this path, and they are one field for exactly
    /// that reason: a build where the two are computed separately is a
    /// build where they can disagree, and a disagreement here is a 404 for
    /// a stylesheet with nothing anywhere saying so.
    pub relative: String,
    /// The name it was written under in the project, relative to the
    /// bundle root: `assets/site.css`.
    ///
    /// Equal to [`relative`](Asset::relative) for everything that is not
    /// hashed. Kept because it is the name a person typed, so it is the
    /// name a diagnostic should use and the name another stylesheet's
    /// `@import` would spell.
    pub declared: String,
    /// Where it came from.
    pub source: PathBuf,
}

/// Everything `assets/` contributes to a build.
#[derive(Default)]
pub struct Assets {
    pub files: Vec<Asset>,
    /// Assets refused because they resolve outside the project, by the
    /// name they were written under.
    ///
    /// Reported rather than silently skipped: a stylesheet that vanishes
    /// from a bundle with no explanation is a worse afternoon than one
    /// that is named (#188).
    pub refused: Vec<String>,
    /// The stylesheets among them, as document-relative hrefs in cascade
    /// order. Sorted by the name they were written under, so the order is
    /// the same on every machine, a developer can control it by naming
    /// files, and it does not move when a file's *content* changes.
    pub stylesheets: Vec<String>,
    /// The site's icon, if the asset directory has one, as a root-absolute
    /// href.
    ///
    /// A browser asks for `/favicon.ico` on its own whether a document
    /// mentions one or not, so a site without this answers a request every
    /// visitor makes with a 404 in the console. Naming it in the head is
    /// also the only way to use any other format or path, which is most of
    /// them: `.svg` scales and `.png` is what a designer hands over.
    ///
    /// Found by name rather than declared, because there is exactly one
    /// icon and a program that had to say so would say it once, in a place
    /// the compiler would then have to invent.
    pub icon: Option<String>,
    /// The bundle-relative paths among [`files`](Assets::files) whose
    /// names carry a content hash, sorted (#137).
    ///
    /// What the emitted cache configuration marks `immutable`, and nothing
    /// else is eligible: see [`crate::cache`] for the rule and for why the
    /// rest of the asset directory is not on this list.
    pub immutable: Vec<String>,
}

/// The assets beside `entry`, or nothing if it has no asset directory.
///
/// A directory that cannot be read is not an error: a program with no
/// assets is the common case, and the difference between "absent" and
/// "unreadable" is not one the build can act on differently.
pub fn discover(entry: &Path) -> Assets {
    let root = match entry.parent() {
        Some(parent) => parent.join(ASSET_DIR),
        None => PathBuf::from(ASSET_DIR),
    };
    let mut assets = Assets::default();
    // The boundary is the project root, fixed from the entry file, exactly
    // as it is for a path a program writes. Assets reach the bundle by a
    // different route and must not reach it under a weaker rule (#188).
    let project = zdc_hir::sandbox::project_root(entry);
    collect(&root, ASSET_DIR, &project, &mut assets);
    assets.refused.sort();
    // Deterministic order, so two builds of the same tree agree — and by
    // the *declared* name, not by the destination, so that changing a
    // stylesheet's content cannot reorder the cascade by changing its
    // hash.
    assets.files.sort_by(|a, b| a.declared.cmp(&b.declared));
    rename_hashed(&mut assets);
    assets.stylesheets = assets
        .files
        .iter()
        // By the *declared* name: hashing renames the file, and a
        // stylesheet is still a stylesheet after its name carries a
        // digest (#137).
        .filter(|asset| is_css(&asset.declared))
        // Root-absolute, not document-relative.
        //
        // `./assets/site.css` resolves against the *document's* directory,
        // so it is only correct for a document at the root. A routed
        // program emits `/writing/<slug>/index.html`, and there the same
        // href asks for `/writing/<slug>/assets/site.css`, which is a 404 —
        // the page renders unstyled and nothing says why. The generated
        // stylesheet beside it was already `/pages/….css`; this is the
        // asset sheet agreeing with it.
        .map(|asset| format!("/{}", asset.relative))
        .collect();
    // The first of the names a browser and a designer between them expect,
    // in the order a browser prefers them: a vector scales, a PNG is what
    // gets handed over, an ICO is what the default request asks for.
    assets.icon = [
        "assets/favicon.svg",
        "assets/favicon.png",
        "assets/favicon.ico",
    ]
    .into_iter()
    .find(|name| {
        assets
            .files
            .iter()
            .any(|asset| asset.relative.eq_ignore_ascii_case(name))
    })
    .map(|name| format!("/{name}"));
    assets
}

/// Whether a bundle-relative path is a stylesheet.
fn is_css(relative: &str) -> bool {
    Path::new(relative)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("css"))
}

/// Put a content hash in the name of every stylesheet whose only reference
/// is the `<link>` this compiler prints — #137.
///
/// Stylesheets and nothing else, because a stylesheet is the one thing in
/// this directory the compiler *links*. A font, an image and a `.woff2`
/// are named by the program's own text (`Image source is
/// "/assets/desk.png"`) or by an author's `url()`, and renaming a file
/// whose references this compiler did not write is how a page ends up
/// missing an image with nothing in the build saying so. [`crate::cache`]
/// argues the whole rule.
///
/// The exception inside the exception is `@import`: one stylesheet may
/// name another, and that reference is the author's too. So the stylesheets
/// that any asset names are found first and left under their own names —
/// they are then served with the host's default, revalidating policy, which
/// is correct if unexciting.
///
/// A file that cannot be read keeps its name. There is nothing to hash, and
/// the failure is not this function's to report: the caller copies these
/// files and will meet the same unreadable file with somewhere useful to
/// say so.
fn rename_hashed(assets: &mut Assets) {
    let declared: BTreeSet<&str> = assets
        .files
        .iter()
        .map(|asset| asset.declared.as_str())
        .collect();

    // Every asset another asset names. Read from the stylesheets only: a
    // reference to a file is written in text, and the text in this
    // directory that can hold one is CSS.
    let mut named: BTreeSet<String> = BTreeSet::new();
    for asset in assets.files.iter().filter(|a| is_css(&a.declared)) {
        let Ok(text) = std::fs::read_to_string(&asset.source) else {
            continue;
        };
        for specifier in references(&text) {
            if let Some(path) = resolve(&asset.declared, &specifier) {
                if declared.contains(path.as_str()) {
                    named.insert(path);
                }
            }
        }
    }

    for asset in &mut assets.files {
        if !is_css(&asset.declared) || named.contains(&asset.declared) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&asset.source) else {
            continue;
        };
        asset.relative = hashed_name(&asset.declared, &bytes);
        assets.immutable.push(asset.relative.clone());
    }
    assets.immutable.sort();
}

/// Every file a stylesheet names, as written.
///
/// Deliberately a scan and not a parser, and deliberately generous: a
/// specifier this misses is a file that gets renamed while something still
/// points at the old name, and a specifier it invents is at worst one
/// stylesheet that keeps its own name and revalidates. The two errors are
/// not the same size, so this leans the cheap way — a commented-out
/// `@import` counts, and so does one inside a string.
fn references(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    // `url(…)`, which is also how the other spelling of `@import` names
    // its target.
    let mut rest = text;
    while let Some(at) = rest.find("url(") {
        rest = &rest[at + 4..];
        if let Some(end) = rest.find(')') {
            out.push(unquote(rest[..end].trim()).to_string());
        }
    }
    // `@import "…"` — the clause ends at the semicolon, and a media query
    // may follow the specifier, so only the first string counts.
    let mut rest = text;
    while let Some(at) = rest.find("@import") {
        rest = &rest[at + 7..];
        let clause = match rest.find(';') {
            Some(end) => &rest[..end],
            None => rest,
        };
        if let Some(specifier) = first_string(clause) {
            out.push(specifier.to_string());
        }
    }
    out
}

/// A CSS specifier without its quotes, if it had any.
fn unquote(specifier: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = specifier
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    specifier
}

/// The first quoted string in a clause, if there is one.
fn first_string(clause: &str) -> Option<&str> {
    let start = clause.find(['"', '\''])?;
    let quote = clause[start..].chars().next()?;
    let rest = &clause[start + quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

/// Where a specifier written inside `from` points, as a path relative to
/// the bundle root, or `None` when it does not name a file in this bundle.
///
/// A `data:` URI, an absolute URL and a protocol-relative one all name
/// something no build owns, so they are not paths here and are not
/// resolved.
fn resolve(from: &str, specifier: &str) -> Option<String> {
    let specifier = specifier.split(['?', '#']).next().unwrap_or(specifier);
    if specifier.is_empty() || specifier.starts_with("//") || specifier.contains("://") {
        return None;
    }
    // A scheme, and not a Windows drive letter or a bare name with a
    // colon in it: anything before a `:` that is not a path separator.
    if let Some(colon) = specifier.find(':') {
        if !specifier[..colon].contains('/') {
            return None;
        }
    }
    let mut parts: Vec<&str> = Vec::new();
    if !specifier.starts_with('/') {
        // Relative to the directory of the file that wrote it.
        if let Some(slash) = from.rfind('/') {
            parts.extend(from[..slash].split('/'));
        }
    }
    for part in specifier.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                // A climb out of the bundle root names nothing this build
                // writes, so it is not a reference to an asset. The
                // sandbox in `collect` is what keeps files from *outside*
                // the project out; this is only about resolving a name.
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn collect(directory: &Path, prefix: &str, project: &Path, out: &mut Assets) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // A dotfile in an asset directory is an editor's business, not the
        // bundle's, and `.DS_Store` should not ship.
        if name.starts_with('.') {
            continue;
        }
        let relative = format!("{prefix}/{name}");
        let path = entry.path();
        // Decided on the *resolved* path, so a symlink cannot launder one.
        // `read_dir` and `is_dir` both follow links, so without this the
        // walk copies whatever the link points at — and a link is a hop
        // the walk does not otherwise notice.
        if zdc_hir::sandbox::escapes(project, &path) {
            out.refused.push(relative);
            continue;
        }
        if path.is_dir() {
            collect(&path, &relative, project, out);
        } else {
            out.files.push(Asset {
                // The destination starts as the declared name and stays
                // it unless `rename_hashed` has a reason to move it.
                relative: relative.clone(),
                declared: relative,
                source: path,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_with_no_asset_directory_contributes_nothing() {
        let assets = discover(Path::new("/nonexistent/app.zd"));
        assert!(assets.files.is_empty());
        assert!(assets.stylesheets.is_empty());
    }

    /// #188. `Path::is_dir` and `read_dir` both follow symlinks, so a link
    /// planted under `assets/` copied a file from outside the project into
    /// the bundle. The walk was the whole policy, and a symlink is a hop it
    /// did not notice.
    ///
    /// Unix only, because planting the link is the fixture and creating
    /// one on Windows needs a privilege an ordinary account does not
    /// have. Gated on the test rather than skipped inside it: the body
    /// used to open with `#[cfg(not(unix))] return;`, which left every
    /// line after it unreachable on Windows and cost a warning the
    /// Linux-only `clippy` never saw (#163). This is the form the other
    /// three symlink tests in this workspace already use.
    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_outside_the_project_is_refused_by_name() {
        let root = std::env::temp_dir().join(format!("zdc-escape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let directory = root.join(ASSET_DIR);
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        std::fs::write(directory.join("ok.css"), "a{}").expect("an honest asset");

        // The target is outside the project by construction: a sibling of
        // the project root, not under it.
        let outside =
            std::env::temp_dir().join(format!("zdc-escape-{}-secret", std::process::id()));
        std::fs::write(&outside, "not yours").expect("a file outside the project");

        std::os::unix::fs::symlink(&outside, directory.join("stolen.css")).expect("a symlink");

        let assets = discover(&root.join("app.zd"));

        let shipped: Vec<&str> = assets.files.iter().map(|a| a.declared.as_str()).collect();
        assert_eq!(
            shipped,
            ["assets/ok.css"],
            "only the file that is really inside the project ships"
        );
        assert_eq!(
            assets.refused,
            vec!["assets/stolen.css".to_string()],
            "and the one that is not is refused by name"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn stylesheets_are_linked_in_a_stable_order() {
        let root = std::env::temp_dir().join(format!("zdc-assets-{}", std::process::id()));
        let directory = root.join(ASSET_DIR);
        std::fs::create_dir_all(directory.join("deep")).expect("a temporary directory");
        for (path, body) in [
            ("2-later.css", "b{}"),
            ("1-first.css", "a{}"),
            ("logo.svg", "<svg/>"),
            (".DS_Store", "junk"),
            ("deep/nested.css", "c{}"),
        ] {
            std::fs::write(directory.join(path), body).expect("a temporary file");
        }

        let assets = discover(&root.join("app.zd"));
        // The cascade is ordered by the name a developer typed, which is
        // the only thing they can control. A hash sorts arbitrarily, so
        // ordering by the destination would let editing one rule reorder
        // the whole cascade.
        let linked: Vec<&str> = assets.stylesheets.iter().map(String::as_str).collect();
        assert_eq!(linked.len(), 3);
        assert!(linked[0].starts_with("/assets/1-first."), "{linked:?}");
        assert!(linked[1].starts_with("/assets/2-later."), "{linked:?}");
        assert!(linked[2].starts_with("/assets/deep/nested."), "{linked:?}");
        let copied: Vec<&str> = assets
            .files
            .iter()
            .map(|asset| asset.declared.as_str())
            .collect();
        assert_eq!(
            copied,
            [
                "assets/1-first.css",
                "assets/2-later.css",
                "assets/deep/nested.css",
                "assets/logo.svg"
            ],
            "a dotfile is not part of the bundle"
        );

        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// The bug this whole design is arranged against: a document that
    /// links a file the build did not write. It is asserted as an identity
    /// rather than as two strings that match, because the href *is* the
    /// destination — there is one field, and this is the test that says so
    /// out loud (#137).
    #[test]
    fn every_linked_href_is_a_file_the_build_writes() {
        let root = std::env::temp_dir().join(format!("zdc-href-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let directory = root.join(ASSET_DIR);
        std::fs::create_dir_all(directory.join("deep")).expect("a temporary directory");
        for (path, body) in [
            ("site.css", "a{color:red}"),
            ("deep/nested.css", "b{color:blue}"),
            ("logo.svg", "<svg/>"),
        ] {
            std::fs::write(directory.join(path), body).expect("a temporary file");
        }

        let assets = discover(&root.join("app.zd"));
        let written: BTreeSet<&str> = assets
            .files
            .iter()
            .map(|asset| asset.relative.as_str())
            .collect();
        assert_eq!(assets.stylesheets.len(), 2);
        for href in &assets.stylesheets {
            // Root-absolute, so that a routed page at `/writing/<slug>/`
            // asks for the same file the root does.
            let path = href.strip_prefix('/').expect("a root-absolute href");
            assert!(
                written.contains(path),
                "the document links `{href}`, which no file in {written:?} is"
            );
        }

        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// Both directions of the property a hashed name exists for. Same
    /// bytes, same URL — or every deploy invalidates every cache. Changed
    /// bytes, changed URL — or a browser keeps serving the old ones out of
    /// an entry it was told never to revisit.
    #[test]
    fn a_stylesheets_name_is_stable_for_its_content_and_changes_with_it() {
        let root = std::env::temp_dir().join(format!("zdc-rehash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let directory = root.join(ASSET_DIR);
        std::fs::create_dir_all(&directory).expect("a temporary directory");

        std::fs::write(directory.join("site.css"), "a{color:red}").expect("a temporary file");
        let first = discover(&root.join("app.zd")).stylesheets;
        let again = discover(&root.join("app.zd")).stylesheets;
        assert_eq!(first, again, "the same bytes must produce the same URL");
        assert_ne!(
            first,
            ["./assets/site.css".to_string()],
            "a stylesheet's name carries a hash"
        );

        std::fs::write(directory.join("site.css"), "a{color:blue}").expect("a temporary file");
        let changed = discover(&root.join("app.zd")).stylesheets;
        assert_ne!(
            first, changed,
            "one changed declaration must produce a different URL"
        );

        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// An `@import` is the author's reference, not the compiler's, so the
    /// file it names keeps the name it was given. The importer is still
    /// hashed: renaming it moves nothing that the import resolves against,
    /// because the directory does not change.
    #[test]
    fn a_stylesheet_another_one_imports_keeps_its_name() {
        let root = std::env::temp_dir().join(format!("zdc-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let directory = root.join(ASSET_DIR);
        std::fs::create_dir_all(directory.join("deep")).expect("a temporary directory");
        std::fs::write(
            directory.join("site.css"),
            "@import \"./deep/base.css\";\na{color:red}",
        )
        .expect("a temporary file");
        std::fs::write(directory.join("deep/base.css"), "b{}").expect("a temporary file");

        let assets = discover(&root.join("app.zd"));
        let destination = |declared: &str| {
            assets
                .files
                .iter()
                .find(|asset| asset.declared == declared)
                .map(|asset| asset.relative.clone())
                .unwrap_or_else(|| panic!("{declared} is not in the bundle"))
        };
        assert_eq!(
            destination("assets/deep/base.css"),
            "assets/deep/base.css",
            "the import is written in the author's stylesheet, so the file it names cannot move"
        );
        assert_ne!(
            destination("assets/site.css"),
            "assets/site.css",
            "the importer is only reached by the link this compiler prints"
        );
        assert_eq!(
            assets.immutable,
            vec![destination("assets/site.css")],
            "only a hashed name may be served immutably"
        );

        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// The other half of the rule. An image is named by the program's own
    /// text — `Image source is "/assets/logo.svg"` — and this compiler
    /// never wrote that string, so it may not move the file.
    #[test]
    fn everything_that_is_not_a_stylesheet_keeps_the_name_it_was_given() {
        let root = std::env::temp_dir().join(format!("zdc-verbatim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let directory = root.join(ASSET_DIR);
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        for (path, body) in [("logo.svg", "<svg/>"), ("Inter.woff2", "not really a font")] {
            std::fs::write(directory.join(path), body).expect("a temporary file");
        }

        let assets = discover(&root.join("app.zd"));
        for asset in &assets.files {
            assert_eq!(
                asset.relative, asset.declared,
                "{} moved, and nothing here can rewrite what names it",
                asset.declared
            );
        }
        assert!(
            assets.immutable.is_empty(),
            "nothing carries a hash, so nothing may be cached for a year"
        );

        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// The answer to #88's second question. A font file ships because the
    /// asset walk copies everything, and the `@font-face` naming it works
    /// because its stylesheet is linked after the generated one.
    #[test]
    fn a_font_file_ships_and_the_rule_that_names_it_is_linked_last() {
        let root = std::env::temp_dir().join(format!("zdc-font-{}", std::process::id()));
        let directory = root.join(ASSET_DIR);
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        std::fs::write(directory.join("Inter.woff2"), "not really a font")
            .expect("a temporary file");
        std::fs::write(
            directory.join("fonts.css"),
            "@font-face { font-family: Inter; src: url(./Inter.woff2); }",
        )
        .expect("a temporary file");

        let assets = discover(&root.join("app.zd"));
        let copied: Vec<&str> = assets
            .files
            .iter()
            .map(|asset| asset.declared.as_str())
            .collect();
        assert_eq!(copied, ["assets/Inter.woff2", "assets/fonts.css"]);
        assert_eq!(assets.stylesheets.len(), 1);
        assert!(
            assets.stylesheets[0].starts_with("/assets/fonts."),
            "{:?}",
            assets.stylesheets
        );
        // The `src: url(./Inter.woff2)` is relative to the stylesheet's own
        // directory, and hashing changed the stylesheet's name and not its
        // directory. So the font is still where the rule says it is —
        // which is the property that made stylesheets safe to rename and
        // the font itself not (#137).
        let font = assets
            .files
            .iter()
            .find(|asset| asset.declared == "assets/Inter.woff2")
            .expect("the font ships");
        assert_eq!(font.relative, "assets/Inter.woff2");

        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_specifier_resolves_against_the_stylesheet_that_wrote_it() {
        assert_eq!(
            resolve("assets/deep/site.css", "./base.css").as_deref(),
            Some("assets/deep/base.css")
        );
        assert_eq!(
            resolve("assets/deep/site.css", "../base.css").as_deref(),
            Some("assets/base.css")
        );
        assert_eq!(
            resolve("assets/site.css", "/assets/base.css").as_deref(),
            Some("assets/base.css")
        );
        // Nothing this build owns, so nothing to keep a name for.
        assert_eq!(
            resolve("assets/site.css", "https://example.com/a.css"),
            None
        );
        assert_eq!(resolve("assets/site.css", "//example.com/a.css"), None);
        assert_eq!(resolve("assets/site.css", "data:text/css,a{}"), None);
        // A climb past the bundle root names no file in the bundle.
        assert_eq!(resolve("assets/site.css", "../../base.css"), None);
        // A query or a fragment is not part of the name on disk.
        assert_eq!(
            resolve("assets/site.css", "./base.css?v=2").as_deref(),
            Some("assets/base.css")
        );
    }

    #[test]
    fn both_spellings_of_an_import_are_found() {
        let found = references(
            "@import \"a.css\";\n\
             @import url(b.css);\n\
             @import url('c.css') screen;\n\
             @import 'd.css' layer(base);\n\
             a { background: url(\"e.png\") }",
        );
        for expected in ["a.css", "b.css", "c.css", "d.css", "e.png"] {
            assert!(found.iter().any(|f| f == expected), "{expected}: {found:?}");
        }
    }
}
