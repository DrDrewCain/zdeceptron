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

## [0.2.0] — 2026-08-18

### Changed

- **A scheduled cell is Untrusted.** `Writers::of`'s clock conjunct now
  covers a schedule too. Without it, G-SIG clause 2 read the cell as holding
  its resting `0` — a literal, and so Trusted — which gave a platform
  timestamp the authority of a constant. No new grant: the closed set is
  still eight, and default-closed gives the right answer once clause 2's
  premise is false.

  **This can refuse a program that `0.1.1` accepted**, and it is the only
  change here that can. A scheduled cell's declaration carries a resting
  `0` so that every pass sees an expression rather than a hole, and clause
  2 read that literal as the cell's value — so a beat came out Trusted on
  the strength of a number nothing ever reads. It is Untrusted now, and a
  program that let one reach a place requiring Trusted stops building.

  **The repair is not a grant, and there is no grant to reach for.** The
  cadence is as trusted as the program text it was generated from, but the
  time is not the cadence: it is the platform's reading of a clock, and
  `clock` is admitted by §21.9 only behind a `gives pure` marker precisely
  so that a reading cannot launder itself into evidence. The set did not
  grow for this and is still closed at eight. So a program that required
  trust of a beat was relying on the literal, and the repair is to stop
  requiring it — `crates/zdc-graph/tests/integrity.rs`'s
  `a_beat_is_untrusted_and_needs_no_new_grant` is the worked case.

- **`h` and `d` are readable duration units**, and a browser timer refuses
  them by naming the construct that owns them instead of reporting that `d`
  is not a unit. An hour keeps one spelling on each side of the word:
  `"60m"` to the clock, `"1h"` to a schedule.

- **`zdc explain E0322` stops saying the scheduled construct is unbuilt** and
  shows it instead. What it still refuses on a `server` declaration is
  `after`: a delay needs a moment to count from, and a serverless invocation
  starts when a request arrives.

- **`inbound` is a soft keyword, refused by name** — `E0108`, a new parse
  code for a declaration that names a construct the language has designed and
  not built. A webhook handler used to be told that its `state` declaration
  needed a value. `zdc explain E0107` carries what is missing: `REL-PLACE′`
  forbidding a `release` from an unauthenticated root, the `pc_i = Untrusted`
  seed at one, and at-least-once redelivery with no uniqueness constraint to
  make a double-append safe.


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

### Added

- **Source maps, for statements** (#6). `zdc build` writes `client.js.map`
  beside the bundle and the emission names it. A minified bundle carries
  neither the map nor the trailer: minifying reflows the text, so every
  mapping would name a line that has moved, and a map pointing at the wrong
  line is worse than none. `zdc dev` serves the unminified emission with its
  map, which is where a person debugs.
- **A stylesheet's name carries a content hash** (#137), and the emitted
  cache headers mark exactly those names `immutable`. The name in the
  document and the name on disk are one string, settled once the bytes are.
- **A routed site ships one base stylesheet, not one per page** (#136). Each
  document links the shared `base.css` first and its own generated rules
  second, which is the cascade the single file used to have between its two
  halves, restated as document order.
- **`record … unique` — identity keys for lists** (#2). A list keyed on the
  field its record declares moves a node rather than rebuilding it: the
  reconciler's swap went from six DOM operations to two, and clearing a
  1,000-row list from 2,986 to 1.
- **`bothOf` — two `Remote of T`s as one** (#20). `Pair of A to B` is what it
  was waiting for. `Loading` wins over `Failed` and `Failed` over `Ready`,
  and the rule is applied rather than stated: the `Failed` arm asks the other
  half too, so the answer does not depend on which remote was written first.
- **The wire format carries a version** (#144), and a server reading another
  refuses by name before decoding a body rather than after a decode goes
  wrong.
- **`every` on a `server` declaration is a job the deployment runs on a
  schedule** — §14G.4, issue #18. The same word as the browser's clock, in
  the same slot, selected by the placement on the left, because moving a job
  from the browser to the deployment should be a one-word edit rather than a
  different construct.

  ```zd
  state hourly is server Whole every "1h"
      add 1 to visits
  ```

  The block is required and is a real server root at `(Server, Trigger)` —
  the second root kind, which `zdc-graph` had declared, tested and left
  without a producer since it was written. Its statements are typechecked in
  the trigger-rooted read table, its writes are ordinary store writes rather
  than commands, and the information-flow write rule applies to them:
  appending a secret to a public durable list from a job is `E-IFC-03`, which
  is §14G.4 revision 4's own worked example being refused.

  The cell holds the beat's **scheduled** start time in seconds since 1970,
  so a beat the platform ran late still reports when it was due and a skipped
  beat shows as a jump larger than the cadence.

  A cadence is one of nineteen — the durations that divide their unit,
  `"1m"` through `"30m"`, `"1h"` through `"12h"`, and `"1d"`. That is not
  tidiness: a cron expression cannot say "every seven minutes", because
  `*/7` steps 0, 7, … 56 and then jumps back, while AWS's `rate(7 minutes)`
  genuinely can — so accepting `"7m"` would mean one program meaning two
  things depending on `--target`. One cadence has one spelling, so `"60m"` is
  an error naming `"1h"`.

- **`zdc deploy --target cloudflare` schedules it**, with `[triggers] crons`
  in `wrangler.toml` and a `scheduled()` export that matches on
  `controller.cron` and passes `controller.scheduledTime`. **The other three
  targets refuse a program with a job**, each naming the platform fact that
  stops it rather than a note about effort: Lambda's entry is
  `streamifyResponse`-shaped for an HTTP request and a scheduled invocation
  is a bare event; a Vercel cron *is* an HTTP request to a route, so the job
  would need a public URL guarded only by a `CRON_SECRET` this router does
  not check, and on Hobby it is additionally once a day; `Deno.cron` is not
  available on the platform that adapter targets. A job written out and never
  scheduled is a failure nothing later reports.

- **`examples/schedule.zd`**, and `docs/reference.md` gains *The schedule*.


- **`zdc build --report` writes `dist/report.json`: every claim about
  JavaScript the program's integrity rests on, and nothing that says whether
  the program is safe.** Residual risk R6 (#31).

  Two of the eight ways a value becomes Trusted are asserted rather than
  checked: `gives pure T` says a foreign's result is a function of its
  arguments, and `gives trusted T` says it is not attacker-chosen whatever
  went in. Both are a human's word about a module the compiler cannot read,
  and §21.7's soundness argument leans on them — so the design specified a
  report that would let a reviewer find them, and the report was never built.
  A reader of a bundle had one route to the assertion holding their program
  up, which was to read the source knowing what to look for.

  The file lists, per assertion: the declaration and its line, the module and
  export it imports, every call site, and every `release` whose body reaches
  it. That last list is the useful one — a release is where a program
  declassifies, so an entry there says *this unchecked claim is what lets that
  release compile*. Every `trusted p` clause is listed too, which makes true a
  sentence E-REL-08's help text had been shipping since before the flag
  existed. The prelude's twenty-seven purity grants are named rather than
  located, because a prelude file is parsed on its own and a line number
  resolved against the wrong file is worse than none.

  **There is no `attackerReachable` field**, and there is not going to be one.
  The design specified it and §21.8 withdrew it twice over. The reason worth
  repeating is that giving a purity grant an argument chain — which is what
  #31 asked for — would not help: the channel is inside the JavaScript, not in
  the argument list. A query-string reader takes a string literal and reads
  `location.search`, so a walk over its arguments answers "no
  attacker-controlled value reaches this" about the exact grant a visitor
  steers with a query string. An available, cheap, false answer is worse than
  none. The report says which assertions exist and which releases rest on
  them, and its own `notClaimed` array says the rest.

- **`build parts` — a post can name a component, and the widget set is the
  program's to declare.** #305, and the one thing the MDX pipeline did that
  this language could not.

  A post used to be one `Markup`, rendered into one `Prose`. `Prose` has no
  children and cannot grow them: interleaving parsed nodes with templated ones
  would make the sibling offsets every binding is scheduled against depend on
  how many nodes a *file* parsed into, which is not known at compile time. So
  a document that wants an interactive chart in the middle of it is not one
  node. It is a **list**, and `build parts` is what makes one — a `List of
  Part`, each part either a run of prose or a named widget, rendered by an
  ordinary `each`. Each part is its own node, so no parsed subtree ever shares
  a parent with a templated one and the offset problem never arises.

  A file names a widget with a fence whose info string begins with `zd`; every
  other fence stays a code block, and every run of prose goes through the same
  rewriting pass `build markdown` already did, so a `<script>` in a post is
  still shown rather than run. **No markdown parser ships**, exactly as before.

  **The widget set is closed and the program declares it**, as a `choice`
  named `Widget`. A component cannot be resolved from a file's text —
  components are resolved statically and a name out of a `.md` is not a name
  the compiler saw — so a document naming a widget the program does not offer
  is a failed build (`E11`) naming the widget and listing the ones on offer,
  rather than a blank space on a page. That is a stronger bargain than MDX
  makes, where an `import` inside a content file can reach anything on disk.

  `Part` is a prelude record, which costs `Part` as a name every program could
  otherwise have used. `examples/parts.zd` and
  `examples/content/parts/spacetrader-wars.md` are the whole story running.
- **`FileInput` — a file picker, and the smallest honest type for what it
  yields.** Issue #47, whose "done when" is a value with a declared type and
  *the placement rules for that type written down before any upload path is
  built*. Both are here; the upload path is not.

  **What the program gets is an `Option of Text`: the name of the file a
  reader chose.** Not the bytes, not the size, not the media type, not the
  last-modified time, and not a handle onto the file. `state chosen is client
  Option of Text starting None` and `FileInput chosen`, and a `when` over it.

  **Why a name and not a file.** The three larger answers each need a language
  change this element is the wrong place to make. *Bytes* need a `Bytes` type
  — §5.4 makes a `Text` UTF-8 and a PNG is not text — and reading them is
  asynchronous and fallible, so the value is a `Remote of Bytes` and the
  element has acquired a second failure mode beside the one it already has.
  *A handle* is what the browser actually hands a script, and `Handle` is the
  type this language already has for it — and it cannot be used here, for a
  reason the existing rule states rather than one invented for the occasion:
  `E0317` admits a `Handle` in state only in a `client` signal declared
  `starting`, **acquired once and never written**, because nothing runs a
  `destroy` on the object a second write drops. A picker writes its signal
  every time somebody chooses. Widening that to admit a replaceable handle
  would weaken it for the renderers and audio contexts it exists for. *Size
  and type* need a record whose fields the compiler synthesises, and there is
  no built-in record type.

  **The placement rules, which are the rules `Text` already has.** That is
  the whole benefit of the choice above: nothing new crosses a boundary,
  because no file is ever a value. The binding is §14B.5's, unchanged — the
  signal is `client` or `remembered` and `starting`, and `server`, `durable`
  and `static` are refused in the words they already had. A *name* may travel
  to a server, because it is text; the file cannot, because the program never
  held it. And the name is **untrusted**: whoever made the file chose it, and
  nothing was added to the integrity lattice to say so — `Site::Bind` already
  records a two-way binding as a writer, so the cell fails G-SIG's second
  clause exactly as an `Input`'s does.

  **The binding is one and a half directions, and the half is the point.** No
  script may put a file into a file picker: the DOM refuses any assignment to
  `value` but the empty string, which is what stops a page handing itself a
  file the reader never chose. So the read half is the whole binding, and the
  write half is the one write the browser permits — `None` empties the
  control. That is what keeps `set chosen to None` after an upload from
  leaving last week's file named in a picker under a program that believes
  nothing is chosen. ⚠️ The reverse is **not** available: a `Some` the program
  invented names a file the control has never held, and nothing reports it.

  **One file, not several.** No `multiple`. A picker that admits several
  yields a list, and there is no list-valued two-way binding in the language:
  every `Bound` is a scalar, `Select` is one variant, `Radio` is one of a
  group. Adding one means deciding what a reader adding a second file does to
  a list the program has been editing, which is a question about `List`.

  `accept is "image/*"` narrows the dialog. ⚠️ **Advisory, not a guarantee** —
  every browser offers a way past the filter and nothing here validates what
  arrives. No `hint`: `placeholder` does nothing on a file input, as it does
  nothing on a date one, and the accessible name comes from a `Label` with
  `controls`.

- **`Dialog` — a modal, and the whole of it is the accessibility (#53).**

  ```zd
  state confirming is client Truth starting no

  view
      Column
          Button "Delete"
              on click
                  set confirming to yes
          Dialog confirming, label is "Delete this file?"
              Text "This cannot be undone."
              Button "Cancel"
                  on click
                      set confirming to no
  ```

  `widgets/README.md` carried a section called *"`Modal` — not
  expressible"*, with four reasons and one root: nothing a program writes
  in this language moves focus. Every reason was correct. What was wrong
  was the conclusion that the language therefore needed a statement that
  moves focus — because `<dialog>` opened with `showModal()` already has
  all four, specified by HTML and implemented by the browser. Focus moves
  in when it opens; focus is *trapped*, not by a Tab handler but because
  everything outside the top layer is inert, which shuts out the pointer
  and find-in-page too; Escape closes it; and focus **returns to whatever
  opened it**, which is the half a hand-rolled modal forgets. So this is
  one element and one binding rather than a focus statement, a `tabindex`
  argument and a keydown handler that redirects Tab.

  Whether the modal is showing is the `client Truth` it binds, and the
  binding is two-way for a reason that is not symmetry: a close request
  closes a `<dialog>` **without asking the program**, so a signal left
  saying `yes` is a page whose next click does nothing and whose failure
  is reported nowhere. The binding is idempotent against `dialog.open` —
  what the DOM is doing — rather than against the last value written,
  because `showModal()` throws on a dialog that is already open and the
  browser can invalidate a remembered flag at any moment.

  There is no `open` argument and no non-modal dialog: `open` as an
  attribute shows the box with none of the four properties above, under
  markup that looks like it has them. `label` is required, following
  `Image`'s `alt` and `Frame`'s `title`, because a modal moves focus into
  itself and an unnamed one is announced as "dialog".

  The one thing the compiler adds beyond the element is a deferral. Every
  binding runs while the tree is still a clone of a `<template>`, and
  `showModal()` throws on a node that is not in the document, so a dialog
  whose signal *starts* `yes` would have thrown at load and taken module
  evaluation with it — #205's shape. `crates/zdc-cli/tests/browser.rs`
  asks a real browser for all of it, because the embedded engine has no
  focus, no top layer and no `inert` to ask about.

- **`prelude/math.zd` — the transcendental functions, geometry, matrices and
  the beginnings of a numerical toolkit.**

  `number.zd` had `sqrt` and `power` and recorded honestly that `power` for a
  fractional exponent needed "the exponential and the logarithm the language
  does not have". It has them now, and the trigonometric family is the same
  argument again: the platform's `Math.sin` is correctly rounded, and a series
  expansion written in ZDeceptron would be a second answer to a question that
  already has one.

  - constants `pi`, `tau`, `eulerNumber`, and `radians of` / `degrees of`
  - `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`
  - `exp`, `ln`, `log10`, `log2`, `cbrt`, `hypotenuse`, `hyperbolicTangent`
  - vectors: `dot`, `magnitude`, `normalized`, `scaled`, `added`,
    `subtracted`, `distance`, `axis`
  - interpolation and easing: `mix`, `progress`, `clamped`, `clamped01`,
    `smoothStep`, `smootherStep`, `easeIn`, `easeOut`
  - angles and geometry: `wrapAngle`, `angleDelta`, `heading`, `fromAngle`,
    `cross2`, `cross3`, `angleBetween`, `rotated2`, `projected`, `reflected`
  - matrices: `transposed`, `applied`, `matrixProduct`, `matrixScaled`,
    `matrixAdded`, `rowOf`, `columnOf`, `rowCount`, `columnCount`
  - statistics and activations: `mean`, `variance`, `standardDeviation`,
    `sigmoid`, `rectified`, `leakyRectified`, `softmax`

  **Every primitive gives an `Option`, under the rule `sqrt` and `power`
  already carried: `None` unless the answer is a finite number.** There is
  deliberately no total variant beside it — that is a second spelling of one
  operation, which §4.1 refuses. The vector, matrix and statistical
  operations are written in ZDeceptron rather than declared as primitives,
  and are total.

  A vector is a `List of Decimal` and a matrix a `List of List of Decimal`.
  There is no `Matrix` type and there should not be one until the language
  has a shape to check: a type that cannot say "n by m" is a rename of the
  list, and a rename is not a guarantee. Nothing checks conformability, so a
  ragged input gives a ragged answer.

  `softmax` shifts by the largest term before exponentiating. The unshifted
  form is one line shorter and gives `NaN` for inputs a real network reaches.

- **`from scroll` — where the reader is, as a signal.** §10 said `resize`,
  `scroll` and `pointermove` "have no form at all: they are not events but
  quantities, and want a different construct". This is that construct for
  the first of the three.

  ```zd
  state travelled is client Decimal from scroll
  ```

  A `Decimal` from 0 to 100, written by the browser, carrying the clock's
  four rules: `client` only (`E0362`), nothing may write it, it is
  Untrusted, and it is disposed with its view. One `passive` listener per
  program however many times it is read, coalesced to the animation frame —
  a scroll fires far faster than a repaint, and a write per event schedules
  work the compositor throws away.

  A percentage and not a pixel offset, because an offset means nothing
  without the document height and the language exposes no way to read one.

- **`build markdown` renders GitHub-flavoured CommonMark.** Footnotes alone
  was the whole option set, so a table rendered as pipes, `~~a~~` as
  tildes, and a task list as brackets. Tables, strikethrough and task lists
  are on now, matching the `remark-gfm` that real markdown is written
  against.

- **A `static` signal may hold a `Map`.** A lookup table computed once from
  a file at build time is the most `static` thing a program has, and it was
  the one value the placement could not hold: the build host wrote its
  answers as JSON, `JSON.stringify` turns a `Map` into `{}`, so a `Map` was
  refused rather than inlined as an empty table.

  ```zd
  state rates is static Map of Text to Decimal from ratesFrom of (
      build read "data/rates.csv")
  ```

  The build host is asked for a JavaScript expression now rather than for
  JSON, so the table inlines as `new Map([…])` — the same form this
  compiler already emits everywhere else. Nothing that is not a map changes
  by a byte. A value that genuinely has no literal form — a function, an
  absent value — is still refused, in the same words.

- **An asset stylesheet is linked from the root.** `./assets/site.css`
  resolves against the *document's* directory, so it was correct only for a
  document at the root; a routed program's `/writing/<slug>/index.html`
  asked for `/writing/<slug>/assets/site.css` and rendered unstyled with
  nothing saying why. The generated sheet beside it was already
  `/pages/….css`.

### Fixed

- **A shown `Truth` reads `yes` or `no`, not `true` or `false`** (#297).

  ```zd
  state flag is client Truth starting yes

  view
      Column
          Text flag
  ```

  rendered `true`, which is not a word in this language. `text of` a
  `Truth` has given `yes`/`no` since the prelude's primitive layer landed,
  so `Text (text of flag)` and `Text flag` — the same value into the same
  text node — disagreed about the same conversion. They agree now, and
  `Text yes` writes `yes` into the markup rather than computing it.

  Two things deliberately do not change. The `true`/`false` an ARIA state
  argument carries is ARIA's own vocabulary and stays: a token outside its
  enumeration is *mapped* onto `true` rather than ignored, so a tab
  announcing `aria-selected="yes"` would announce itself selected.
  And a page wanting other words still chooses them itself, with `if` in
  the view, which is what `examples/preferences.zd` already does.

  Nothing was added to the shipped runtime for this: the conversion is a
  preamble helper the emission already had, so a program with no `Truth`
  in its view carries no extra byte.

## [0.1.1] — 2026-08-12
### Added

- **A label on a `choice` variant.** `Select` rendered a variant's identifier
  as its option text, so a dropdown over `DirtBike`/`LawnMower` read
  `DirtBike` and `LawnMower` and there was no way to say otherwise — a
  variant's name is an identifier and cannot hold a space.

  ```zd
  choice Equipment
      DirtBike  is "Dirt Bike"
      ATV
      LawnMower is "Lawn Mower"
  ```

  The label is the option's **text**; the option's **value** is still the
  variant's name, because that is what the runtime round-trips on the way
  back. Two variants may therefore share a label and stay distinct, and a
  label may repeat another variant's name without colliding. Nothing inside
  the program can read one: `when` dispatches on the variant, and an arm
  written with the label does not parse. An arm with no label shows its name,
  so this changes nothing about a `choice` that does not ask for it.

  `Name is "text"` is deliberately the same shape as a `route`'s
  `Home is "/"`, because it is the same idea — the string a variant is known
  by outside the program. A `route`'s variants take no label; the string
  after `is` is already spoken for, and it is the URL.

  Found by recreating a real commercial site, where the equipment dropdown
  was the one thing that could not be said.


### Changed

- **`examples/edit-distance.zd`'s `distance` is now `editDistance`.** The
  prelude gained `distance` (the metric one, over two vectors), and a
  prelude name and a program name may not be the same. This is the
  compatibility cost of adding to the prelude, and it is recorded here
  rather than absorbed silently: **any program with a top-level `distance`,
  `mean`, `variance`, `mix`, `applied`, `heading` or any other new name above
  will need to rename it.** That is a language change in a patch release,
  which the versioning note at the top of this file says should be a minor
  one; it is called 0.1.1 because 0.1.0 is eight days old and nothing depends
  on it yet, and calling it otherwise would be ceremony rather than honesty.
## [0.1.0] — 2026-08-11

### Added

- **Source maps: a browser stack trace now names the `.zd` line.** #6, the
  last unlanded item of milestone M5b.

  `zdc build` writes `client.js.map` beside `client.js`, and
  `pages/<slug>.js.map` beside each routed module, with a
  `//# sourceMappingURL` line in the bundle naming it. `client.js:198:5`
  resolves to `edit-distance.zd:94:5`. The page's Content-Security-Policy
  needs no exception: the policy governs what the *document* loads, and a map
  is fetched by devtools rather than by the page.

  **What the map claims is narrower than "source maps" usually means, and the
  narrowness is the point.** One mapping per emitted *statement*, at that
  statement's first character, inside a top-level `function` or a `state` /
  `derived` declaration. A segment claims every generated position at or
  after it, so a trace at any column inside a statement is answered with that
  statement's own line — which is the granularity the emitter genuinely has.
  Mapping sub-expressions would need the expression emitter to return offsets
  from the fifty-odd sites that compose one, and a column claim it could not
  support would be worse than none: the reader only learns not to trust it
  after making the trip.

  So three things are deliberately unmapped, each with the reason recorded
  where the decision is made. **Event handlers and view code**, because a
  handler's body is trimmed, re-indented, wrapped in an arrow and
  interpolated into a binding, and none of that carries an offset. **The
  prelude**, because §17.4.1 resolves the library into the same arenas as the
  program but its spans index the library's own sources — a mark from one
  would point at an arbitrary byte of the user's file. **Server functions**,
  because a server stack trace happens on a host that has the `.zd` file, and
  neither Deno nor Node reads a `//#` comment unless asked.

  **A released map names the source and does not carry it.** `sourcesContent`
  is what lets devtools show the line rather than only naming it, and it works
  by putting the program's text in the map — which sits at a guessable public
  URL once deployed. Publishing a program's source is an author's decision and
  not a compiler's, so `zdc build` and `zdc deploy` omit it and `zdc dev`
  embeds it. The dev server has to: the `.zd` file is outside the served root,
  so devtools would otherwise show a mapped position with no text under it,
  and that bundle never leaves the machine that built it.

  **Where it is counted.** The `//# sourceMappingURL` line is 35 bytes of
  `client.js`, and `BENCHMARKS.md`'s size tables now carry it, because it is
  downloaded. The `.map` beside it is not in any of those columns and should
  not be — a browser fetches it only when devtools is open. What it costs on
  disk is stated instead: 98 bytes for `counter.zd`, 748 for
  `edit-distance.zd`.

  **Every assertion decodes.** `crates/zdc-codegen/tests/sourcemap.rs` carries
  its own base64 VLQ decoder and resolves each segment back to a real line of
  both files. A sign bit read as a continuation, a source index reset per line
  instead of carried, a column counted in bytes rather than UTF-16 units:
  none of those is visible in the string and all of them point the reader at
  the wrong line.

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
