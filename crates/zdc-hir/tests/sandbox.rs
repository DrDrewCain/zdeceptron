use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use zdc_hir::sandbox::{project_root, refuse, Refusal};

static NEXT_TREE: AtomicUsize = AtomicUsize::new(0);

struct TempTree {
    base: PathBuf,
    project: PathBuf,
}

impl TempTree {
    fn new() -> TempTree {
        let serial = NEXT_TREE.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("zdc-hir-sandbox-{}-{serial}", std::process::id()));
        let project = base.join("project");
        std::fs::create_dir_all(project.join("views")).expect("create the project tree");
        std::fs::create_dir_all(base.join("outside")).expect("create the outside tree");
        TempTree { base, project }
    }

    fn root(&self) -> &Path {
        &self.project
    }

    fn outside(&self) -> PathBuf {
        self.base.join("outside")
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[test]
fn project_root_is_the_canonical_parent_of_the_entry_file() {
    let tree = TempTree::new();
    let entry = tree.root().join("app.zd");

    assert_eq!(
        project_root(&entry),
        tree.root().canonicalize().expect("canonical project root")
    );
}

#[test]
fn existing_and_missing_targets_inside_the_project_are_allowed() {
    let tree = TempTree::new();
    let existing = tree.root().join("views/list.zd");
    std::fs::write(&existing, "view\n").expect("write the fixture");

    assert_eq!(refuse(tree.root(), "./views/list.zd", &existing), None);
    assert_eq!(
        refuse(
            tree.root(),
            "./views/not-created-yet.zd",
            &tree.root().join("views/not-created-yet.zd")
        ),
        None
    );
}

#[test]
fn a_parent_segment_is_allowed_when_the_resolved_target_stays_inside() {
    let tree = TempTree::new();
    let sibling = tree.root().join("model.zd");
    std::fs::write(&sibling, "record Model\n").expect("write the fixture");

    assert_eq!(refuse(tree.root(), "../model.zd", &sibling), None);
}

#[test]
fn malformed_specifiers_are_refused_before_filesystem_resolution() {
    let tree = TempTree::new();
    let target = tree.root().join("file.zd");
    let cases = [
        ("", "is empty"),
        ("/etc/passwd", "is an absolute path"),
        ("\\server\\share", "is an absolute path"),
        ("C:secrets.zd", "names a drive or a scheme"),
        ("https:module", "names a drive or a scheme"),
        ("views/", "names a directory rather than a file"),
    ];

    for (specifier, reason) in cases {
        let refusal = refuse(tree.root(), specifier, &target).expect("must be refused");
        assert!(matches!(refusal, Refusal::Syntax(_)));
        assert_eq!(refusal.reason(), reason);
    }
}

#[test]
fn a_missing_target_outside_the_project_is_still_refused() {
    let tree = TempTree::new();
    let target = tree.outside().join("not-created-yet.zd");

    assert_eq!(
        refuse(tree.root(), "../outside/not-created-yet.zd", &target),
        Some(Refusal::Climbs)
    );
}

#[cfg(unix)]
#[test]
fn an_in_project_symlink_to_an_outside_file_is_refused() {
    use std::os::unix::fs::symlink;

    let tree = TempTree::new();
    let outside = tree.outside().join("secret.zd");
    std::fs::write(&outside, "secret state key\n").expect("write outside fixture");
    let link = tree.root().join("linked.zd");
    symlink(&outside, &link).expect("create the fixture symlink");

    assert_eq!(
        refuse(tree.root(), "linked.zd", &link),
        Some(Refusal::Links)
    );
}

#[test]
fn a_root_that_does_not_exist_cannot_act_as_a_boundary() {
    let tree = TempTree::new();
    let missing_root = tree.base.join("missing-root");
    let target = tree.root().join("file.zd");

    assert_eq!(
        refuse(&missing_root, "file.zd", &target),
        Some(Refusal::Links)
    );
}
