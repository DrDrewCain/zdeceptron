//! `zdc check` sees every diagnostic `zdc build` can produce, and so does
//! the language server.
//!
//! Code generation runs last (§17.1.2) because it reads every earlier
//! pass's product. That ordering was silently read as "codegen runs only in
//! `zdc build`", which split the diagnostic set in two along a line no rule
//! justifies: a refusal ended up invisible to `zdc check` because of *where
//! the information happened to be*, not because of what kind of rule it
//! was. Two defects hid in that gap — a working secret exfiltration that
//! passed `zdc check` with exit 0, and every `only_children`/`only_inside`
//! shape check in §16.3.6.
//!
//! Three things are asserted here, and none of them is a list anyone has to
//! maintain:
//!
//!   1. **Reachability.** Every diagnostic site in `zdc-codegen`'s source is
//!      either produced by a program in `tests/refusals/` through the
//!      pipeline `zdc check` runs, or carries an `// unreached:` comment in
//!      the source saying which earlier pass answers first. The sites are
//!      extracted from the source text, so adding a refusal to codegen
//!      fails this test until one or the other is true of it.
//!   2. **Equality.** For every one of those programs and every checked-in
//!      example, `zdc check` and `zdc build` print the same diagnostics and
//!      exit the same way. Exactly one refusal is `zdc build`'s alone — a
//!      file with no `view` has no page to build, which is an answer to the
//!      command rather than a fault in the program — and the source says so
//!      at the refusal itself, in a form this file counts.
//!   3. **The editor agrees.** The language server's own analysis path
//!      produces exactly the messages the command line does. §14's
//!      language-server section claims the two cannot disagree; this is
//!      that claim, mechanised.

use std::path::{Path, PathBuf};
use std::process::Command;

use zdc_diagnostics::Diagnostic;

fn repository(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// The programs that exercise codegen's refusals, each with the phrase it
/// must produce written into it as a comment.
struct Refusal {
    path: PathBuf,
    name: String,
    source: String,
    expected: String,
}

fn corpus() -> Vec<Refusal> {
    let directory = repository("crates/zdc-codegen/tests/refusals");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&directory).expect("the refusal corpus directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("zd") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("a readable refusal");
        let expected = source
            .lines()
            .find_map(|line| line.strip_prefix("# expect: "))
            .unwrap_or_else(|| panic!("{} has no `# expect:` line", path.display()))
            .to_string();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 file name")
            .to_string();
        found.push(Refusal {
            path,
            name,
            source,
            expected,
        });
    }
    assert!(!found.is_empty(), "the refusal corpus is empty");
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn examples() -> Vec<PathBuf> {
    let directory = repository("examples");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&directory).expect("the examples directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("zd") {
            found.push(path);
        }
    }
    found.sort();
    found
}

// --- the diagnostics the language server produces -------------------------

/// Every diagnostic the compiler gives for one source, through the language
/// server's own entry point.
///
/// This is the same pipeline `zdc check` runs — parse, resolve, typecheck,
/// then code generation with the bundle dropped — reached through the crate
/// the editor talks to rather than through the command line, which is what
/// makes the comparison below a comparison of two implementations.
fn editor_messages(source: &str) -> Vec<String> {
    zdc_lsp::Analysis::of(source)
        .diagnostics()
        .iter()
        .map(|diagnostic: &Diagnostic| diagnostic.message.clone())
        .collect()
}

/// The diagnostics `zdc-codegen` itself raised for one source.
///
/// The coverage test below cannot use `editor_messages`, because two passes
/// sometimes give a refusal the same words: `zdc-types` and `zdc-codegen`
/// both say *"Only a top-level `function` can be called"*, and matching on
/// text alone would let the checker's copy vouch for the emitter's. Asking
/// the emitter directly is the only way to know which one spoke.
fn codegen_messages(source: &str) -> Vec<String> {
    let Ok(program) = zdc_parser::parse(source) else {
        return Vec::new();
    };
    let Ok(hir) = zdc_resolve::Resolver::new(&program).resolve() else {
        return Vec::new();
    };
    let split = zdc_graph::split(&hir);
    if split.has_errors() {
        return Vec::new();
    }
    let verdict = zdc_graph::ifc(&hir, &split);
    let Ok(table) = zdc_types::check(&hir, &split) else {
        return Vec::new();
    };
    // The emitter runs only on a program the flow pass cleared, and the
    // token is the proof. A source that leaks has nothing for codegen to
    // say about it that the flow pass has not said first.
    let Some(cleared) = verdict.clearance() else {
        return Vec::new();
    };
    zdc_codegen::check(&zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
        cleared,
    })
    .into_iter()
    .map(|error| error.message)
    .collect()
}

// --- the diagnostic sites in codegen's own source -------------------------

/// One place `zdc-codegen` raises a diagnostic.
#[derive(Debug)]
struct Site {
    file: String,
    line: usize,
    /// The literal fragments of the message, split at its `{}` holes. A
    /// produced message belongs to this site when it contains all of them
    /// in order.
    fragments: Vec<String>,
    /// Whether the source says no program reaches it, and why.
    unreached: bool,
    /// Whether the source says this one belongs to `zdc build` alone.
    build_only: bool,
}

/// Every `.error(...)` and `CodegenError { message: ... }` in `zdc-codegen`.
///
/// Read out of the source text rather than listed here, because a list here
/// would be exactly the thing that rots: a refusal added to codegen next
/// week would not appear in it, and the gap this whole file exists to close
/// would quietly reopen one diagnostic at a time.
fn sites() -> Vec<Site> {
    let directory = repository("crates/zdc-codegen/src");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&directory)
        .expect("the codegen source directory")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("rs"))
        .collect();
    paths.sort();

    let mut sites = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).expect("a readable source file");
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 file name")
            .to_string();
        let bytes: Vec<char> = text.chars().collect();
        for (start, opening) in openings(&text) {
            let Some(message) = literal_after(&bytes, opening) else {
                continue;
            };
            sites.push(Site {
                file: file.clone(),
                line: text[..start].matches('\n').count() + 1,
                fragments: fragments(&message),
                unreached: marked(&text, start, "// unreached:"),
                build_only: marked(&text, start, "// build-only:"),
            });
        }
    }
    assert!(
        sites.len() > 40,
        "the extractor found only {} diagnostic sites, which is too few to be right",
        sites.len()
    );
    sites
}

/// The byte offset of each diagnostic call, and the character offset just
/// past the token that introduces its message.
///
/// A bare `message:` is not enough on its own. `zdc-codegen` also builds
/// `EvaluationError`, which has a `message` field and is **not** a refusal
/// of the program: it is the build host reporting that the JavaScript it
/// ran would not finish or would not load, raised by `zdc build` before
/// the emitter is called and carrying its own code and help. Counting one
/// as a codegen refusal would demand a fixture that cannot exist, because
/// nothing `zdc_codegen::check` calls can produce it. So the struct is
/// named, and a third error type added tomorrow is a compile error in
/// `zdc-codegen` and a missing site here — not a silent exemption.
fn openings(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(".error(") {
        let start = from + at;
        let after = start + ".error(".len();
        out.push((start, text[..after].chars().count()));
        from = after;
    }
    from = 0;
    while let Some(at) = text[from..].find("message:") {
        let start = from + at;
        let after = start + "message:".len();
        if encloses_a_codegen_error(text, start) {
            out.push((start, text[..after].chars().count()));
        }
        from = after;
    }
    out.sort();
    out
}

/// Whether the struct literal this `message:` sits in is a `CodegenError`.
///
/// Read backwards to the nearest `<Something>Error {`, which is the
/// literal being built: field order is not relied on, and a `message:`
/// with no struct before it belongs to nothing and is skipped.
fn encloses_a_codegen_error(text: &str, at: usize) -> bool {
    let before = &text[..at];
    match before.rfind("Error {") {
        Some(opening) => before[..opening + "Error".len()].ends_with("CodegenError"),
        None => false,
    }
}

/// The Rust string literal that follows, skipping a `format!(` wrapper.
///
/// `None` when what follows is not a literal — `message: message.into()` in
/// `CodegenError`'s own constructor is the case that matters, and it is not
/// a diagnostic site but the place they all pass through.
fn literal_after(chars: &[char], mut at: usize) -> Option<String> {
    while chars.get(at).is_some_and(|c| c.is_whitespace()) {
        at += 1;
    }
    let format = "format!(";
    if chars[at..].starts_with(&format.chars().collect::<Vec<_>>()[..]) {
        at += format.len();
        while chars.get(at).is_some_and(|c| c.is_whitespace()) {
            at += 1;
        }
    }
    if chars.get(at) != Some(&'"') {
        return None;
    }
    at += 1;

    let mut out = String::new();
    while let Some(&c) = chars.get(at) {
        match c {
            '"' => return Some(out),
            '\\' => {
                let escaped = *chars.get(at + 1)?;
                at += 2;
                match escaped {
                    // A backslash before a newline joins the two lines and
                    // eats the indentation, which is how every multi-line
                    // message in this compiler is written.
                    '\n' => {
                        while chars.get(at).is_some_and(|c| *c == ' ' || *c == '\t') {
                            at += 1;
                        }
                    }
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    other => out.push(other),
                }
            }
            other => {
                out.push(other);
                at += 1;
            }
        }
    }
    None
}

/// The message's literal parts, in order, with its `{}` holes removed.
fn fragments(message: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = message;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        push_fragment(&mut out, before);
        match after.find('}') {
            Some(close) => rest = &after[close + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    push_fragment(&mut out, rest);
    assert!(
        !out.is_empty(),
        "a diagnostic with no literal text at all: {message:?}"
    );
    out
}

fn push_fragment(out: &mut Vec<String>, fragment: &str) {
    if !fragment.trim().is_empty() {
        out.push(fragment.to_string());
    }
}

/// Whether the comment block directly above a diagnostic opens with
/// `marker`.
fn marked(text: &str, start: usize, marker: &str) -> bool {
    let mut line_start = text[..start].rfind('\n').map(|at| at + 1).unwrap_or(0);
    loop {
        let Some(previous_end) = text[..line_start].rfind('\n') else {
            return false;
        };
        let previous_start = text[..previous_end]
            .rfind('\n')
            .map(|at| at + 1)
            .unwrap_or(0);
        let previous = text[previous_start..previous_end].trim();
        if !previous.starts_with("//") {
            return false;
        }
        if previous.starts_with(marker) {
            return true;
        }
        line_start = previous_start;
    }
}

fn produced_by(site: &Site, message: &str) -> bool {
    let mut at = 0;
    for fragment in &site.fragments {
        match message[at..].find(fragment.as_str()) {
            Some(found) => at += found + fragment.len(),
            None => return false,
        }
    }
    true
}

// --- the tests ------------------------------------------------------------

/// One assertion per program in the corpus: the diagnostic it was written
/// for reaches `zdc check`.
///
/// Failing here means a refusal is emitter-only again, which is the shape
/// of both defects this file was written after.
#[test]
fn every_refusal_in_the_corpus_reaches_zdc_check() {
    let found = corpus();
    // A corpus that failed to load is a green test asserting nothing. The
    // floor is deliberately below the current count so adding a refusal
    // does not have to touch it; an empty directory is what it catches.
    assert!(
        found.len() >= 15,
        "the refusal corpus did not load: {} programs",
        found.len()
    );

    for refusal in found {
        let output = Command::new(env!("CARGO_BIN_EXE_zdc"))
            .args(["check", refusal.path.to_str().expect("utf-8 path")])
            .output()
            .expect("failed to run the zdc binary");
        let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}: `zdc check` accepted a program `zdc build` refuses",
            refusal.name
        );
        assert!(
            stderr.contains(&refusal.expected),
            "{}: expected `zdc check` to say {:?}, it said:\n{stderr}",
            refusal.name,
            refusal.expected,
        );
    }
}

/// Every diagnostic codegen can raise is either reached from `zdc check` by
/// a program in the corpus, or says in the source why no program reaches it.
///
/// This is what makes the corpus above a proof rather than a sample. The
/// sites come out of `zdc-codegen`'s own text, so a refusal added to the
/// emitter and reachable only from `zdc build` fails this test on the
/// commit that adds it.
#[test]
fn every_diagnostic_codegen_can_raise_is_accounted_for() {
    let produced: Vec<String> = corpus()
        .iter()
        .flat_map(|refusal| codegen_messages(&refusal.source))
        .collect();

    let mut unaccounted = Vec::new();
    let mut stale = Vec::new();
    let mut build_only = Vec::new();
    for site in sites() {
        let reached = produced
            .iter()
            .any(|message| produced_by(&site, message.as_str()));
        if site.build_only {
            build_only.push(format!("{}:{}", site.file, site.line));
            continue;
        }
        if site.unreached {
            // A comment claiming nothing reaches a refusal stops being true
            // the moment an earlier pass stops answering first, and a stale
            // one would exempt a live diagnostic from everything above.
            if reached {
                stale.push(format!("  {}:{}", site.file, site.line));
            }
            continue;
        }
        if reached {
            continue;
        }
        unaccounted.push(format!(
            "  {}:{} — {}",
            site.file,
            site.line,
            site.fragments.join("…")
        ));
    }
    assert!(
        stale.is_empty(),
        "these refusals are marked `// unreached:` and a program in the corpus reaches them.          Delete the comment:\n{}",
        stale.join("\n")
    );
    assert!(
        unaccounted.is_empty(),
        "these diagnostics can be produced by `zdc build` and by nothing in \
         `crates/zdc-codegen/tests/refusals/`. Add a program that reaches each, or an \
         `// unreached:` comment above it naming the pass that answers first:\n{}",
        unaccounted.join("\n")
    );
    // **No refusal is `zdc build`'s alone**, and the one that used to be
    // is gone rather than exempted: a file with no `view` is a module
    // (§14D.2) and `compile` now emits one, with `index_html: None`, in
    // place of refusing to build a page. So the count is zero, and it is
    // asserted rather than dropped — a `// build-only:` comment appearing
    // here is the split this file exists to close, growing back one
    // comment at a time.
    assert!(
        build_only.is_empty(),
        "no refusal is `zdc build`'s alone. Found {build_only:?}"
    );
}

/// The two sets are the same set.
///
/// `zdc check` and `zdc build` are run over every refusal and every
/// checked-in example, and their diagnostics compared byte for byte. There
/// is no rule deciding which diagnostics belong to which command, because
/// there is no longer anything to decide.
#[test]
fn zdc_check_and_zdc_build_report_the_same_diagnostics() {
    let out = std::env::temp_dir().join(format!("zdc-{}-agree", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);

    let mut sources: Vec<PathBuf> = corpus().into_iter().map(|refusal| refusal.path).collect();
    sources.extend(examples());

    for source in sources {
        let path = source.to_str().expect("utf-8 path");
        let checked = Command::new(env!("CARGO_BIN_EXE_zdc"))
            .args(["check", path])
            .output()
            .expect("failed to run the zdc binary");
        let built = Command::new(env!("CARGO_BIN_EXE_zdc"))
            .args(["build", path, "--out", out.to_str().expect("utf-8 path")])
            .output()
            .expect("failed to run the zdc binary");

        let checked_stderr = strip_ansi(&String::from_utf8_lossy(&checked.stderr));
        let built_stderr = strip_ansi(&String::from_utf8_lossy(&built.stderr));

        // The one permitted difference, and it is a difference about the
        // command rather than about the program: a module with no `view` is
        // a module (§14D.2), and only a request for a page is answered with
        // that. `examples/model.zd` is the case.
        if checked_stderr.is_empty() && !built_stderr.is_empty() {
            assert!(
                built_stderr.contains("This program has no `view`"),
                "{}: `zdc build` reports something `zdc check` does not:\n{built_stderr}",
                source.display()
            );
            continue;
        }

        assert_eq!(
            checked_stderr,
            built_stderr,
            "{}: `zdc check` and `zdc build` disagree about what is wrong with this program",
            source.display()
        );
        assert_eq!(
            checked.status.code(),
            built.status.code(),
            "{}: `zdc check` and `zdc build` disagree about whether it compiles",
            source.display()
        );
    }

    let _ = std::fs::remove_dir_all(&out);
}

/// The editor says what the command line says.
///
/// §14's language-server section rests on the server running the compiler
/// rather than approximating it. That is only true while the two run the
/// same passes, and for as long as codegen was missing from one of them it
/// was false for every diagnostic in this corpus.
#[test]
fn the_language_server_reports_what_the_command_line_reports() {
    let found = corpus();
    assert!(
        found.len() >= 15,
        "the refusal corpus did not load: {} programs",
        found.len()
    );

    for refusal in found {
        let messages = editor_messages(&refusal.source);
        assert!(
            messages
                .iter()
                .any(|message| message.contains(&refusal.expected)),
            "{}: the editor would show a clean file for a program that will not build. \
             Expected {:?}, got:\n{}",
            refusal.name,
            refusal.expected,
            messages.join("\n")
        );

        let output = Command::new(env!("CARGO_BIN_EXE_zdc"))
            .args(["check", refusal.path.to_str().expect("utf-8 path")])
            .output()
            .expect("failed to run the zdc binary");
        let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
        for message in &messages {
            assert!(
                stderr.contains(message.as_str()),
                "{}: the editor shows {message:?} and the command line does not:\n{stderr}",
                refusal.name
            );
        }
    }
}

/// `ariadne` colours the source line character by character, so a message
/// quoted back inside a label is shot through with escape sequences. Only
/// the report's own first line is compared here, and that one is plain, but
/// stripping is what makes the comparison independent of whether the test
/// harness has a terminal.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}
