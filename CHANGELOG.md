# Changelog

What changed, when, and why it mattered. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

**What is versioned here is `zdc` — the compiler binary and the language it
accepts.** The eighteen crates in `crates/` are published to crates.io so that
`cargo install zdc-cli` works — from the first tagged release; nothing is
published yet. They carry the same version because they are one compiler
released together, not eighteen libraries with their own lives. Their APIs are internal: depend on `zdc-codegen` and a patch release may
change it under you. The language is the thing with a compatibility promise.

While the major version is `0`, a minor bump may change the language. What that
means in practice is that a program is guaranteed to keep compiling across a
patch release and is not guaranteed to across a minor one — and any minor
release that breaks a program will say so here, with the repair.

## [Unreleased]

### Added

- **`zdc new <path>` starts a project.** Two files — a program with one signal,
  one derived from it and one event handler, and an `assets/style.css` linked
  after the generated stylesheet — and then the `zdc dev` command that runs
  them. Until now a program started at a blank file and whatever the reader
  remembered of the examples, which meant the first thing the compiler said to
  a new reader was a diagnostic about a construct they had not met. A directory
  that already contains anything is refused and nothing is written. The
  scaffold is checked and built by the test suite, so a template that drifts
  out of sync with the language fails CI rather than a reader's first five
  minutes. (#168)
- **Two ways to install**, both landing with the first tagged release: `zdc`
  goes to crates.io — `cargo install zdc-cli` — and is built for five targets — macOS on Apple silicon and
  Intel, Linux on x86-64 and arm64 (musl, statically linked), and Windows on
  x86-64 — with a checksum per artefact and a POSIX `sh` installer that
  verifies it. Until now the only way to get a compiler was to build one.
  (#166)
- A declared minimum supported Rust version, enforced in CI, so "does this
  build on my machine" has an answer that is not "try it". (#164)
- `setAt` in the list prelude: replace one element at an index. The guard is
  load-bearing — a naive composition *appends* past the end, because
  `listTake` saturates. (#195)
- This changelog. (#182)

### Fixed

- **A `foreign … gives view` says which declaration broke the contract.** The
  imported name has to be `mount(node, props) -> { update(props), destroy() }`,
  a shape no type in the language can hold — so `from "three" as "Scene"`
  compiled, and then failed on the first render with an engine `TypeError`
  raised inside `runtime/foreign.js`, naming a local nobody wrote. The
  lifecycle now checks the contract at mount and refuses in the declaration's
  own name, saying what was expected and what arrived instead: a binding that
  is not callable, a class (which is what every visual library exports, and is
  callable as far as `typeof` can tell), or a handle missing `update` or
  `destroy`. A module that meets the contract is untouched. The declarative
  `constructs with … / mounts through …` form sketched in the same issue is
  not part of this and remains an open question. (#239)
- **Windows line endings are read rather than refused.** The lexer rejected
  any carriage return, which made Windows a platform the language did not run
  on: Git there rewrites LF to CRLF on checkout, so a Windows clone got a
  working `zdc` and a tree of `.zd` files that same binary rejected. A CRLF
  program now emits byte-identical output to its LF twin — asserted, because
  indentation is the block structure here and a carriage return counted as a
  column would reshape a program rather than fail it. A lone carriage return
  is still refused. (#242)
- **A cycle of tail calls is a loop, not a stack of frames.** The rewrite that
  turns `give f …` inside `f` into a jump fired on a self-call and nothing
  else, so two functions that give the result of calling each other stayed
  recursion — one frame per hop. A merge split across two functions died at
  3,200 elements where the one-function spelling merged a hundred thousand;
  it now merges a hundred thousand too. The unit is the cycle rather than the
  pair, so `f → g → h → f` is covered as much as `f → g → f`. Self-calls are
  untouched and still allocate nothing, and all twenty-six example bundles are
  byte-identical to before the change. (#198)
- Assets are contained on the resolved path, so a symlink under `assets/`
  can no longer copy a file from outside the project into a bundle. An asset
  that resolves outside is refused by name rather than silently skipped.
  (#188)
- A quadratic flatten in list building: 21 s to 1.1 s at depth, counted by
  instrumentation rather than timed. (#192)
- Exponent literals (`1e10`, `1e-9`) are lexed and diagnosed rather than
  silently mis-scanned. (#184)
- Span-key collisions in the mutation cross-check, which conflated two writes
  at the same source position. (#13)
- Near-miss suggestions for value names, not just variant names, so a typo in
  a function name is answered with the function you meant. (#150)
- `NO_COLOR` and `--no-color` are honoured by every command that can print a
  diagnostic. (#153)

### Changed

- Dijkstra's frontier minimum is extracted in one pass instead of four:
  building an intermediate cost list, scanning it, and then walking the
  frontier again to find where that cost was is three walks to answer what one
  walk answers. Same route, same toll, same pop count.
- The README says how to *run* what `zdc build` produces. It emitted `dist/`
  and stopped there, and because the document loads ES modules, the obvious
  next move — opening `index.html` — fails silently.

---

## About the versions that are not here

There are none before this. `0.1.0` will be the first tagged release, and this
file starts at the point where the project began recording changes rather than
reconstructing them from the git log after the fact. What happened before is in
[`STATUS.md`](STATUS.md), which is the milestone-by-milestone account, and in
the commit history, which is complete.
