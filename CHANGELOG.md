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

- **`NumberInput` — a field that yields a number.** `Input` binds `Text` and no
  `Text`-to-number conversion exists in the prelude, so a quantity, a price or
  an age had no route from the field to the type the program computes with.
  This binds `Option of Whole` or `Option of Decimal`, and the listener reads
  `valueAsNumber`, so what arrives is a number rather than the text of one.
  **The `Option` is the point**: an empty box, a lone `-` and a half-written
  `1e` all report `NaN`, which is not a value this language has, and zero would
  be worse because zero is a number somebody may have meant. `least`, `most`
  and `step` are the browser's `min`, `max` and `step`, optional unlike
  `Slider`'s, because a number field does not clamp and a required bound would
  be a number invented to satisfy the compiler. What it does **not** enforce is
  integrality: a reader can type `1.5` into a `Whole` field, which is the same
  gap `Slider` has with a fractional `step`. (#45)
- **`DateInput` — a native date picker, yielding a moment.** No `Date` type was
  invented, because the language already had the answer: `prelude/time.zd`
  fixes a point in time as a `Whole` of milliseconds since the epoch in UTC —
  what `clock` gives — and `civilDateOf`, `civilTimeOf`, `weekdayOf`, `dayOf`
  and `momentOf` read one apart and put it back together. HTML defines a date
  field's `valueAsNumber` as exactly that number, so the control and the
  prelude agree in both directions and **nothing in the compiler or its runtime
  formats a date**. The binding is `Option of Whole`, empty being `None`, and
  `Option of Decimal` is refused rather than floored out of sight. **Not
  delivered:** an earliest or a latest day. A date input's `min` and `max` are
  ISO date strings and the shared argument table types `least` and `most` as
  numbers, so that bound is not expressible and is left out rather than
  approximated. (#48)
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
- **A `foreign` can reach a package without a JavaScript file in between.** A
  URL specifier — `from "https://esm.sh/marked@15.0.7"` — now compiles, and is
  emitted as written. It was refused on the grounds that a remote origin runs
  with the page's origin, which was true and bought nothing: the alternative
  was a two-line `.js` file importing the same URL, which moved the remote code
  somewhere the compiler could not see it, report it, or ever pin it. A bare
  specifier — `from "marked"` — resolves through a project-level mapping in
  `zd.toml` beside the entry file:

  ```toml
  [packages]
  three   = "https://esm.sh/three@0.180.0"
  slugify = "./vendor/slugify.js"
  ```

  The compiler emits an import map into the head from that mapping, before the
  module script, carrying only the packages the document actually imports. A
  relative target is shipped with the bundle by the same machinery a directly
  written path already used. Nothing is guessed: a bare specifier with no
  mapping is now a compile error naming the file to add it to, replacing the
  old failure mode where it compiled and the page could not load, and one
  specifier mapped twice is refused rather than resolved last-writer-wins.
  Every remote origin the bundle imports — client and endpoint together — is
  listed under `origins` in `manifest.json`, so a deploy target writing a
  Content-Security-Policy and a reader auditing what the page talks to can both
  enumerate it without running the compiler.

  What is *not* allowed is unchanged and now enforced earlier. A specifier that
  names a file — `./x.js`, `../x.js`, and a `[packages]` target of the same
  shape — is bounded by the rule `use` is: it must resolve inside the project
  directory, checked on the resolved path so that both a `..` and a symbolic
  link planted inside the project are refused. `zd.toml` is a second place a
  path can be written, so it is checked too rather than being the way round.
  Every other scheme — `data:`, `file:`, `npm:`, a protocol-relative
  `//host/x.js` — stays refused, because none of them names a place a browser
  fetches a module from. Nothing is fetched at build time: `zdc` never resolves,
  downloads or executes a URL, it writes it into the emitted `import` and
  reports its origin. (#238)
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

- **`set key to value in table` records the write instead of copying the
  table.** A map write is a link onto the map it was given, flattened to a
  real `Map` the first time anything reads it — what `append` has always done
  for a list. A fold that writes a map and reads it at the end is linear
  rather than quadratic: writing ten thousand keys wrote 50,005,000 entries
  into a map to end up holding ten thousand, and now writes 10,000. Every
  builder in the map prelude — `mapOf`, `mapMerge`, `mapRemove`, `mapValues`
  — is that fold.

  The map is still a value: a write is not visible to any earlier version of
  the map, and `keys`, `values` and `mapKeyAt` still report insertion order.

  What did not change is a fold that reads the map *between* its writes, such
  as a visited set. A read flattens, so the next write copies the flattened
  map, which is one copy per write — exactly what the old code did at the
  moment of the write. That shape measures the same before and after, and
  removing it needs a structure with no flatten in it. (#233)
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
