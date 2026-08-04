# ZDeceptron for VS Code

Syntax highlighting for `.zd` files, and a language server that asks the compiler.

## Install locally

Build the compiler and put it on `PATH`, install the extension's one dependency, then link the extension:

```sh
cargo install --path crates/zdc-cli          # provides `zdc`
npm install --prefix editors/vscode          # provides the language client
ln -s "$(pwd)/editors/vscode" ~/.vscode/extensions/zdeceptron
```

Then reload VS Code (`Cmd+Shift+P` → *Developer: Reload Window*). Open any `.zd` file — the language indicator in the status bar should read **ZDeceptron**.

If `zdc` is not on your `PATH`, set `zdeceptron.server.path` in `settings.json` to wherever it is. The extension runs `zdc lsp`; there is no second binary to install, because the language server is a subcommand of the compiler.

## What the language server does

Everything below is computed by running the compiler's real passes on the file in your editor. Nothing is re-implemented, so the editor and `zdc check` cannot disagree.

- **Diagnostics** — the compiler's own errors, inline, as you type. Every syntax error names the single valid phrasing (spec §7.3), and the type checker's messages arrive with their `help` attached. Every resolution error and every type error is reported, not just the first.
- **Hover** — the inferred type, and **where the value lives**. Hovering a `server` signal read from the view says it is `Remote of List of Item` and that the read crosses the network. That is the language's thesis, in a tooltip.
- **Go to definition** — a lookup in the resolver's output, so it follows the same edge the compiler compiled. Forward references work, because top-level declarations are order-independent.
- **Semantic tokens** — see below.
- **Completion** — built-in elements, the base types and constructors, the three placements after a declaration's `is`, and the names the file declares.

The whole pipeline re-runs on every change. That is not incremental and is not pretending to be; it is fast enough at the size of file this language is for, and it is the same code path as the command line.

### Colouring by placement

Semantic tokens carry a modifier naming the placement of whatever a reference refers to — `client`, `server`, or `durable` — so the network boundary can be visible while you write. No colours are imposed on your theme; to switch it on, put this in `settings.json`:

```json
"editor.semanticTokenColorCustomizations": {
  "rules": {
    "*.client": { "foreground": "#4ec9b0" },
    "*.server": { "foreground": "#dcdcaa" },
    "*.durable": { "foreground": "#c586c0", "bold": true }
  }
}
```

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

## What the TextMate grammar does, and deliberately does not, do

The grammar in `syntaxes/` classifies **tokens only**: keywords, literals, comments, type names, and the three placements. It does **not** model structure or scope.

That limit is intentional. TextMate grammars are regular expressions, and ZDeceptron has constructs a regular expression cannot resolve:

- **`is` does three different jobs** — it introduces a declaration (`state x is client …`), marks a named argument (`hint is "search"`), and tests equality (`a is b`). All three get one scope here, because nothing short of parsing can tell them apart.
- **Capitalised identifiers** are either user-defined types or view elements. Distinguishing them requires name resolution.
- **Indentation is syntax.** The `indentationRules` here are heuristics; the compiler's layout pass is the authority.
- **Dialects** (spec §4.6) will let the same program be written in different surface syntaxes. A regex grammar would need one copy per dialect, each drifting independently.

Encoding the full grammar here would mean maintaining a second, less accurate parser that disagrees with the real one. Each of the first two is now resolved by the language server instead, which asks the compiler: `zdc-lsp` gives the three jobs of `is` three different scopes, and colours a capitalised name by what the resolver made it. The grammar is what remains when the server is not running, and it stays deliberately coarse.

## Status

Both halves work. Highlighting falls back to the grammar when the server is unavailable, and the server refines it when it is.

Four things are known to be missing rather than broken:

- `record` and `choice` are highlighted but not implemented in the lexer — they are specified (§14B.1) and pending.
- Completion does not offer locals. The compiler records no owning body for a binding, so there is no way to tell which names are in scope without a pass the server does not have; offering all of them would suggest names that are not.
- Rename, find-references, and formatting are not implemented.
- Documents are synchronised in full and re-analysed whole. That is fine at the size of file this language is for and would need incremental compiler passes to change.
