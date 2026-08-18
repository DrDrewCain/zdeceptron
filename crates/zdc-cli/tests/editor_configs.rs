//! Every configuration in `editors/` launches the compiler this tree
//! builds, and binds the file extension this tree's own editor already
//! binds.
//!
//! A configuration file is a copy of two facts about the compiler — the
//! name of the subcommand that serves the protocol, and the suffix a
//! source file has — kept somewhere `cargo build` never looks. That kind
//! of copy drifts, and this repository has already paid for it once:
//! `scripts/check-grammar-drift.py` was written after the VS Code grammar
//! spent several releases highlighting four words the lexer rejects, so a
//! program using them looked valid in the editor and would not parse.
//!
//! The failure here would be quieter than that one. An editor whose
//! language server fails to start reports nothing — there is no error to
//! show, only an absence of diagnostics, hover and highlighting, which
//! looks exactly like a language server that has not finished thinking.
//! Renaming `zdc lsp` would break every editor but VS Code, whose
//! extension nobody would think to grep, and the test suite would stay
//! green.
//!
//! `crates/zdc-cli/tests/lsp.rs` proves the subcommand *works* by speaking
//! the protocol to it. This proves the files that name it agree with it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn editors() -> PathBuf {
    repository().join("editors")
}

fn editor_file(relative: &str) -> String {
    let path = editors().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("`editors/{relative}` could not be read: {error}"))
}

/// The subcommand every configuration in `editors/` spells out.
const SUBCOMMAND: &str = "lsp";

/// Assert one configuration contains the text that launches the server.
///
/// Matched as a literal rather than parsed, because what is being pinned
/// is the *whole* invocation — the binary and its argument together. A
/// configuration that named the right binary and no argument, or the
/// right argument on the wrong binary, would satisfy any check that took
/// the two apart, and would start nothing.
#[track_caller]
fn launches(relative: &str, invocation: &str) {
    let source = editor_file(relative);
    assert!(
        source.contains(invocation),
        "`editors/{relative}` no longer contains\n\n    {invocation}\n\n\
         which is how that editor starts the language server. Either the \
         invocation moved and this line should follow it, or `zdc {SUBCOMMAND}` \
         was renamed and every file in `editors/` has to be renamed with it."
    );
}

/// The one language VS Code's extension contributes, as it declares it.
///
/// VS Code is the source of truth for the association because its
/// declaration is the oldest and the one a marketplace reader sees. The
/// other editors are then checked against it rather than against a
/// literal written here, so the four configurations cannot disagree about
/// what a ZDeceptron file is called.
fn vscode_language() -> (String, String) {
    let package: serde_json::Value = serde_json::from_str(&editor_file("vscode/package.json"))
        .expect("`editors/vscode/package.json` is JSON");
    let language = &package["contributes"]["languages"][0];
    let id = language["id"]
        .as_str()
        .expect("the contributed language has an `id`")
        .to_string();
    let suffix = language["extensions"][0]
        .as_str()
        .expect("the contributed language has an extension")
        .trim_start_matches('.')
        .to_string();
    (id, suffix)
}

#[test]
fn the_subcommand_the_editors_launch_is_one_the_binary_has() {
    let output = Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args([SUBCOMMAND, "--help"])
        .output()
        .expect("failed to run `zdc`");

    assert!(
        output.status.success(),
        "`zdc {SUBCOMMAND} --help` exited with {:?}. Every file in `editors/` \
         launches that subcommand, so if it no longer exists none of those \
         editors has a language server.",
        output.status.code()
    );
}

#[test]
fn every_editor_launches_the_language_server_the_same_way() {
    // VS Code, through the language client the extension constructs.
    launches("vscode/extension.js", r#"args: ["lsp"]"#);
    // Neovim, through the table `vim.lsp.enable` reads off the runtime path.
    launches("neovim/zdeceptron.lua", r#"cmd = { "zdc", "lsp" }"#);
    // Helix, through the `[language-server.zdc]` stanza of `languages.toml`.
    launches("helix/languages.toml", r#"command = "zdc""#);
    launches("helix/languages.toml", r#"args = ["lsp"]"#);
    // And the prose a reader of any other editor's client copies from.
    launches("README.md", "`zdc lsp`");
}

#[test]
fn every_editor_binds_the_extension_and_the_name_vs_code_contributes() {
    let (id, suffix) = vscode_language();

    let helix = editor_file("helix/languages.toml");
    assert!(
        helix.contains(&format!("file-types = [\"{suffix}\"]")),
        "`editors/vscode/package.json` contributes `.{suffix}`, so \
         `editors/helix/languages.toml` should say `file-types = [\"{suffix}\"]`. \
         Helix opens a file it has no `file-types` entry for as plain text and \
         starts no language server for it."
    );
    assert!(
        helix.contains(&format!("name = \"{id}\"")),
        "`editors/vscode/package.json` calls the language `{id}`, and \
         `editors/helix/languages.toml` calls it something else. The two names \
         reach the same server, so the disagreement is invisible until someone \
         writes a query or a grammar against one of them."
    );

    // Neovim splits the association in two: `init.lua` maps the suffix to a
    // filetype, and the server configuration matches on that filetype. Both
    // halves are checked, because either alone starts nothing.
    assert!(
        editor_file("neovim/README.md").contains(&format!(
            "vim.filetype.add({{ extension = {{ {suffix} = \"{id}\" }} }})"
        )),
        "`editors/neovim/README.md` tells the reader to map `.{suffix}` to the \
         filetype `{id}`, and no longer does. Neovim has never heard of \
         `.{suffix}`, so without that line nothing matches the server's \
         `filetypes` and it is never started — silently, because nothing \
         went wrong."
    );
    assert!(
        editor_file("neovim/zdeceptron.lua").contains(&format!("filetypes = {{ \"{id}\" }}")),
        "`editors/neovim/zdeceptron.lua` matches on a filetype that is not \
         `{id}`, which is the one `README.md` tells the reader to create."
    );
}

/// A new editor is a new copy of the same two facts, so the index has to
/// gain a row for it — that row is where a reader learns whether anyone
/// has run the thing.
#[test]
fn every_editor_directory_is_named_in_the_index() {
    let index = editor_file("README.md");

    let mut directories: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(editors()).expect("`editors/` is a directory") {
        let path = entry.expect("a directory entry").path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a directory with a name")
            .to_string();
        directories.push(name);
    }
    directories.sort();

    assert!(
        directories.len() >= 3,
        "`editors/` holds {} directories, which is fewer than the three this \
         test was written over ({directories:?}). Something was deleted, or \
         this is not the directory it thinks it is.",
        directories.len()
    );

    let unlisted: Vec<&String> = directories
        .iter()
        .filter(|name| !index.contains(&format!("]({name}/README.md)")))
        .collect();
    assert!(
        unlisted.is_empty(),
        "`editors/README.md` has no row linking to {unlisted:?}. The table is \
         where a reader finds out which editors were actually run against the \
         compiler and which were only written down; an editor missing from it \
         is an editor nobody can tell the state of."
    );
}
