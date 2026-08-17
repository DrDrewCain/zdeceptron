# Changelog

What changed, when, and why it mattered. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

**What is versioned here is `zdc` — the compiler binary and the language it
accepts.** Twenty of the twenty-one crates in `crates/` are published to
crates.io so that `cargo install zdc-cli` works; `zdc-wasm` is the exception
and its manifest says why. They carry the same version because they are one
compiler released together, not twenty libraries with their own lives. Their
APIs are internal: depend on `zdc-codegen` and a patch release may change it
under you. The language is the thing with a compatibility promise.

While the major version is `0`, a minor bump may change the language. What that
means in practice is that a program is guaranteed to keep compiling across a
patch release and is not guaranteed to across a minor one — and any minor
release that breaks a program will say so here, with the repair.

## [Unreleased]

### Changed

- **`zdc build` and `zdc deploy` write a minified bundle** (#135). `counter.zd` shipped 27,022
  bytes and now ships **10,717** — 16,305 bytes, 60% of the page, taken off a first visit. The
  runtime loses about 70% of itself, because that is where this repository keeps its prose.

  **What "minify" means here is deliberately small: comments and redundant whitespace, in
  JavaScript and in CSS, and nothing else.** No identifier is renamed, no expression is
  rewritten, no statement is removed, and no two tokens are ever joined. Renaming would need a
  real JavaScript parser and a correct scope analysis, and the runtime is full of the case that
  decides it — property names spelled like identifiers, which must not be renamed. A renamer
  that gets one wrong emits a bundle that *parses and misbehaves*, which is the one defect a
  size gate would report as a success. `crates/zdc-runtime/src/minify.rs` records that as a
  decision rather than leaving it as an omission.

  Nothing is spawned to do it. `zdc` is still one binary, for the reason
  `crates/zdc-codegen/src/evaluate.rs` gives at length.

  **What you will notice.** A built `client.js`, `styles.css`, `boot.js` and the runtime files
  under `runtime/` are no longer formatted, and the `// zdc … generated, do not edit` header is
  gone from them — it is a comment, and a minified bundle is not a file anyone edits by hand.
  `zdc dev` is unchanged and still serves the compiler's own formatted emission with its
  `// $dev` assertions intact, so a debugger still shows readable code. `index.html`,
  `manifest.json` and server functions are not minified; `minify.rs` gives a reason for each.

  The size gate records both sides. `BENCHMARKS.md` now reports emitted and shipped bytes per
  file, so the saving is a measurement rather than a claim, and so that a change which grows
  the emission is still visible after the minifier has been over it.

## [0.1.0] — 2026-08-11

### Added

- **`on key "Escape"` — a keystroke the whole page hears, and a capability
  narrowed at its source rather than labelled after the fact.** Part of #19,
  and the half PR #283 named as unfinished when it landed `every`/`after`/
  `frame`.

  `on keydown` bound to an `Input` hears what that field was sent. A
  *document* listener hears keystrokes aimed at every element on the page,
  including a field the program never declared, and that is a strictly larger
  capability than anything in the language before it. The question was whether
  the payload needed a secrecy label of its own. **It does not, and it must
  not be given one:** `secret` in this language means *secret from the
  visitor* — it is the label that stops a value reaching the browser — and a
  keystroke originates in the browser. Labelling it `Secret` would forbid the
  one place the value is already known and protect nobody. The lattice has no
  point for "the program should not learn this about its visitor", so the
  capability is removed instead of described.

  Two removals, both enforced rather than documented. **The production has no
  `with`**, so `stroke.key` is not a thing a program can write: a handler
  learns one bit about one key spelled out in its own source. And **the
  emitted listener stands down while an `input`, `textarea`, `select` or
  `contenteditable` has focus**, which is what makes a printable key safe —
  `on key "r"` cannot see the `r` in a password. Together: a handler learns
  only that the key it named itself was pressed while nobody was typing.

  **Its position is its lifetime, and only that.** It is a view node, refused
  indented under an element — `on key` under a `Button` reads as "while this
  button has focus" and does not mean it. Written inside `if open`, the
  listener is added when the branch appears and **removed** when it goes;
  `dom.js`'s `on` never detaches, and is right not to, because the node it is
  attached to is what gets removed. A document listener has no such node, and
  one left behind keeps firing into a graph nothing renders. Two open/close
  cycles in Chrome: `added:2, removed:2, live:0`.

  `key` is a **soft** keyword, meaningful only in the slot after `on`, so
  `key is Text` still parses and §14G.7.7's budget is untouched — the same
  objection that made `unique` a hard keyword does not apply to a word that is
  never in leading position. The key literal is checked against
  `KeyboardEvent.key`'s own spellings, because `on key "Esc"` is a listener
  that can never run and a browser reports that as silence. `runtime/keys.js`
  is linked only by a program that writes one, and imports `signal.js` alone,
  so the null program still links exactly two files and ships 22,046 bytes.

  E0364 refuses a document listener in a region with no browser.
  `Region::has_a_document` states which regions have one **positively**, so a
  region added later has to answer rather than inherit permission; the
  diagnostic site itself is defence in depth today, because the splitter walks
  the view from `Ctx::CLIENT_VIEW` and from nowhere else, and that is recorded
  in `inline_budget.rs`'s `UNREACHABLE` table rather than covered by a fixture
  that only pretends to reach it.

  **Not delivered, and said plainly.** Modifier chords (`control k`) and
  `keyup`. `resize` ×4, `scroll` ×2 and `pointermove` ×3 from the same survey:
  those are not events but *quantities*, and want the shape `every` gave a
  clock rather than the shape a keystroke has. `IntersectionObserver` ×2 —
  unstarted; an observer watches a node, so it wants `on` on a view node,
  which is a different shape again. And `preventDefault` remains absent, so
  the arrow keys still scroll the page under a game.
- **A program can make an outbound HTTP request, and a `secret` cannot ride
  out on one.** #19's last unaddressed part.

  ```zd
  request quote is client
      from  "/quote.txt"
      with  topic is subject
      gives Text
  ```

  The declaration **is** a signal: reading `quote` gives `Remote of Text`, so
  it is spent with the three-armed `when` §5 already requires, and it re-runs
  when one of its `with` arguments changes. Nothing downstream learned a new
  kind of definition — `request` lowers to a `client` signal whose initialiser
  is the one expression in the language that leaves the machine.

  **The destination is written down.** `from` takes a quoted URL and nothing
  else: a name, a concatenation or a call in that position is a parse error
  naming the reason. A computed destination could not be checked by any pass
  and could not be named in a Content-Security-Policy — and
  `fetch("https://x/?k=" + apiKey)` is a leak with no body at all.

  **The arguments are the query string, and that is where the flow pass rules
  on them.** §14G.1.3(c)'s sink 7 — `Sink::OutboundRequest`, which existed for
  the URL-bearing attributes a browser dereferences — gains a second producing
  site rather than the list gaining an eighth member: a mechanism is not a
  medium, and both of these are one medium, an HTTP request leaving the
  browser for a host the program named. A `secret` in any argument is
  `E-IFC-11`. There is no header clause and no body: a request is a `GET`
  carrying one `accept` header the runtime chose, so `Authorization: Bearer
  <secret>` has no syntax at all.

  **A cross-origin destination widens the emitted policy, and nothing else
  does.** `connect-src 'self'` was true because a program could not name a
  host; it now reads `connect-src 'self' https://api.example.org`, taken from
  the `from` line. Not `https:`, which would permit every host on the web. A
  program with no cross-origin request emits the policy it emitted before,
  byte for byte.

  **What comes back is Untrusted.** The integrity lattice is default-closed
  and no grant describes an answer a host gave, so a response body cannot
  reach a place declared `trusted`.

  `is client` is the only placement. A request the *deployment* sends would
  spend its own credentials and its position inside a private network, which
  is a different medium with a different reader and would need a sink of its
  own; `E0363` refuses it rather than quietly giving it the browser's rules.
  `runtime/request.js` is linked only by a program that declares one.
- **A pipeline can accumulate, and what is inside an `Option` or a `Remote`
  can be transformed** — #33, and the design half of #103 and #104. Two
  binder forms, and neither makes a function a value.

  ```zd
  function revenue of rows
      from rows
      keep each row where row.active
      fold each row into total starting 0 to total + row.amount
  ```

  ```zd
  state doubled is client Option of Whole from map each n in chosen to n * 2
  ```

  **The trade is that a lambda is syntax rather than a value.** The body of
  each form is written where it is used, so nothing is passed anywhere: a
  call inside one still resolves to a top-level name at compile time and the
  call graph stays exact, which is what reactivity, the placement split and
  the information-flow pass all depend on. Real function values were the
  alternative and were rejected — they would have made `Type::Function`
  inhabitable, and a function value that crosses a placement boundary is not
  serialisable, so it would have needed the whole of `Handle`'s `E0317`
  treatment for two library functions' worth of gain.

  `fold each` ends its pipeline: it gives one value rather than a sequence,
  so a clause after it is refused by name rather than emitting `.filter`
  against a number. A fold over an empty list is the seed. `map each … in`
  passes `None`, `Loading` and `Failed` through untouched, which is the thing
  `readyOr` cannot do and is flagged in `prelude/remote.zd` for not doing; a
  `List` is refused there and the refusal names the pipeline, because one
  construct may not have two spellings.

  **Cost against §14G.7.7's reserved-word budget: zero.** `fold` and `into`
  are soft keywords, so `function fold of xs` and a field called `into` still
  compile; `map`, `each`, `in`, `to` and `starting` were already keywords.

  **Information flow.** A binder carries the label of what it walks, and both
  forms carry more than that. A fold's answer depends on *how many* elements
  there were, so the list's `shape` flows into it — otherwise `keep each row
  where <secret predicate>` followed by a count would hand the predicate back
  as a number. `map each x in v to e` keeps the container's tag, so the
  result is at least as secret as whether there was anything there — the
  alternative would let `map each x in secret to 0` come out public while
  still leaking one bit per read. `crates/zdc-graph/tests/flow.rs` fails on
  both if either join is dropped.

  `prelude/list.zd`'s `sumOf`, `countOf`, `minOf`, `maxOf` and `flatten` are
  now one pipeline each and five hand-threaded helper functions are gone;
  `anyOf`, `allOf` and `listContains` are not, because they stop early and a
  clause that visits every element cannot. `examples/sorting.zd` folds a
  record — the sorted list and the comparison count together — and
  `examples/poker.zd` folds two. `flattenOption` and `flattenRemote` join the
  prelude, which is what makes the pair of them `andThen`.

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
- **The clock: `every "250ms"`, `every frame` and `after "2s"`.** A browser
  timer, an animation frame loop and a delay, none of which is a callback.
  **That is the whole design, and it is the reason this took a construct
  rather than a `foreign`:** the language's claim is that state is
  declarative and there is no callback, and a `setInterval` that takes a
  function would have reintroduced an imperative escape hatch at the one
  place where the language cannot see what happens next. So the clause takes
  no block and runs nothing — it declares a **source signal whose writer is
  the browser's scheduler rather than a handler**, and everything downstream
  is the `from` and the bindings the language already had. A clock is a text
  box that types by itself. `every` holds the milliseconds elapsed, `after`
  holds `no` and then `yes`; nothing in the program may write either, so a
  tick cannot start a request, append to a list or reach the store, and a
  program that wants an animation to *cause* something says so with `from`
  where it is visible. `client` only — `E0322`, which for `server` and
  `durable` says the honest thing: what those ask for is a *scheduled* state,
  a construct the language has sketched and not built, rather than "timers
  are client-only". `remembered` is refused on its own ground, because it is
  the one non-`client` placement that *is* on the browser and could therefore
  tick: what it would keep is an elapsed time measured from a visit that has
  already ended. A clock reading is **Untrusted**, the same verdict the
  prelude's `clock` gets and for the same reason: a visitor controls their own
  clock. `every`, `after` and `frame` are soft keywords, so they cost nothing
  against the reserved-word budget and a record may still have a field called
  `frame`. `runtime/clock.js` is linked only by programs that reach it, for
  the reason `list.js` and `markup.js` are: the null-program size gate keeps a
  two-kilobyte reserve, and a clock in `signal.js` would be paid for by every
  program forever. Disposal is proved against a scheduler the test suite
  controls — a timer that outlives its view is a leak with no symptom — and
  `examples/timers.zd` is loaded in a real browser by
  `a_clock_signal_ticks_in_a_real_browser`. **This moves a line the project
  had drawn deliberately**: `examples/tree/` used to say that "a signal that
  changes sixty times a second is not state, it is an animation". That was
  right while animating meant recomputing a graph sixty times a second; this
  runtime is fine-grained, so a frame reaches exactly the bindings that read
  the frame signal — in `timers.zd`, one attribute write. **Not delivered:**
  document-level `keydown`, `resize`, `scroll` and `pointermove`, which are
  the other half of #19's browser-event surface and have an information-flow
  question of their own; a scheduled `server` state; and a delay that restarts
  when an input changes, which is what a debounce needs. (#19)
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
- **`zdc test`, and a `test` declaration for it to run.** A program can now
  state what it should compute and have that checked: `test "…"` names a
  claim in prose and one indented `expect` line gives the evidence. A claim
  is lowered to the `static Truth` it is, so it is resolved, typechecked and
  placed by the passes that already exist — a claim about a deleted function
  fails to compile, and an expectation that is not a `Truth` is a type error
  rather than a silent pass. `zdc test` compiles the program exactly as
  `zdc build` does and calls the expectations in the module the compiler
  printed, so what a claim is checked against is the code that ships. A
  false claim renders as an ordinary diagnostic (`E-TEST-01`), shows what
  each side of an `is` came to, and exits non-zero; one that cannot be
  decided is reported apart from one that is false (`E-TEST-02`).

  **What it cannot reach.** An expectation is evaluated at `static`
  placement, so it may read pure functions, other `static` state and the
  prelude, and it may **not** read `client`, `server` or `durable` state or
  render a `view`. Pure computation is testable; interaction is not yet.
  Both `test` and `expect` are soft keywords, so no existing program loses
  an identifier. (#169)
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

There are none before this. `0.1.0` is the first tagged release, and this
file starts at the point where the project began recording changes rather than
reconstructing them from the git log after the fact. What happened before is in
[`STATUS.md`](STATUS.md), which is the milestone-by-milestone account, and in
the commit history, which is complete.
