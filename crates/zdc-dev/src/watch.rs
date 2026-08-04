//! Noticing that the source changed.
//!
//! **Why polling rather than a native-notification crate.** Every editor
//! worth using saves atomically: write a temporary file, then rename it
//! over the original. A native watcher registered against the *inode*
//! therefore stops seeing the file after the first save, which is the
//! failure mode where the dev server works once and then quietly never
//! rebuilds again. Watching the *path* by stat is immune to that, is
//! byte-identical on macOS, Linux and Windows, needs no platform bindings
//! at all — so it cannot compromise the single-static-binary property of
//! spec §7 — and, because it is a pure function of two stat results, it
//! can be tested without timing assumptions.
//!
//! The cost is up to one poll interval of latency. At the default of 120 ms
//! that is below the time a browser takes to repaint, and the stat of a
//! handful of paths is not measurable next to the compile it triggers.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The default gap between polls.
pub const POLL: Duration = Duration::from_millis(120);

/// What a stat tells us about one path.
///
/// Length is recorded alongside the timestamp because filesystem
/// timestamp granularity is coarse enough (one second, on some) that two
/// saves inside one tick would otherwise look like no save at all.
/// A missing file is `None`, which is a state like any other: deleting the
/// source is a change, and so is putting it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

/// Watches a fixed set of paths for any change.
#[derive(Debug)]
pub struct Watcher {
    paths: Vec<PathBuf>,
    seen: Vec<Option<Stamp>>,
}

impl Watcher {
    /// Start watching, taking the current state as the baseline: a watcher
    /// created just after a build must not immediately report a change.
    pub fn new(paths: Vec<PathBuf>) -> Watcher {
        let seen = paths.iter().map(|p| stamp(p)).collect();
        Watcher { paths, seen }
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Whether anything watched has changed since the last call.
    ///
    /// Consuming: a change is reported once, then the new state becomes the
    /// baseline. A caller that polls in a loop therefore gets one rebuild
    /// per save rather than a rebuild every tick forever.
    pub fn changed(&mut self) -> bool {
        let mut changed = false;
        for (path, seen) in self.paths.iter().zip(self.seen.iter_mut()) {
            let now = stamp(path);
            if now != *seen {
                *seen = now;
                changed = true;
            }
        }
        changed
    }
}

/// The paths a build of `entry` depends on.
///
/// Today that is the entry file alone: modules and imports are a v1
/// non-goal (spec §13), so a program is one file, and the runtime library
/// it links against is compiled into this binary rather than read from
/// disk. This function is where the import graph attaches when §14G lands
/// — the watcher above already takes a set, so nothing else has to change.
pub fn watch_set(entry: &Path) -> Vec<PathBuf> {
    vec![entry.to_path_buf()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// A file in a private temporary directory, removed when the test ends
    /// whether it passed or not.
    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("zdc-watch-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("could not create the scratch directory");
            Scratch { dir }
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.dir.join(name);
            let mut file = std::fs::File::create(&path).expect("could not write the scratch file");
            file.write_all(contents.as_bytes()).expect("write failed");
            file.sync_all().expect("sync failed");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_fresh_watcher_reports_no_change() {
        let scratch = Scratch::new("fresh");
        let path = scratch.write("a.zd", "state x is client Whole starting 0\n");
        let mut watcher = Watcher::new(vec![path]);
        assert!(!watcher.changed(), "a build that just ran must not rebuild");
    }

    #[test]
    fn rewriting_the_file_reports_a_change_exactly_once() {
        let scratch = Scratch::new("rewrite");
        let path = scratch.write("a.zd", "one\n");
        let mut watcher = Watcher::new(vec![path.clone()]);

        scratch.write("a.zd", "one and two\n");
        assert!(watcher.changed(), "the edit was not noticed");
        assert!(
            !watcher.changed(),
            "one save must not rebuild forever afterwards"
        );
    }

    #[test]
    fn a_change_of_length_alone_is_noticed() {
        // Filesystem timestamps are coarse; two saves within one tick can
        // share an mtime. Without the length the second would be invisible.
        let scratch = Scratch::new("length");
        let path = scratch.write("a.zd", "one\n");
        let mut watcher = Watcher::new(vec![path.clone()]);

        let baseline = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, "one two three\n").unwrap();
        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        // Only meaningful if the clock did not tick between the writes; if
        // it did, the mtime alone would have caught it and there is nothing
        // to prove here.
        if baseline == after {
            assert!(watcher.changed(), "a same-mtime edit went unnoticed");
        }
    }

    #[test]
    fn an_atomic_rename_over_the_file_is_noticed() {
        // How every serious editor saves: write a temporary, rename over
        // the target. A watcher bound to the original inode goes deaf here.
        let scratch = Scratch::new("rename");
        let path = scratch.write("a.zd", "one\n");
        let mut watcher = Watcher::new(vec![path.clone()]);

        let temp = scratch.write("a.zd.tmp", "one and two\n");
        std::fs::rename(&temp, &path).expect("rename failed");

        assert!(watcher.changed(), "an atomic save went unnoticed");
    }

    #[test]
    fn deleting_and_restoring_the_file_are_both_changes() {
        let scratch = Scratch::new("delete");
        let path = scratch.write("a.zd", "one\n");
        let mut watcher = Watcher::new(vec![path.clone()]);

        std::fs::remove_file(&path).expect("could not delete");
        assert!(watcher.changed(), "the deletion went unnoticed");
        assert!(!watcher.changed(), "the deletion was reported twice");

        scratch.write("a.zd", "one\n");
        assert!(watcher.changed(), "the file coming back went unnoticed");
    }

    #[test]
    fn a_watcher_on_a_file_that_does_not_exist_yet_notices_it_appearing() {
        // `zdc dev` on a path the developer is about to create should
        // start working when they create it, not require a restart.
        let scratch = Scratch::new("absent");
        let path = scratch.dir.join("later.zd");
        let mut watcher = Watcher::new(vec![path.clone()]);
        assert!(!watcher.changed(), "an absent file is not a change");

        scratch.write("later.zd", "state x is client Whole starting 0\n");
        assert!(watcher.changed(), "the new file went unnoticed");
    }

    #[test]
    fn a_change_to_any_watched_path_is_a_change() {
        let scratch = Scratch::new("many");
        let a = scratch.write("a.zd", "one\n");
        let b = scratch.write("b.zd", "two\n");
        let mut watcher = Watcher::new(vec![a, b]);

        scratch.write("b.zd", "two and a half\n");
        assert!(watcher.changed(), "a change to the second path was missed");
    }

    #[test]
    fn the_watch_set_of_a_single_file_program_is_that_file() {
        let entry = Path::new("examples/counter.zd");
        assert_eq!(watch_set(entry), vec![entry.to_path_buf()]);
    }
}
