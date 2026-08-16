# ZDeceptron for Neovim

The language server, with no plugin and no plugin manager. Everything a
Neovim user sees comes out of `zdc lsp`, which is the same subcommand the
VS Code extension launches, so the two editors cannot disagree.

## Install

Build the compiler onto your `PATH`, then copy one file:

```sh
cargo install --path crates/zdc-cli          # provides `zdc`
mkdir -p ~/.config/nvim/lsp
cp editors/neovim/zdeceptron.lua ~/.config/nvim/lsp/zdeceptron.lua
```

Two lines in `init.lua` finish it:

```lua
vim.filetype.add({ extension = { zd = "zdeceptron" } })
vim.lsp.enable("zdeceptron")
```

The first is not optional and is easy to skip. Neovim has never heard of
`.zd`, so without it every `.zd` file has an empty `filetype`, nothing
matches the `filetypes` in `zdeceptron.lua`, and the server is never
started — with no error, because nothing went wrong.

If `zdc` is not on your `PATH`, change `cmd` in the copied file to name it
outright: `cmd = { "/path/to/zdc", "lsp" }`.

**Neovim 0.11 or later.** `vim.lsp.config` and `vim.lsp.enable`, and the
`lsp/<name>.lua` runtime path directory they read, all arrived in 0.11.
See [older Neovim](#older-neovim) below for what to do instead.

## Why a file you copy, and not an `nvim-lspconfig` entry

`nvim-lspconfig` 2.x is a directory of `lsp/<name>.lua` files in exactly
the shape of the one beside this README, because 0.11 moved the mechanism
into Neovim itself. So the choice is not between two designs — it is only
about which repository the file lives in, and this one is the better
answer for three reasons:

- **It needs nothing installed.** A user who has never installed a plugin
  manager can copy one file and be done. Depending on `nvim-lspconfig`
  would make a plugin the price of admission for a configuration that is
  four lines long.
- **It can be held to the compiler.** `crates/zdc-cli/tests/editor_configs.rs`
  asserts this file launches the subcommand `zdc` actually has. A copy in
  another repository is a copy nothing here can check, and
  `scripts/check-grammar-drift.py` exists because that kind of copy drifts.
- **Upstreaming is still open, and cheap.** The file is already in the
  shape `nvim-lspconfig` wants, so contributing it there later is a
  matter of moving it, not of writing it again.

## Syntax highlighting comes from the language server

**There is no tree-sitter grammar for ZDeceptron, and no Vim syntax file
in this repository.** Colour is not lost, though, because Neovim applies
LSP semantic tokens natively — and the semantic tokens are the better
half of the answer, since they are the compiler's own classification
rather than a regular expression's guess at it.

Verified with `:Inspect` on a `.zd` buffer: no tree-sitter parser, no
`syntax` match, and keywords, types, literals and names all carrying
`@lsp.type.*` groups, which link to the ordinary Treesitter highlight
groups your colourscheme already styles.

The one thing worth switching on is placement. The server tags every
reference with the placement of what it refers to — `client`, `server` or
`durable` — as a semantic token *modifier*, which Neovim leaves unstyled
by default. Three lines in `init.lua` make the network boundary visible
while you write:

```lua
vim.api.nvim_set_hl(0, "@lsp.mod.client.zdeceptron",  { fg = "#4ec9b0" })
vim.api.nvim_set_hl(0, "@lsp.mod.server.zdeceptron",  { fg = "#dcdcaa" })
vim.api.nvim_set_hl(0, "@lsp.mod.durable.zdeceptron", { fg = "#c586c0", bold = true })
```

The `.zdeceptron` suffix is the client's name and is not decoration:
Neovim emits `@lsp.mod.<modifier>.<client>` groups, so the unsuffixed
`@lsp.mod.client` is a group nothing sets.

## Two things that are off until you ask

**Inlay hints** are off by default in Neovim. The server annotates every
binder with the type the checker gave it, which is where a value that
arrived across the network reads as `Remote of T`:

```lua
vim.lsp.inlay_hint.enable(true)
```

**Nothing formats.** `vim.lsp.buf.format()` does nothing on a `.zd`
buffer, because the server does not advertise `textDocument/formatting`
yet (#77). `zdc fmt path/to/file.zd` is the formatter until it does; the
layout it writes is the one CI holds every example to.

## What the server does

The full list, with the reasoning behind each answer, is in
[`../vscode/README.md`](../vscode/README.md#what-the-language-server-does)
and is not repeated here — it is one server and one set of answers, so a
second copy of the list is a second thing to keep true. In Neovim's own
vocabulary it reaches `vim.diagnostic`, `K`, `grr`, `gri`, `grn`, `gO`,
`gd`, completion through `vim.lsp.completion`, folds via
`foldexpr=v:lua.vim.lsp.foldexpr`, and the call hierarchy and workspace
symbol pickers of whichever picker plugin you use.

## What was actually run

Neovim **0.12.4** on macOS (arm64), against the compiler built from this
tree, using exactly the configuration above:

- **Diagnostics arrive.** A file with an undefined name published one
  error at line 5, column 14 — the same position and the same message
  `zdc check` prints for it.
- **Hover, inlay hints and semantic tokens answer.** Hover on a `client`
  signal returned its declaration and the sentence about browser memory;
  six inlay hints appeared on `examples/guestbook.zd`; the placement
  modifiers `client`, `server` and `durable` all appeared as semantic
  token modifiers on that file.
- **One server serves every buffer.** Three `.zd` buffers in two
  directories attached to a single client, which is what makes a save
  republish diagnostics for the other files that import what you saved.

Not run here: any Neovim older than 0.12, and any of this on Windows.

## Older Neovim

Before 0.11 there is no `vim.lsp.config`, and `vim.lsp.start` is called
from an autocommand instead. Put this in `init.lua` and copy no file:

```lua
vim.filetype.add({ extension = { zd = "zdeceptron" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "zdeceptron",
  callback = function(args)
    vim.lsp.start({ name = "zdeceptron", cmd = { "zdc", "lsp" } }, { bufnr = args.buf })
  end,
})
```

This shape was run on 0.12.4 and publishes the same diagnostic, so it is
not written from memory — but `vim.lsp.start` has existed since 0.8 and
**no Neovim older than 0.12 was run here**, so on 0.8 to 0.10 it is a
claim about a stable API rather than something observed.
