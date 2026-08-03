# Status

Where ZDeceptron actually stands, established by running the compiler rather than by reading
prose. Every claim below has a command, a test name, or a file behind it.

**Measured on `feature/front-end`** after the `static` placement, the durable store and the
host adapter, `zdc deploy`, the supply-chain gates and `zdc explain`, the adversarial input
work, and the scaling harness were merged into it. `cargo test --workspace` passes with
**1040 tests, 0 failures, 4 ignored**, in about eight minutes — the benchmark suite is 360s of
that and is not hung.

Anything that moved after the last merge recorded here is not in this file.

---

## 1. Milestones

Milestones M0–M12 are defined in the design spec's §12 table. A milestone is marked **done**
only where there is a passing test, a working command, or a file to point at. A milestone with
no evidence is marked not done, regardless of what any other document says.

| # | Milestone | Verdict | Evidence |
|---|---|---|---|
| **M0** | Repository, workspace, CI, spec | ✅ **done** | 14-crate Cargo workspace; `.github/workflows/ci.yml` runs `fmt --check`, `clippy -D warnings`, `scripts/check-forbid-unsafe.sh`, `scripts/check-grammar-drift.py`, and `cargo test --workspace`. |
| **M1** | Indentation-sensitive lexer + parser + AST, snapshot tests | ✅ **done** *(one deviation)* | `zdc-lexer` 53 tests including `src/layout.rs`; `zdc-parser` 112 tests across boundary-focused files; `zdc-ast` 3. `zdc parse examples/hello.zd` exits 0. **Deviation:** the spec's testing table asks for `insta` snapshot tests; `insta` is not a dependency of any crate. The coverage exists as ordinary assertions instead. |
| **M2** | HIR and name resolution | ✅ **done** | `zdc-hir` 8 tests, `zdc-resolve` 90 tests. Two-pass resolver reports every error, not the first: `crates/zdc-resolve/tests/resolution.rs`. `zdc check` runs it. |
| **M3** | Type checker (placement-unaware) | ✅ **done** | `zdc-types` 127 tests, of which `tests/checking.rs` is 67 and `tests/prelude.rs` is 17. Hindley–Milner over `Text`, `Whole`, `Decimal`, `Truth`, `List of T`, `Map of K to V`, `Option of T`, `Remote of T`, records and choices. `tests/examples.rs` is 7 tests over the checked-in examples. |
| **M4** | Signal graph, placement coloring, IFC pass + negative test suite | ✅ **done** | `zdc-graph` 59 tests: `tests/split.rs` 26, `tests/flow.rs` 20 (the negative leak suite §11 calls the crown jewels), `tests/public_contract.rs` 5. `zdc check examples/guestbook.zd` exits 0; the built bundle's `client.js` contains neither `apiKey` nor `GREETING_API_KEY`. |
| **M5** | JS codegen + runtime; client-only programs run in a browser; benchmark suite in CI | ✅ **done**, except the React/Solid arm | `zdc-codegen` 88 tests, `zdc-runtime` 11 (which execute `runtime/signal.test.js`'s 12 cases and `runtime/dom.test.js`'s 35 under an embedded pure-Rust JS engine), `zdc-bench` 21. `BENCHMARKS.md` is regenerated from the suite and exact-match gated. **Not delivered:** §14A.4's React and SolidJS arms, which need a package manager CI does not have. `BENCHMARKS.md` states this itself. |
| **M5b** | `when`, `each`, view-position `if`, scoped classes, source maps | ◐ **partial** | Landed: `when` and `each` as anchored holes (`examples/todo.zd` builds and its `client.js` imports `whenInto` and `eachInto`); generated scoped classes (`zdc-codegen/src/styles.rs`, 4 unit tests). Landed since: view-position `if`, which came in with components — `if` inside a `view` parses, typechecks and emits (`examples/disclosure.zd` uses one). **Not landed:** source maps — no `sourceMap` anywhere in the tree. |
| **M6** | `server` placement, RPC generation, `zdc dev` | ✅ **done — emits *and* executes** | Landed: `zdc dev` (in-binary HTTP server, file watcher, SSE live reload, diagnostic-on-page). Landed: server emission — `zdc build examples/guestbook.zd` writes `functions/greeting.js`, `functions/visits.js`, `functions/visits.incr.js` and a `manifest.json` recording endpoints, wire order and `"durable":["visits"]`; `runtime/rpc.js` is the client half. **Landed since:** execution. `zdc-host` is §8.2's platform adapter — it binds `$env` and `$store` and runs the emitted handler in the compiler's own `boa_engine`, so `POST /_zd/greeting` against `zdc dev` returns a value rather than a 404. |
| **M7** | `durable` placement, store, SSE sync | ✅ **done — one deviation** | Landed: `zdc-store`, a durable store with five operations over one total order; `runtime/store.js` and `runtime/wire.js`, the browser half; live sync over a transport seam, with `streamTransport` for platforms that can hold a stream and `pollTransport` for the two Lambda shapes that cannot. **The evidence is `crates/zdc-host/tests/two_windows.rs`:** one window increments, the other is told the new value with no round trip, a reconnecting window is replayed exactly what it missed, and two windows over a reopened database agree with what was stored. **Deviation:** the store is `redb`, not SQLite — a pure-Rust embedded key-value store, chosen because SQLite would link a C library and forfeit §7's single static binary. |
| **M8** | Style compilation to static CSS | ◐ **partial — its own first layer** | Landed: `styles.rs` interns one class per *distinct* declaration set and emits `styles.css` as `runtime/base.css` plus generated rules; signal-dependent styles become `bindStyle` (`runtime/dom.js:163`). Its own module doc calls this "the first layer of M8". |
| **M9** | Dialect layer, `zdc show --dialect`, round-trip tests | ⬜ **not started** | Only the M1 enabling structure exists: `word_to_kind` is the single keyword table, keyword tokens carry no text, and diagnostics are phrased to take a dialect spelling. No dialect, no `show` subcommand, no round-trip test. |
| **M10** | Demo application | ⬜ **not started** | `examples/` are language samples, not an application; five of the twelve do not build. `runtime/demo/` is two hand-written JavaScript pages exercising the runtime, not a ZDeceptron program. |
| **M11** | Multi-target deploy (Vercel, AWS Lambda, Cloudflare) with hosted KV | ◐ **partial — generates, never deploys** | Landed: `zdc-deploy` and `zdc deploy --target cloudflare\|lambda\|vercel\|deno`, each writing an entry shim, a store binding, a portable router, an endpoint table and platform configuration, plus a capability report naming what that platform cannot do. `tests/portability.rs` pins that the handler bodies and the router are byte-identical across all four. **Not delivered:** any of this run against a real account. The adapters are checked against vendor documentation and against each other, not against the vendors. Azure Functions is deliberately absent and `--target azure` says why. |
| **M12** | Writeup | ◐ **partial** | `BENCHMARKS.md` is a substantial, self-critical piece of it — it contradicts three of the spec's own claims with measurements. `README.md` and this file exist. There is no writeup document. |

---

## 2. Examples

`cargo run -p zdc-cli -- check <file>` and `build <file>` over every file in `examples/`.
**Eleven of twelve check; seven of twelve build.** Two of the twelve — `writing.zd` and
`tally.zd` — arrived with the `static` placement and the durable store.

| File | `check` | `build` | If it fails, why |
|---|---|---|---|
| `examples/hello.zd` | ✅ | ✅ | — |
| `examples/counter.zd` | ✅ | ✅ | — |
| `examples/todo.zd` | ✅ | ✅ | — |
| `examples/guestbook.zd` | ✅ | ✅ | Builds all three placements, emits three server function files, **and runs them** — see §4. |
| `examples/writing.zd` | ✅ | ✅ | `client` + `static`. Its content is computed at build time and inlined, and it emits `rss.xml` into the bundle. Builds with an empty `PATH`. |
| `examples/tally.zd` | ✅ | ✅ | A `durable Map`, which is the only example storing something other than a number. |
| `examples/voting-board.zd` | ✅ | ❌ | One codegen refusal: *"`Row` has no leading argument in `elements.js`, yet four checked-in examples write one. §16.3.6 recommends giving `Row` and `Column` a leading text slot as `Button` already has; until that is ratified in §4.4 the compiler refuses rather than inventing the semantics."* |
| `examples/leaderboard.zd` | ✅ | ❌ | The same `Row` leading-argument refusal. |
| `examples/components.zd` | ✅ | ❌ | The same `Row` leading-argument refusal. |
| `examples/blog.zd` | ❌ | ❌ | *"Expected a line break after the declaration."* at line 38. `static` landed, so that is no longer what stops it; what stops it now is `readMarkdown "content/blog"` — a call written with a bare argument, which has no syntax, against a function the file's own header says is undeclared on purpose. |
| `examples/disclosure.zd` | ✅ | ✅ | A component with its own `state`, rendered. |
| `examples/model.zd` | ✅ | ❌ | A module, imported by `blog.zd`. `build` correctly refuses a file with no `view`; `check` is the command for one. |

Not in `examples/`, but compiled by the test suite:

| File | `check` | `build` |
|---|---|---|
| `crates/zdc-bench/bench/row.zd` | ✅ | ✅ |

---

## 3. Tests

**1040 passing, 0 failing, 4 ignored.** By crate:

| Crate | Tests | Note |
|---|---|---|
| `zdc-codegen` | 149 | See the coverage note below. Includes `tests/static_placement.rs` (16), `tests/live.rs` (12) and `tests/writes.rs` (8). |
| `zdc-types` | 127 | `tests/checking.rs` alone is 67, and `tests/prelude.rs` adds 17. |
| `zdc-parser` | 117 | Split across boundary-focused files, plus `tests/nesting_depth.rs` (12). |
| `zdc-dev` | 102 | Self-contained unit suites plus five integration files, of which `tests/endpoints.rs` (13) drives the running server functions. |
| `zdc-resolve` | 90 | |
| `zdc-lsp` | 87 | |
| `zdc-graph` | 64 | 23 of them the information-flow negative suite. |
| `zdc-cli` | 63 | End-to-end over the real binary, including a seeded fuzz harness (5). |
| `zdc-lexer` | 57 | |
| `zdc-host` | 48 | §8.2's platform adapter. `tests/two_windows.rs` (7) is the live-sync milestone evidence. |
| `zdc-store` | 33 | The durable store, including `tests/restart.rs` (5). |
| `zdc-bench` | 32 | Includes the exact-match `BENCHMARKS.md` gate. |
| `zdc-deploy` | 29 | Four platform adapters and the portability claim. |
| `zdc-diagnostics` | 17 | The 200-character inline budget and the `zdc explain` coverage gate. |
| `zdc-runtime` | 11 | Two of these run the JavaScript suites — 47 further assertions the count above does not see. |
| `zdc-hir` | 8 | |
| `zdc-ast` | 3 | |
| `zdc-lib` | 3 | The prelude's surface, pinned so an operation cannot stop being declared unnoticed. |

**The four ignored, each deliberate and each with a written reason.** Three are
`zdc-bench/tests/scaling.rs`'s `survey_*` tests, which print the survey behind `BENCHMARKS.md`
and are reports rather than gates. The fourth is
`zdc-codegen/tests/emission.rs`'s record of a real disagreement: `zdc check` accepts and `zdc
build` refuses the same program when a `keep` binder is compared against a function parameter.
It is ignored because it documents a defect that is not yet fixed, not to make the suite pass.

### The weakest coverage relative to risk: `zdc-codegen/src/server.rs`

Not the smallest crate — the smallest *ratio of tests to consequence*. `server.rs` is the
module that decides what a server endpoint's source is: its parameters, its wire order, its
`$env` reads, its `$store` calls. It carries **zero unit tests**, and exactly two integration
tests in `crates/zdc-codegen/tests/emission.rs` touch server or durable emission at all
(`a_server_signal_the_browser_never_reads_costs_the_bundle_nothing`,
`a_durable_write_becomes_a_command_and_a_generated_function`).

For comparison, the pass that *decides* what `server.rs` prints — `zdc-graph`'s split — has 26
tests, and the information-flow pass has 20. The decision is well tested; the printing of it is
not. `src/view.rs`, `src/expr.rs`, `src/stmt.rs`, `src/analysis.rs` and `src/lib.rs` likewise
carry no unit tests; all of `zdc-codegen`'s 18 unit tests live in `elements.rs`, `js.rs`,
`names.rs` and `styles.rs`.

This matters more than a normal coverage gap because server emission is the half of the compiler
nothing can execute yet (§4). A test suite is currently the *only* thing that could catch a
wrong endpoint, and there is almost none.

---

## 4. What this language cannot do yet

### It cannot deploy — only generate a deployment

**The gap this section used to describe is closed.** `server` and `durable` programs run:
`zdc-host` is §8.2's platform adapter, it binds `$env` and `$store` and executes the emitted
handler in the compiler's own `boa_engine`, and `zdc dev` uses it, so `POST /_zd/greeting`
returns a value instead of *"not part of this bundle"*. `zdc-store` persists, and
`crates/zdc-host/tests/two_windows.rs` shows two windows moving together over live sync.

What is left is one step further out. `zdc deploy --target cloudflare|lambda|vercel|deno`
writes a complete deployment — entry shim, store binding, portable router, endpoint table,
platform configuration — and prints a capability report saying what that platform cannot do.
It **does not deploy**, and nothing in this repository has been run against a real account on
any of the four. The adapters are checked against their vendors' documented limits and against
each other for portability; they are not checked against the vendors.

### The `/_zd/` surface has one spelling

`runtime/store.js` subscribes to `/_zd/live` and polls `/_zd/poll`, and every server — the dev
server and all four deploy adapters — answers those two names. There was briefly a second
spelling, `/_zd/~watch`, generated by the deploy adapters and aliased by the dev server so that
either resolved. It is retired: the client half is emitted by the compiler, so the client's
spelling is the one that cannot be wrong, and two names for one endpoint is a disagreement
waiting to be found in a browser rather than in a test.
`zdc-deploy/tests/portability.rs::the_router_routes_the_transport_paths_the_client_runtime_requests`
compares the two sides directly, which is what nothing did before.

### It has a standard library, and it is young

§14F's gap is closed: `crates/zdc-lib/prelude/` is seven `.zd` files — text, list, map, number,
option, remote, time — written in ZDeceptron above a `foreign` primitive layer, resolved into
the program's own arenas ahead of it (§17.4.1). `length of`, `at`, `contains` and the `Option`
helpers exist, which is why `leaderboard.zd` now checks. `crates/zdc-lib/src/lib.rs` pins the
library's whole surface so an operation cannot silently stop being declared.

What it is not yet is *complete*. The surface is what the checked-in examples needed, and
§17.4.10's seventeen primitives are the whole of what cannot be written in the language.

### The following syntax does not parse

| Construct | Status |
|---|---|
| `unique` in a record field | *"Expected a line break after the field."* This is why every list reconciles positionally — see `BENCHMARKS.md`. |
| `Row item.name` — a leading argument to `Row`/`Column` | Parses; refused at codegen pending a §4.4 decision. It is the only thing standing between three examples and a successful `build`. |
| `readMarkdown "content/blog"` — a call with a bare argument | Every call is written `f with a, b`. `blog.zd` writes one this way against a function it deliberately never declares, which is what stops that file now that `static` has landed. |

Landed since this section was first written: `use "./m" for X`, `component X with …` and
`children`, `foreign f is anywhere`, `if` in view position, and `state x is static …` together
with its `emitting "rss.xml"` clause. Each has an example that exercises it — `model.zd`,
`disclosure.zd`, the prelude, `disclosure.zd` again, and `writing.zd`.

### Other absences

- **No source maps.** A browser stack trace points at generated JavaScript.
- **No dialects.** Only `english`. The enabling structure is in place; no second surface exists.
- **`foreign` parses but is not lowered.** A `foreign` declaration resolves and typechecks;
  codegen has nothing to emit for a call to one.
- **No `record … unique`.** Every list reconciles positionally.

---

## 5. Defects found during this audit

Reported, not fixed — other branches own this code.

| # | Severity | Where | What |
|---|---|---|---|
| 1 | ~~**Medium**~~ **fixed** | `crates/zdc-codegen/src/lib.rs` (`runtime_files`) | `runtime/rpc.js` was written into *every* bundle unconditionally, including client-only ones that never imported it — the import was conditional, the shipped file was not, and §14A.1's dead-code claim is about bytes shipped. `feature/store` then added `wire.js` and `store.js` on the same unconditional footing. **Fixed during the integration merge:** `Bundle::runtime` records the transitive closure of the modules `client.js` actually imports, computed from the same `RuntimeImports` that decided the import lines, and `runtime_files` takes the bundle. `counter.zd` now ships `signal.js` and `dom.js` and nothing else; `guestbook.zd` ships those plus `rpc.js`, `store.js` and `wire.js`, the last of which no import list names — both pinned in `crates/zdc-codegen/tests/emission.rs`. |
| 2 | **Low** | `crates/zdc-codegen/src/lib.rs:14` | Module doc says *"**What this milestone covers.** `client` placement only — M5a in §16.5"* and *"Everything that would need `zdc-graph` or `zdc-types` emits a diagnostic naming what is missing"*. Both are stale: the crate emits server functions and consumes a `TypeTable`. |
| 3 | ~~**Low**~~ **fixed** | `crates/zdc-dev/src/lib.rs`, `crates/zdc-dev/src/compile.rs` | Two doc comments asserted `zdc dev` is *"Client-only. `server` and `durable` placements are refused by `zdc-codegen`."* They were never refused; they built. They now also **run**, which made the comments false twice over, so they were corrected rather than reported again. |
| 4 | **Low** | `crates/zdc-bench/bench/row.zd:9` | Header comment says *"The list this row belongs to is NOT here, because `each` in the view is refused by this compiler (spec §16.5, M5b)."* `each` is not refused; `crates/zdc-bench/tests/fidelity.rs` pins that it compiles. |
| 5 | **Informational** | `README.md`, before this audit | Its front-page example did not parse — the column-aligned `state` declarations read as an indented block, and it used `Int`, which is not a type. Corrected: the example is now `examples/guestbook.zd`, verified to build. |

The three doc-comment defects (2, 3, 4) share a cause worth naming: they are all statements about
what a *different* crate refuses, written when that was true and never rechecked when it stopped
being. A doc comment that describes another crate's behaviour is a claim no compiler checks.

### Known defects carried forward

Found during integration, out of scope for the branch that found them, and **not fixed**. They
are here so they are not lost.

| # | Severity | Where | What |
|---|---|---|---|
| 6 | **Medium** | `crates/zdc-lsp/src/server.rs:199-207` | Go-to-definition resolves its span against the **entry document** after linking. A span is a byte offset into the linker's combined buffer, so a name defined in an imported module resolves to that offset *in the entry file* — the editor jumps to the wrong offset in the wrong file. `zdc-cli` solves the same problem with `Linked::locate`; the language server does not use it. Anything that follows a definition across a `use` is affected. |
| 7 | **Medium** | `zdc_resolve::load`'s error path | A parse error in an **imported** file is rendered against the *entry* file's text. The span does not fall inside that text, so `ariadne` prints the message with no location at all — the reader is told what is wrong and not which of their files it is in, or where. The successful path already carries per-file text; only the error path does not. |

### The span-aliasing class, re-audited

`ifc.rs` keyed proof obligations on `Span` alone, so two instances of one component shared a key
and one instance's `secret` place discharged the other's `public` obligation — a leak with no
diagnostic. It was re-keyed on `(Span, ObligationKind)`.

The whole tree was swept for the same shape during this merge. The mechanism is worth stating
once, because it decides which keys are safe: `zdc-resolve`'s `instantiate.rs` copies a
component's body per call site, and its `expr` ends in `self.hir.exprs.alloc(...)` — **`ExprId`,
`LocalId` and `DefId` are freshly allocated per instance, and `Span` is copied verbatim.** So an
`ExprId`-keyed map is sound and a `Span`-keyed map is not.

One further offender was found and fixed: `TypeTable::arm_gives` was `HashMap<Span, bool>`, keyed
by a `when` arm's span, with a comment asserting that a span "is unique to its arm". It is not,
after monomorphisation. It is now keyed on `(scrutinee ExprId, arm index)`. The collision was
benign in practice — two instances of one component compute the same answer for the same arm — so
this is a latent trap closed rather than a bug observed. `crossings`, `exprs_of`, `operators`,
`empties`, `whens` and `operator_targets` are all `ExprId`- or `DefId`-keyed and are clear. No
`Span`-keyed map remains in `crates/*/src`.

---

## 6. `TODO` / `FIXME` / `unimplemented!()` / `todo!()`

**None.** A tree-wide scan of `crates/`, `runtime/`, `scripts/`, `editors/` and `.github/` for
`TODO`, `FIXME`, `XXX`, `HACK`, `unimplemented!` and `todo!(` returns nine matches, and every one
is a test fixture named after the *todo-list example*, not a marker:

- `crates/zdc-types/tests/checking.rs` — a `const TODO` holding the source of a `record Todo`, used by eight tests.
- `crates/zdc-codegen/tests/dom_parity.rs` — a `const TODO_DRIVER` and its one use.

Unfinished work in this codebase is not marked in comments. It is expressed as a **compiler
diagnostic that names the milestone and the spec section** — as in `voting-board.zd`'s two
refusals above, each of which cites §16.3.6 and §16.7 and says what has to be decided first. That
is a better mechanism than a `TODO`, because it is impossible to miss and it fails a build rather
than a grep. It does mean this section is not a useful measure of remaining work; §4 and
[`ROADMAP.md`](ROADMAP.md) are.

---

## 7. Documentation claims corrected

Stale claims found and fixed in this pass. Each was verified against the compiler.

| Document | Claim | Reality |
|---|---|---|
| `README.md` | "Name resolution → HIR: ⬜ planned" | `zdc-resolve`, 90 tests. |
| `README.md` | "Type checker (Hindley–Milner): ⬜ planned" | `zdc-types`, 127 tests. |
| `README.md` | "Placement + information-flow pass: ⬜ planned" | `zdc-graph`, 59 tests. |
| `README.md` | "JS codegen, runtime, dev server: ⬜ planned" | `zdc-codegen` 61, `zdc-runtime` 11, `zdc-dev` 80 — and the same README then documented `zdc dev` two sections later. |
| `README.md` | "**The front end works** — `zdc parse` … The type checker, placement pass, and code generator do not exist yet, so nothing runs." | All three exist. Client programs run. |
| `README.md` | "Nothing here compiles a ZDeceptron program end to end yet." | Four of eight examples produce a runnable bundle. |
| `README.md` | Front-page example | Did not parse: column-aligned declarations read as an indented block, and `Int` is not a type. Replaced with a verified one. |
| `README.md` | "`./target/release/zdc parse` … For a client-only program there is also a dev server" | True, but it omitted that non-client programs now build and then cannot run. Stated explicitly now. |
| `BENCHMARKS.md` | "**a list cannot be written in ZDeceptron today**" and its three reasons — `each` refused, `empty` refused, list literal does not lex | All three landed. `crates/zdc-bench/tests/fidelity.rs` pins the opposite in three tests and says in its own module doc that this section "describes a compiler that no longer exists". |
| `BENCHMARKS.md` | "the benchmark's rows have fields and `record` declarations do not exist yet" | `record` declarations exist; `examples/todo.zd` declares one and builds. |
| `BENCHMARKS.md` | "This compiler refuses every non-`client` placement (§16.5, M6), so no server function is emitted." | Server functions are emitted. The measurement is still impossible, for the different reason in §4. |
| `BENCHMARKS.md` | "Anything about `server` or `durable` placement, which this compiler does not emit." | Same. |

**Twelve corrections.** Not corrected, because they are still true and were checked: the entire
generated results region of `BENCHMARKS.md` (exact-match gated, and the gate passes); its bundle
sizes (`hello.zd` 668, `counter.zd` 1,006, `bench/row.zd` 873 bytes — reproduced byte-for-byte
when built from the repository root, and path-length-sensitive because the source path is in the
header comment); `signal.js` + `dom.js` = 18,153 bytes; the twelve regression gates; the "about
two minutes" claim for `cargo test -p zdc-bench` (measured: 134s); and `record … unique` still
being unavailable.
