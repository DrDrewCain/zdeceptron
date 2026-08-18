# ZDeceptron for Helix

Two stanzas of TOML and no plugin: Helix reads `languages.toml`, and a
language server is a `command` and its arguments.

## Install

```sh
cargo install --path crates/zdc-cli          # provides `zdc`
cat editors/helix/languages.toml >> ~/.config/helix/languages.toml
```

Append rather than copy — Helix merges your file over its built-in one, so
whatever is already in there stays. Then check it:

```sh
hx --health zdeceptron
```

`Configured language servers: ✓ zdc: /path/to/zdc` means Helix found the
compiler. Open a `.zd` file and diagnostics arrive as you type.

## No syntax highlighting, and this is not a setting you missed

**Helix will not colour a `.zd` file at all.** Two independent reasons,
either of which alone would be enough:

- **There is no tree-sitter grammar for ZDeceptron.** Helix's highlighting
  is tree-sitter and only tree-sitter, and no grammar exists to point it
  at. `hx --health zdeceptron` reports `Highlight queries: ✘`, which is
  the line to read. (It also reports `Tree-sitter parser: ✓` on Helix
  25.07, which is wrong — there is no `zdeceptron` grammar in its runtime
  directory to have loaded.)
- **Helix does not implement LSP semantic tokens.** That is the route by
  which Neovim colours `.zd` without a grammar, and it is closed here:
  the `initialize` request Helix 25.07.1 sends declares no
  `textDocument.semanticTokens` capability at all, so the server is never
  asked.

Everything the compiler knows is still there — diagnostics, hover, go to
definition, references, rename, symbols, signature help — as monochrome
text. See [the root `README.md`](../README.md#syntax-highlighting-is-the-one-thing-that-does-not-travel)
for why a tree-sitter grammar is a project of its own rather than a file
that belongs in this directory.

## Nothing formats yet

Leave `auto-format` off. The server does not advertise
`textDocument/formatting` (#77), so `:format` has nothing to call.
`zdc fmt path/to/file.zd` writes the canonical layout in the meantime,
and it is the same layout CI holds every example in `examples/` to.

## What was actually run

Helix **25.07.1** on macOS (arm64), against the compiler built from this
tree, with exactly the `languages.toml` beside this file:

- **The file type is recognised.** The `textDocument/didOpen` Helix sent
  carried `"languageId": "zdeceptron"`, so `file-types = ["zd"]` binds.
- **The handshake completes.** The server answered `initialize` with its
  full capability set — hover, definition, references, rename, symbols,
  folding, semantic tokens, signature help, completion, call hierarchy.
- **Diagnostics arrive.** Opening a file with an undefined name produced
  a `textDocument/publishDiagnostics` carrying the compiler's own
  message at line 5, character 13–23 — the same error `zdc check` prints.

Read for yourself with `hx -vv file.zd` and then
`~/.cache/helix/helix.log`, which is where the above was observed.

Not run here: Helix on Windows or Linux, and any Helix older than 25.07.
