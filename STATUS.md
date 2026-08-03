# Status

Where ZDeceptron actually stands, established by running the compiler rather than by reading
prose. Every claim below has a command, a test name, or a file behind it.

**Measured on `feature/docs` @ `f6c6519`**, branched from `feature/front-end` at the same
commit, after the element vocabulary, routing, event payloads, `static` placement, the durable
store, the deploy adapters, the standard library, the foreign-function layer and the
adversarial-input work were merged into it.

`cargo test --workspace` passes with **1360 passing, 0 failing, 4 ignored**, across **18
crates**.

On timing, honestly: test *execution* measured 210 seconds, of which the benchmark suite was
157s. A second run of the benchmark suite alone took 244s. Both runs were on a machine
compiling several other checkouts concurrently, so **treat these as an order of magnitude, not
as a figure** — a few minutes of execution, dominated by the benchmark suite, with a cold
compile costing several times more than that. The benchmark suite is slow because it runs the
workload through an embedded JavaScript interpreter; it is not hung.

Run it in two halves — the whole thing from cold is long enough that it is worth splitting:

```sh
cargo test --workspace --exclude zdc-bench --no-fail-fast   # 1328 passed, 0 failed, 1 ignored
cargo test -p zdc-bench --no-fail-fast                      #   32 passed, 0 failed, 3 ignored
```

Anything that moved after `f6c6519` is not in this file. Several branches carrying security
fixes and features were **unmerged at the time of measurement** — see [§8](#8-what-is-not-on-this-branch).

---

## 1. Milestones

Milestones M0–M12 are defined in the design spec's §12 table. A milestone is marked **done**
only where there is a passing test, a working command, or a file to point at. A milestone with
no evidence is marked not done, regardless of what any other document says.

| # | Milestone | Verdict | Evidence |
|---|---|---|---|
| **M0** | Repository, workspace, CI, spec | ✅ **done** | **18-crate** Cargo workspace. `.github/workflows/ci.yml` runs `fmt --check`, `clippy -D warnings`, and **eight** scripted gates: `check-forbid-unsafe.sh`, `check-wildcard-arms.sh`, `check-vacuous-tests.py`, `check-emitted-strings.sh`, `check-grammar-drift.py`, `check-advisory-exceptions.sh`, `cargo deny`, `cargo audit`, plus `check-dependency-unsafe.sh` via `cargo-geiger`. `cargo test --workspace --no-fail-fast` is a CI step. |
| **M1** | Indentation-sensitive lexer + parser + AST, snapshot tests | ✅ **done** *(one deviation)* | `zdc-lexer` 63 tests including `src/layout.rs`; `zdc-parser` 146 across boundary-focused files; `zdc-ast` 4. `zdc parse examples/hello.zd` exits 0. **Deviation:** the spec's testing table asks for `insta` snapshot tests; `insta` is not a dependency of any crate. The coverage exists as ordinary assertions instead. |
| **M2** | HIR and name resolution | ✅ **done** | `zdc-hir` 17 tests, `zdc-resolve` 111. Two-pass resolver reports every error, not the first: `crates/zdc-resolve/tests/resolution.rs`. `zdc check` runs it. `crates/zdc-resolve/src/sandbox.rs` bounds every path a `use` can reach. |
| **M3** | Type checker (placement-unaware) | ✅ **done** | `zdc-types` 180 tests. Hindley–Milner over `Text`, `Whole`, `Decimal`, `Truth`, `List of T`, `Map of K to V`, `Option of T`, `Remote of T`, records and choices. |
| **M4** | Signal graph, placement coloring, IFC pass + negative test suite | ✅ **done** | `zdc-graph` 86 tests, including the negative leak suite §11 calls the crown jewels. **Verified by building:** `zdc build examples/guestbook.zd` emits a `client.js` containing neither `apiKey` nor `GREETING_API_KEY` — grepped for both in the built bundle, zero hits. |
| **M5** | JS codegen + runtime; client-only programs run in a browser; benchmark suite in CI | ✅ **done**, except the React/Solid arm | `zdc-codegen` 316 tests, `zdc-runtime` 11 (which execute `runtime/signal.test.js` and `runtime/dom.test.js` under an embedded pure-Rust JS engine), `zdc-bench` 32. `BENCHMARKS.md`'s generated region (lines 119–240) is regenerated from the suite and exact-match gated. **Not delivered:** §14A.4's React and SolidJS arms, which need a package manager CI does not have. |
| **M5b** | `when`, `each`, view-position `if`, scoped classes, source maps | ◐ **partial** | Landed: `when` and `each` as anchored holes; view-position `if` (`examples/disclosure.zd`); generated scoped classes (`zdc-codegen/src/styles.rs`, 4 unit tests). **Not landed: source maps.** Verified by grep — no `sourceMap` or `sourcemap` anywhere in `crates/` or `runtime/`. |
| **M6** | `server` placement, RPC generation, `zdc dev` | ✅ **done — emits *and* executes** | `zdc dev` is an in-binary HTTP server with a file watcher, SSE live reload and diagnostic-on-page (`zdc-dev`, 106 tests). `zdc build examples/guestbook.zd` writes `functions/greeting.js`, `functions/visits.js`, `functions/visits.incr.js` and a `manifest.json` — **verified by building and listing the output.** `zdc-host` (48 tests) is §8.2's platform adapter: it binds `$env` and `$store` and runs the emitted handler in the compiler's own `boa_engine`. |
| **M7** | `durable` placement, store, SSE sync | ✅ **done — one deviation** | `zdc-store` (33 tests), a durable store over one total order; `runtime/store.js` and `runtime/wire.js` are the browser half; live sync over a transport seam, `streamTransport` and `pollTransport`. **Evidence is `crates/zdc-host/tests/two_windows.rs` (7 tests):** one window increments, the other is told the new value with no round trip, a reconnecting window is replayed what it missed, and two windows over a reopened database agree. **Deviation:** the store is `redb`, not SQLite — chosen because SQLite would link a C library and forfeit §7's single static binary. |
| **M8** | Style compilation to static CSS | ◐ **partial — its own first layer** | `styles.rs` interns one class per *distinct* declaration set and emits `styles.css` as `runtime/base.css` plus generated rules; signal-dependent styles become `bindStyle`. Its own module doc calls this "the first layer of M8", and the CSS property set is still small. |
| **M9** | Dialect layer, `zdc show --dialect`, round-trip tests | ⬜ **not started** | Only the M1 enabling structure exists: `word_to_kind` is the single keyword table, keyword tokens carry no text, and diagnostics are phrased to take a dialect spelling. No dialect, no `show` subcommand, no round-trip test. |
| **M10** | Demo application | ⬜ **not started** | `examples/` are language samples, not an application. `runtime/demo/` is hand-written JavaScript exercising the runtime, not a ZDeceptron program. Seventeen of eighteen examples now build ([§2](#2-examples)), which is a stronger language claim than it is an application. |
| **M11** | Multi-target deploy (Vercel, AWS Lambda, Cloudflare) with hosted KV | ◐ **partial — generates, never deploys** | `zdc-deploy` (29 tests) and `zdc deploy --target cloudflare\|lambda\|vercel\|deno`, each writing an entry shim, a store binding, a portable router, an endpoint table and platform configuration, plus a capability report naming what that platform cannot do. **Verified by running** `zdc deploy examples/tally.zd --target cloudflare`, which prints the Cloudflare capability report. `tests/portability.rs` pins that handler bodies and router are byte-identical across all four. **Not delivered:** any of this run against a real account. Azure is deliberately absent and `--target azure` says why. |
| **M12** | Writeup | ◐ **partial** | `BENCHMARKS.md` is a substantial, self-critical piece of it. `README.md` and this file exist. There is no writeup document. |

---

## 2. Examples

`./target/debug/zdc check <file>` and `build <file>` over every file in `examples/`, run on
`f6c6519`. **`examples/` holds eighteen files. Seventeen check and seventeen build.**

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
| `examples/page.zd` | ✅ | ✅ | Document head and per-page metadata. |
| `examples/site.zd` | ✅ | ✅ | Declared routes; one bundle per URL. |
| `examples/dungeon.zd` | ✅ | ✅ | A placement loop written with a tail-recursive accumulator. |
| `examples/terminal-help.zd` | ✅ | ✅ | A `"""` block literal in a program. |
| `examples/blog.zd` | ❌ | ❌ | **The only failure.** See below. |

`blog.zd` fails at **`examples/blog.zd:46:54`**:

> Expected a line break after the declaration. Each declaration goes on its own line.
> ZDeceptron has exactly one way to write this.

on `state posts is static List of Post from readMarkdown "content/blog"`. The `static`
placement it was written to need **has landed**; what stops it now is `readMarkdown "…"` — a
call written with a bare argument, which has no syntax — against a function the file's own
header says is undeclared on purpose. There is no `examples/content/` directory on this branch
and no build-time file-reading capability, so the file remains aspirational. It is left in
deliberately: it documents what a real program needs.

Not in `examples/`, but compiled by the test suite:

| File | `check` | `build` |
|---|---|---|
| `crates/zdc-bench/bench/row.zd` | ✅ | ✅ |

---

## 3. Tests

**1360 passing, 0 failing, 4 ignored**, across 18 crates. Per-crate counts below are the
passing figure; they sum to 1360 and reconcile exactly with a static count of `#[test]` and
`#[tokio::test]` attributes in `crates/` (1364, less the 4 ignored). There are no doc-tests.

| Crate | Tests | Note |
|---|---|---|
| `zdc-codegen` | 316 | Plus 1 ignored. See the coverage note below. |
| `zdc-types` | 180 | |
| `zdc-parser` | 146 | Split across boundary-focused files. |
| `zdc-resolve` | 111 | Includes the `use`-sandbox suite. |
| `zdc-dev` | 106 | Self-contained unit suites plus integration files driving the running server. |
| `zdc-lsp` | 88 | |
| `zdc-graph` | 86 | Including the information-flow negative suite. |
| `zdc-cli` | 66 | End-to-end over the real binary, including a seeded fuzz harness. |
| `zdc-lexer` | 63 | |
| `zdc-host` | 48 | §8.2's platform adapter. `tests/two_windows.rs` (7) is the live-sync evidence; `tests/round_trip.rs` (18) and `tests/emitted.rs` (13). |
| `zdc-store` | 33 | The durable store. |
| `zdc-bench` | 32 | Plus 3 ignored. Includes the exact-match `BENCHMARKS.md` gate. |
| `zdc-deploy` | 29 | Four platform adapters and the portability claim. |
| `zdc-diagnostics` | 21 | The inline budget and the `zdc explain` coverage gate. |
| `zdc-hir` | 17 | |
| `zdc-runtime` | 11 | Two of these run the JavaScript suites — further assertions the count above does not see. |
| `zdc-ast` | 4 | |
| `zdc-lib` | 3 | The prelude's surface, pinned so an operation cannot stop being declared unnoticed. |

### The four ignored, each deliberate and each with a written reason

Three are in `crates/zdc-bench/tests/scaling.rs` — the `survey_*` tests, each carrying
`#[ignore = "prints the survey behind BENCHMARKS.md; not a gate"]`. They print the scaling
survey rather than asserting on it, and are reports rather than gates.

The fourth is in `crates/zdc-codegen/tests/emission.rs:1337`, carrying
`#[ignore = "known defect: 'zdc check' accepts this and 'zdc build' refuses it"]`. It records a
real disagreement: `zdc check` accepts and `zdc build` refuses the same program when a `keep`
binder is compared against a function parameter. **It is ignored because it documents a defect
that is not yet fixed, not to make the suite pass.** That is the right use of `#[ignore]` and it
is worth more visible than a green tick.

### Coverage relative to risk: `zdc-codegen/src/server.rs`

**This section previously said `server.rs` carried zero unit tests. That is no longer true and
the figure should not be quoted: it carries 13.** `server.rs` decides what a server endpoint's
source is — its parameters, its wire order, its `$env` reads, its `$store` calls — and it is
now the second-best-covered file in `zdc-codegen/src`.

`zdc-codegen`'s unit tests are no longer confined to four files. They are: `server.rs` 13,
`elements.rs` 10, `js.rs` 10, `styles.rs` 4, `events.rs` 3, `names.rs` 3, `assets.rs` 2,
`intrinsics.rs` 2 — 47 in total, against the 18 this file used to report.

The *shape* of the original concern survives the numbers: server emission is exercised
end-to-end by `zdc-host` (48 tests, which actually execute the emitted handlers) rather than by
`zdc-codegen` alone, so the two crates should be read together when judging whether a wrong
endpoint would be caught. It would be.

---

## 4. What this language cannot do yet

The most useful section in this file. Everything here was re-verified on `f6c6519` rather than
inherited.

### A value cannot become markup

**Verified open on this branch.** There is no `Markup` type in `crates/zdc-types/src/ty.rs` and
no `Prose` element in `zdc-codegen`'s 36-element vocabulary. Every value a program computes
reaches the DOM through `nodeValue`, `setAttribute`, `.value` or `.checked`, none of which
parses HTML — so a string containing `<h1>Hello</h1>` renders as those literal characters.

The one `innerHTML` in `runtime/` is `runtime/dom.js:122`, inside `template()`, and it is **not**
a value path: the compiler interpolates only compile-time string *literals* into it, HTML-escaped.
Its doc comment states this and it is correct. A runtime value cannot reach it.

This is the largest single gap for content-shaped programs, and it is why `blog.zd` would still
not render a post even if its call syntax were fixed.

### It cannot deploy — only generate a deployment

`zdc deploy --target cloudflare|lambda|vercel|deno` writes a complete deployment — entry shim,
store binding, portable router, endpoint table, platform configuration — and prints a capability
report saying what that platform cannot do. **It does not deploy**, and nothing in this
repository has been run against a real account on any of the four. The adapters are checked
against their vendors' documented limits and against each other for portability; they are not
checked against the vendors.

### `Whole` overflow is uncaught on the client path

**Verified by reading the emitter.** `crates/zdc-codegen/src/expr.rs:1016,1018` emits bare
JavaScript `+` and `*` with no guard. So on the client path a `Whole` silently loses integer
precision above 2⁵³ and silently becomes `Infinity` above `Number.MAX_VALUE` (≈1.7977 × 10³⁰⁸).

The narrowing operations *are* guarded — `crates/zdc-codegen/src/intrinsics.rs:274,279` wrap
`floor of` and `round of` in `Number.isFinite` and give an `Option`. **That guard does not
extend to `*` or `+`.** The durable path is covered; the client path is not.

### The emitter is near-quadratic in view size

Documented in its own source: `crates/zdc-codegen/src/analysis.rs:109,116,271` and
`crates/zdc-codegen/src/lib.rs:300` say "quadratic in definitions × pages", "quadratic in
functions", and "split is already quadratic in definitions × roots". `BENCHMARKS.md` measures
it. It is real and documented; at present view sizes it is not felt. **The cost lands per
keystroke in the editor**, because the language server runs the real passes.

### The following syntax does not parse

| Construct | Status, verified on this branch |
|---|---|
| `unique` in a record field | **Refused.** Compiled a probe: *"Expected `is` after the field name."* This is why every list reconciles positionally — see `BENCHMARKS.md`. |
| `readMarkdown "content/blog"` — a call with a bare argument | **Refused.** Every call is written `f with a, b`. This is what stops `blog.zd`. |

**`Row item.name` — a leading argument to `Row`/`Column` — now works.** It was listed here as
the one thing standing between three examples and a successful build; those three examples
(`voting-board.zd`, `leaderboard.zd`, `components.zd`) all build now.

### Other absences, each re-verified

- **No source maps.** Grepped: no `sourceMap` anywhere in `crates/` or `runtime/`. A browser
  stack trace points at generated JavaScript.
- **No dialects.** Only `english`. The enabling structure is in place; no second surface exists.
- **No `record … unique`.** Every list reconciles positionally.
- **No build-time file reading.** No `build read` / `build list` / `build markdown` capability
  on this branch, which is the other half of why `blog.zd` cannot work.

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

The builders that are *not* linear are named at their definitions: `slice`, `dropFirst`,
`startsWith` and `endsWith` concatenate one code point at a time, so they cost O(k²) and O(k)
stack depth in the characters they copy. They are for affixes, which are literals in practice.

---

## 5. Known defects carried forward

Found, verified on this branch, and **not fixed**. Recording rather than fixing is deliberate —
other branches own this code.

| # | Severity | Where | What |
|---|---|---|---|
| 1 | **Medium** | `crates/zdc-lsp/src/server.rs:192-207` | **Go-to-definition resolves an imported span against the entry document.** The handler computes a span, then renders it with `analysis.lines().range(analysis.text(), span)` and returns the *entry* `uri`. A span is a byte offset into the linker's combined buffer, so a name defined in an imported module resolves to that offset in the entry file — the editor jumps to the wrong offset in the wrong file. `zdc-cli` solves exactly this with `Linked::locate` (`crates/zdc-resolve/src/modules.rs:86`, used at `crates/zdc-cli/src/main.rs:397`); the language server does not use it. Anything that follows a definition across a `use` is affected. |
| 2 | **Medium** | `zdc_resolve::load`'s error path, at `crates/zdc-cli/src/main.rs:270-285` | **A parse error in an imported file is rendered against the entry file's text.** The error arm does `std::fs::read_to_string(file)` — the *entry* path — and renders every error against it. The span does not fall inside that text, so `ariadne` prints the message with no location at all: the reader is told what is wrong and not which of their files it is in, or where. The successful path already carries per-file text through `Linked::locate`; only the error path does not. |
| 3 | **Low** | `crates/zdc-graph/src/split.rs:156` | **`mutations_at` still carries a `Span` inside a composite key:** `BTreeMap<(Span, Ctx, DefId), MutCrossing>`. See §7 for why this shape is a hazard. It is the last substantive survivor of the span-aliasing family. |
| 4 | **Low** | `crates/zdc-codegen/src/lib.rs` | Module doc still describes an earlier milestone's scope. |

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
  *a boundary re-based at each hop is not a boundary.* `crates/zdc-resolve/src/sandbox.rs`.
- **Five span-aliasing bugs.** One root cause: the resolver copies a component's body per call
  site and keeps the spans, so a `Span` stopped being an identity. Everything keyed on one broke
  at once — including an IFC obligation map where one instance's `secret` place discharged
  another instance's `public` obligation, **a leak with no diagnostic.**
- **Six tests that could not fail.** `assert_eq!(CLOSED_LIST.len(), 6)` on a `[Sink; 6]` is
  `assert_eq!(6, 6)`. A test that looped over zero diagnostics and passed however they were
  treated — **proved vacuous by putting a `panic!` in the loop body and watching it still
  pass.** Another proved by aiming its directory walk at `examples/` and watching the assertion
  hold. Two CI gates now exist for this class (`scripts/check-vacuous-tests.py`,
  `scripts/check-wildcard-arms.sh`).
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
- `crates/zdc-graph/src/ifc.rs:965` — `errors: BTreeMap<Span, GraphError>`. This one is
  **deliberate and documented**: its doc comment says it is "keyed by span so that a view walked
  from two roots reports one error rather than two". It is diagnostic de-duplication, not an
  identity claim, and it is sound for that purpose.

The distinction matters: one is a bug waiting to happen, the other is a design choice with a
written rationale. A blanket "no `Span`-keyed maps" claim hid both.

---

## 8. What is not on this branch

Recorded because a reader comparing this file against the repository's other branches will
otherwise be misled. At `f6c6519` the following were **unmerged**, verified by checking the tree
rather than by branch ancestry — several branch tips read as unmerged while their content had
already landed by a different route, so ancestry alone is not evidence:

- **Safe markup** (a `Markup` type and a `Prose` element). Absent — see §4.
- **Build-time file capabilities** (`build read` / `build list` / `build markdown`). Absent.
- **A blog rendering real markdown from disk.** Absent: `blog.zd` does not compile and there is
  no `examples/content/` directory.
- **Transactional durable writes.** `crates/zdc-store` has an atomic single-key `incr`, but
  there is no `~atomic` endpoint and no multi-write transaction on this branch.

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
| This branch, `f6c6519` | **UNVERIFIED** | — |

**Why it is unverified here rather than measured:** the measurement needs a feature inventory of
the target (the 49-feature decomposition and its 13,389-line denominator) and a probe corpus,
neither of which is in this repository. Reproducing it means re-deriving the inventory and
hand-porting against it — a substantial analysis, not a command. I did not do it, so I am not
reporting a number for it.

What can be said honestly: the mainline has since absorbed the element vocabulary (36
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
