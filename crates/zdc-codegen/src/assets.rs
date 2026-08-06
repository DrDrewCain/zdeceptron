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
//! What that leaves is one honest gap, stated rather than glossed:
//! [`collect`] resolves symlinks, because `Path::is_dir` follows them, so
//! a symlink planted under `assets/` copies a file from outside the
//! project into the bundle. That is a build reading its own directory
//! rather than a program escaping a sandbox, and whoever can plant the
//! symlink can also edit the source, but it is a difference from the
//! capability sandbox's rule and not an application of it.

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
    /// The stylesheets among them, as document-relative hrefs in cascade
    /// order. Sorted by path, so the order is the same on every machine
    /// and a developer can control it by naming files.
    pub stylesheets: Vec<String>,
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
    collect(&root, ASSET_DIR, &mut assets.files);
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
        .map(|asset| format!("./{}", asset.relative))
        .collect();
    assets
}

fn collect(directory: &Path, prefix: &str, out: &mut Vec<Asset>) {
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
        if path.is_dir() {
            collect(&path, &relative, out);
        } else {
            out.push(Asset {
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
                "./assets/1-first.css",
                "./assets/2-later.css",
                "./assets/deep/nested.css"
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
        assert_eq!(assets.stylesheets, ["./assets/fonts.css"]);

        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
