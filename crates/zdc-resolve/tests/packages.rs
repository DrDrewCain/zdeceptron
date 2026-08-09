//! Where a `foreign`'s module specifier resolves, and what the project has
//! to say for it to resolve at all (#238).
//!
//! Two rules used to make a package unreachable without a hand-written
//! `.js` file in between. A URL specifier was refused outright, which did
//! not prevent remote code — it relocated it into a two-line module that
//! imported the same URL, where the compiler could no longer see it. And a
//! bare specifier compiled and then could not load: `import … from 'three'`
//! with nothing shipped and no import map, so the browser failed on the
//! first import before any of the program ran.
//!
//! Both are settled here. A URL is written in the declaration, where `zdc`
//! can see it and report it. A bare specifier resolves through `zd.toml`
//! beside the entry file, and one with no entry there is a compile error
//! naming the file and the line to add — "compiles and cannot load" is no
//! longer one of the outcomes.

use std::path::{Path, PathBuf};

use zdc_hir::{DefKind, ModuleTarget};

/// A throwaway project directory: an entry file, and whatever `zd.toml`
/// the case under test needs beside it.
struct Project {
    root: PathBuf,
}

impl Project {
    fn new(name: &str) -> Project {
        let root = std::env::temp_dir().join(format!("zdc-packages-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a temporary directory");
        Project { root }
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::write(&path, contents).expect("writing a test file");
        path
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn resolve(entry: &Path) -> Result<zdc_hir::Hir, Vec<String>> {
    let linked =
        zdc_resolve::load(entry).unwrap_or_else(|failure| panic!("{}", failure.errors[0].message));
    zdc_resolve::Resolver::linked(&linked)
        .resolve()
        .map_err(|errors| errors.into_iter().map(|e| e.message).collect())
}

fn target_of(hir: &zdc_hir::Hir, name: &str) -> ModuleTarget {
    let (_, def) = hir
        .defs
        .iter()
        .find(|(_, def)| def.name == name)
        .unwrap_or_else(|| panic!("no definition named `{name}`"));
    let DefKind::Foreign(foreign) = &def.kind else {
        panic!("`{name}` is not a foreign");
    };
    foreign
        .target
        .clone()
        .unwrap_or_else(|| panic!("`{name}` imports, so it resolves a target"))
}

const RENDERER: &str = "foreign renderer is client\n\
                        \x20   from \"three\" as \"WebGLRenderer\"\n\
                        \x20   gives Text\n\
                        view\n\
                        \x20   Column\n";

/// The refusal did not prevent remote code, it hid it: `examples/tree/`
/// imports the same CDN URL from inside its own JavaScript, where nothing
/// in the compiler reasons about it. Stated in the declaration, it is
/// visible to the manifest and pinnable later.
#[test]
fn a_url_specifier_compiles() {
    let project = Project::new("url");
    let entry = project.write(
        "app.zd",
        "foreign renderer is client\n\
         \x20   from \"https://esm.sh/three@0.180.0\" as \"WebGLRenderer\"\n\
         \x20   gives Text\n\
         view\n\
         \x20   Column\n",
    );

    let hir = resolve(&entry).expect("a URL specifier resolves");
    assert_eq!(target_of(&hir, "renderer"), ModuleTarget::AsWritten);
}

/// A URL is fetched over the network, so `http:` and `https:` are the two
/// schemes that name one. Every other scheme names something else — the
/// code inline, a path on the build machine, a registry no browser
/// resolves — and stays refused.
#[test]
fn a_scheme_that_is_not_a_fetchable_url_is_still_refused() {
    for module in [
        "data:text/javascript,alert(1)",
        "file:///etc/passwd",
        "npm:left-pad",
        "//evil.example/x.js",
    ] {
        let project = Project::new("scheme");
        let entry = project.write(
            "app.zd",
            &format!(
                "foreign parse is anywhere\n\
                 \x20   from \"{module}\" as \"parse\"\n\
                 \x20   gives Text\n"
            ),
        );

        let errors = resolve(&entry).expect_err("`{module}` is not a URL a browser fetches");
        assert!(
            errors.iter().any(|e| e.contains("imports from")),
            "`{module}` must be refused, got {errors:?}"
        );
    }
}

/// The failure this replaces: `from \"three\"` compiled, shipped nothing,
/// wrote no import map, and the page died on its first import. A refusal
/// that names the file and the exact line to write is strictly better than
/// an artifact that cannot load.
#[test]
fn a_bare_specifier_with_no_mapping_is_refused_and_names_the_file() {
    let project = Project::new("unmapped");
    let entry = project.write("app.zd", RENDERER);

    let errors = resolve(&entry).expect_err("nothing resolves `three`");
    let message = errors.first().expect("one diagnostic").clone();
    assert!(
        message.contains("zd.toml"),
        "the repair is a line in a file, so the message names it: {message}"
    );
    assert!(
        message.contains("three = \"https://"),
        "the repair is stated as the line to write: {message}"
    );
}

/// The project says once, and the declaration stays a type signature. This
/// is what keeps `zdc-codegen/src/lib.rs:163`'s rule intact: nothing is
/// guessed, because there is now somewhere for the program to say.
#[test]
fn a_bare_specifier_mapped_in_zd_toml_resolves_to_its_target() {
    let project = Project::new("mapped");
    project.write(
        "zd.toml",
        "[packages]\nthree = \"https://esm.sh/three@0.180.0\"\n",
    );
    let entry = project.write("app.zd", RENDERER);

    let hir = resolve(&entry).expect("the project mapped `three`");
    assert_eq!(
        target_of(&hir, "renderer"),
        ModuleTarget::Mapped("https://esm.sh/three@0.180.0".to_string())
    );
}

/// A vendored copy under `assets/` is a mapping like any other, so it goes
/// through the `linked_module` machinery #223 already built rather than
/// through a second filesystem path.
#[test]
fn a_relative_mapping_target_resolves() {
    let project = Project::new("vendored");
    project.write("zd.toml", "[packages]\nthree = \"./assets/three.js\"\n");
    let entry = project.write("app.zd", RENDERER);

    let hir = resolve(&entry).expect("a vendored copy is a mapping too");
    assert_eq!(
        target_of(&hir, "renderer"),
        ModuleTarget::Mapped("./assets/three.js".to_string())
    );
}

/// One specifier is one module. Last-writer-wins would make which build of
/// three.js a page loads depend on the order of two lines, which is the
/// version skew the project-level mapping exists to prevent.
#[test]
fn one_specifier_mapped_to_two_targets_is_refused() {
    let project = Project::new("conflict");
    project.write(
        "zd.toml",
        "[packages]\n\
         three = \"https://esm.sh/three@0.180.0\"\n\
         three = \"https://esm.sh/three@0.179.0\"\n",
    );
    let entry = project.write("app.zd", RENDERER);

    let errors = resolve(&entry).expect_err("two targets for one specifier");
    let message = errors.first().expect("one diagnostic").clone();
    assert!(
        message.contains("0.180.0") && message.contains("0.179.0"),
        "both targets are named, because the reader has to choose: {message}"
    );
    assert!(
        message.contains("twice"),
        "the claim is that the file maps it twice: {message}"
    );
}

/// A mapping resolves a bare specifier, so a mapping *to* a bare specifier
/// resolves nothing — it moves the same unanswerable question one line
/// over.
#[test]
fn a_mapping_target_that_resolves_nothing_is_refused() {
    let project = Project::new("bare-target");
    project.write("zd.toml", "[packages]\nthree = \"three\"\n");
    let entry = project.write("app.zd", RENDERER);

    let errors = resolve(&entry).expect_err("a bare target resolves nothing");
    assert!(
        errors[0].contains("zd.toml"),
        "the mistake is in the mapping, so the message names it: {}",
        errors[0]
    );
}

/// A `zd.toml` this build cannot read is a stop, not a shrug: carrying on
/// with an empty mapping would report every bare specifier as unmapped and
/// hide the one mistake that caused it.
#[test]
fn a_zd_toml_that_is_not_a_package_mapping_is_reported() {
    let project = Project::new("malformed");
    project.write("zd.toml", "[packages]\nthree\n");
    project.write("app.zd", RENDERER);
    let entry = project.root.join("app.zd");

    let failure = zdc_resolve::load(&entry).expect_err("the mapping does not parse");
    assert!(
        failure.errors[0].message.contains("zd.toml"),
        "got {}",
        failure.errors[0].message
    );
    assert!(
        failure.errors[0].message.contains("line 2"),
        "the line is named, because that is where the repair goes: {}",
        failure.errors[0].message
    );
}

// --- the boundary ---------------------------------------------------------
//
// Allowing a URL widened what a `foreign` may name, so what it may *not*
// name is worth stating in tests rather than in a comment. A relative
// specifier is the only form the build turns into a file it opens, and it
// is bounded by `zdc_hir::sandbox` — the same entry point `use` and the
// build-time capabilities go through, so the three cannot drift apart.
//
// Note which half of the rule each of these exercises. `..` is lexical and
// a reader can see it; a symbolic link is not visible in the specifier at
// all and is caught only because the check is on the resolved path. And
// `zd.toml` is a second place a path can be written, so both are checked
// twice: once as written in the declaration, once as written in the
// mapping.

/// A directory outside the project, with something in it worth stealing.
struct Outside {
    root: PathBuf,
}

impl Outside {
    fn new(name: &str) -> Outside {
        let root = std::env::temp_dir().join(format!(
            "zdc-packages-outside-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a directory outside the project");
        std::fs::write(root.join("secret.js"), "export const parse = 1;\n")
            .expect("a file to steal");
        Outside { root }
    }
}

impl Drop for Outside {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("the symbolic link");
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("the symbolic link");
}

/// A relative specifier is copied out of the project directory into the
/// bundle, so it is the one form of specifier that decides which bytes on
/// the build host enter the program — and it is bounded exactly as `use`
/// is.
#[test]
fn a_specifier_that_climbs_out_of_the_project_is_refused() {
    let outside = Outside::new("climb");
    let project = Project::new("climb");
    let entry = project.write(
        "app.zd",
        &format!(
            "foreign parse is anywhere\n\
             \x20   from \"../{}/secret.js\" as \"parse\"\n\
             \x20   gives Text\n\
             view\n\
             \x20   Column\n",
            outside
                .root
                .file_name()
                .expect("a directory name")
                .to_str()
                .expect("utf-8")
        ),
    );

    let errors = resolve(&entry).expect_err("a specifier may not leave the project");
    assert!(
        errors[0].contains("climbs out of the project"),
        "the phrase is `zdc_hir::sandbox::Refusal`'s, so the rule is visibly the same one: {}",
        errors[0]
    );
}

/// `zd.toml` is a second place a path can be written, so a mapping that
/// was not bounded would be a way around the bound rather than a feature.
#[test]
fn a_mapping_target_that_climbs_out_of_the_project_is_refused() {
    let outside = Outside::new("mapped-climb");
    let project = Project::new("mapped-climb");
    project.write(
        "zd.toml",
        &format!(
            "[packages]\nthree = \"../{}/secret.js\"\n",
            outside
                .root
                .file_name()
                .expect("a directory name")
                .to_str()
                .expect("utf-8")
        ),
    );
    let entry = project.write("app.zd", RENDERER);

    let errors = resolve(&entry).expect_err("a mapping may not leave the project either");
    assert!(
        errors[0].contains("climbs out of the project"),
        "got {}",
        errors[0]
    );
    assert!(
        errors[0].contains("zd.toml"),
        "the repair is in the mapping, so the message names it: {}",
        errors[0]
    );
}

/// The half of the rule that is not visible in the specifier. This one
/// contains no `..` and no leading `/`; it reads as an ordinary sibling
/// module, and only resolving the path finds where it goes. It is caught
/// because the check is on the resolved path, which is the reason the rule
/// is trustworthy rather than merely discouraging.
#[test]
fn a_specifier_that_links_outside_the_project_is_refused() {
    let outside = Outside::new("link");
    let project = Project::new("link");
    symlink(
        &outside.root.join("secret.js"),
        &project.root.join("vendor.js"),
    );
    let entry = project.write(
        "app.zd",
        "foreign parse is anywhere\n\
         \x20   from \"./vendor.js\" as \"parse\"\n\
         \x20   gives Text\n\
         view\n\
         \x20   Column\n",
    );

    let errors = resolve(&entry).expect_err("a link out of the project is still out of it");
    assert!(
        errors[0].contains("points outside the project"),
        "got {}",
        errors[0]
    );
}

/// The same link, reached through the mapping instead. Both spellings go
/// through one rule, so neither is the way round.
#[test]
fn a_mapping_target_that_links_outside_the_project_is_refused() {
    let outside = Outside::new("mapped-link");
    let project = Project::new("mapped-link");
    symlink(
        &outside.root.join("secret.js"),
        &project.root.join("vendor.js"),
    );
    project.write("zd.toml", "[packages]\nthree = \"./vendor.js\"\n");
    let entry = project.write("app.zd", RENDERER);

    let errors = resolve(&entry).expect_err("a link out of the project is still out of it");
    assert!(
        errors[0].contains("points outside the project"),
        "got {}",
        errors[0]
    );
}

/// A URL is not a path, so the containment rule has nothing to say about
/// it — and must not say anything, or allowing the URL would have been
/// undone by the check added beside it.
#[test]
fn the_containment_rule_does_not_touch_a_url() {
    let project = Project::new("url-unbounded");
    project.write(
        "zd.toml",
        "[packages]\nthree = \"https://esm.sh/three@0.180.0/../three@0.179.0\"\n",
    );
    let entry = project.write("app.zd", RENDERER);

    let hir = resolve(&entry).expect("a `..` inside a URL is the URL's business, not the build's");
    assert_eq!(
        target_of(&hir, "renderer"),
        ModuleTarget::Mapped("https://esm.sh/three@0.180.0/../three@0.179.0".to_string())
    );
}

/// A copy vendored *inside* the project is the case the rule exists to
/// allow. Without this, every assertion above would be satisfied by a
/// compiler that refused every path.
#[test]
fn a_vendored_copy_inside_the_project_still_resolves() {
    let project = Project::new("vendored-inside");
    std::fs::create_dir_all(project.root.join("assets")).expect("an assets directory");
    std::fs::write(
        project.root.join("assets/three.js"),
        "export const WebGLRenderer = 1;\n",
    )
    .expect("the vendored copy");
    project.write("zd.toml", "[packages]\nthree = \"./assets/three.js\"\n");
    let entry = project.write("app.zd", RENDERER);

    let hir = resolve(&entry).expect("a copy inside the project is what the rule allows");
    assert_eq!(
        target_of(&hir, "renderer"),
        ModuleTarget::Mapped("./assets/three.js".to_string())
    );
}
