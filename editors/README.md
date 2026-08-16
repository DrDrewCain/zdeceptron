# Editors

`zdc lsp` is a language server, and the Language Server Protocol is the
point of a language server: one binary, every editor. What differs between
editors is four lines of configuration and, unhappily, whether the file
gets any colour at all.

| Editor | What is here | Colour | Run against the compiler |
| --- | --- | --- | --- |
| [VS Code](vscode/README.md) | extension, TextMate grammar, icon theme | grammar, refined by semantic tokens | pre-dates this directory |
| [Neovim](neovim/README.md) | `neovim/zdeceptron.lua` | semantic tokens only | ✅ 0.12.4, macOS arm64 |
| [Helix](helix/README.md) | `helix/languages.toml` | **none** | ✅ 25.07.1, macOS arm64 |
| [Zed](#zed) | nothing — see below | — | — |
| anything else | [the four facts below](#any-editor-that-speaks-lsp) | depends | — |

"Run against the compiler" means a real editor was started against a real
`.zd` file with the configuration in this directory and observed to
receive the compiler's diagnostics. Each editor's README says what was
observed and what was only written down.

## Any editor that speaks LSP

Four facts, and there is no fifth:

1. **The command is `zdc lsp`.** No arguments, no port, no environment.
   It speaks JSON-RPC on stdin and stdout, and stdout carries nothing
   else — a failure to start goes to stderr, which is where your editor's
   log is. The server is a subcommand of the compiler, so if you have
   `zdc` you already have it.
2. **The extension is `.zd`.** Give it the language id `zdeceptron` if
   your client wants one, to match the VS Code grammar's
   `source.zdeceptron` scope. The server itself does not read
   `languageId`, so nothing breaks if you pick another.
3. **There is nothing to configure.** The server ignores `initialize`'s
   parameters entirely: no `initializationOptions`, no `rootUri`, no
   workspace folders. This language has no project manifest — `zdc new`
   writes a `.zd` file and a stylesheet, because the file *is* the
   program — so `use` resolves against the importing file's own directory
   and a root would mean nothing. A client that sends no root is correct.
4. **Documents synchronise in full.** The server advertises
   `TextDocumentSyncKind.Full` and means it: it treats the last content
   change as the whole document. A client that sends ranged edits anyway
   hands the compiler a fragment and gets diagnostics about a file nobody
   wrote.

Two things follow that are worth knowing before you debug them.
Diagnostics are **pushed** — `textDocument/publishDiagnostics` on open, on
change, and on save; there is no pull-model `textDocument/diagnostic`
handler, so a client that only pulls will show a clean file forever. And a
save republishes for *every* open document, not just the saved one,
because a module is read from disk by whatever imports it.

An unsaved, untitled buffer is still analysed. What it loses is `use`:
there is no directory to resolve a neighbouring file against.

## Syntax highlighting is the one thing that does not travel

The language server is portable and the colouring is not, because every
editor wants a different artefact: VS Code wants a TextMate grammar,
which is in `vscode/syntaxes/`; Zed and Helix want tree-sitter; Neovim
takes either but is happy with neither, and gets its colour from the
server's semantic tokens instead.

**No tree-sitter grammar for ZDeceptron exists, and writing one is out of
scope here.** Not out of scope because it is tedious — out of scope
because of what it is. `vscode/README.md` explains why the TextMate
grammar is deliberately coarse: encoding this language's structure into a
grammar file means maintaining a second, less accurate parser that
disagrees with the real one. A tree-sitter grammar is exactly that second
parser, and a more convincing one, which makes the disagreement worse
rather than better. It would also have to model indentation as syntax
through a hand-written external scanner, and §4.6's dialects would
eventually need one copy each.

Semantic tokens are the escape from that, and they are strictly better
where a client supports them: the compiler classifies the tokens itself,
so `is` gets three different scopes for its three different jobs and a
capitalised name is coloured by what the resolver made it. Neovim proves
the route works — a `.zd` buffer there has no parser and no `syntax`
match, and is fully coloured. Helix does not implement semantic tokens,
which is why its row above says **none**.

If someone does write a tree-sitter grammar, it belongs in a repository of
its own: Zed fetches grammars by repository URL and pinned revision, and
Helix expects to build them from a `[[grammar]]` source. That, not the
extension packaging, is the whole of what stands between this directory
and a Zed extension.

## Zed

**There is no Zed extension, and this is a decision rather than an
omission.** A Zed language extension needs two things, and the first one
is the blocker:

- **A tree-sitter grammar, required.** `languages/<name>/config.toml`
  takes a `grammar` key, it is not optional, and `extension.toml` must
  resolve that name to a `repository` and a `rev`. Zed has no
  settings-only escape hatch either: `file_types` maps a suffix onto a
  language Zed already knows, and the `lsp` setting configures a server
  some extension already declared. So there is no partial version of this
  — no way to ship the language server today and the colouring later.
- **A Rust extension compiled to WebAssembly**, implementing
  `language_server_command` to tell Zed how to launch `zdc lsp`. This
  part is genuinely small, and it is not what is missing.

Shipping a stub that cannot load would be worse than shipping nothing: it
would put ZDeceptron in Zed's extension list and then fail in a way a user
cannot act on. When a grammar exists, the extension is an afternoon.

## Keeping these honest

`crates/zdc-cli/tests/editor_configs.rs` holds every configuration in this
directory to the compiler it launches: each one must invoke the subcommand
`zdc` actually has, and must associate the extension the compiler actually
reads. That gate exists because the sibling one already earned its keep —
`scripts/check-grammar-drift.py` was written after the TextMate grammar
spent several releases highlighting four keywords the lexer rejects, so a
program that used them looked valid in the editor and would not parse. A
copy of a fact about the compiler, kept in `editors/`, drifts. These are
now four such copies.
