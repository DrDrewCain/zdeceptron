# Status

Where ZDeceptron actually stands, established by running the compiler rather than by reading
prose. Every claim below has a command, a test name, or a file behind it.

**Re-measured on `main` @ `f48eb76`, 2026-08-07.** The figures below were taken on that tree,
not inherited from the branch this file was written on.

`cargo test --workspace --no-fail-fast` passes with **2358 passing, 0 failing, 9 ignored**,
across **20 crates** — re-taken on `feature/zdc-fmt`, which adds the twentieth crate and its
tests. See [§3](#3-tests) for how, why the flag is not optional, and which of the per-crate
rows below moved.

The two branch names this paragraph used to cite — `feature/front-end` and
`feature/algorithm-examples` — were both merged long before it was read again, and it carried
**2041** while [§3](#3-tests) carried **2079**, each measured on a different tree and neither
re-taken. A figure with a branch beside it decays the moment that branch merges; a figure with
a *command* beside it can be re-derived. Both now have the command.

On timing, honestly: test *execution* measured 210 seconds, of which the benchmark suite was
157s. A second run of the benchmark suite alone took 244s. Both runs were on a machine
compiling several other checkouts concurrently, so **treat these as an order of magnitude, not
as a figure** — a few minutes of execution, dominated by the benchmark suite, with a cold
compile costing several times more than that. The benchmark suite is slow because it runs the
workload through an embedded JavaScript interpreter; it is not hung.

Run it in two halves — the whole thing from cold is long enough that it is worth splitting:

```sh
cargo test --workspace --exclude zdc-bench --no-fail-fast   # 2001 passed, 0 failed, 2 ignored
cargo test -p zdc-bench --no-fail-fast                      #   40 passed, 0 failed, 3 ignored
```

**Both halves re-measured on `feature/algorithm-examples`**, which is `7f3b442` plus six
algorithm examples and their tests. The first line used to read 1514 across 94 binaries; it is
2001 across **137** binaries now, and only 20 of the 487 are this branch's. The rest arrived
with merges made after this file was last re-measured, which is worth knowing before the
numbers here are treated as a baseline for anything.

A test binary killed by a signal prints no `test result:` line at all, so the original figures
were taken by running each test binary separately and recording its exit status, rather than by
reading a summary line. A missing result line counts as a failure.

The re-measurement above was taken a cheaper way and it is worth saying which: one
`--no-fail-fast` run per half, with the `test result:` lines counted and `cargo`'s own exit
status checked. **137 result lines came back for the 137 binaries of the first half, and 6 for the second**, so no binary went missing,
which is the property the per-binary run was buying.

---

## 1. Milestones

Milestones M0–M12 are defined in the design spec's §12 table. A milestone is marked **done**
only where there is a passing test, a working command, or a file to point at. A milestone with
no evidence is marked not done, regardless of what any other document says.

| # | Milestone | Verdict | Evidence |
|---|---|---|---|
| **M0** | Repository, workspace, CI, spec | ✅ **done** | **20-crate** Cargo workspace — `zdc-fmt` is the twentieth (#167). `.github/workflows/ci.yml` runs `fmt --check`, `clippy -D warnings`, `zdc fmt --check` over every `.zd` file under `examples/`, and **eight** scripted gates: `check-forbid-unsafe.sh`, `check-wildcard-arms.sh`, `check-vacuous-tests.py`, `check-emitted-strings.sh`, `check-grammar-drift.py`, `check-advisory-exceptions.sh`, `cargo deny`, `cargo audit`, plus `check-dependency-unsafe.sh` via `cargo-geiger`. `cargo test --workspace --no-fail-fast` is a CI step, and two further jobs run what a plain `cargo test` skips: `browser` (a real Chromium) and `mutation` (#160, the runtime mutation sweep). |
| **M1** | Indentation-sensitive lexer + parser + AST, snapshot tests | ✅ **done** *(one deviation)* | `zdc-lexer` 96 tests including `src/layout.rs`; `zdc-parser` 206 across boundary-focused files; `zdc-ast` 12. `zdc parse examples/hello.zd` exits 0. **Deviation:** the spec's testing table asks for `insta` snapshot tests; `insta` is not a dependency of any crate. The coverage exists as ordinary assertions instead. |
| **M2** | HIR and name resolution | ✅ **done** | `zdc-hir` 17 tests, `zdc-resolve` 123. Two-pass resolver reports every error, not the first: `crates/zdc-resolve/tests/resolution.rs`. `zdc check` runs it. `crates/zdc-hir/src/sandbox.rs` bounds every path a `use` can reach. |
| **M3** | Type checker (placement-unaware) | ✅ **done** | `zdc-types` 188 tests. Hindley–Milner over `Text`, `Whole`, `Decimal`, `Truth`, `List of T`, `Map of K to V`, `Option of T`, `Remote of T`, records and choices. |
| **M4** | Signal graph, placement coloring, IFC pass + negative test suite | ✅ **done** | `zdc-graph` 141 tests, including the negative leak suite §11 calls the crown jewels. **Verified by building:** `zdc build examples/guestbook.zd` emits a `client.js` containing neither `apiKey` nor `GREETING_API_KEY` — grepped for both in the built bundle, zero hits. |
| **M5** | JS codegen + runtime; client-only programs run in a browser; benchmark suite in CI | ✅ **done**, except the React/Solid arm | `zdc-codegen` 947 tests, `zdc-runtime` 62 (which execute `runtime/signal.test.js` and `runtime/dom.test.js` under an embedded pure-Rust JS engine), `zdc-bench` 50 (plus 3 ignored surveys). `BENCHMARKS.md`'s generated region (lines 119–240) is regenerated from the suite and exact-match gated. **Not delivered:** §14A.4's React and SolidJS arms, which need a package manager CI does not have. |
| **M5b** | `when`, `each`, view-position `if`, scoped classes, source maps | ◐ **partial** | Landed: `when` and `each` as anchored holes; view-position `if` (`examples/disclosure.zd`); generated scoped classes (`zdc-codegen/src/styles.rs`, 4 unit tests). **Not landed: source maps.** Verified by grep — no `sourceMap` or `sourcemap` anywhere in `crates/` or `runtime/`. |
| **M6** | `server` placement, RPC generation, `zdc dev` | ✅ **done — emits *and* executes** | `zdc dev` is an in-binary HTTP server with a file watcher, SSE live reload and diagnostic-on-page (`zdc-dev`, 116 tests). `zdc build examples/guestbook.zd` writes `functions/greeting.js`, `functions/visits.js`, `functions/visits.incr.js` and a `manifest.json` — **verified by building and listing the output.** `zdc-host` (103 tests) is §8.2's platform adapter: it binds `$env` and `$store` and runs the emitted handler in the compiler's own `boa_engine`. |
| **M7** | `durable` placement, store, SSE sync | ✅ **done — one deviation** | `zdc-store` (63 tests), a durable store over one total order; `runtime/store.js` and `runtime/wire.js` are the browser half; live sync over a transport seam, `streamTransport` and `pollTransport`. **Evidence is `crates/zdc-host/tests/two_windows.rs` (7 tests):** one window increments, the other is told the new value with no round trip, a reconnecting window is replayed what it missed, and two windows over a reopened database agree. The retry is bounded (#143): exponential backoff from 1 s to a 30 s ceiling with full jitter, giving up after eight consecutive failures, at which point every durable cell moves to `Failed` with an `Unreachable` code so the program's third arm can say so rather than the page stalling on a value nothing is keeping current. `crates/zdc-codegen/tests/live.rs` drives that through an emitted bundle without sleeping. **Deviation:** the store is `redb`, not SQLite — chosen because SQLite would link a C library and forfeit §7's single static binary. |
| **M8** | Style compilation to static CSS | ✅ **done** | `styles.rs` interns one class per *distinct* declaration set and emits `styles.css` as `runtime/base.css` plus generated rules; signal-dependent styles become `bindStyle`. **The surface is no longer small: 33 style arguments (`elements.rs::STYLE_ARGUMENTS`) plus six global ones, each with a value grammar in `crates/zdc-codegen/src/style.rs`, and 38 of them take any of seven conditional prefixes** (`hover`, `focus`, `active`, `disabled`, `narrow`, `wide`, `dark`), so one class carries its own `:hover`, breakpoint and `prefers-color-scheme` rules and the interning property still holds. Tests: `class_and_style.rs` 8, `injection.rs` 28, `styles.rs` 6 unit. **Verified by building:** `zdc build examples/todo.zd` emits `text-decoration-line: line-through` for a done item, which is the one visual state the canonical benchmark is about and could not previously render. `runtime/base.css` is 3,321 bytes, up from 927. |
| **M9** | Dialect layer, `zdc show --dialect`, round-trip tests | ⬜ **not started** | Only the M1 enabling structure exists: `word_to_kind` is the single keyword table, keyword tokens carry no text, and diagnostics are phrased to take a dialect spelling. No dialect, no `show` subcommand, no round-trip test. |
| **M10** | Demo application | ⬜ **not started** | `examples/` are language samples, not an application. `runtime/demo/` is hand-written JavaScript exercising the runtime, not a ZDeceptron program. All thirty-four examples now check and build ([§2](#2-examples)), which is a stronger language claim than it is an application. The six algorithm examples move it slightly: they compute rather than demonstrate, and each has a working interface, but none of them is an application either. |
| **M11** | Multi-target deploy (Vercel, AWS Lambda, Cloudflare) with hosted KV | ◐ **partial — generates, never deploys** | `zdc-deploy` (44 tests) and `zdc deploy --target cloudflare\|lambda\|vercel\|deno`, each writing an entry shim, a store binding, a portable router, an endpoint table and platform configuration, plus a capability report naming what that platform cannot do. **Verified by running** `zdc deploy examples/tally.zd --target cloudflare`, which prints the Cloudflare capability report. `tests/portability.rs` pins that handler bodies and router are byte-identical across all four. **Not delivered:** any of this run against a real account. Azure is deliberately absent and `--target azure` says why. |
| **M12** | Writeup | ◐ **partial** | `BENCHMARKS.md` is a substantial, self-critical piece of it. `README.md` and this file exist. There is no writeup document. |

---

## 2. Examples

`./target/release/zdc check <file>` and `build <file>` over every file in `examples/`.
**`examples/` holds thirty-seven files. All thirty-seven check and all thirty-seven
build.** Thirty-five sit directly in `examples/` and two — `tree/` and `tree-webgl/` —
are in a directory of their own because each has assets beside it. Thirty-six of the
thirty-seven are programs; the odd one out is named below.

The odd one out is `sorting.test.zd`, which is a file rather than a program: it declares no
view of its own, states six claims about `sorting.zd`, and is run by `zdc test` (#169). It
checks and builds like the rest because a `test` is an ordinary declaration and a file holding
one is an ordinary module — which is the point of lowering a claim to a `static Truth` rather
than inventing a second file format for it.

The count was wrong before this branch, and not because of the six new files: `gauge.zd`
landed with the foreign-view work and was never added to the table below, so "nineteen" was
already twenty. It is listed now. Verified by running `check` and `build` over every `*.zd` in
`examples/` and recording each exit status, rather than by counting rows.

The last six are a different kind of example from the nineteen above them. Those demonstrate a
construct; these run an algorithm whose answer is not obvious from reading the source, and
every answer is pinned by `crates/zdc-codegen/tests/algorithms.rs` (19 tests), which compiles
each file the way `zdc build` compiles it, runs the bundle, and reads the answer back out.
Three of the six are checked against a reference implementation written in Rust in that file
rather than against a recorded number.

| File | `check` | `build` | Note |
|---|---|---|---|
| `examples/hello.zd` | ✅ | ✅ | — |
| `examples/counter.zd` | ✅ | ✅ | — |
| `examples/todo.zd` | ✅ | ✅ | Declares a `record`. |
| `examples/guestbook.zd` | ✅ | ✅ | All three placements; emits three server function files, **and runs them** — see M6. |
| `examples/writing.zd` | ✅ | ✅ | `client` + `static`. Content computed at build time and inlined; emits `rss.xml` into the bundle. **Verified to build with an empty `PATH`** (`env -i PATH= …`), so no toolchain is consulted. |
| `examples/tally.zd` | ✅ | ✅ | A `durable Map` — the only example storing something other than a number. It exists because of the bug in [§6](#6-what-was-found-and-fixed). |
| `examples/voting-board.zd` | ✅ | ✅ | Writes `Row item.name`. The leading-argument refusal that used to block this file **is gone.** |
| `examples/leaderboard.zd` | ✅ | ✅ | Indexes with `at`, which the prelude now supplies. |
| `examples/components.zd` | ✅ | ✅ | — |
| `examples/disclosure.zd` | ✅ | ✅ | A component with its own `state`; uses view-position `if`. |
| `examples/model.zd` | ✅ | ✅ | A view-less module. `build` **no longer refuses** one. |
| `examples/content.zd` | ✅ | ✅ | — |
| `examples/events.zd` | ✅ | ✅ | Event payloads on handlers. |
| `examples/booking.zd` | ✅ | ✅ | `NumberInput` and `DateInput` (#45, #48). Both bind an `Option`, because a box with nothing usable in it holds no number and `NaN` is not a value this language has. The date is a *moment* — the `Whole` of milliseconds `prelude/time.zd` already reads apart — so no `Date` type was invented and no calendar is written twice. |
| `examples/page.zd` | ✅ | ✅ | Document head and per-page metadata. |
| `examples/site.zd` | ✅ | ✅ | Declared routes; one bundle per URL, and one stylesheet for the site (#136). |
| `examples/dungeon.zd` | ✅ | ✅ | A placement loop written with a tail-recursive accumulator. |
| `examples/terminal-help.zd` | ✅ | ✅ | A `"""` block literal in a program. |
| `examples/layout.zd` | ✅ | ✅ | A view-less module of components, `use`d by `blog.zd`. `children` as a slot. |
| `examples/blog.zd` | ✅ | ✅ | **Was the one failure; is not any more.** Reads `examples/content/blog/*.md` off disk through the `build` capabilities, renders the markdown at compile time, and puts the result on the page through `Prose`. **Verified to build with an empty `PATH`.** |
| `examples/gauge.zd` | ✅ | ✅ | A `foreign … gives view` that owns its own `<div>` and is driven by a signal. Was missing from this table. |
| `examples/parts.zd` | ✅ | ✅ | A post that names a component (#305). `build parts` splits `examples/content/parts/*.md` at the fences this compiler owns, so a document is a `List of Part` — prose runs and named widgets — rendered by an ordinary `each` rather than by one `Prose`. That shape is forced: `Prose` has no children because interleaving parsed nodes with templated ones would make the sibling offsets every binding is scheduled against depend on how many nodes a *file* parsed into. **The widget set is closed and the program declares it**, as `choice Widget`, and a document naming anything else is `E11` at build time rather than a blank space on a page — which is a stronger bargain than MDX makes, where an `import` in a content file can reach anything on disk. |
| `examples/graph-traversal.zd` | ✅ | ✅ | Depth-first and breadth-first over one declared graph, stepped a visit at a time. Verified: DFS visits `0 1 3 7 4 5 2 6`, BFS visits `0 1 2 3 4 5 6 7`. Both frontiers are `List of Whole` — a stack and a queue — and the visited set is a `Map of Whole to Truth` (#233), which says what it means without being faster: the walk reads the set between every pair of writes, and that is the shape the map write chain does not make cheaper. Still O(v * (v + e)). |
| `examples/shortest-path.zd` | ✅ | ✅ | Dijkstra over eleven weighted roads. Verified against a Dijkstra written in Rust in the test: toll 14 over six roads, where the fewest-roads route costs 19. **23 frontier extractions to settle 7 towns**, which is what the missing priority queue costs. |
| `examples/scene.zd` | ✅ | ✅ | One drawing written twice — `Svg` and `Scene` — with the same five children under each. The `Svg` is DOM nodes the browser keeps; the `Scene` is the same shapes handed to Canvas 2D, WebGL or WebGPU, whichever the machine has. Verified in a real browser under both `--disable-gpu` (Canvas 2D) and SwiftShader (WebGL): the two drawings are the same picture. |
| `examples/sorting.zd` | ✅ | ✅ | Insertion sort and merge sort written in ZDeceptron, beside `sort each … by`. Verified: all three produce the same list as Rust's own sort; 119 comparisons against 63. Measures that the built-in sort is stable today, which is evidence on #114. |
| `examples/edit-distance.zd` | ✅ | ✅ | Levenshtein over a **flat** DP table, with two live `Input`s. Verified cell by cell against a Levenshtein written in Rust; kitten to sitting is 3 and the edit script is pinned. The flat table is the finding: filed as #195. |
| `examples/knapsack.zd` | ✅ | ✅ | 0/1 knapsack with a traceback, next to the greedy answer it beats. Verified against a knapsack written in Rust: 42 against greedy's 39 at a capacity of 21. |
| `examples/poker.zd` | ✅ | ✅ | Five-card draw, two hands, full ranking. Every hand folds to one `Whole` — the category above five base-sixteen digits of tie-break — because §17.2.5 refuses comparators, so `>` on that number *is* the rules of poker. Verified against an independent reference: 500 dealt hands match score for score, and thirteen constructed hands cover every category and both straight edges, including the wheel ranking below a six-high straight. Shuffled with the prelude's mulberry32; hand frequencies over 40,000 hands match the true ones. |
| `examples/queens.zd` | ✅ | ✅ | N queens by backtracking, board size and arrangement on buttons. Verified against OEIS A000170: 2, 10, 4, 40, 92 arrangements for boards of 4 to 8, and the board drawn is checked to have no queen attacking another. |
| `examples/sorting.test.zd` | ✅ | ✅ | Six `test` declarations about `sorting.zd`, run by `zdc test` (#169). The two comparison counts `sorting.zd`'s header quotes — 119 and 63 — were prose until now, with nothing to notice when they stopped being true. Pinned by `zdc-cli/tests/expectations.rs`, which runs the command and requires `6 held`. |
| `examples/timers.zd` | ✅ | ✅ | The clock: `every "100ms"`, `every frame` and `after "2s"` (#19). **A timer is a callback and this language has none**, so the clause takes no block and runs nothing — it declares a source signal whose writer is the browser's scheduler, and everything downstream is `from`. Nothing may write a clock cell, so a tick cannot start a request or reach the store. `runtime/clock.js` is linked only by programs that use it, for the size gate's reason; disposal is proved deterministically in `runtime/clock.test.js` against a scheduler that suite controls, and the whole thing is loaded in a real browser by `zdc-cli/tests/browser.rs::a_clock_signal_ticks_in_a_real_browser`. |
| `examples/tree/tree.zd` | ✅ | ✅ | One of two examples in a directory of its own, because it has a stylesheet beside it. **It contains no JavaScript, and it used to contain 226 lines of it.** A revolving three-dimensional tree with no `foreign` and no trigonometry: each branch is a child *element* of the branch it grows from, so the compositor multiplies each branch's rotation by its parent's and nothing ever needs a branch's absolute position. The program names only a branch's angle *within its parent*, and a fan of one to four has ten of those. §6.1's `class is` takes a computed `Text`, which is how `"zt-y" + (text of fan) + "-" + (text of turn)` becomes a rotation; `assets/tree.css` holds those ten names, the `perspective` and the one `@keyframes`, and is linked after the generated sheet so it wins with no `!important`. What it cost: a branch is two DOM elements inside a `preserve-3d` subtree the compositor cannot flatten into a layer, so the budget is a few hundred branches rather than the few thousand a WebGL renderer instanced without noticing — the depth ceiling is derived from the fan (`ceilingFor`) to hold the worst case at 364. |
| `examples/tree-webgl/webgl.zd` | ✅ | ✅ | The same tree in **real WebGL, through three.js, with no hand-written JavaScript anywhere in the directory** — the acceptance test for #271 stage 3. Every three.js class is a `foreign … gives new Handle`; `renderer.domElement`, `.ownerDocument` and `.body` are `of Handle as` property reads; `scene.add`, `renderer.render` and `renderer.setSize` are `gives nothing` foreigns run by `do` statements; and three.js arrives through `zd.toml`'s `[packages]` map with nothing vendored. **What it demonstrates is the lifetime.** The `mount`/`update` split the deleted `draw.js` kept by hand — acquire one WebGL context, rebuild only the meshes — is now the difference between `starting` and `from`: the context, the camera, the shared cylinder geometry and the material are `client Handle starting`, which the compiler refuses to let anything recompute or overwrite (E0317), and one derived signal rebuilds the branch meshes and draws a frame. Verified in Chromium over HTTP: a WebGL2 context on a 2160×1252 buffer, 9,841 branches at nine levels of a three-way fan against the CSS version's ceiling of 364, one canvas after 120 rebuilds, and zero console messages. **What it does not have:** an effect construct, so the redraw is written as a derivation whose value is the branch count; and no frame loop, for the reason `tree.zd` gives. |

`blog.zd` was the last aspirational example, and it is the one this branch changed most. It
used to fail at `examples/blog.zd:46:54` on
`state posts is static List of Post from readMarkdown "content/blog"` — a call written with a
bare argument, which has no production in §4.4, naming a build-time `foreign` with no host to
import from. Both halves are now the `build` capability form: the posts come off
`examples/content/blog/`, the markdown is rendered by the compiler, and what reaches the
browser is a string literal with no path to fetch and no renderer shipped.

Two tests hold that end to end:
`crates/zdc-cli/tests/cli.rs::the_blog_builds_from_files_on_disk_with_nothing_to_fetch`, which
builds it with an empty `PATH` and asserts the rendered `<h1>` is inlined, and
`crates/zdc-codegen/tests/markup.rs::the_blog_renders_its_posts_as_headings_and_paragraphs`,
which runs the bundle and reads the headings and paragraphs back out of the DOM.

Not in `examples/`, but compiled by the test suite:

| File | `check` | `build` |
|---|---|---|
| `crates/zdc-bench/bench/row.zd` | ✅ | ✅ |

---

## 3. Tests

**2649 passing, 0 failing, 15 ignored**, across 21 crates and 159 test binaries plus 20
doc-test targets, measured on `test/prove-the-tests-can-fail` with `cargo test --workspace
--no-fail-fast`. `scripts/check-vacuous-tests.py` walks the same tree and reports **2664 tests
in 275 files** from a static count of the attributes, and 2649 passing plus 15 ignored is 2664,
so the two figures reconcile exactly and the run is not quietly skipping a binary. Five of the
fifteen are the deliberate ones enumerated below. The other ten are ignored for their cost and
have CI jobs that run them anyway: nine in `crates/zdc-cli/tests/browser.rs`, and the mutation
sweep in `crates/zdc-runtime/tests/mutation.rs` (#160).

No commit hash beside it this time, because a figure taken from the tree a commit records
cannot name that commit's own hash — the hash is not known until after the file is written.
The command is given instead, which is the thing the paragraph below argues for anyway, and
the static count is re-derivable from the tree by anyone who doubts the runtime one.

**`--no-fail-fast` is load-bearing, not decoration.** A bare `cargo test --workspace` stops at
the first failing target, and #192's wall-clock ratio test fails often enough that a bare run
reports **279 passing and stops** — about an eighth of the suite, behind a tail that reads like
an ordinary summary. Any figure taken without the flag is a truncation, not a measurement.

**Corrected 2026-08-07.** This section said 2079 across 144 binaries while the header of this
same file said 2041; the two were measured on different branches and neither was re-taken. The
binary count is now given as what `cargo` actually prints — 134 `Running` lines and 18
`Doc-tests` lines — because a single number for it had no definition anyone could reproduce.

**The per-crate table below is measured, not counted** — issue #259.

It used to quote what a `cargo test` run printed, and that number kept
rotting for a reason worth stating: it had no definition anybody could
reproduce. A bare run stops at the first failing target and reports about
an eighth of the suite; a run with `--no-fail-fast` reports all of it; a
slow machine changes which of #192's wall-clock tests pass. Three people
measuring "the number of tests" got three answers, all honest.

So the table now counts something with one answer: **the `#[test]` and
`#[tokio::test]` functions each crate declares.** That is the number of
tests *written*, which is what the table is cited as evidence of, and it
does not move when a run is truncated, when a machine is slow, or when an
`#[ignore]` is added.

`crates/zdc-cli/tests/documented_counts.rs` asserts every row against the
tree and fails when one drifts, so a stale figure here is a red build
rather than a claim nobody is checking — the treatment §14A.4 already
gives `BENCHMARKS.md`. It also asserts that every crate *has* a row: when
the gate was first written `zdc-wasm` had none, and a table offered as the
coverage story with a crate missing is a worse kind of wrong than a stale
number.

When this landed every row was stale, not the six #259 had measured, and
the total had grown from about 1,546 to 2,661. It is the sum of the
column below and moves with it — 2,664 as the mutation harness (#160)
lands.

| Crate | Tests | Note |
|---|---|---|
the total had grown from about 1,546 to 2,661. The table sums to 2,670
today; the 2,661 stays as written, because it is a measurement of a past
tree and updating it in place would make a sentence about that day false
about it.

| Crate | Tests | Note |
|---|---|---|
| `zdc-codegen` | 947 | The largest suite, and the only row re-measured on this branch. Includes `tests/algorithms.rs`, the 19 tests that run the six algorithm examples and read their answers back out. |
| `zdc-types` | 233 | Plus 2 ignored, both recording an open language decision. |
| `zdc-parser` | 213 | Split across boundary-focused files. |
| `zdc-graph` | 221 | Including the information-flow negative suite, the failure channel, and `tests/report.rs` — §19.5's audit trail, which is what `zdc build --report` prints. |
| `zdc-resolve` | 178 | Includes the `use`-sandbox suite and the instantiation bounds. |
| `zdc-dev` | 121 | Self-contained unit suites plus integration files driving the running server. |
| `zdc-lsp` | 179 | Re-counted when `zdc doc` landed. |
| `zdc-cli` | 146 | Re-counted here. End-to-end over the real binary, including a seeded fuzz harness and `tests/fmt_examples.rs`, which mangles every example, lays it out again and compares the emitted bundle byte for byte. |
=======
| `zdc-codegen` | 947 | The largest suite, and the only row re-measured on this branch. Includes `tests/algorithms.rs`, the 19 tests that run the six algorithm examples and read their answers back out. |
| `zdc-types` | 233 | Plus 2 ignored, both recording an open language decision. |
| `zdc-parser` | 213 | Split across boundary-focused files. |
| `zdc-graph` | 221 | Including the information-flow negative suite and the failure channel. |
| `zdc-resolve` | 178 | Includes the `use`-sandbox suite and the instantiation bounds. |
| `zdc-dev` | 121 | Self-contained unit suites plus integration files driving the running server, including the wire format's refusal over a real socket (#144). |
| `zdc-lsp` | 179 | Re-counted when `zdc doc` landed. |
| `zdc-cli` | 146 | Re-counted here. End-to-end over the real binary, including a seeded fuzz harness and `tests/fmt_examples.rs`, which mangles every example, lays it out again and compares the emitted bundle byte for byte. |
>>>>>>> febe6c8 (Trim the runtime's copy of the rule and re-measure what it costs)
| `zdc-host` | 103 | §8.2's platform adapter. `tests/two_windows.rs` is the live-sync evidence. |
| `zdc-lexer` | 100 | Re-counted here. Includes the check that every reserved word can say what it is reserved for. |
| `zdc-store` | 63 | The durable store and its transactions. |
| `zdc-bench` | 60 | Plus 3 ignored. Includes the exact-match `BENCHMARKS.md` gate. |
| `zdc-deploy` | 47 | Four platform adapters and the portability claim, which now drives the router in the engine rather than only grepping it. |
| `zdc-doc` | 25 | New. The generated pages, asserted on what they *claim* — a placement, a `Remote of T`, a derived endpoint — rather than on a file existing. |
| `zdc-diagnostics` | 66 | Re-counted when type errors gained codes (#148). The inline budget, the `zdc explain` coverage gate — now over four code families, `E02xx` being the new one — and `tests/caret_labels.rs`, which asserts on rendered output because the caret's message is a rendering decision. |
| `zdc-fmt` | 27 | New here (#167). The layout rules, the two refusals, and the block-literal cases — which compare the *values* the lexer reads back rather than the source text, because a literal is what this formatter is most able to damage and least able to see. |
| `zdc-hir` | 40 | |
| `zdc-runtime` | 85 | Two of these run the JavaScript suites — further assertions the count above does not see. `tests/wire_version.rs` pins the wire format's version across its three spellings (#144). |
| `zdc-ast` | 12 | |
| `zdc-wasm` | 11 | The front end as a WebAssembly module. Not published to crates.io — nothing links it — and `ci.yml` builds it for two `wasm32` targets, which is the only build where `zdc-diagnostics`'s engine-free dependency edge means anything. |
| `zdc-lib` | 10 | The prelude's surface, pinned so an operation cannot stop being declared unnoticed. |

### The deliberate ignores, each with a written reason

**Five of them record something open or unassertable, and are the ones this
section is about.** The rest — nine today, and one more with this branch —
are ignored because of what they *cost*, and each has a CI job that runs it
anyway; they are at the end of this section.

Three of the five are in `crates/zdc-bench/tests/scaling.rs` — the `survey_*` tests, each carrying
`#[ignore = "prints the survey behind BENCHMARKS.md; not a gate"]`. They print the scaling
survey rather than asserting on it, and are reports rather than gates. The fourth is #23's
`survey_cross_root_duplication`; the run figures in [§3](#3-tests) were taken before it landed
and are not re-taken here, because a figure is of the tree it was measured on.

The sixth is `crates/zdc-bench/tests/asymptotics.rs`, and it is the one ignore here that is
*not* optional: it is a timed sweep, it needs a release build to be measuring the emitter
rather than the compiler's own missing inlining, and it costs tens of seconds. `#[ignore]` is
what keeps it off every laptop's `cargo test`; `ci.yml`'s `asymptotics` job is what makes that
different from nobody running it, which is the same arrangement `crates/zdc-cli/tests/
browser.rs` has. It refuses to run at all on a debug build rather than returning early, because
returning early is how `crates/zdc-lsp/tests/latency.rs`'s ten-millisecond budget came to be
enforced by nothing.

**The fourth this file used to name is gone.** It was in
`crates/zdc-codegen/tests/emission.rs`, recording that `zdc check` accepted a program `zdc
build` refused. `zdc check` now runs the emitter, so the two commands cannot answer differently
about any program, and the test stays as an ordinary regression test with its `#[ignore]`
removed. A rationale saying the compiler disagrees with itself is a false statement about the
repository once it does not.

Two new ones replace it, both in `crates/zdc-types/tests/checking.rs`, and both record a
**language decision that has not been made** rather than a fix nobody got to:

- `a_give_after_a_pipeline_run_is_refused` — a `give` written after a pipeline run typechecks
  and is emitted as unreachable code. Refusing the body outright is one coherent reading;
  letting the trailing `give` win is another, and it would need `block` to stop emitting the
  run's own `return`.
- `an_input_binds_a_components_own_state` — `Input` cannot bind a component's own `state`,
  though a handler can write it. §14B.5 is written in terms of a `state` *signal* and says
  nothing about a component's per-instance cell; admitting it decides that the two are the same
  thing for the purpose of writing back.

Both were re-run against this branch with `--ignored` and both still fail, so neither is a
defect that quietly closed. **They are ignored because they document decisions that are open,
not to make the suite pass.**

**Ten more are ignored for a reason that is not in that list at all, and never were part of the
five.** Nine are in `crates/zdc-cli/tests/browser.rs` and one is
`crates/zdc-runtime/tests/mutation.rs::no_mutation_of_the_runtime_goes_unnoticed` (#160). Neither
records anything open. They are `#[ignore]`d because of what they cost a contributor who typed
`cargo test` — a real browser in one case, two hundred and thirty-six mutated runtimes each
running a whole JavaScript suite in the other — and `ci.yml` gives each of them a job that runs
it with `--ignored`. The ignore moves the cost off a laptop, not off the build.

### Coverage relative to risk: `zdc-codegen/src/server.rs`

**This section previously said `server.rs` carried zero unit tests. That is no longer true and
the figure should not be quoted: it carries 16.** `server.rs` decides what a server endpoint's
source is — its parameters, its wire order, its `$env` reads, its `$store` calls — and it is
still the second-best-covered file in `zdc-codegen/src`, behind `elements.rs`.

`zdc-codegen`'s unit tests are no longer confined to four files. They are: `elements.rs` 17,
`server.rs` 16, `js.rs` 12, `assets.rs` 10, `capability.rs` 10, `hash.rs` 7, `styles.rs` 6,
`events.rs` 5, `names.rs` 5, `style.rs` 5, `cache.rs` 3, `intrinsics.rs` 2 — 98 in total,
against the 18 this file used to report. `hash.rs` and `cache.rs` are new with #137, and
`assets.rs` grew with it: a name that is a function of a file's bytes has two properties worth
testing in both directions, and the rule deciding *which* files may carry one is the part a
reader is most likely to get wrong.

The *shape* of the original concern survives the numbers: server emission is exercised
end-to-end by `zdc-host` (74 tests, which actually execute the emitted handlers) rather than by
`zdc-codegen` alone, so the two crates should be read together when judging whether a wrong
endpoint would be caught. It would be.

---

## 4. What this language cannot do yet

The most useful section in this file. Every entry was re-verified against the merged
`feature/front-end` tree rather than inherited — two of them changed sign and say so.

### A value can become markup, and only one kind of value can

**This section previously read "a value cannot become markup" and said there was no `Markup`
type and no `Prose` element. Both now exist, and the entry is kept rather than deleted because
what replaced the gap is narrower than simply closing it.**

`Markup` is a base type (`crates/zdc-types/src/ty.rs`) and `Prose` is an element in
`zdc-codegen`'s 37-element vocabulary. `Text` and `Markup` are as unrelated as `Text` and
`Truth`: neither converts to the other, and `Text + Markup` does not typecheck in either
order. The one expression that *produces* a `Markup` is `build markdown`, which runs inside the
compiler over a file in the project directory, escapes every raw HTML span, and rewrites every
non-`http(s)` URL before returning.

So the honest statement of the limit is now the opposite shape: a value a *program* computes
still cannot become markup. Only the compiler can make one, from a file on disk, at build time.

Three assignments to `innerHTML` exist in `runtime/`, all in `runtime/dom.js`: `template()`,
whose argument is a compile-time string literal of the program and never a value, and
`markup()`/`bindMarkup()`, which the emitter reaches only from `Slot::Rendered` — which only
`Prose` has, and whose argument must be a `Markup`.
`crates/zdc-codegen/tests/markup.rs::the_blog_bundle_names_inner_html_nowhere_outside_the_runtimes_markup_path`
holds that list to those three and asserts generated code never names the property at all.

### It cannot deploy — only generate a deployment

`zdc deploy --target cloudflare|lambda|vercel|deno` writes a complete deployment — entry shim,
store binding, portable router, endpoint table, platform configuration — and prints a capability
report saying what that platform cannot do. **It does not deploy**, and nothing in this
repository has been run against a real account on any of the four. The adapters are checked
against their vendors' documented limits and against each other for portability; they are not
checked against the vendors.

### `Whole` overflow is uncaught on the client path

**Verified by reading the emitter.** `crates/zdc-codegen/src/expr.rs`'s `BinOp::Add` and
`BinOp::Mul` arms emit bare JavaScript `+` and `*` with no guard. So on the client path a
`Whole` silently loses integer precision above 2⁵³ and silently becomes `Infinity` above
`Number.MAX_VALUE` (≈1.7977 × 10³⁰⁸). §14A.3 decided the representation — f64, and document the
bound — so the *type* is behaving as specified; what is undecided is what an operator should do
at the bound, and #5 is that question rather than a defect in the choice.

The narrowing operations *are* guarded — `crates/zdc-codegen/src/intrinsics.rs` wraps `floor of`
and `round of` in `Number.isFinite` and gives an `Option`. **That guard does not extend to `*`
or `+`.** The durable path is covered; the client path is not.

**A literal is now held to the bound.** `state n is client Whole starting 100000000000000000000000`
used to compile and emit a number 8,388,608 smaller; the check compared the value's *shortest
round-tripping* decimal against the digits rather than the value itself. It is refused, and the
message names `99999999999999991611392`, which is what the machine holds.

Named rather than cited by line: the numbers here moved twice while the claims stayed still.

### ~~The emitter is near-quadratic in view size~~ — fixed, and it was the wrong pass (#8)

This section used to cite three source comments — "quadratic in definitions × pages",
"quadratic in functions", "split is already quadratic in definitions × roots" — as evidence
that the *emitter* was near-quadratic in view size. All three comments are accurate and none of
them is about that. They describe the tier split and the per-page analysis, which are functions
of the definition and root sets; on the program that was actually slow, `split` was 0.3 ms of
18.8.

What was superlinear is the emitter's **path scheduling**, and it was cubic rather than
quadratic: it ran a breadth-first search for the shortest walk between two nodes of a structure
that has exactly one walk between any two nodes it connects, once per node already named. It is
now a climb up a parent chain. Emission is byte-identical.

**The cost landed per keystroke in the editor**, because the language server runs the real
passes, and that is where it was found: a six-kilobyte file is 7.2 ms against 18.8, and a
sixty-kilobyte one 14.5 ms against 95. Two thirds of the old figure was not the emitter at all
but §17.3.4's witness reconstruction in the flow pass, which now runs only on a program that has
a secret to explain. `BENCHMARKS.md` has both sets of numbers and how the measurement that
pointed at the wrong pass was itself wrong.

**What is left is flat.** Six of the remaining 7.2 ms does not depend on the file's size: it is
§17.4.1's prelude, re-analysed from nothing on every keystroke. There is no incremental pipeline
for it to be kept in.

### The following syntax is refused

| Construct | Status, verified on this branch by running `zdc check` |
|---|---|
| `unique` in a record field | **Parses, and is refused after it.** The probe now reports *"`Todo` declares `id` as its identity, and `unique` is not implemented past the parser yet (#2). Removing the word compiles, and reconciles by position."* — so the parser rule has landed and the type table and the emitter have not. This is still why every list reconciles positionally; see `BENCHMARKS.md`. |
| `readMarkdown "content/blog"` — a call with a bare argument | **Refused.** Every call is written `f with a, b`. This used to be what stopped `blog.zd`; the file is now written in the `build` capability form and builds. |

**`Row item.name` — a leading argument to `Row`/`Column` — now works.** It was listed here as
the one thing standing between three examples and a successful build; those three examples
(`voting-board.zd`, `leaderboard.zd`, `components.zd`) all build now.

### Other absences, each re-verified

- **No source maps.** Grepped: no `sourceMap` anywhere in `crates/` or `runtime/`. A browser
  stack trace points at generated JavaScript.
- **No dialects.** Only `english`. The enabling structure is in place; no second surface exists.
- **No `record … unique`.** Every list reconciles positionally.
- **~~No build-time file reading.~~ Landed.** `build read`, `build list` and `build markdown`
  all exist, run in the compiler's own sandbox over the project directory, and are what
  `writing.zd` and `blog.zd` are built from. `crates/zdc-hir/src/sandbox.rs` bounds what a
  build may reach, and both examples are verified to build with an empty `PATH`.

**`foreign` is no longer among these.** This file previously said *"`foreign` parses but is not
lowered — codegen has nothing to emit for a call to one."* **That is stale.** A called `foreign`
is lowered and imported by the bundle that calls it, pinned by
`crates/zdc-codegen/tests/foreign_import.rs::a_called_foreign_is_imported_by_the_bundle_that_calls_it`,
with a companion test that an uncalled one is *not* imported. The same file pins that a foreign
from a remote origin and an export name that would close the import clause both never reach
emission.

### The standard library exists, and it is young

`crates/zdc-lib/prelude/` is seven `.zd` files — `text`, `list`, `map`, `number`, `option`,
`remote`, `time` — written in ZDeceptron above a `foreign` primitive layer, resolved into the
program's own arenas ahead of it (§17.4.1).

**Twenty-one** `foreign` declarations are the whole of what cannot be written in the language.
The authority is `crates/zdc-lib/src/lib.rs::most_of_the_library_is_written_in_zdeceptron`,
which asserts the count and, in a comment beside it, gives the reason for each: `textLength` and
`textAt` because nothing can take a `Text` apart from inside the language; `uppercase`,
`lowercase` and `trim` because Unicode case and whitespace are tables, not rules; `listLength`,
`listAt` and `mapLength` because they are O(1) on the platform and writing them would make
`length of` linear; `mapAt` and `mapKeyAt` because a map cannot be taken apart from inside the
language; `floor`, `round` and `decimalOf` because they are statements about the f64
representation the language gives no way to observe. **Read the test, not any table.**

`newline` is not among them: the `"""` block literal takes its lines from the source, so
`prelude/text.zd` writes the line separator directly. `trim` did not follow it out, for the
reason above.

`entries`, `mapOf`, `mapRemove`, `merge`, `mapValues` and `zip` did not move it either, and
that is the point of naming them. Each hands back a collection, which was the reason every one
of them was unwritable, and what closed the gap was two language forms rather than two more
primitives: `set key to value in table` is the map's `append`, and `Pair of K to V` is the
return type `zip` and `entries` had no way to name.

The builders that are *not* linear are named at their definitions: `slice`, `dropFirst`,
`startsWith` and `endsWith` concatenate one code point at a time, so they cost O(k²) and O(k)
stack depth in the characters they copy. They are for affixes, which are literals in practice.

---

## 5. Known defects carried forward

Found, verified on this branch, and **not fixed** unless a row says otherwise. Recording rather
than fixing is deliberate: other branches own this code.

| # | Severity | Where | What |
|---|---|---|---|
| 1 | *Fixed* | `crates/zdc-lsp/src/server.rs` | **Go-to-definition resolved an imported span against the entry document.** Fixed on `feature/lsp-editor-surface`. `Analysis` now keeps the `Linked` the loader produced, and every answer this server gives that carries a location is built by one function that puts the span through `Linked::locate` first, so the file and the offset come from the module that owns the span. The number is kept rather than reused: the rows below were numbered against it. |
| 2 | **Medium** | `zdc_resolve::load`'s error path, at `crates/zdc-cli/src/main.rs:298-314` | **A parse error in an imported file is rendered against the entry file's text.** The error arm does `std::fs::read_to_string(file)` — the *entry* path — and renders every error against it. The span does not fall inside that text, so `ariadne` prints the message with no location at all: the reader is told what is wrong and not which of their files it is in, or where. The successful path already carries per-file text through `Linked::locate`; only the error path does not. |
| 3 | **Low** | `crates/zdc-graph/src/split.rs:156` | **`mutations_at` still carries a `Span` inside a composite key:** `BTreeMap<(Span, Ctx, DefId), MutCrossing>`. See §7 for why this shape is a hazard. It is the last substantive survivor of the span-aliasing family. |
| 4 | **Low** | `crates/zdc-codegen/src/lib.rs` | Module doc still describes an earlier milestone's scope. |
| 5 | **Medium** | `$force`, `crates/zdc-codegen/src/intrinsics.rs:221` | **A forced `$Ap` keeps its `base`, so an append chain that is read as it is built retains every intermediate array.** A forced node answers out of `flat` and never reads `base` again, but the field holds the rest of the chain alive, and every node in it that was also forced is holding a whole array. `examples/graph-traversal.zd` is the shape that hits it: `order` is an append chain read once per step, so every intermediate copy stays reachable. Found while giving `$mapForce` the same structure — the map's flatten drops its `base` for exactly this reason, and the measurement is in that helper's note. Not fixed here because changing `$force` rewrites the bundle of every program that appends, which is a different branch's diff. |

---

## 6. What was found and fixed

Not a changelog. It is here because a reader deciding whether to trust this compiler's
guarantees deserves to know they were *tested*, and by what. The following were found across
the branches merged into this one:

- **Three working secret exfiltrations.** A command endpoint created by a cross-region write
  was ruled on by neither the `Remote` read rule nor the declaration rule, so a `secret durable`
  counter returned its value on the wire. A body that both `give`s and pipes compiled to two
  returns and was labelled by one, so a credential came back wrapped in a one-element list. A
  secret in an `href`/`src` hit no sink at all — it renders no visible text and reaches no
  response body, so the browser sends it to whatever host the value names *before paint*.
- **Three code-injection holes**, one shape: a `format!` writing an opening quote, then
  something off the program, then a closing quote. The generated class getter, the folded
  stylesheet, and the module import clause. The fix is a string type (`js::Quoted`,
  `crates/zdc-codegen/src/js.rs:20`) that only escaping can construct, plus a CI gate
  (`scripts/check-emitted-strings.sh`) forbidding the quote-around-placeholder adjacency.
- **A path traversal through transitive `use`.** `use` joined an unconstrained relative path
  and read any `.zd` on disk — transitively, since a dependency can `use` paths of its own. The
  check now runs *before* the read, and the project root is fixed once per build:
  *a boundary re-based at each hop is not a boundary.* `crates/zdc-hir/src/sandbox.rs`.
- **Five span-aliasing bugs.** One root cause: the resolver copies a component's body per call
  site and keeps the spans, so a `Span` stopped being an identity. Everything keyed on one broke
  at once — including an IFC obligation map where one instance's `secret` place discharged
  another instance's `public` obligation, **a leak with no diagnostic.**
- **Six tests that could not fail.** `assert_eq!(CLOSED_LIST.len(), 6)` on a `[Sink; 6]` is
  `assert_eq!(6, 6)`. A test that looped over zero diagnostics and passed however they were
  treated — **proved vacuous by putting a `panic!` in the loop body and watching it still
  pass.** Another proved by aiming its directory walk at `examples/` and watching the assertion
  hold. Three CI gates now exist for this class. Two are static —
  `scripts/check-vacuous-tests.py` and `scripts/check-wildcard-arms.sh`, which read test
  source for shapes that cannot fail. The third asks the question the other way round:
  `crates/zdc-runtime/tests/mutation.rs` (#160) changes the runtime's JavaScript and checks
  that some suite goes red, because the four gates found in a single day that measured the
  wrong thing all had the shape of real tests and no syntactic rule could have seen them.
  It runs 236 mutants in a job of its own; 58 survive, and `SURVIVORS` in that file says of
  each group whether the coverage is a crate away, the mutant is equivalent, or it is a hole.
  Six are holes.
- **A compiler denial-of-service.** Nested parentheses, `not`, `List of` and indentation each
  recursed without a bound; 26 components expanded to 2²⁶ nodes. Overflowing raises SIGABRT,
  which no `catch_unwind` contains, so `zdc parse` on a truncated or binary file died silently.
  Depth limits and a seeded fuzz harness followed.
- **A `switch` fallthrough.** A statement `when` lowers to a JS `switch`, and a case block that
  does not leave the block runs the next arm's body. An event handler never returns, so
  `when step { First: add 1; Second: add 10 }` **added 11**. Neither static pass could see it:
  both *join* over arms, and a join over-approximates fall-through rather than contradicting it.
  It is visible only in the answer the emitted program computes, which is why the regression
  test drives emission through the embedded engine and asserts the **value**.
- **A durable `Map` that serialised to `{}`.** `JSON.stringify(new Map(...))` is `{}`, so every
  `durable Map` silently stored an empty object, and with no example exercising that path
  nothing noticed. `runtime/wire.js` is the tagged codec that fixed it, and
  **`examples/tally.zd` exists because of this bug.**
- **A durable `List` built with `append` that reached the store as the chain.** `append`
  compiles to a chain of links, so that appending is O(1), and the class carries a `toJSON` that
  flattens it. `encode` walks a value *before* `JSON.stringify` is called, so `JSON.stringify`
  never met the object and the `toJSON` never ran: `[1]` was stored as
  `{"base":[],"item":1,"flat":null}`. **The same family as the `Map` above, in the codec written
  to fix it.** The fix is general rather than a third branch beside `$map`: `encode` now
  consults `toJSON` first, as `JSON.stringify` does, because walking structurally silently
  overrode *every* `toJSON` in the program and the next type with one would have broken
  identically. The regression test reads the bytes in the store, not the value in memory:
  `$force(chain)` is `[1]`, so every in-memory assertion passed with the bug present.
- **A record literal in a pipeline clause that emitted unparseable JavaScript.** A concise arrow
  body beginning with `{` is a block. `map each n to (Point with x is n, y is n)` emitted
  `(n) => { x: n, y: n }`, a `SyntaxError`; with one field it emitted a block holding a labelled
  statement, so every element became `undefined` and **the count was still right**. `check` and
  `build` both exited 0. Five emission sites took `js::arrow_body`, three of them found by
  auditing every arrow the emitter writes rather than by the report.

**Several of these were found by adversarial passes rather than by the test suite**, and that is
the part worth saying plainly. Two of the injection holes were found *twice, independently*, on
branches that share no ancestry — which says the class was found by method rather than by luck.
The `switch` fallthrough and the durable `Map` were both invisible to every static pass and to
every test that existed; each needed someone to run the emitted program and look at the answer.
A test suite that only asserts on emitted *text* would have caught neither.

---

## 7. The span-aliasing class, and what remains

The mechanism is worth stating once, because it decides which keys are safe:
`zdc-resolve`'s `instantiate.rs` copies a component's body per call site, and its `expr` ends in
`self.hir.exprs.alloc(...)` — **`ExprId`, `LocalId` and `DefId` are freshly allocated per
instance, and `Span` is copied verbatim.** So an `ExprId`-keyed map is sound and a `Span`-keyed
map is not.

This file previously claimed *"No `Span`-keyed map remains in `crates/*/src`."* **That claim is
false and has been corrected.** Two remain:

- `crates/zdc-graph/src/split.rs:156` — `mutations_at`, `BTreeMap<(Span, Ctx, DefId), …>`. The
  `Ctx` and `DefId` make collisions much rarer than the original bare-`Span` version, but the
  `Span` is still load-bearing in the key. **This is a real carried-forward hazard** (§5, #3).
- `crates/zdc-graph/src/ifc.rs:1196` — `errors: BTreeMap<Span, GraphError>`. This one is
  **deliberate and documented**: its doc comment says it is "keyed by span so that a view walked
  from two roots reports one error rather than two". It is diagnostic de-duplication, not an
  identity claim, and it is sound for that purpose.

The distinction matters: one is a bug waiting to happen, the other is a design choice with a
written rationale. A blanket "no `Span`-keyed maps" claim hid both.

---

## 8. What was not on this branch, and now is

This section used to record what `feature/docs` was measured *without*. Everything it listed
has since been merged, and the entries are kept rather than deleted so that a reader comparing
this file against an older copy can see which way each one went.

- **Safe markup** (a `Markup` type and a `Prose` element). **Landed** — see §4. The type is
  unrelated to `Text` in both directions, and the only producer is `build markdown`.
- **Build-time file capabilities** (`build read` / `build list` / `build markdown`). **Landed**,
  running in the compiler's own sandbox over the project directory.
- **A blog rendering real markdown from disk.** **Landed.** `examples/content/blog/` exists,
  `blog.zd` checks and builds, and two tests read the rendered headings back out of the DOM.
- **Transactional durable writes.** **Landed.** `runtime/rpc.js` posts a batch to the reserved
  `~atomic` endpoint and the host commits every write of one handler as a single store
  transaction, retried on conflict and refused rather than half-applied.
- **The unlabelled failure channel.** Not previously listed, and worth naming because it is the
  newest: a `Failed` payload now joins over everything the endpoint *reads*, not only over its
  parameters. Its `code` field is the exception, and it is public by construction — the client
  runtime picks it from the transport outcome and never from a byte the server sent. See §4's
  note on `Code`.

---

## 9. The milestone-7 target: what is and is not known

The milestone-7 target is `/Users/msturman00/portfolio`.

**Nothing about its expressibility has been measured on this tree, and this file will not quote
a number as though it had been.**

The history is worth stating because it is the reason for that caution. Successive figures were
quoted as 0% expressible, then a projected 47.6%, then a "verified" 21.3% — and an audit
established that **no figure ever quoted was measured on a tree that actually existed.** Each
was either a measurement of a commit that had since moved on, or a projection over a *union* of
branches that was never built. The one genuinely measured increment was **+4.9 points**, and it
was measured on `feature/numeric` @ `e680dc2` by porting four programs by hand and compiling
them — on top of a **16.4% projection**, which is not itself a measurement. "Projection plus a
measured increment" is not a measurement.

What is known, with provenance:

| Figure | Kind | Where it holds |
|---|---|---|
| 0.0% strict / 2.4% degraded | **measured** | `feature/front-end` @ `387018f`, now many commits stale |
| +4.9 points | **measured** | `feature/numeric` @ `e680dc2`, four hand-ported programs |
| 16.4%, 21.3%, 24.2%, 47.6% | **projections** | no branch — a union of separately-verified compilers that has never been built |
| This branch | **UNVERIFIED** | — |

**Why it is unverified here rather than measured:** the measurement needs a feature inventory of
the target (the 49-feature decomposition and its 13,389-line denominator) and a probe corpus,
neither of which is in this repository. Reproducing it means re-deriving the inventory and
hand-porting against it — a substantial analysis, not a command. I did not do it, so I am not
reporting a number for it.

What can be said honestly: the mainline has since absorbed the element vocabulary (75
built-ins), routing, event payloads, `static` placement, components and modules, and the
standard library — which are between them the majority of the blockers those analyses named. The
true figure is therefore **bounded below by the last real measurement and above by the
projection**, and is unknown within that range. Measuring it is the single most useful
unmeasured number about this project.

---

## 10. `TODO` / `FIXME` / `unimplemented!()` / `todo!()`

**None.** A tree-wide scan of `crates/`, `runtime/`, `scripts/`, `editors/` and `.github/` for
`TODO`, `FIXME`, `XXX`, `HACK`, `unimplemented!` and `todo!(` returns **nine** matches, and every
one is a test fixture named after the *todo-list example*, not a marker:

- `crates/zdc-types/tests/checking.rs` — a `const TODO` holding the source of a `record Todo`,
  used by seven tests.
- `crates/zdc-codegen/tests/dom_parity.rs` — a `const TODO_DRIVER` and its one use.

Unfinished work in this codebase is not marked in comments. It is expressed as a **compiler
diagnostic that names the milestone and the spec section**, which is a better mechanism than a
`TODO` because it is impossible to miss and it fails a build rather than a grep. It does mean a
`TODO` scan is not a useful measure of remaining work; §4 and [`ROADMAP.md`](ROADMAP.md) are.
