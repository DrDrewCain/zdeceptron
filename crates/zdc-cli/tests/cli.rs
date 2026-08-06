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
    let source = TempSource::new("syntax-error", "view\n    Text \"a\" Text \"b\"\n");
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

/// `counter.zd` rather than `guestbook.zd`, which this named until `zdc
/// check` began running the emitter: `guestbook.zd` declares `durable`
/// state, and emitting a placement boundary needs `zdc-graph` and
/// `runtime/store.js` (§16.5, M6). It never built, and now it never checks
/// either — the two commands agree about it, which is the point.
#[test]
fn checking_a_valid_file_exits_0_and_says_nothing() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter.zd");
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
    let source = TempSource::new("check-syntax-error", "view\n    Text \"a\" Text \"b\"\n");
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
    // falsifiable: the two arms are the same message on different
    // platforms — Unix says "No such file or directory", Windows says
    // "cannot find the file" — and neither is a substring of any path or
    // of the generic wording this test exists to reject. On any one host
    // exactly one arm can hold, so the disjunction cannot mask the other.
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
    let stderr = String::from_utf8_lossy(&output.stderr);

    // This used to assert that one refusal was left — `is `durable`-placed`
    // — because the emitter could not cross a placement boundary. It can
    // now, so there is nothing left to report and the assertion is the
    // stronger one: the program checks. Neither `error` nor `text` is a
    // name that does not exist, which is what the test is about, and the
    // clean exit says so without needing a refusal to survive.
    assert!(
        !stderr.contains("is not defined"),
        "every pattern binding should be in scope:\n{stderr}"
    );
    assert!(
        stderr.is_empty(),
        "nothing is wrong with this program:\n{stderr}"
    );
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
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

/// A file with no `view` is a module (§14D.2), not a mistake: it declares
/// names for other files to import and renders nothing. `zdc build` builds
/// it, and stops at the module — §16.3.1's page imports a `main` a module
/// does not export, so writing that page would ship a document whose only
/// script throws on load.
#[test]
fn building_a_module_with_no_view_exits_0_and_writes_no_page() {
    let out = TempDir::new("build-module");
    let output = run(&[
        "build",
        example("model.zd").to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "a module is a legitimate program shape, stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        out.path.join("client.js").is_file(),
        "the module itself must be written"
    );
    assert!(
        !out.path.join("index.html").exists(),
        "a module renders nothing, so there is no page to write"
    );

    let client = std::fs::read_to_string(out.path.join("client.js")).expect("client.js");
    assert!(
        client.contains("export function visible(all)"),
        "every top-level declaration is importable (§14D.2):\n{client}"
    );
    assert!(
        !client.contains("main("),
        "a module has no entry point:\n{client}"
    );
}

/// §6.1's claim that existing CSS frameworks work was architecturally
/// sound and practically empty: `class is "prose"` emitted correctly, and
/// there was nowhere to put the file that defines `.prose`. A program's
/// `assets/` is that place.
#[test]
fn a_programs_asset_directory_ships_and_its_stylesheets_are_linked() {
    let workspace = TempDir::new("build-assets-src");
    let assets = workspace.path.join("assets");
    std::fs::create_dir_all(assets.join("fonts")).expect("a temporary asset directory");
    std::fs::write(assets.join("site.css"), ".prose { max-width: 65ch; }\n").expect("site.css");
    std::fs::write(assets.join("fonts/note.txt"), "a font would go here\n").expect("an asset");

    let entry = workspace.path.join("app.zd");
    std::fs::write(
        &entry,
        "view title is \"Notes\"\n    Paragraph \"hello\", class is \"prose\"\n",
    )
    .expect("the entry file");

    let out = TempDir::new("build-assets-out");
    let output = run(&[
        "build",
        entry.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let shipped =
        std::fs::read_to_string(out.path.join("assets/site.css")).expect("assets/site.css");
    assert!(shipped.contains(".prose"), "{shipped}");
    assert!(
        out.path.join("assets/fonts/note.txt").is_file(),
        "an asset directory holds more than stylesheets"
    );

    let page = std::fs::read_to_string(out.path.join("index.html")).expect("index.html");
    assert!(
        page.contains(r#"<link rel="stylesheet" href="./assets/site.css">"#),
        "the stylesheet must be linked, not merely copied:\n{page}"
    );
    assert!(page.contains("<title>Notes</title>"), "{page}");
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
    // `visits` is durable, so it is bound through the live cell rather
    // than a plain `$remote`: another visitor can move it, and a push is
    // how this window finds out.
    assert!(
        client.contains("$durable('visits', 'visits', [])"),
        "{client}"
    );
    assert!(client.contains("$subscribe();"), "{client}");
    // The write goes into the handler's transaction and the transaction is
    // awaited. A discarded promise is a write whose failure nothing can
    // see and whose order against the next write is undefined; a write
    // sent on its own is one that can half-apply beside its siblings.
    assert!(client.contains("const $tx = [];"), "{client}");
    assert!(
        client.contains("$tx.push(['visits.incr', [1]]);"),
        "{client}"
    );
    assert!(client.contains("await $atomic($tx);"), "{client}");
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
    assert!(
        client.contains("$tx.push(['visits.incr', [1]]);"),
        "{client}"
    );
    assert!(client.contains("await $atomic($tx);"), "{client}");

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
    let source = TempSource::new("build-syntax-error", "view\n    Text \"a\" Text \"b\"\n");
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

/// The milestone-7 shape, built by the binary a developer actually runs:
/// `client` + `static`, content fixed at build time (§14C.3b).
///
/// The claim is negative as much as positive. The titles are *in* the
/// bundle, and there is no `$remote`, no `rpc.js` import and no
/// `functions/` directory for them to have come from — a `static` read
/// crosses no boundary, so §5.2's Rule 1 is satisfied rather than excepted
/// (§14G.1.4).
///
/// The build runs with **an empty `PATH`**. `static` values are computed
/// by evaluating the build root in the compiler's own engine, so `zdc`
/// stays the one thing a developer installs — and the way to keep that
/// true is to build where nothing else could possibly be found.
#[test]
fn a_static_program_builds_with_its_content_inlined_and_nothing_to_fetch() {
    let out = TempDir::new("build-static-out");
    let built = run_without_a_path(&[
        "build",
        example("writing.zd").to_str().expect("utf-8 path"),
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
    // The content is `examples/content/*.md`, read from disk by the build
    // capabilities and rendered by `pulldown-cmark`. What reaches the
    // browser is the rendered HTML as a string literal: no path, no
    // renderer, and nothing to fetch.
    assert!(
        client.contains(r#""slug":"content/hello-world.md""#),
        "the content must come from disk and be inlined as a literal:\n{client}"
    );
    assert!(
        client.contains(r#"<h1>Hello, world</h1>"#),
        "the markdown must be rendered at build time:\n{client}"
    );
    assert!(
        client.contains(r#"String("Writing")"#),
        "a derived `static` ships its answer, not its derivation:\n{client}"
    );
    for absent in [
        "$remote",
        "rpc.js",
        "titleFor",
        "$build",
        "readPosts",
        "postFrom",
    ] {
        assert!(
            !client.contains(absent),
            "`{absent}` must not reach the browser for a `static` read:\n{client}"
        );
    }
    assert!(
        !out.path.join("functions").exists(),
        "a `client` + `static` program emits no server function"
    );

    let manifest = std::fs::read_to_string(out.path.join("manifest.json")).expect("manifest.json");
    assert!(manifest.contains(r#""posts":"static""#), "{manifest}");
    assert!(manifest.contains(r#""functions":[]"#), "{manifest}");

    // §14C.3b's sub-requirement: `static` emits files as well as reading
    // them, and `rss.xml` is a file in the bundle rather than an endpoint
    // beside it. It derives from the same state the pages do, so the two
    // cannot drift.
    let feed = std::fs::read_to_string(out.path.join("rss.xml")).expect("rss.xml");
    assert!(feed.contains("<title>Writing</title>"), "{feed}");
    assert!(
        !client.contains("feedFor") && !client.contains("<rss"),
        "a build-time output costs the browser nothing:\n{client}"
    );
}

/// **The blog builds from real files, and shows what it can show.**
///
/// `blog.zd` was the last aspirational example. Its blocking line was
/// `readMarkdown "content/blog"` — a call with a bare argument, which has
/// no production in §4.4 — naming a build-time `foreign` that had no host
/// to import from. Both halves are now the `build` capability form, and
/// this is the acceptance: the posts come off disk, the HTML is rendered
/// by the compiler, and the browser receives literals.
///
/// It also pins the half that is *not* finished. `body` is HTML and
/// `Text` sets `nodeValue`, so the tags render as visible characters. That
/// is asserted here rather than left to be discovered, because the
/// alternative — reaching for `innerHTML` — is the decision this example
/// must not be the reason anyone makes.
#[test]
fn the_blog_builds_from_files_on_disk_with_nothing_to_fetch() {
    let out = TempDir::new("build-blog-out");
    let built = run_without_a_path(&[
        "build",
        example("blog.zd").to_str().expect("utf-8 path"),
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

    // The content is `examples/content/blog/*.md`. What reaches the
    // browser is the rendered HTML as a string literal: no path to fetch,
    // no renderer, and no markdown.
    assert!(
        client.contains(r#""slug":"content/blog/a-blog-is-two-placements.md""#),
        "the posts must come from disk and inline as literals:\n{client}"
    );
    assert!(
        client.contains("<h1>A blog is two placements</h1>"),
        "the markdown must be rendered at build time:\n{client}"
    );

    // Sorted by `published`, on the build host. 2024 before 2026.
    let first = client
        .find("A blog is two placements")
        .expect("the 2024 post");
    let second = client
        .find("Reading a file is not importing one")
        .expect("the 2026 post");
    assert!(
        first < second,
        "`sort each post by post.published` ordering"
    );

    // The draft is filtered on the build host, so its rendered HTML is
    // not in the bundle at all. Nothing is hidden from the reader by the
    // browser, because nothing was sent to the browser to hide.
    for absent in [
        "a-draft-is-still-a-file",
        "A draft is still a file",
        // The build's own machinery never ships either.
        "$build",
        "readPosts",
        "postFrom",
        "titleOf",
        "$remote",
        "marked",
    ] {
        assert!(
            !client.contains(absent),
            "`{absent}` must not reach the browser:\n{client}"
        );
    }

    assert!(
        !out.path.join("functions").exists(),
        "a `client` + `static` program emits no server function"
    );
    let manifest = std::fs::read_to_string(out.path.join("manifest.json")).expect("manifest.json");
    assert!(manifest.contains(r#""visible":"static""#), "{manifest}");
    assert!(manifest.contains(r#""query":"client""#), "{manifest}");
    assert!(manifest.contains(r#""functions":[]"#), "{manifest}");

    // **And it shows them.** The body is `Markup` and the card renders it
    // through `Prose`, so the emitted bundle binds it with `bindMarkup`
    // rather than writing it into a text node — which is what turns `<h1>`
    // from four visible characters into a heading. That it becomes a real
    // heading element is asserted against a mounted DOM in
    // `zdc-codegen`'s `tests/markup.rs`; here the claim is only about what
    // `zdc build` wrote.
    assert!(
        client.contains("bindMarkup("),
        "the post bodies must be rendered rather than shown as text:\n{client}"
    );
    // Generated code still never names the property. The one call that
    // parses is `runtime/markup.js`'s `markup`, its own module since the
    // render path stopped being charged to every page — so this build must
    // have linked *and* shipped it, which a program with no `Prose` must
    // not.
    assert!(
        !client.contains("innerHTML"),
        "generated code must never name `innerHTML`:\n{client}"
    );
    assert!(
        client.contains("/markup.js"),
        "a program with a `Prose` must import the render path:\n{client}"
    );
    let markup =
        std::fs::read_to_string(out.path.join("runtime/markup.js")).expect("runtime/markup.js");
    assert!(
        markup.contains("export function markup("),
        "the one function that parses must be shipped with the bundle"
    );
}

/// **A build reads the project it is building, and nothing else.**
///
/// A build that can open any path is a supply-chain surface: the content
/// it inlines becomes the program, so whoever chooses the path chooses the
/// program. Three escapes are checked, and each is a *different* mechanism
/// rather than three spellings of one — `..` is lexical, an absolute path
/// discards the root, and a symbolic link is neither, which is why the
/// check is on the **resolved** path and not on the written one.
#[test]
fn a_path_that_leaves_the_project_directory_is_a_build_error() {
    let secret = TempDir::new("build-outside");
    std::fs::create_dir_all(&secret.path).expect("the directory outside the project");
    std::fs::write(secret.path.join("stolen.md"), "# outside-the-project\n")
        .expect("a file to steal");

    let project = TempDir::new("build-escape-project");
    std::fs::create_dir_all(project.path.join("content")).expect("the project's content");
    std::fs::write(project.path.join("content/kept.md"), "# kept\n").expect("a file to keep");

    // The symbolic link is inside `content/`, so listing the directory
    // reaches it and no `..` or leading `/` appears anywhere in the
    // program. Only resolving the path finds it.
    symlink(
        &secret.path.join("stolen.md"),
        &project.path.join("content/linked.md"),
    );

    // The phrases are `zdc_hir::sandbox::Refusal`'s, which is the rule
    // `build read` and `build list` now go through — one rule for every
    // path a program can make the build open, `use` included.
    let escapes: [(&str, &str, &str); 3] = [
        ("climbing", "\"..\"", "climbs out of the project"),
        ("absolute", "\"/\"", "is an absolute path"),
        ("linked", "\"content\"", "points outside the project"),
    ];

    // The three are the three ways out, and an empty table would make every
    // assertion below unreachable while the test still passed. Counted as
    // well as sized, so that trimming the table to nothing fails here
    // rather than passing over an empty loop.
    assert_eq!(escapes.len(), 3, "a climb, an absolute path and a link");
    let mut checked = 0;
    for (name, directory, expected) in escapes {
        let source = project.path.join(format!("{name}.zd"));
        std::fs::write(
            &source,
            format!(
                concat!(
                    "record Post\n",
                    "    slug is Text\n",
                    "    body is Markup\n",
                    "state posts is static List of Post from readPosts with directory is {}\n",
                    "function readPosts with directory\n",
                    "    from build list directory\n",
                    "    map each path to postFrom with path\n",
                    "function postFrom with path\n",
                    "    give Post with slug is path, body is build markdown (build read path)\n",
                    "view\n",
                    "    Column\n",
                    "        each post in posts\n",
                    "            Text post.slug\n",
                ),
                directory
            ),
        )
        .expect("the escaping program");

        let out = TempDir::new(&format!("build-escape-out-{name}"));
        let refused = run(&[
            "build",
            source.to_str().expect("utf-8 path"),
            "--out",
            out.path.to_str().expect("utf-8 path"),
        ]);
        assert_eq!(
            refused.status.code(),
            Some(1),
            "`{name}` must not build. stdout was:\n{}",
            String::from_utf8_lossy(&refused.stdout)
        );

        let stderr = strip_ansi(&String::from_utf8_lossy(&refused.stderr));
        assert!(stderr.contains("E11"), "`{name}`: {stderr}");
        assert!(stderr.contains(expected), "`{name}`: {stderr}");
        // The diagnostic names the resolved path, which is the point of
        // resolving it — but the file's *contents* never entered the
        // build, and no bundle was written for them to enter.
        assert!(
            !stderr.contains("outside-the-project"),
            "`{name}` must not have read the file: {stderr}"
        );
        assert!(!out.path.exists(), "`{name}` must write no bundle");
        checked += 1;
    }
    assert_eq!(checked, 3, "only {checked} of the three escapes were tried");
}

/// **The same inputs give the same output.**
///
/// `read_dir` yields whatever the filesystem yields, which is neither
/// sorted nor stable between runs, so a build that inlined it would inline
/// a different program on a different machine. §17.4.7 makes the same
/// argument against seeding a parity test randomly.
#[test]
fn a_directory_listing_is_ordered_by_the_compiler_and_not_by_the_filesystem() {
    let mut bundles = Vec::new();
    for run_number in 0..2 {
        let out = TempDir::new(&format!("build-determinism-{run_number}"));
        let built = run(&[
            "build",
            example("writing.zd").to_str().expect("utf-8 path"),
            "--out",
            out.path.to_str().expect("utf-8 path"),
        ]);
        assert_eq!(built.status.code(), Some(0));
        bundles.push(std::fs::read_to_string(out.path.join("client.js")).expect("client.js"));
    }
    assert_eq!(bundles[0], bundles[1], "a build must be reproducible");

    let hello = bundles[0].find("hello-world").expect("the first post");
    let placement = bundles[0].find("on-placement").expect("the second post");
    let network = bundles[0].find("the-network").expect("the third post");
    assert!(
        hello < placement && placement < network,
        "the listing must be sorted, not whatever the filesystem said"
    );
}

/// A capability is answered by the compiler while the compiler is running.
/// Outside build-time evaluation there is nobody to ask, so this is not a
/// permission that could be granted — it is a question with no answerer.
#[test]
fn asking_for_a_build_capability_outside_the_build_is_a_compile_error() {
    let source = TempSource::new(
        "build-in-the-browser",
        concat!(
            "state page is client Text starting \"\"\n",
            "view\n",
            "    Column\n",
            "        Text page\n",
            "        Button \"read\"\n",
            "            on click\n",
            "                set page to build read \"content/hello-world.md\"\n",
        ),
    );
    let refused = run(&["check", source.path.to_str().expect("utf-8 path")]);
    assert_eq!(refused.status.code(), Some(1));

    let stderr = strip_ansi(&String::from_utf8_lossy(&refused.stderr));
    assert!(stderr.contains("E0361"), "{stderr}");
    assert!(stderr.contains("while the build is running"), "{stderr}");
}

/// The set is closed, and the diagnostic says so and then says what is in
/// it. §4.1 puts the whole weight of guessability on the message.
#[test]
fn an_unknown_build_capability_names_the_closed_set() {
    let source = TempSource::new(
        "build-unknown",
        concat!(
            "state page is static Text starting \"\"\n",
            "state other is static Text from fetched with page\n",
            "function fetched with source\n",
            "    give build download source\n",
            "view\n",
            "    Text page\n",
        ),
    );
    let refused = run(&["check", source.path.to_str().expect("utf-8 path")]);
    assert_eq!(refused.status.code(), Some(1));

    let stderr = strip_ansi(&String::from_utf8_lossy(&refused.stderr));
    assert!(
        stderr.contains("not a capability the compiler provides"),
        "{stderr}"
    );
    assert!(stderr.contains("`read`, `list`, `markdown`"), "{stderr}");
}

/// §14C.3b: "`set`, `append`, and friends are compile errors on it." The
/// diagnostic names the rule and the placement rather than failing at run
/// time against a binding that was never a cell.
///
/// The rule is now named in two places rather than one: the rejection
/// carries the claim and the code, and `zdc explain E0310` carries the
/// reasoning and the spec reference. That is the diagnostic budget's whole
/// bargain, so this asserts both halves — a code with nothing behind it
/// would satisfy the budget and help nobody.
#[test]
fn writing_a_static_signal_is_a_compile_error_naming_the_rule() {
    let source = TempSource::new(
        "static-write",
        concat!(
            "state title is static Text starting \"a\"\n",
            "view\n",
            "    Column\n",
            "        Text title\n",
            "        Button \"rename\"\n",
            "            on click\n",
            "                set title to \"b\"\n",
        ),
    );
    let refused = run(&["check", source.path.to_str().expect("utf-8 path")]);
    assert_eq!(refused.status.code(), Some(1));

    let stderr = strip_ansi(&String::from_utf8_lossy(&refused.stderr));
    assert!(stderr.contains("E0310"), "{stderr}");
    assert!(stderr.contains("computed once at build time"), "{stderr}");
    assert!(
        stderr.contains("run 'zdc explain E0310' for the rule"),
        "{stderr}"
    );

    let explained = run(&["explain", "E0310"]);
    assert_eq!(explained.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&explained.stdout);
    assert!(stdout.contains("build time"), "{stdout}");
    assert!(stdout.contains("state total is static Whole"), "{stdout}");
}

/// §14C.3b claims the existing information-flow rules already reject a
/// `secret static`, with no special case. They do — at the declaration,
/// because §5.3 says only `server` and `durable` may be secret at all, and
/// a `static` value is inlined into the bundle where the reader is.
#[test]
fn a_secret_static_value_is_a_compile_error() {
    let source = TempSource::new(
        "static-secret",
        concat!(
            "secret state key is static Text starting \"sk-live-1\"\n",
            "view\n",
            "    Text key\n",
        ),
    );
    let refused = run(&["check", source.path.to_str().expect("utf-8 path")]);
    assert_eq!(refused.status.code(), Some(1));

    let stderr = strip_ansi(&String::from_utf8_lossy(&refused.stderr));
    assert!(stderr.contains("E0313"), "{stderr}");
    assert!(stderr.contains("`static`-placed"), "{stderr}");
}

/// A symbolic link, on whichever platform the tests are running.
///
/// Written out rather than reached for from a crate: a test that proves a
/// symlink cannot escape the project must create a real one, and this is
/// the whole of what that takes.
#[cfg(unix)]
fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("the symbolic link");
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("the symbolic link");
}

/// `zdc`, run where no other program can be found.
///
/// The binary itself is launched by absolute path, so an empty `PATH`
/// costs nothing legitimate — but any attempt to shell out to `node`, or
/// to anything else, fails.
fn run_without_a_path(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args(args)
        .env("PATH", "")
        .output()
        .expect("failed to run the zdc binary")
}

/// **`zdc` is one binary, and building a `static` program keeps it one.**
///
/// A constraint with no test is a comment, and this is the constraint:
/// `zdc-runtime`'s own module doc says that needing Node to build
/// ZDeceptron "would be the first crack in the claim that a developer
/// installs one binary and nothing else". §17.4.8 proposed exactly that
/// crack. It is not taken, and this is what stops it being taken again by
/// whoever next reaches for an easy evaluator.
///
/// Checked two ways, because either alone is escapable. The behavioural
/// half is above: a `static` example builds with an empty `PATH`. This is
/// the structural half — no compiler crate spawns a process at all, so a
/// spawn on a path no test happens to take is caught too.
#[test]
fn no_compiler_crate_spawns_a_subprocess() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates");
    let mut offenders = Vec::new();
    let mut scanned = 0;
    let mut roots = 0;

    for entry in std::fs::read_dir(&crates).expect("crates/ must exist") {
        let source = entry.expect("entry").path().join("src");
        if !source.is_dir() {
            continue;
        }
        roots += 1;
        visit_rust_files(&source, &mut |path, text| {
            scanned += 1;
            // `zdc-dev` serves a browser and `zdc-lsp` speaks to an editor;
            // neither starts anything. If one ever needs to, it says so
            // here rather than by surprising a developer at build time.
            if text.contains("std::process::Command") || text.contains("process::Command::new") {
                offenders.push(path.display().to_string());
            }
        });
    }

    // A walk that reads nothing finds no offenders. `crates/*/src` is not
    // a promise the layout makes to this test, so what was actually read is
    // counted before the finding is trusted — the same reason
    // `scripts/check-forbid-unsafe.sh` counts its crate roots.
    //
    // The floors are written down rather than derived from `crates`. The
    // first attempt at this counted the directories under the same path the
    // walk had just used, so pointing the walk somewhere with no crates in
    // it moved both numbers to zero and the assertion agreed with itself —
    // which is the defect this test is being hardened against. A literal
    // cannot move with the walk. Bumping it when a crate is added is the
    // point, not the cost.
    assert!(
        roots >= 14,
        "the workspace has at least fourteen crates, the walk entered {roots}"
    );
    assert!(
        scanned >= 60,
        "the workspace has at least sixty source files, the walk read {scanned}"
    );

    assert!(
        offenders.is_empty(),
        "the compiler must not spawn anything — `static` is evaluated in `zdc-runtime`'s own \
         engine (spec §17.4.8, as corrected). Found: {offenders:?}"
    );
}

/// **A build makes no network request, and the crates that run one cannot.**
///
/// The other half of "one binary, nothing installed". §17.4.8's rejected
/// alternative — importing `marked` — would have needed a registry, and a
/// registry is a network. The capabilities that replaced it read the
/// project directory and nothing else, so the crates that answer them have
/// no socket in them at all.
///
/// `zdc-dev` and `zdc-cli` are excluded and named: one *is* a web server
/// and the other starts it. Neither is on the path `zdc build` takes.
#[test]
fn no_crate_on_the_build_path_opens_a_socket() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates");
    let serves_a_browser = ["zdc-dev", "zdc-cli"];

    let mut offenders = Vec::new();
    let mut scanned = 0;

    for entry in std::fs::read_dir(&crates).expect("crates/ must exist") {
        let crate_root = entry.expect("entry").path();
        let name = crate_root
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 crate name")
            .to_string();
        if serves_a_browser.contains(&name.as_str()) {
            continue;
        }
        let source = crate_root.join("src");
        if !source.is_dir() {
            continue;
        }
        visit_rust_files(&source, &mut |path, text| {
            scanned += 1;
            for socket in ["std::net", "TcpStream", "TcpListener", "UdpSocket"] {
                if text.contains(socket) {
                    offenders.push(format!("{} ({socket})", path.display()));
                }
            }
        });
    }

    assert!(scanned > 20, "the scan must have read something: {scanned}");
    assert!(
        offenders.is_empty(),
        "a build reads the project directory and nothing else — no build-path crate may open a \
         socket. Found: {offenders:?}"
    );
}

fn visit_rust_files(directory: &Path, each: &mut impl FnMut(&Path, &str)) {
    for entry in std::fs::read_dir(directory).expect("readable directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            visit_rust_files(&path, each);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable source");
        each(&path, &text);
    }
}

/// **Progressive disclosure, end to end.**
///
/// Barik et al. measured that message length costs reading time, so the
/// inline diagnostic states the claim and points at the rule. This asserts
/// the pointer is there, that it is the *only* help line, and that
/// following it prints something worth the trip.
#[test]
fn a_rejection_points_at_the_rule_and_the_rule_can_be_read() {
    let original = std::fs::read_to_string(example("guestbook.zd")).expect("guestbook is readable");
    let leaked = original.replace(
        "        Input name, hint is \"your name\"",
        "        Input name, hint is \"your name\"\n        Text apiKey",
    );
    let source = TempSource::new("check-explain", &leaked);

    let output = run(&["check", source.path.to_str().expect("utf-8 path")]);
    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    assert!(
        stderr.contains("run 'zdc explain E-IFC-05' for the rule"),
        "the diagnostic must end by naming the command that explains it:\n{stderr}"
    );
    // Every help line the rejection carries is the pointer and nothing
    // else. Not "exactly one line": one leaked read is refused by more
    // than one rule — E-IFC-05 where the browser reads it, E-IFC-08 where
    // the endpoint hands it back — and each carries its own pointer. What
    // must not appear is a help line that is prose instead.
    let helps = stderr.matches("Help:").count();
    assert!(helps >= 1, "the rejection carries no help line:\n{stderr}");
    assert_eq!(
        stderr.matches("run 'zdc explain ").count(),
        helps,
        "every help line is the pointer to the rule and nothing else:\n{stderr}"
    );

    let explained = run(&["explain", "E-IFC-05"]);
    assert_eq!(explained.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&explained.stdout);
    for section in ["What it means", "Why the rule exists", "How to fix it"] {
        assert!(stdout.contains(section), "missing `{section}`:\n{stdout}");
    }
    assert!(
        stdout.contains("secret state apiKey is server Text"),
        "the rule must show a worked repair, not only prose:\n{stdout}"
    );
}

/// A code is case-insensitive, because a reader retypes it from a
/// diagnostic and a shell is not a compiler.
#[test]
fn explain_accepts_a_lowercase_code() {
    let output = run(&["explain", "e-ifc-05"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("E-IFC-05"));
}

/// An unknown code fails, and lists the codes that exist rather than
/// leaving the reader to guess which one they mistyped.
#[test]
fn explain_refuses_an_unknown_code_and_lists_the_real_ones() {
    let output = run(&["explain", "E-9999"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("There is no diagnostic code"));
    assert!(
        stderr.contains("E-IFC-05"),
        "the list is printed:\n{stderr}"
    );
    assert!(stderr.contains("E0301"), "the list is complete:\n{stderr}");
}

/// A routed program writes one document per URL, at the path that URL
/// names, plus the manifest that maps them (spec §14G.2).
///
/// The layout is what a static host already serves with no
/// configuration — `/writing/rust` is `writing/rust/index.html` — which
/// is the point of §14G.2's prerendering being total.
#[test]
fn a_routed_build_writes_one_document_per_url() {
    let out = TempDir::new("build-site");
    let output = run(&[
        "build",
        example("site.zd").to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for expected in [
        "index.html",
        "writing/index.html",
        "writing/routing/index.html",
        "writing/folding/index.html",
        "404/index.html",
        "pages/index.js",
        "pages/writing-routing.js",
        "routes.json",
        "runtime/dom.js",
    ] {
        assert!(
            out.path.join(expected).is_file(),
            "the site is missing {expected}"
        );
    }
    // One bundle per page, never one bundle for the site.
    assert!(
        !out.path.join("client.js").exists(),
        "a routed build has no single client.js"
    );

    let home = std::fs::read_to_string(out.path.join("pages/index.js")).expect("read");
    let post = std::fs::read_to_string(out.path.join("pages/writing-routing.js")).expect("read");
    assert_ne!(home, post, "per-route output must actually differ");
    assert!(
        !home.contains("titleOf"),
        "the home page carries a helper only a post uses:\n{home}"
    );

    let manifest = std::fs::read_to_string(out.path.join("routes.json")).expect("read");
    assert!(
        manifest.contains("\"url\":\"/writing/routing\""),
        "{manifest}"
    );
}

/// **#28, through the binary.** A typo in a type name used to produce a
/// successful build: `zdc build` wrote a complete bundle for a program
/// whose state named two types nothing declares.
#[test]
fn building_a_program_that_names_a_type_that_does_not_exist_fails() {
    let source = TempSource::new(
        "undeclared-type",
        concat!(
            "state votes is client Map of Id to Int starting empty\n",
            "view\n",
            "    Text \"hi\"\n",
        ),
    );
    let out = TempDir::new("undeclared-type-out");
    let output = run(&[
        "build",
        source.path.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);

    assert_eq!(output.status.code(), Some(1), "expected exit code 1");
    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    for name in ["Id", "Int"] {
        assert!(
            stderr.contains(&format!("`{name}` is not a type")),
            "`{name}` must be named as the mistake:\n{stderr}"
        );
    }
    assert!(
        !out.path.join("client.js").exists(),
        "no bundle may be written for a program that does not check"
    );
}
