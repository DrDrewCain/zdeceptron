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

/// A directory under the system temporary directory, removed when the test
/// ends whether it passed or not.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("zdc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
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

#[test]
fn checking_a_valid_file_exits_0_and_says_nothing() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/guestbook.zd");
    let output = run(&["check", example.to_str().expect("utf-8 path")]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit code 0, stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "a clean check says nothing at all"
    );
}

/// Resolution reports every error it finds. Three undefined names is
/// three diagnostics from one run, not one diagnostic three runs
/// running.
#[test]
fn checking_a_file_with_three_undefined_names_reports_all_three() {
    let source = TempSource::new(
        "undefined-names",
        "state a is client Whole from nope\n\
         state b is client Whole from alsonope\n\
         state c is client Whole from thirdnope\n",
    );
    let path = source.path.to_str().expect("utf-8 path");
    let output = run(&["check", path]);

    assert_eq!(output.status.code(), Some(1), "expected exit code 1");

    let stderr = String::from_utf8_lossy(&output.stderr);
    for name in ["nope", "alsonope", "thirdnope"] {
        assert!(
            stderr.contains(name),
            "every undefined name must be reported, `{name}` was not:\n{stderr}"
        );
    }
}

/// A file that does not parse cannot be resolved, so `check` reports the
/// syntax error rather than a cascade of names it could not read.
#[test]
fn checking_a_file_with_a_syntax_error_reports_the_syntax_error() {
    let source = TempSource::new("check-syntax-error", "view Text\n");
    let path = source.path.to_str().expect("utf-8 path");
    let output = run(&["check", path]);

    assert_eq!(output.status.code(), Some(1), "expected exit code 1");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("line break"),
        "stderr must carry the parse error:\n{stderr}"
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

#[test]
fn checking_accepts_a_forward_reference() {
    let source = TempSource::new(
        "forward-reference",
        concat!(
            "state doubled is client Whole from count + count\n",
            "state count is client Whole starting 1\n",
        ),
    );
    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "forward references are order-independent:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[test]
fn checking_reports_a_duplicate_top_level_name_once() {
    let source = TempSource::new(
        "duplicate-name",
        concat!(
            "state item is client Whole starting 1\n",
            "function item\n",
            "    give empty\n",
        ),
    );
    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stderr.matches("already declared").count(), 1, "{stderr}");
    assert!(stderr.contains("item"), "{stderr}");
    assert!(output.stdout.is_empty());
}

#[test]
fn checking_reports_unknown_elements_and_variants_together() {
    let source = TempSource::new(
        "bad-view-names",
        concat!(
            "state status is client Whole starting 1\n",
            "view\n",
            "    Colunm\n",
            "    when status\n",
            "        Loadng show Spinner\n",
        ),
    );
    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr.contains("Colunm") && stderr.contains("Column"),
        "{stderr}"
    );
    assert!(
        stderr.contains("Loadng") && stderr.contains("Loading"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
}

/// A pattern binder is in scope in its arm, and it has the type of the
/// field it names (spec §14G.1.2).
#[test]
fn checking_accepts_a_binding_from_a_named_variant_pattern() {
    let source = TempSource::new(
        "variant-bindings",
        concat!(
            "state status is durable Text starting \"\"\n",
            "view\n",
            "    when status\n",
            "        Loading           show Spinner\n",
            "        Failed with error show ErrorBar message is error.message\n",
            "        Ready with text   show Text text\n",
        ),
    );
    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "every pattern binding should be in scope:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

/// Binders are positional over the variant's declared fields, so binding
/// more of them than the variant has is a type error naming both counts.
#[test]
fn checking_rejects_a_pattern_that_binds_more_names_than_the_variant_has() {
    let source = TempSource::new(
        "variant-overbinding",
        concat!(
            "state status is durable Text starting \"\"\n",
            "view\n",
            "    when status\n",
            "        Loading                   show Spinner\n",
            "        Failed with why, moment   show Spinner\n",
            "        Ready with text           show Text text\n",
        ),
    );
    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("1 field"), "{stderr}");
    assert!(
        stderr.contains('2'),
        "the message should say how many: {stderr}"
    );
}

/// The headline guarantee: `Remote of T` cannot be read without writing
/// all three arms, in every context (spec §14G.1.6).
#[test]
fn checking_rejects_a_when_that_forgets_an_arm() {
    let source = TempSource::new(
        "missing-arm",
        concat!(
            "state visits is durable Whole starting 0\n",
            "view\n",
            "    when visits\n",
            "        Loading          show Spinner\n",
            "        Ready with total show Text total\n",
        ),
    );
    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("`Failed`"), "{stderr}");
    assert!(stderr.contains("Remote of Whole"), "{stderr}");
}

/// Three type errors, three diagnostics, one run.
#[test]
fn checking_a_file_with_three_type_errors_reports_all_three() {
    let source = TempSource::new(
        "three-type-errors",
        concat!(
            "state a is client Text  starting 1\n",
            "state b is client Whole starting \"two\"\n",
            "state c is client Truth starting 3\n",
        ),
    );
    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(
        stderr.matches("Error:").count(),
        3,
        "checking must not stop at the first type error:\n{stderr}"
    );
}

// --- build ----------------------------------------------------------------

/// Exit 0 and a complete `dist/`: the success half of the contract a deploy
/// script depends on. `elements.js` is deliberately absent — generated code
/// never imports it (spec §16.3.1).
#[test]
fn building_a_client_only_example_exits_0_and_writes_the_bundle() {
    let out = TempDir::new("build-hello");
    let output = run(&[
        "build",
        example("hello.zd").to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit code 0, stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "a clean build says nothing at all"
    );

    for expected in [
        "client.js",
        "styles.css",
        "index.html",
        "manifest.json",
        "runtime/signal.js",
        "runtime/dom.js",
    ] {
        assert!(
            out.path.join(expected).is_file(),
            "the bundle is missing {expected}"
        );
    }
    assert!(
        !out.path.join("runtime/elements.js").exists(),
        "elements.js must not be shipped"
    );

    let client = std::fs::read_to_string(out.path.join("client.js")).expect("client.js");
    assert!(
        client.contains("export function main(container)"),
        "{client}"
    );
    assert!(client.contains("template("), "{client}");

    let styles = std::fs::read_to_string(out.path.join("styles.css")).expect("styles.css");
    assert!(styles.contains(".zd-col"), "{styles}");
}

/// `guestbook.zd` now **checks** — the split derives its network, the type
/// checker types its `Remote of Text`, and the flow pass clears it — and
/// still does not **build**, because view-position `when` needs the hole
/// machinery of milestone M5b. The remaining refusal names that milestone
/// rather than the missing pass, which is the whole distinction.
#[test]
fn guestbook_checks_and_its_remaining_build_refusal_names_the_milestone() {
    let checked = run(&[
        "check",
        example("guestbook.zd").to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "guestbook must check clean:\n{}",
        String::from_utf8_lossy(&checked.stderr)
    );

    let out = TempDir::new("build-guestbook");
    let output = run(&[
        "build",
        example("guestbook.zd").to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);

    assert_eq!(output.status.code(), Some(1), "expected exit code 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("M5b"),
        "the diagnostic must name the milestone that covers it:\n{stderr}"
    );
    assert!(
        stderr.contains("guestbook.zd"),
        "stderr must name the path:\n{stderr}"
    );
    assert!(output.stdout.is_empty());
    assert!(
        !out.path.exists(),
        "a failed build must not leave a half-written bundle behind"
    );
}

/// A program that crosses a placement boundary and needs no `when` builds,
/// and the network between the halves is the split's, not a hand-written
/// route table.
#[test]
fn a_cross_region_write_builds_into_a_client_bundle_and_a_server_function() {
    let source = TempSource::new(
        "build-command",
        concat!(
            "state visits is durable Whole starting 0\n",
            "view\n",
            "    Column\n",
            "        Button \"sign\"\n",
            "            on click\n",
            "                add 1 to visits\n",
        ),
    );
    let out = TempDir::new("build-command-out");
    let output = run(&[
        "build",
        source.path.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let client = std::fs::read_to_string(out.path.join("client.js")).expect("client.js");
    assert!(client.contains("$call('visits.incr', 1)"), "{client}");

    let function = std::fs::read_to_string(out.path.join("functions/visits.incr.js"))
        .expect("the generated command");
    // §16.3.12 assertion A: a function bundle has no imports and touches
    // no browser global. Its only external references are `$env` and
    // `$store`, injected by the platform adapter.
    for forbidden in ["import ", "document", "window"] {
        assert!(
            !function.contains(forbidden),
            "a function bundle must not contain `{forbidden}`:\n{function}"
        );
    }
    assert!(function.contains("$store.incr('visits'"), "{function}");

    let manifest = std::fs::read_to_string(out.path.join("manifest.json")).expect("manifest.json");
    assert!(manifest.contains("\"visits.incr\""), "{manifest}");
    assert!(manifest.contains("\"durable\":[\"visits\"]"), "{manifest}");
}

#[test]
fn building_a_file_with_a_syntax_error_reports_the_syntax_error() {
    let source = TempSource::new("build-syntax-error", "view Text\n");
    let out = TempDir::new("build-syntax-error-out");
    let output = run(&[
        "build",
        source.path.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("line break"), "{stderr}");
    assert!(!out.path.exists());
}

/// The two `--unchecked` gates from §16.7: refused by default, emitted with
/// the flag. An unenforced guarantee is worse than a refused build.
#[test]
fn unchecked_is_what_lets_a_type_gated_construct_through() {
    // `+` on two proved `Whole`s is emitted now — §16.7 item 1 is
    // answered — so the gate is shown on the construct that is still
    // behind it: a statement-position `when`, where a missing arm becomes
    // a runtime throw rather than a compile error.
    let source = TempSource::new(
        "build-unchecked",
        concat!(
            "state g is server Text starting \"x\"\n",
            "state shown is client Text from unwrap with 0\n",
            "\n",
            "function unwrap with ignore\n",
            "    when g\n",
            "        Loading           show \"...\"\n",
            "        Failed with error show \"!\"\n",
            "        Ready with text   show text\n",
            "\n",
            "view\n",
            "    Text shown\n",
        ),
    );
    let out = TempDir::new("build-unchecked-out");
    let args = [
        "build",
        source.path.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ];

    let refused = run(&args);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("--unchecked"),
        "the diagnostic must name the flag that lets it through"
    );

    let mut with_flag = args.to_vec();
    with_flag.push("--unchecked");
    let allowed = run(&with_flag);
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let client = std::fs::read_to_string(out.path.join("client.js")).expect("client.js");
    assert!(client.contains("switch ($w0.tag)"), "{client}");
    // And the boundary the split derived is in the same bundle: `g` is
    // `server`, so it is read through the generated RPC rather than
    // emitted into the browser.
    assert!(client.contains("$remote('g', [])"), "{client}");
}
