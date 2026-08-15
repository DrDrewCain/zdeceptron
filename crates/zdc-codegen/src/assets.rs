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
//! [`collect`] now decides containment on the **resolved** path, against
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

use std::path::{Path, PathBuf};

/// The directory a program's assets live in, relative to its entry file.
pub const ASSET_DIR: &str = "assets";

/// One file to copy into the bundle.
pub struct Asset {
    /// Where it goes, relative to the bundle root: `assets/site.css`.
    pub relative: String,
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
    /// order. Sorted by path, so the order is the same on every machine
    /// and a developer can control it by naming files.
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
    // Deterministic order, so two builds of the same tree agree.
    assets.files.sort_by(|a, b| a.relative.cmp(&b.relative));
    assets.stylesheets = assets
        .files
        .iter()
        .filter(|asset| {
            Path::new(&asset.relative)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("css"))
        })
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
                relative,
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

        let shipped: Vec<&str> = assets.files.iter().map(|a| a.relative.as_str()).collect();
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
        assert_eq!(
            assets.stylesheets,
            [
                "/assets/1-first.css",
                "/assets/2-later.css",
                "/assets/deep/nested.css"
            ]
        );
        let copied: Vec<&str> = assets
            .files
            .iter()
            .map(|asset| asset.relative.as_str())
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
            .map(|asset| asset.relative.as_str())
            .collect();
        assert_eq!(copied, ["assets/Inter.woff2", "assets/fonts.css"]);
        assert_eq!(assets.stylesheets, ["/assets/fonts.css"]);

        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
