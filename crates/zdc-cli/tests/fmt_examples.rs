//! `zdc fmt` held against every file in `examples/`.
//!
//! Three properties, in increasing order of how much they would catch.
//!
//! 1. **Every example is already in the canonical layout.** The layout
//!    was read off `examples/` rather than invented, so this is the test
//!    that says so — and from here on it is the test that stops the
//!    examples drifting away from it.
//!
//! 2. **Formatting is idempotent.** Asserted over each example and over a
//!    deliberately mangled copy of it, because an example that is already
//!    canonical exercises nothing: `format(x) == x` is idempotent for
//!    free. The mangled copy is where the formatter has work to do.
//!
//! 3. **The emitted bundle is byte-identical.** This is the one that
//!    matters. Comparing source text before and after only says the
//!    formatter agrees with itself; comparing what the compiler *emits*
//!    from the two says the program did not change. Indentation is the
//!    block structure here, so a formatter bug does not produce an ugly
//!    file, it produces a different program — a handler that moves one
//!    level out attaches to a different element and still compiles. That
//!    is invisible in a text diff and impossible to miss in the emitted
//!    JavaScript.

use std::path::{Path, PathBuf};
use std::process::Command;

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Every `.zd` file under `examples/`, at any depth.
///
/// **Recursively, and that is the point of the word.** The first version
/// read only the top level, which silently omitted `examples/tree/tree.zd`
/// — the one example that lives in a directory of its own because it ships
/// a `draw.js` and an `assets/` beside it. A formatter gate that skips a
/// file is a formatter gate that does not cover it, and the file it
/// skipped was the least like the others.
fn examples() -> Vec<PathBuf> {
    fn walk(at: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(at).expect("readable directory") {
            let path = entry.expect("readable directory entry").path();
            if path.is_dir() {
                walk(&path, into);
            } else if path.extension().and_then(|e| e.to_str()) == Some("zd") {
                into.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&examples_dir(), &mut found);
    found.sort();
    found
}

/// The path of an example relative to `examples/`.
///
/// Both the name a failure message uses and the key that finds the same
/// example inside a copied tree, which is why it is a relative path and not
/// a file name: `tree/tree.zd` joined onto a copy is the file, `tree.zd`
/// joined onto a copy is nothing.
fn name(path: &Path) -> String {
    path.strip_prefix(examples_dir())
        .expect("an example lives under examples/")
        .to_str()
        .expect("utf-8 path")
        .to_string()
}

/// A directory under the system temporary directory, removed when the
/// test ends whether it passed or not.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("zdc-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("failed to create the temporary directory");
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn every_example_is_already_in_the_canonical_layout() {
    let files = examples();
    assert!(
        files.len() > 20,
        "the scan found only {} examples, so it is not reading examples/",
        files.len()
    );

    let mut unformatted: Vec<String> = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).expect("readable example");
        let laid_out = zdc_fmt::format(&src)
            .unwrap_or_else(|e| panic!("{} could not be laid out: {}", name(path), e.message()));
        if laid_out != src {
            unformatted.push(name(path));
        }
    }
    assert!(
        unformatted.is_empty(),
        "these examples are not in the canonical layout: {unformatted:?}"
    );
}

/// The mangling the idempotence and emission tests run against.
///
/// Every deviation this formatter is supposed to repair, applied at once:
/// each level of indentation is doubled, a stray space is hung off the end
/// of every line, comments are flung to the left margin, blank lines are
/// tripled, and the final newline is removed.
///
/// **Block text literals are shifted, never scaled.** A literal's value is
/// each interior line with the closing delimiter's indentation removed, so
/// a *uniform* shift of the literal leaves the value alone and doubling
/// the indentation of its lines would not: with the closing delimiter at 4
/// and an interior line at 6, doubling gives 8 and 12, which turns a
/// two-space inset into a four-space one. The mangler has to preserve
/// meaning too, or the emission test would be comparing two different
/// programs and would fail for a reason that is nothing to do with the
/// formatter.
fn mangle(src: &str) -> String {
    let tokens = zdc_lexer::tokenize(src).expect("the examples lex");
    let lines: Vec<&str> = src.split('\n').collect();

    // The offset each line starts at, so a token span can be turned into a
    // line number.
    let mut starts: Vec<usize> = Vec::with_capacity(lines.len());
    let mut at = 0;
    for line in &lines {
        starts.push(at);
        at += line.len() + 1;
    }
    let line_of = |offset: u32| -> usize {
        starts
            .partition_point(|start| *start <= offset as usize)
            .saturating_sub(1)
    };

    // Which lines are the interior of a block literal, and which code line
    // opened it. Everything else is code, a comment, or blank.
    let mut owner: Vec<Option<usize>> = vec![None; lines.len()];
    let mut is_code = vec![false; lines.len()];
    for token in &tokens {
        match token.kind {
            zdc_lexer::TokenKind::Indent
            | zdc_lexer::TokenKind::Dedent
            | zdc_lexer::TokenKind::Newline
            | zdc_lexer::TokenKind::Eof => continue,
            _ => {}
        }
        let first = line_of(token.span.start);
        is_code[first] = true;
        let last = line_of(token.span.end.saturating_sub(1));
        for opened_on in owner.iter_mut().take(last + 1).skip(first + 1) {
            *opened_on = Some(first);
        }
    }

    let indent_of = |text: &str| text.len() - text.trim_start_matches(' ').len();
    let mut out: Vec<String> = Vec::new();
    for (index, text) in lines.iter().enumerate() {
        if let Some(opened_on) = owner[index] {
            // The same shift the opening line got, so the offsets inside
            // the literal — which are its value — do not move.
            let shift = indent_of(lines[opened_on]);
            out.push(format!("{}{text}", " ".repeat(shift)));
            continue;
        }
        if text.trim().is_empty() {
            out.push(String::new());
            out.push(String::new());
            out.push(String::new());
            continue;
        }
        if !is_code[index] {
            // A comment: to the left margin, where it says nothing about
            // which block it belongs to.
            out.push(format!("{} ", text.trim()));
            continue;
        }
        out.push(format!("{}{} ", " ".repeat(indent_of(text)), text.trim()));
    }

    let mut text = out.join("\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text
}

/// Formatting twice is formatting once, over every example and over a
/// mangled copy of each.
#[test]
fn formatting_is_idempotent_over_every_example() {
    let files = examples();
    assert!(
        files.len() > 20,
        "the scan found only {} examples, so it is not reading examples/",
        files.len()
    );

    for path in &files {
        let src = std::fs::read_to_string(path).expect("readable example");
        for (label, subject) in [("as written", src.clone()), ("mangled", mangle(&src))] {
            let once = zdc_fmt::format(&subject).unwrap_or_else(|e| {
                panic!(
                    "{} ({label}) could not be laid out: {}",
                    name(path),
                    e.message()
                )
            });
            let twice = zdc_fmt::format(&once).unwrap_or_else(|e| {
                panic!(
                    "{} ({label}) could not be laid out twice: {}",
                    name(path),
                    e.message()
                )
            });
            assert_eq!(
                once,
                twice,
                "{} ({label}) is not a fixed point of the formatter",
                name(path)
            );
        }
    }
}

/// Mangling an example and laying it out again gives the example back,
/// byte for byte.
///
/// Only sound because the first test in this file asserts the examples are
/// canonical to begin with; without that this would be comparing the
/// formatter against itself.
#[test]
fn a_mangled_example_is_restored_exactly() {
    let files = examples();
    assert!(
        files.len() > 20,
        "the scan found only {} examples, so it is not reading examples/",
        files.len()
    );

    for path in &files {
        let src = std::fs::read_to_string(path).expect("readable example");
        let mangled = mangle(&src);
        assert_ne!(
            mangled,
            src,
            "{} was not actually mangled, so this asserts nothing",
            name(path)
        );
        let restored = zdc_fmt::format(&mangled)
            .unwrap_or_else(|e| panic!("{} could not be laid out: {}", name(path), e.message()));
        assert_eq!(restored, src, "{} did not come back intact", name(path));
    }
}

/// One emitted bundle: every file in it, as a path relative to the bundle
/// root and the bytes at that path, sorted by path.
type Bundle = Vec<(String, Vec<u8>)>;

/// Every file `zdc build` writes, keyed by its path inside the bundle.
fn bundle(out: &Path) -> Bundle {
    fn walk(root: &Path, at: &Path, into: &mut Bundle) {
        let Ok(entries) = std::fs::read_dir(at) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("readable directory entry").path();
            if path.is_dir() {
                walk(root, &path, into);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("a bundle file is under the bundle")
                .display()
                .to_string();
            into.push((
                relative,
                std::fs::read(&path).expect("readable bundle file"),
            ));
        }
    }
    let mut files = Vec::new();
    walk(out, out, &mut files);
    files.sort();
    files
}

/// **The strong one.** Build every example, mangle and re-lay-out a copy
/// of the whole `examples/` tree, build that, and require every emitted
/// byte to match — `client.js` above all, since that is where a block
/// that changed shape would show up.
///
/// The whole tree is copied rather than one file at a time because a `use`
/// resolves against the importing file's directory (§14D.2), so an example
/// formatted in isolation would not find its imports.
///
/// `examples/site.zd` is routed and emits `pages/*.js` rather than
/// `client.js` (§14G.2). Comparing the entire bundle rather than one named
/// file covers it, and covers `index.html`, the stylesheet, the manifest
/// and every emitted server function besides.
///
/// **Both builds run from the same path**, with the copy rewritten in
/// place between them. `zdc build` writes the source path into the banner
/// on line 1 of `client.js` — `// zdc 0.1.0 · examples/blog.zd ·
/// generated, do not edit` — so building the original from `examples/`
/// and the formatted copy from the temporary directory made every single
/// example differ on line 1. That is a fact about the emitter and not
/// about the formatter, and a test that tripped over it would have to be
/// weakened to ignore line 1, which is the line most likely to change if
/// somebody later put a hash of the source there.
#[test]
fn the_emitted_bundle_is_byte_identical_after_formatting() {
    let scratch = TempDir::new("fmt-emission");
    let source_tree = scratch.path.join("examples");
    copy_tree(&examples_dir(), &source_tree);

    // One bundle directory, reused and emptied between builds, and the
    // emitted bytes held in memory instead. Twenty-seven examples times
    // two builds is fifty-four bundles at 44K each, and a test suite that
    // needs two and a half megabytes of free disk to pass is a test suite
    // that fails for a reason nobody will connect to the formatter — this
    // one did, on a machine that was at 100%.
    let out = scratch.path.join("bundle");
    let mut before: Vec<(String, Bundle)> = Vec::new();
    for path in examples() {
        let _ = std::fs::remove_dir_all(&out);
        if build(&source_tree.join(name(&path)), &out) {
            before.push((name(&path), bundle(&out)));
        }
    }
    assert!(
        before.len() > 20,
        "only {} examples built at all, so this test is inspecting almost nothing",
        before.len()
    );

    // Mangle and re-lay-out every example, in place. What is built the
    // second time is therefore text this formatter produced, at the same
    // path as the text a person wrote.
    let mut laid_out = 0;
    for path in examples() {
        let copy = source_tree.join(name(&path));
        let src = std::fs::read_to_string(&copy).expect("readable copy");
        let formatted = zdc_fmt::format(&mangle(&src))
            .unwrap_or_else(|e| panic!("{} could not be laid out: {}", name(&path), e.message()));
        std::fs::write(&copy, &formatted).expect("writable copy");
        laid_out += 1;
    }
    assert!(
        laid_out > 20,
        "only {laid_out} examples were laid out, so this test is inspecting almost nothing"
    );

    let mut compared = 0;
    for (example, emitted_before) in &before {
        let _ = std::fs::remove_dir_all(&out);
        assert!(
            build(&source_tree.join(example), &out),
            "{example} built before it was laid out and not after"
        );
        let emitted_after = bundle(&out);

        assert!(
            emitted_before.iter().any(|(path, _)| path.ends_with(".js")),
            "{example} emitted no JavaScript, so there is nothing to compare"
        );
        assert_eq!(
            emitted_before.iter().map(|(p, _)| p).collect::<Vec<_>>(),
            emitted_after.iter().map(|(p, _)| p).collect::<Vec<_>>(),
            "{example} emitted a different set of files after formatting"
        );
        for ((relative, before_bytes), (_, after_bytes)) in
            emitted_before.iter().zip(emitted_after.iter())
        {
            assert!(
                before_bytes == after_bytes,
                "{example}: {relative} changed when the source was re-laid-out. Formatting \
                 moved a block, which in an indentation-significant language is a different \
                 program."
            );
        }
        compared += 1;
    }
    assert!(
        compared > 20,
        "only {compared} examples were compared, so this test is inspecting almost nothing"
    );
}

/// Compile one file into `out`, reporting whether it compiled.
fn build(file: &Path, out: &Path) -> bool {
    Command::new(env!("CARGO_BIN_EXE_zdc"))
        .arg("build")
        .arg(file)
        .arg("--out")
        .arg(out)
        .output()
        .expect("failed to run the zdc binary")
        .status
        .success()
}

/// Copy a directory and everything under it.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("failed to create the destination directory");
    for entry in std::fs::read_dir(from).expect("readable source directory") {
        let path = entry.expect("readable directory entry").path();
        let target = to.join(path.file_name().expect("a directory entry has a name"));
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            std::fs::copy(&path, &target).expect("failed to copy a file");
        }
    }
}
