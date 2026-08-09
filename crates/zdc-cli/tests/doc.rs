//! `zdc doc`, end to end, through the binary a developer actually runs.
//!
//! Every assertion here is about a claim the *program* made — a placement,
//! a secret, a derived endpoint, a URL — reaching the generated Markdown.
//! A documentation generator that prints headings and no facts would pass
//! a test that only checked the file existed, so nothing below checks that
//! a file exists without also checking what it says.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args(args)
        .output()
        .expect("failed to run the zdc binary")
}

/// A directory under the system temporary directory, removed when the test
/// ends whether it passed or not.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("zdc-doc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        TempDir { path }
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path.join(relative)).unwrap_or_else(|e| {
            panic!(
                "{} was not written: {e}. The directory holds: {:?}",
                relative,
                std::fs::read_dir(&self.path)
                    .map(|entries| entries
                        .filter_map(|entry| entry.ok())
                        .map(|entry| entry.file_name())
                        .collect::<Vec<_>>())
                    .unwrap_or_default()
            )
        })
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

fn document(name: &str, source: &str) -> TempDir {
    let out = TempDir::new(name);
    let output = run(&[
        "doc",
        example(source).to_str().expect("utf-8 path"),
        "-o",
        out.path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

/// The claim the whole command exists for: a reader sees the program's
/// client/server/durable split without opening the source.
#[test]
fn the_overview_says_where_every_signal_lives() {
    let out = document("placements", "guestbook.zd");
    let index = out.read("index.md");

    for (signal, placement) in [
        ("apiKey", "server"),
        ("visits", "durable"),
        ("name", "client"),
        ("greeting", "server"),
    ] {
        let row = index
            .lines()
            .find(|line| line.contains(&format!("`{signal}`")) && line.starts_with('|'))
            .unwrap_or_else(|| panic!("no table row for `{signal}` in:\n{index}"));
        assert!(
            row.contains(placement),
            "`{signal}` is {placement}-placed and its row does not say so: {row}"
        );
    }

    // The split, counted. Four signals over three placements.
    assert!(
        index.contains("2 `server`"),
        "the overview must count the placements:\n{index}"
    );
}

/// `Remote of T` is the language's whole thesis: the network is in the
/// type, so it must be in the documentation.
#[test]
fn a_durable_signal_is_documented_as_remote_from_the_browser() {
    let out = document("remote", "guestbook.zd");
    let index = out.read("index.md");
    let row = index
        .lines()
        .find(|line| line.contains("`visits`") && line.starts_with('|'))
        .expect("a row for visits");
    assert!(
        row.contains("Remote of Whole"),
        "reading `visits` from the browser yields `Remote of Whole`: {row}"
    );

    // …and a `client` signal must not be dressed up as remote, or the
    // column says nothing.
    let name = index
        .lines()
        .find(|line| line.contains("`name`") && line.starts_with('|'))
        .expect("a row for name");
    assert!(
        !name.contains("Remote"),
        "`name` is client-placed and crosses nothing: {name}"
    );
}

/// A `secret` cannot be read from the browser at all, and the row that
/// claims a type for it would be the one lie in the file.
#[test]
fn a_secret_signal_is_documented_as_unreadable_from_the_browser() {
    let out = document("secret", "guestbook.zd");
    let index = out.read("index.md");
    let row = index
        .lines()
        .find(|line| line.contains("`apiKey`") && line.starts_with('|'))
        .expect("a row for apiKey");
    assert!(
        row.contains("secret"),
        "the row must name the rule that stops the read: {row}"
    );
    assert!(
        !row.contains("Remote of Text"),
        "a secret is not readable from the browser at any type: {row}"
    );
}

/// The endpoints are *derived*. No other language's doc generator can
/// print this table, because in every other language the developer wrote
/// the routes by hand and the generator is only reading them back.
#[test]
fn the_derived_endpoints_are_listed_with_the_files_they_become() {
    let out = document("endpoints", "guestbook.zd");
    let index = out.read("index.md");
    for path in [
        "functions/greeting.js",
        "functions/visits.js",
        "functions/visits.incr.js",
    ] {
        assert!(
            index.contains(path),
            "`{path}` is emitted for this program and the network section omits it:\n{index}"
        );
    }
}

/// The environment key is part of the program's surface: it is what a
/// deployment has to be given before the program runs at all.
#[test]
fn the_environment_a_program_needs_is_documented() {
    let out = document("environment", "guestbook.zd");
    let index = out.read("index.md");
    assert!(
        index.contains("GREETING_API_KEY"),
        "the key `apiKey` reads must be named:\n{index}"
    );
}

/// The comment above a declaration is where this codebase keeps its
/// reasoning, so it is most of what documentation is for.
#[test]
fn the_comment_above_a_declaration_becomes_its_documentation() {
    let out = document("comments", "guestbook.zd");
    let page = out.read("guestbook.md");
    assert!(
        page.contains("Lives in a persistent store"),
        "the comment above `state visits` is its documentation:\n{page}"
    );
    // The comment above `politeGreeting` is one line of `#` and must not
    // be swallowed by the four-line comment above the signal before it.
    assert!(
        page.contains("Computed in a serverless invocation"),
        "the comment above `greeting` belongs to `greeting`:\n{page}"
    );
}

/// A `#` inside a text literal is text. The harvest reads lines, so this
/// is the case that would fool a naive one.
#[test]
fn a_hash_inside_a_text_literal_is_not_a_comment() {
    let out = TempDir::new("hash-in-text");
    let source = out.path.join("hash.zd");
    std::fs::create_dir_all(&out.path).expect("the temporary directory is creatable");
    std::fs::write(
        &source,
        concat!(
            "state banner is client Text starting \"\"\"\n",
            "# this is a heading, not a comment\n",
            "\"\"\"\n",
            "state count is client Whole starting 0\n",
            "view\n",
            "    Text banner\n",
        ),
    )
    .expect("the temporary source is writable");

    let output = run(&[
        "doc",
        source.to_str().expect("utf-8 path"),
        "-o",
        out.path.join("doc").to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let page = std::fs::read_to_string(out.path.join("doc/hash.md")).expect("the module page");
    assert!(
        !page.contains("this is a heading, not a comment"),
        "the line is inside a text literal and is not documentation:\n{page}"
    );
}

/// A program is its modules. `site.zd` imports `content.zd`, and a reader
/// asking what the site declares needs both.
#[test]
fn a_program_of_two_modules_gets_a_page_for_each() {
    let out = document("modules", "site.zd");
    let index = out.read("index.md");
    assert!(index.contains("site.md"), "the index links its own module");
    assert!(index.contains("content.md"), "and the one it imports");

    let content = out.read("content.md");
    assert!(
        content.contains("`slugs`"),
        "the imported module's declarations are on its own page:\n{content}"
    );
}

/// Routes are URLs, and a URL is the most public part of a program's
/// surface.
#[test]
fn every_url_a_routed_program_serves_is_listed() {
    let out = document("routes", "site.zd");
    let index = out.read("index.md");
    for url in ["/writing", "/writing/{slug}"] {
        assert!(
            index.contains(url),
            "`{url}` is one of this program's URLs:\n{index}"
        );
    }
}

/// Documentation is written from a *checked* program. A file that does not
/// compile has no settled placements and no types, so the honest answer is
/// the diagnostic and no files.
#[test]
fn a_program_that_does_not_compile_is_refused_and_nothing_is_written() {
    let out = TempDir::new("refused");
    std::fs::create_dir_all(&out.path).expect("the temporary directory is creatable");
    let source = out.path.join("broken.zd");
    std::fs::write(&source, "state x is client Whole starting nope\n")
        .expect("the temporary source is writable");

    let output = run(&[
        "doc",
        source.to_str().expect("utf-8 path"),
        "-o",
        out.path.join("doc").to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "the program does not compile"
    );
    assert!(
        !out.path.join("doc").exists(),
        "no half-written documentation for a program the compiler refused"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nope"),
        "the diagnostic must name what it could not resolve:\n{stderr}"
    );
}

/// The default has to be somewhere, and `dist` is taken by `zdc build`.
#[test]
fn the_default_output_directory_is_doc() {
    let out = TempDir::new("default-out");
    std::fs::create_dir_all(&out.path).expect("the temporary directory is creatable");
    let source = out.path.join("hello.zd");
    std::fs::write(
        &source,
        "state greeting is client Text starting \"hi\"\nview\n    Text greeting\n",
    )
    .expect("the temporary source is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args(["doc", "hello.zd"])
        .current_dir(&out.path)
        .output()
        .expect("failed to run the zdc binary");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let index = std::fs::read_to_string(out.path.join("doc/index.md"))
        .expect("doc/index.md is the default");
    assert!(index.contains("`greeting`"), "{index}");
}

/// #170's actual complaint: the standard library is eight files of
/// ZDeceptron and the only way to read its surface is to open them.
#[test]
fn the_prelude_documents_itself_file_by_file() {
    let out = TempDir::new("prelude");
    let output = run(&[
        "doc",
        "--prelude",
        "-o",
        out.path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let index = out.read("index.md");
    assert!(index.contains("The ZDeceptron prelude"), "{index}");
    // The library is colourless, and the overview reports that from the
    // declarations rather than from a sentence written here (§17.4.1).
    assert!(index.contains("declares no state"), "{index}");

    // Each file's own page, with a declaration only that file makes. This
    // is what fails if the per-file span offsets are ever lost: every
    // prelude source is parsed from zero on its own.
    let text = out.read("text.md");
    assert!(
        text.contains("### `textLength`"),
        "`textLength` is declared in text.zd:\n{text}"
    );
    let list = out.read("list.md");
    assert!(
        list.contains("### `listLength`"),
        "`listLength` is declared in list.zd:\n{list}"
    );
    assert!(
        !text.contains("### `listLength`"),
        "and not in text.zd:\n{text}"
    );

    // A `foreign` is the one construct whose types are asserted, so its
    // full signature is the thing worth reading.
    assert!(
        text.contains("from \"zd:text\" as \"length\"") && text.contains("gives pure Whole"),
        "a foreign's asserted signature belongs on its page:\n{text}"
    );

    // The comment above a prelude declaration is most of what the prelude
    // is, so losing it would leave the pages saying nothing.
    let option = out.read("option.md");
    assert!(
        option.contains("Eliminating an `Option`"),
        "option.zd's own header:\n{option}"
    );
}

/// The prelude is not a file, and asking for it as one is the mistake
/// worth catching at the argument parser rather than in the resolver.
#[test]
fn a_file_and_the_prelude_cannot_be_asked_for_together() {
    let output = run(&["doc", "--prelude", "examples/guestbook.zd"]);
    assert_eq!(output.status.code(), Some(2), "clap refuses the pair");
}

/// A subcommand with no argument at all must say what it wanted, not
/// document something arbitrary.
#[test]
fn doc_with_neither_a_file_nor_the_prelude_is_refused() {
    let output = run(&["doc"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("prelude"), "{stderr}");
}

/// The help text is read more often than the documentation it generates.
#[test]
fn the_help_names_what_the_command_reads_and_what_it_writes() {
    let output = run(&["doc", "--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.contains("Markdown"),
        "the help says what comes out:\n{help}"
    );
    assert!(
        help.contains("placement"),
        "and what makes it worth reading:\n{help}"
    );
    assert!(
        help.contains("--prelude"),
        "and that the standard library is one of the things it reads:\n{help}"
    );
}
