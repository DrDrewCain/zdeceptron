# Changelog

What changed, when, and why it mattered. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

**What is versioned here is `zdc` — the compiler binary and the language it
accepts.** The twenty crates in `crates/` are published to crates.io so that
`cargo install zdc-cli` works — from the first tagged release; nothing is
published yet. They carry the same version because they are one compiler
released together, not twenty libraries with their own lives. Their APIs are internal: depend on `zdc-codegen` and a patch release may
change it under you. The language is the thing with a compatibility promise.

While the major version is `0`, a minor bump may change the language. What that
means in practice is that a program is guaranteed to keep compiling across a
patch release and is not guaranteed to across a minor one — and any minor
release that breaks a program will say so here, with the repair.

## [Unreleased]

### Added

- **A `foreign` can read a property off a handle, hand nothing back, and be
  kept alive in `state`.** The three things #276 named as blocking stage 3 of
  #271, which is a real library driven from the language with no hand-written
  JavaScript.

  `of Handle as "domElement"` reads a **property** — the minimal pair with
  #276's `on Handle as "m"`, and the pair is the design: `on` a host object is
  something you do to it and emits `x.m(…)`, `of` is something it has and
  emits `x.p` with no argument list at all, because `renderer.domElement()` is
  a `TypeError` and not a canvas. `of` is already a keyword in two other jobs,
  so the form costs nothing against §14G.7.7's budget, and a property takes
  only its receiver — a second parameter is refused at the declaration rather
  than dropped at emission.

  `gives nothing` says **no ZDeceptron value comes back**, which is the claim
  `gives view` already makes and is about this program rather than about
  JavaScript: `scene.add(mesh)` returns the object for chaining and this
  program takes none of it. A call to one has a type nothing accepts, so it
  can only be written as the new `do` statement — which is therefore not a
  discard. Every other statement form consumes a value, which is why a call
  made for its effect had no position in the grammar before this. `nothing`
  and `do` are soft keywords and cost no reserved word.

  **A handle may now live in `client` state declared `starting`.** #276
  refused `state` outright, and its reason — a derived signal recomputes,
  nothing releases the value it replaces, so a WebGL context would be dropped
  on every update — is an argument about *replacement*, not about storage. So
  the rule is the argument's own shape: `client`, `starting` rather than
  `from`, and never written. `E0317` refuses each separately. What that buys
  is a lifetime the language can state — the document's: acquired once when
  the bundle loads, released when the page is. Releasing one sooner is a call
  the program makes, not an obligation the compiler enforces; a `destroy`
  obligation on the type was rejected because the compiler would have to know
  which method disposes of which host object, and `renderer.dispose()` does
  not release the canvas.

  **Information flow is unchanged and that is the point.** A property's
  receiver is its first parameter and `Walk::foreign` joins every parameter's
  label into the result, so a property read carries the handle's label without
  a rule being added for it — and `do` walks its call rather than skipping it,
  because every rule that catches a secret argument fires while the arguments
  are walked. Both are asserted against a deliberately broken build: with the
  join removed, and with the `do` arm emptied, the fixtures compile with zero
  diagnostics and the tests fail.

  [`examples/tree-webgl/`](examples/tree-webgl/) is the acceptance test: the
  revolving tree again, this time in real WebGL through three.js, with no
  hand-written `.js` anywhere in it. It reaches 9,841 branches where
  [`examples/tree/`](examples/tree/)'s CSS 3D version is capped at 364, and
  the `mount`/`update` split the deleted `draw.js` kept by hand is now the
  difference between `starting` and `from`. (#271)
- **`aria-*`, as eleven named arguments.** An argument name is a UAX#31
  identifier (§16.3.6), which admits no hyphen, so `aria-selected` was not
  merely absent from the closed argument set — it was **unspellable**, and
  `widgets/README.md` records what that cost: this language could express the
  structure and the state of every widget in #241's list and the ARIA half of
  none of them. `role` alone is often worse than nothing (a `role="tab"` with
  no `aria-selected` names a control a tab and never says which is chosen), so
  where the pair could not be completed no role was written at all.
  `selected`, `expanded`, `pressed`, `checked`, `disabled` and `decorative`
  take a `Truth`; `controls`, `describedBy` and `labelledBy` take an `id`;
  `current` and `live` take one word from a closed set. `label` became global
  at the same time and reaches `aria-label` on anything with no text beside it
  to wrap. A table of names that translate, the way `expansion` already stands
  for `title` and `decoration is "struck"` for `line-through` — not an `aria`
  argument taking a record, which would be an open attribute set arriving as a
  value nothing can check the spelling of. No new syntax and no reserved word.

  **An ARIA state is not a boolean attribute**, and that is the one thing in
  it that is not a rename. `setAttribute` implements HTML's booleans — `false`
  removes the attribute — while `aria-selected` is an *enumerated* attribute
  whose values are the words `true` and `false`. A tab strip whose closed tabs
  simply lack the attribute renders identically and is announced as one with
  nothing chosen, so the literal is baked as the word and a bound getter is
  wrapped where it is bound. `crates/zdc-cli/tests/browser.rs` asks a real
  browser for both halves. The `disabled` style prefix now selects
  `[aria-disabled="true"]` as well as `:disabled`, which matched nothing this
  language could write. (#241)
- **`widgets/toggle.zd` — a `Switch` and a `ToggleButton`.** A button plus one
  bound ARIA state each, and the only widgets in that directory that give up
  nothing: a `button` is already focusable, in the tab order and operated by
  Enter and Space, so the only thing missing was a way to say what state it is
  in. `widgets/tabs.zd` is now a real ARIA tablist, `breadcrumbs.zd` and
  `pagination.zd` name their landmarks and mark the current position, and
  pagination's Previous on the first page is present and announced unavailable
  rather than absent. (#241)
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
- **`zdc doc`**, which writes a program's own declarations out as Markdown —
  one page per source file, plus an overview whose first table is the whole
  deployment shape: every signal, where it lives, and what a read of it from
  the browser costs. The last column is the one no other language's generator
  can have: it is `read_kind` itself answering, so a row cannot claim `Text`
  where the checker says `Remote of Text`. The derived endpoints are listed
  with the files they are emitted to, because nobody wrote them down. The
  sentences are shared with the language server rather than copied, so a
  hover and a page cannot disagree. `zdc doc --prelude` documents the
  standard library, which until now could only be read by opening its eight
  files. (#170)
- **`zdc fmt` — one canonical layout.** `zdc fmt <files>` rewrites in place;
  `zdc fmt --check <files>` writes nothing and exits non-zero if anything
  would change, which is what CI runs. Indentation is the block structure
  here, so a formatter is not a cosmetic tool: a line at the wrong depth is
  a different program.

  It works on the **source text**, not on the syntax tree, and that is
  forced rather than chosen. Comments are `logos::skip`ped in the lexer, so
  they never reach a token, let alone a tree — `zdc-ast` has no comment node
  — and a formatter that printed the tree back out would delete every
  comment in the repository. Only the whitespace at the front and the end of
  a line ever changes.

  Canonical: four spaces a level; no trailing whitespace; exactly one line
  break at the end of the file; no leading blank line and no run of two; a
  comment at the indentation of the line it introduces; a `"""` block's
  closing delimiter one level inside the line that opens it, with the
  interior carried along so the value cannot change. Deliberately untouched:
  the spacing *within* a line, so the aligned `is` columns the examples use
  survive. A file the compiler will not parse is refused rather than
  guessed at.

  **Two things it will not do, said here rather than discovered.** It cannot
  repair a first line that is indented — the lexer refuses such a file
  outright, so there is no block structure to lay out, and dedenting it
  would be the formatter having an opinion about what the author meant. And
  it refuses one program that is perfectly legal: a second `"""` literal
  opened on the line that closes the first, whose indentation is part of a
  value and part of the block structure at once. Both are reported with a
  caret and neither rewrites anything.

  Held to two properties over every file in `examples/`: formatting is
  idempotent, and the bundle `zdc build` emits is byte-identical before and
  after — the second is what would catch a formatter that reshaped a block,
  which is invisible in a text diff. (#167)
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
- **The emitted page carries a Content Security Policy.** `default-src 'none'`
  with the seven directives that follow from what the compiler actually emits:
  no `'unsafe-inline'`, no `'unsafe-eval'`, `object-src`, `base-uri` and
  `form-action` refused outright because the element vocabulary cannot reach
  any of them. `script-src 'self'` is only honest because the page no longer
  has an inline script — the two lines that mounted the program moved to a
  `boot.js` it loads — so a build now writes one more file per document, and
  each document is 240 bytes larger. Verified in a browser against a served
  build, not only in the shim. (#146)
- **A routed program marks the link to the page you are on** with
  `aria-current="page"`, which is what tells a screen reader which navigation
  item is current. It is written into the markup rather than computed in the
  browser: the address fold already knows the document's URL and the link's
  destination while it emits. An unrouted program is left alone, because its
  `index.html` may be hosted at any path. (#142)
- **A development build carries assertions a release build does not**, inside
  `// $dev` … `// $end` blocks that `zdc build` strips and `zdc dev` keeps.
  `wire.js` checks that nothing `JSON.stringify` would silently write as `{}`
  survived encoding, which is the family the durable `Map` bug belonged to;
  `list.js` checks that a list's rows are between its anchors in the list's
  order. They cost 2,474 bytes on a program with an `each`, 5,433 on a
  `durable` one, and nothing at all on a reader's download. (#140)

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
- **Reordering a keyed list moves the fewest rows it can.** The reconciler
  computed the new order with a single left-to-right walk that reinserted
  every row it found out of place, so exchanging the second and
  second-to-last of a thousand rows moved 997 of them; it now takes a
  longest increasing subsequence of where the surviving rows already sit and
  moves only the rest. Measured, before → after: 997 → 2 moves at N=1,000
  and 4,997 → 2 at N=5,000, and unchanged on a reversal, which cannot be
  improved. (#207)
- **A program with no list no longer downloads the reconciler.** `each`,
  `eachInto` and the key function moved from `runtime/dom.js` to
  `runtime/list.js`, which a bundle links only when the program has an
  `each` — the split `runtime/foreign.js` and `runtime/markup.js` already
  use. A page without a list ships 4,455 fewer bytes. (#207)
- **A handler that throws is contained to that handler and reported.** It used
  to do whatever JavaScript happened to do. Now the page keeps running, the
  writes the handler made before it threw stand, and the exception goes to
  `reportError` — the platform's own uncaught-error channel, so an error
  monitor already on the page sees it unchanged. Killing the runtime and
  rolling the handler's writes back were both considered and are argued
  against in [the reference](docs/reference.md). (#139)
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
