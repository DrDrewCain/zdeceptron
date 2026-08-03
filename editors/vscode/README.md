# ZDeceptron for VS Code

Syntax highlighting for `.zd` files.

## Install locally

```sh
ln -s "$(pwd)/editors/vscode" ~/.vscode/extensions/zdeceptron
```

Then reload VS Code (`Cmd+Shift+P` → *Developer: Reload Window*). Open any `.zd` file — the language indicator in the status bar should read **ZDeceptron**.

## File icons for `.zd`

**VS Code does not let an extension add a single file icon to whatever theme you already use.** That has been an open request since 2016 ([microsoft/vscode#14662](https://github.com/microsoft/vscode/issues/14662)). An extension can only ship a *complete* icon theme, which replaces yours. So there are two routes:

### Route 1 — use the bundled theme (no other icon theme installed)

`Cmd+Shift+P` → **Preferences: File Icon Theme** → **ZDeceptron**

`.zd` files get the mark; everything else gets a plain outline file/folder icon. Fine if you're on VS Code's default and mostly want `.zd` to stand out.

### Route 2 — keep a rich icon theme and add an association (recommended)

Install [Material Icon Theme](https://marketplace.visualstudio.com/items?itemName=PKief.material-icon-theme), then in `settings.json`:

```json
"workbench.iconTheme": "material-icon-theme",
"material-icon-theme.files.associations": {
  "*.zd": "../../../zdeceptron/editors/vscode/fileicons/zd.svg"
}
```

You keep accurate icons for every other file type and still get the ZDeceptron mark on `.zd`. The path is relative to Material's `dist` folder and must resolve inside `~/.vscode/extensions`.

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
