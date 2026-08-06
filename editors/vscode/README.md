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

- **Diagnostics.** The compiler's own errors, inline, as you type and again when you save. Every syntax error names the single valid phrasing (spec §7.3), and the type checker's messages arrive with their `help` attached. Every resolution error and every type error is reported, not just the first. A save republishes for every open file, because a module you saved is read from disk by whatever imports it.
- **Hover.** The inferred type, and **where the value lives**. Hovering a `server` signal read from the view says it is `Remote of List of Item` and that the read crosses the network. That is the language's thesis, in a tooltip.
- **Inlay hints.** The same answer at every binder, without asking. A parameter or a loop variable is annotated with the type the checker gave it, so a value that arrived across the network reads as `Remote of T` where it is bound.
- **Go to definition.** A lookup in the resolver's output, so it follows the same edge the compiler compiled. Forward references work, because top-level declarations are order-independent, and a definition in a file you imported opens that file at that offset.
- **Go to type definition.** From a value to the `record` or `choice` behind it. Types are inferred, so this is the only route there.
- **Find references, document highlight and rename.** One traversal of the resolver's output with three answers wanted, so the three cannot disagree. All of them cross a `use`: renaming a function declared in another file rewrites its declaration, the `use` line that borrowed it, and every call. Rename is refused outright for anything whose occurrences cannot all be found, because a partial rename leaves a file that no longer compiles.
- **Document symbols and workspace symbols.** An outline of one file, and a search across every file your program reaches.
- **Call hierarchy.** What calls a function and what it calls. This language has no first-class functions, so naming a callable is calling it, and the call graph is also the region graph: a call from the view into a `server`-rooted callable is where the network is.
- **Signature help.** The parameters of the call you are writing, with their inferred types, from the moment you type `with`.
- **Folding ranges.** The block structure, which is the layout pass's own output rather than a second measurement of your indentation.
- **Code actions.** The one repair the compiler can derive rather than paraphrase: a name a file you already import declares, but that your `use` line did not borrow.
- **Semantic tokens.** See below. Both the whole-document form and the range form.
- **Completion.** Built-in elements, the base types and constructors, the three placements after a declaration's `is`, and the names the file declares.

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

Five things are known to be missing rather than broken:

- `record` and `choice` are highlighted but not implemented in the lexer — they are specified (§14B.1) and pending.
- Completion does not offer locals. The compiler records no owning body for a binding, so there is no way to tell which names are in scope without a pass the server does not have; offering all of them would suggest names that are not.
- Formatting is not implemented, and cannot be until the lexer keeps comments. `#` comments are discarded before layout (`crates/zdc-lexer/src/raw.rs`), no token or syntax node carries one, and the lexeme that names one is private to that crate. A formatter written on what the compiler exposes today would print a correct file with every comment deleted.
- Rename refuses a `record`, a `choice`, a variant name and a field. Types are not resolved yet (§14B.1), so a name in type position carries no resolution and its occurrences cannot be enumerated. Renaming what can only be found in part is what this refuses to do.
- Documents are synchronised in full and re-analysed whole. That is fine at the size of file this language is for and would need incremental compiler passes to change.
