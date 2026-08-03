# ZDeceptron for VS Code

Syntax highlighting for `.zd` files.

## Install locally

```sh
ln -s "$(pwd)/editors/vscode" ~/.vscode/extensions/zdeceptron
```

Then reload VS Code (`Cmd+Shift+P` → *Developer: Reload Window*). Open any `.zd` file — the language indicator in the status bar should read **ZDeceptron**.

## What this does, and deliberately does not, do

This grammar classifies **tokens only**: keywords, literals, comments, type names, and the three placements. It does **not** model structure or scope.

That limit is intentional. TextMate grammars are regular expressions, and ZDeceptron has constructs a regular expression cannot resolve:

- **`is` does three different jobs** — it introduces a declaration (`state x is client …`), marks a named argument (`hint is "search"`), and tests equality (`a is b`). All three get one scope here, because nothing short of parsing can tell them apart.
- **Capitalised identifiers** are either user-defined types or view elements. Distinguishing them requires name resolution.
- **Indentation is syntax.** The `indentationRules` here are heuristics; the compiler's layout pass is the authority.
- **Dialects** (spec §4.6) will let the same program be written in different surface syntaxes. A regex grammar would need one copy per dialect, each drifting independently.

Encoding the full grammar here would mean maintaining a second, less accurate parser that disagrees with the real one. Structural and semantic highlighting belongs to a language server, which asks the compiler rather than guessing.

## Planned: language server

A `zdc-lsp` crate wrapping the existing compiler, exposing:

- **Diagnostics** — the compiler's real errors inline as you type. The compiler already names the single valid phrasing for every syntax error (spec §7.3), so this is mostly plumbing rather than new work.
- **Semantic tokens** — accurate scopes for `is`, types vs elements, and placement-derived colouring.
- **Hover** — inferred types, and *where a value lives* (`client`, `server`, `durable`).
- **Go to definition**, completion.

Hover showing placement is the interesting one: it makes the network boundary visible in the editor, which is the whole thesis of the language.

## Status

Highlighting works. `record` and `choice` are highlighted but not yet implemented in the lexer — they are specified (§14B.1) and pending.
