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

/// **`guestbook.zd`'s own comment, made true.**
///
/// "Writing `Text apiKey` anywhere in the view below is a compile error."
/// It never was, in nine previous stages. This is that test, end to end,
/// through the binary a developer actually runs.
#[test]
fn rendering_the_secret_is_a_compile_error_naming_the_escape_path() {
    let original = std::fs::read_to_string(example("guestbook.zd")).expect("guestbook is readable");
    let leaked = original.replace(
        "        Input name, hint is \"your name\"",
        "        Input name, hint is \"your name\"\n        Text apiKey",
    );
    assert_ne!(
        leaked, original,
        "the fixture must actually change the view"
    );
    let source = TempSource::new("check-leak", &leaked);

    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);
    assert_eq!(output.status.code(), Some(1), "the leak must be refused");

    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    assert!(
        stderr.contains("E-IFC-05"),
        "the view sink must be the one that rejected it:\n{stderr}"
    );
    // §7.3: the path, not merely the fact.
    assert!(
        stderr.contains("declared secret"),
        "the path must start at the declaration:\n{stderr}"
    );
    assert!(
        stderr.contains("in the browser"),
        "the path must end where the browser would see it:\n{stderr}"
    );
}

/// The same file, untouched, checks clean. Without this the rule above is
/// indistinguishable from "reject anything containing `secret`".
#[test]
fn guestbook_itself_checks_clean() {
    let output = run(&[
        "check",
        example("guestbook.zd").to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}

/// §14A.1: the client bundle *provably* excludes server logic. Asserted
/// against the emitted bytes as well as against the walk, because the
/// claim is about what ships.
#[test]
fn the_emitted_client_bundle_contains_no_server_logic() {
    let source = TempSource::new(
        "build-exclusion",
        concat!(
            "secret state apiKey is server Text from environment \"GREETING_API_KEY\"\n",
            "state name is client Text starting \"\"\n",
            "state greeting is server Text from politeGreeting with name, apiKey\n",
            "\n",
            "function politeGreeting with who, key\n",
            "    give who\n",
            "\n",
            "state shown is client Text from unwrap with 0\n",
            "\n",
            "function unwrap with ignore\n",
            "    when greeting\n",
            "        Loading           show \"...\"\n",
            "        Failed with error show \"!\"\n",
            "        Ready with text   show text\n",
            "\n",
            "view\n",
            "    Column\n",
            "        Input name, hint is \"your name\"\n",
            "        Text shown\n",
        ),
    );
    let out = TempDir::new("build-exclusion-out");
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
    for excluded in ["apiKey", "GREETING_API_KEY", "politeGreeting", "$env"] {
        assert!(
            !client.contains(excluded),
            "`{excluded}` must not reach the browser:\n{client}"
        );
    }
    assert!(client.contains("$remote('greeting', [name])"), "{client}");

    // ... and the server half has it, and only it.
    let function =
        std::fs::read_to_string(out.path.join("functions/greeting.js")).expect("the endpoint");
    assert!(function.contains("$env('GREETING_API_KEY')"), "{function}");
    assert!(function.contains("function politeGreeting("), "{function}");
    assert!(
        function.contains("export async function handler({ name })"),
        "{function}"
    );
    // Dependencies first. A `const` referenced before its declaration is a
    // temporal-dead-zone `ReferenceError`, not a hoisted `undefined`.
    let env_at = function.find("$env(").expect("the environment read");
    let use_at = function
        .find("politeGreeting(name, apiKey)")
        .expect("the call that uses it");
    assert!(
        env_at < use_at,
        "the binding must precede its use:\n{function}"
    );
    for forbidden in ["import ", "document", "window"] {
        assert!(
            !function.contains(forbidden),
            "a function bundle must not contain `{forbidden}`:\n{function}"
        );
    }
}

/// `ariadne` colours character by character, so a test that reads the text
/// has to take the escapes back out.
fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
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

/// `guestbook.zd` checks **and builds**. The split derives its network, the
/// type checker types its `Remote of Text`, the flow pass clears it, and
/// M5b's hole machinery emits the view-position `when`s the build used to
/// refuse. Three placements, and every one of them comes out of the
/// compiler rather than out of a route table.
#[test]
fn guestbook_checks_and_builds_across_all_three_placements() {
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
    assert_eq!(
        output.status.code(),
        Some(0),
        "guestbook must build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The client half: two `Remote` reads through the generated RPC, the
    // durable write as a command, and no trace of the secret.
    let client = std::fs::read_to_string(out.path.join("client.js")).expect("client.js");
    assert!(client.contains("$remote('greeting', [name])"), "{client}");
    assert!(client.contains("$remote('visits', [])"), "{client}");
    assert!(client.contains("$call('visits.incr', 1)"), "{client}");
    assert!(client.contains("whenInto("), "{client}");
    for excluded in ["apiKey", "GREETING_API_KEY", "politeGreeting", "$env"] {
        assert!(
            !client.contains(excluded),
            "`{excluded}` must not reach the browser:\n{client}"
        );
    }

    // The server half: the secret, and only there.
    let greeting =
        std::fs::read_to_string(out.path.join("functions/greeting.js")).expect("the endpoint");
    assert!(greeting.contains("$env('GREETING_API_KEY')"), "{greeting}");
    assert!(greeting.contains("function politeGreeting("), "{greeting}");
    assert!(
        greeting.contains("export async function handler({ name })"),
        "{greeting}"
    );
    for excluded in ["import ", "document", "window"] {
        assert!(
            !greeting.contains(excluded),
            "`{excluded}` must not reach a function bundle:\n{greeting}"
        );
    }
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

/// §16.7 items 1 and 2 were gated behind `--unchecked` while there was no
/// checker to consult. There is one, `build` runs it, and its verdict is
/// what codegen reads — so the flag is gone and the operators are emitted.
#[test]
fn a_typechecked_program_emits_the_operators_that_needed_a_verdict() {
    let source = TempSource::new(
        "build-operators",
        concat!(
            "state a is client Whole starting 1\n",
            "state b is client Whole from a + 1\n",
            "state same is client Truth from a is 1\n",
            "view\n",
            "    Column\n",
            "        Text b\n",
            "        Text same\n",
        ),
    );
    let out = TempDir::new("build-operators-out");
    let built = run(&[
        "build",
        source.path.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        built.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let client = std::fs::read_to_string(out.path.join("client.js")).expect("client.js");
    assert!(client.contains("a() + 1"), "{client}");
    assert!(client.contains("a() === 1"), "{client}");
}

/// The other construct that was gated behind `--unchecked`: a
/// statement-position `when`, where a missing arm used to become a runtime
/// throw. Exhaustiveness is the checker's verdict now (§14G.1.6), so it
/// builds unflagged — and the crossing the split derived is in the same
/// bundle.
#[test]
fn a_statement_when_over_a_remote_builds_and_keeps_the_crossing() {
    let source = TempSource::new(
        "build-statement-when",
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
    let out = TempDir::new("build-statement-when-out");
    let built = run(&[
        "build",
        source.path.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        built.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let client = std::fs::read_to_string(out.path.join("client.js")).expect("client.js");
    assert!(client.contains("switch ($w0.tag)"), "{client}");
    // `g` is `server`, so it is read through the generated RPC rather than
    // emitted into the browser.
    assert!(client.contains("$remote('g', [])"), "{client}");
}

/// A program that does not typecheck produces no bundle. Building past a
/// type error is exactly the case §16.7 names.
#[test]
fn a_type_error_refuses_the_build_and_writes_nothing() {
    let source = TempSource::new(
        "build-type-error",
        concat!(
            "state a is client Whole starting \"not a number\"\n",
            "view\n",
            "    Text a\n",
        ),
    );
    let out = TempDir::new("build-type-error-out");
    let refused = run(&[
        "build",
        source.path.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(!out.path.exists());
}
