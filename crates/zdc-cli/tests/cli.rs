use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args(args)
        .output()
        .expect("failed to run the zdc binary")
}

/// A file under the system temporary directory, removed when the test
/// ends whether it passed or not.
struct TempSource {
    path: PathBuf,
}

impl TempSource {
    fn new(name: &str, contents: &str) -> TempSource {
        let path = std::env::temp_dir().join(format!("zdc-{}-{name}.zd", std::process::id()));
        std::fs::write(&path, contents).expect("failed to write the temporary source file");
        TempSource { path }
    }
}

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Exit 0 and a tree on stdout: the success half of the contract a shell
/// script or CI job depends on.
#[test]
fn parsing_a_valid_file_exits_0_and_prints_the_tree() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.zd");
    let output = run(&["parse", example.to_str().expect("utf-8 path")]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit code 0, stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("StateDecl") && stdout.contains("ViewDecl"),
        "expected the syntax tree on stdout:\n{stdout}"
    );
    assert!(
        output.stderr.is_empty(),
        "a successful parse must print nothing to stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Exit 1 and a diagnostic on stderr. A syntax error and an unreadable
/// file are both failures and both exit 1; what differs is the message.
#[test]
fn parsing_a_file_with_a_syntax_error_exits_1_and_reports_it() {
    let source = TempSource::new("syntax-error", "view Text\n");
    let path = source.path.to_str().expect("utf-8 path");
    let output = run(&["parse", path]);

    assert_eq!(output.status.code(), Some(1), "expected exit code 1");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(path),
        "stderr must name the path:\n{stderr}"
    );
    assert!(
        stderr.contains("line break"),
        "stderr must carry the parse error:\n{stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "a failed parse must not print a tree:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// End-to-end check: parsing a file that does not exist must exit 1 and
/// the rendered stderr must name both the path and the underlying OS
/// error, not a generic "could not read the file" message.
#[test]
fn parsing_a_nonexistent_file_exits_1_and_names_the_cause() {
    let missing = "this-file-does-not-exist-anywhere.zd";
    let output = run(&["parse", missing]);

    assert_eq!(output.status.code(), Some(1), "expected exit code 1");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(missing),
        "stderr must name the path:\n{stderr}"
    );
    assert!(
        stderr.contains("No such file or directory") || stderr.contains("cannot find the file"),
        "stderr must include the OS error text:\n{stderr}"
    );
}
