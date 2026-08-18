-- The Neovim half of `zdc lsp`.
--
-- Copy this file to `~/.config/nvim/lsp/zdeceptron.lua`. Neovim reads
-- `lsp/<name>.lua` off the runtime path, so the file's *name* is the
-- server's name and nothing here has to repeat it.
--
-- Two lines in `init.lua` finish the job — Neovim has never heard of `.zd`,
-- so nothing would ever match `filetypes` below without the first of them:
--
--     vim.filetype.add({ extension = { zd = "zdeceptron" } })
--     vim.lsp.enable("zdeceptron")
--
-- See `README.md` beside this file for the rest, including why this is a
-- file you copy rather than an entry in `nvim-lspconfig`.

return {
  -- The language server is a subcommand of the compiler, not a second
  -- binary: if `zdc` is on your `PATH`, so is the server. Point this at an
  -- absolute path if it is not.
  cmd = { "zdc", "lsp" },

  filetypes = { "zdeceptron" },

  -- No `root_markers`, and that is the language's design rather than an
  -- omission: a ZDeceptron project has no manifest to look for. `zdc new`
  -- writes a `.zd` file and a stylesheet and nothing else, because the file
  -- *is* the program, and `use` resolves against the importing file's own
  -- directory. So there is nothing to find, and asking Neovim to find it
  -- would only produce a root that means nothing to the server, which
  -- ignores `rootUri` for the same reason.
  --
  -- The visible consequence is a good one: with no root, Neovim runs a
  -- single server for every `.zd` buffer you open, which is what makes a
  -- save republish diagnostics for the other files that import it.
}
