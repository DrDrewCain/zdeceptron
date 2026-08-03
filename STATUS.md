# Status

Where ZDeceptron actually stands, established by running the compiler rather than by reading
prose. Every claim below has a command, a test name, or a file behind it.

**Measured at commit `f62c9f3` on `feature/front-end`** (115 commits). `cargo test --workspace`
passes with **685 tests, 0 failures**, in about four minutes — the benchmark suite is 134s of
that and is not hung.

Anything that moved after `f62c9f3` is not in this file.

---

## 1. Milestones

Milestones M0–M12 are defined in the design spec's §12 table. A milestone is marked **done**
only where there is a passing test, a working command, or a file to point at. A milestone with
no evidence is marked not done, regardless of what any other document says.

| # | Milestone | Verdict | Evidence |
|---|---|---|---|
| **M0** | Repository, workspace, CI, spec | ✅ **done** | 14-crate Cargo workspace; `.github/workflows/ci.yml` runs `fmt --check`, `clippy -D warnings`, `scripts/check-forbid-unsafe.sh`, `scripts/check-grammar-drift.py`, and `cargo test --workspace`. |
| **M1** | Indentation-sensitive lexer + parser + AST, snapshot tests | ✅ **done** *(one deviation)* | `zdc-lexer` 48 tests including `src/layout.rs`; `zdc-parser` 89 tests across five files; `zdc-ast` 3. `zdc parse examples/hello.zd` exits 0. **Deviation:** the spec's testing table asks for `insta` snapshot tests; `insta` is not a dependency of any crate. The coverage exists as ordinary assertions instead. |
| **M2** | HIR and name resolution | ✅ **done** | `zdc-hir` 8 tests, `zdc-resolve` 57 tests. Two-pass resolver reports every error, not the first: `crates/zdc-resolve/tests/resolution.rs`. `zdc check` runs it. |
| **M3** | Type checker (placement-unaware) | ✅ **done** | `zdc-types` 106 tests, of which `tests/checking.rs` is 64. Hindley–Milner over `Text`, `Whole`, `Decimal`, `Truth`, `List of T`, `Map of K to V`, `Option of T`, `Remote of T`, records and choices. `tests/examples.rs` typechecks five checked-in examples and pins the sixth's failure. |
| **M4** | Signal graph, placement coloring, IFC pass + negative test suite | ✅ **done** | `zdc-graph` 59 tests: `tests/split.rs` 26, `tests/flow.rs` 20 (the negative leak suite §11 calls the crown jewels), `tests/public_contract.rs` 5. `zdc check examples/guestbook.zd` exits 0; the built bundle's `client.js` contains neither `apiKey` nor `GREETING_API_KEY`. |
| **M5** | JS codegen + runtime; client-only programs run in a browser; benchmark suite in CI | ✅ **done**, except the React/Solid arm | `zdc-codegen` 61 tests, `zdc-runtime` 11 (which execute `runtime/signal.test.js`'s 12 cases and `runtime/dom.test.js`'s 35 under an embedded pure-Rust JS engine), `zdc-bench` 21. `BENCHMARKS.md` is regenerated from the suite and exact-match gated. **Not delivered:** §14A.4's React and SolidJS arms, which need a package manager CI does not have. `BENCHMARKS.md` states this itself. |
| **M5b** | `when`, `each`, view-position `if`, scoped classes, source maps | ◐ **partial** | Landed: `when` and `each` as anchored holes (`examples/todo.zd` builds and its `client.js` imports `whenInto` and `eachInto`); generated scoped classes (`zdc-codegen/src/styles.rs`, 4 unit tests). **Not landed:** view-position `if` — `if` inside a `view` is refused with *"Expected a view node, found the keyword `if`"*; source maps — no `sourceMap` anywhere in the tree. |
| **M6** | `server` placement, RPC generation, `zdc dev` | ◐ **partial — emits, never executes** | Landed: `zdc dev` (80 `zdc-dev` tests; in-binary HTTP server, file watcher, SSE live reload, diagnostic-on-page). Landed: server emission — `zdc build examples/guestbook.zd` writes `functions/greeting.js`, `functions/visits.js`, `functions/visits.incr.js` and a `manifest.json` recording endpoints, wire order and `"durable":["visits"]`; `runtime/rpc.js` is the client half. **Not landed:** any execution. See §4. |
| **M7** | `durable` placement, SQLite store, SSE sync | ◐ **partial — split only** | Landed: `durable` is understood by the split and by codegen — a durable write becomes a command endpoint emitting `$store.incr('visits', …)`, and durable reads yield `Remote of T`. **Not landed:** `runtime/store.js` does not exist. No SQLite, no persistence, no data sync. The dev server's SSE stream carries live-reload only. |
| **M8** | Style compilation to static CSS | ◐ **partial — its own first layer** | Landed: `styles.rs` interns one class per *distinct* declaration set and emits `styles.css` as `runtime/base.css` plus generated rules; signal-dependent styles become `bindStyle` (`runtime/dom.js:163`). Its own module doc calls this "the first layer of M8". |
| **M9** | Dialect layer, `zdc show --dialect`, round-trip tests | ⬜ **not started** | Only the M1 enabling structure exists: `word_to_kind` is the single keyword table, keyword tokens carry no text, and diagnostics are phrased to take a dialect spelling. No dialect, no `show` subcommand, no round-trip test. |
| **M10** | Demo application | ⬜ **not started** | `examples/` are language samples, not an application; three of the eight do not compile. `runtime/demo/` is two hand-written JavaScript pages exercising the runtime, not a ZDeceptron program. |
| **M11** | Multi-target deploy (Vercel, AWS Lambda, Cloudflare) with hosted KV | ⬜ **not started** | No adapter, no deploy subcommand. `zdc --help` lists exactly `parse`, `check`, `build`, `lsp`, `dev`. |
| **M12** | Writeup | ◐ **partial** | `BENCHMARKS.md` is a substantial, self-critical piece of it — it contradicts three of the spec's own claims with measurements. `README.md` and this file exist. There is no writeup document. |

---

## 2. Examples

`cargo run -p zdc-cli -- check <file>` and `build <file>` over every file in `examples/`, at
`f62c9f3`. **Five of eight check; four of eight build.**

| File | `check` | `build` | If it fails, why |
|---|---|---|---|
| `examples/hello.zd` | ✅ | ✅ | — |
| `examples/counter.zd` | ✅ | ✅ | — |
| `examples/todo.zd` | ✅ | ✅ | — |
| `examples/guestbook.zd` | ✅ | ✅ | Builds all three placements. Emits three server function files. Does not run — see §4. |
| `examples/voting-board.zd` | ✅ | ❌ | Two codegen refusals. (1) *"`Row` has no leading argument in `elements.js`, yet four checked-in examples write one. §16.3.6 recommends giving `Row` and `Column` a leading text slot as `Button` already has; until that is ratified in §4.4 the compiler refuses rather than inventing the semantics."* (2) *"`at` cannot be compiled yet. The checker says which container this is, but indexing yields `Option of T` (spec §5.4) and the runtime has no `$at` to build one with — that is §14F's standard library, not a type question (spec §16.7 item 5)."* |
| `examples/leaderboard.zd` | ❌ | ❌ | Type errors, and the file's own header says so. (1) *"``at`` gives `Option of Whole`, but `Whole` is expected here."* — `Option` is eliminated only by `when`, a statement, so it cannot be unwrapped inside a sort key. (2) *"`Text` has no fields, so there is no `name` to read."* |
| `examples/blog.zd` | ❌ | ❌ | Parse error at line 19: *"Expected a declaration, found a name. A file contains `state`, `record`, `choice`, `function`, and `view` declarations."* — `use` does not parse. The file is marked ASPIRATIONAL and also needs `static` placement, `foreign`, and a standard library, none of which exist. |
| `examples/components.zd` | ❌ | ❌ | Parse error at line 8, same message — `use` does not parse. Also needs `component` and `children`. Marked ASPIRATIONAL. |

Not in `examples/`, but compiled by the test suite:

| File | `check` | `build` |
|---|---|---|
| `crates/zdc-bench/bench/row.zd` | ✅ | ✅ |

---

## 3. Tests

**685 passing, 0 failing, 0 ignored.** By crate:

| Crate | Tests | Note |
|---|---|---|
| `zdc-types` | 106 | Largest suite; `tests/checking.rs` alone is 64. |
| `zdc-parser` | 89 | Split across five boundary-focused files. |
| `zdc-lsp` | 87 | |
| `zdc-dev` | 80 | Four modules with self-contained unit suites plus three integration files. |
| `zdc-codegen` | 61 | See the coverage note below. |
| `zdc-graph` | 59 | 20 of them the information-flow negative suite. |
| `zdc-resolve` | 57 | |
| `zdc-lexer` | 48 | |
| `zdc-cli` | 44 | End-to-end over the real binary. |
| `zdc-bench` | 21 | Includes the exact-match `BENCHMARKS.md` gate. |
| `zdc-diagnostics` | 11 | |
| `zdc-runtime` | 11 | Two of these run the JavaScript suites — 47 further assertions the count above does not see. |
| `zdc-hir` | 8 | |
| `zdc-ast` | 3 | |

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
carry no unit tests; all of `zdc-codegen`'s 16 unit tests live in `elements.rs`, `js.rs`,
`names.rs` and `styles.rs`.

This matters more than a normal coverage gap because server emission is the half of the compiler
nothing can execute yet (§4). A test suite is currently the *only* thing that could catch a
wrong endpoint, and there is almost none.

---

## 4. What this language cannot do yet

### It cannot run a program that has `server` or `durable` state

This is the single largest gap, and it is invisible from `zdc build`'s exit code.

`zdc build examples/guestbook.zd` succeeds. It writes a client bundle, `runtime/rpc.js`, and one
file per endpoint. Each emitted function's only free names are `$env` and `$store`, which its own
header says are *"injected by the platform adapter (§8.2)"*. **No platform adapter exists.**
`runtime/store.js` does not exist. There is no host, no store, and no `$env`.

`zdc dev` does not close the gap: it serves the generated function *sources* as static assets, so
the browser can read them, but it does not execute them. Verified directly —

```
$ zdc dev examples/guestbook.zd --port 4398
$ curl -X POST http://127.0.0.1:4398/_zd/greeting -d '[""]'
/_zd/greeting is not part of this bundle.
```

`/_zd/<name>` is the URL `runtime/rpc.js` posts to. So a `guestbook.zd` served by `zdc dev`
renders, shows its `Spinner`, and stays there.

### It has no standard library

§14F records this and it is the cause of several failures above. There are no text operations, no
`length`, no `isEmpty`, and no `Option` helpers. `at` correctly yields `Option of T` — the
bounds-checked lookup §5.4 asks for — but `Option` can be eliminated only by `when`, which is a
*statement*, so an index cannot be used inside an expression. `leaderboard.zd` fails on exactly
this, and `voting-board.zd`'s build fails because the runtime has no `$at`.

### The following syntax does not parse

| Construct | Status |
|---|---|
| `use "./m" for X` | Not in the grammar. Blocks `blog.zd`, `components.zd`. |
| `component X with …`, `children` | Not in the grammar. |
| `foreign f is anywhere` (FFI, §14E) | Not in the grammar. |
| `state x is static …` | `static` is not a placement the lexer knows; only `client`, `server`, `durable`. (`zdc-graph` has a `Region::Static` and a `BUILD` root internally, unreachable from source.) |
| `unique` in a record field | *"Expected `is` after the field name."* This is why every list reconciles positionally — see `BENCHMARKS.md`. |
| `if` in view position | *"Expected a view node, found the keyword `if`."* `blog.zd` uses it; the grammar puts `if` under `stmt`. |
| `Row item.name` — a leading argument to `Row`/`Column` | Parses; refused at codegen pending a §4.4 decision. |

### Other absences

- **No source maps.** A browser stack trace points at generated JavaScript.
- **No dialects.** Only `english`. The enabling structure is in place; no second surface exists.
- **No deploy.** No adapters for any target, and no `deploy` subcommand.
- **No cross-file modules.** One file is one program.

---

## 5. Defects found during this audit

Reported, not fixed — other branches own this code.

| # | Severity | Where | What |
|---|---|---|---|
| 1 | **Medium** | `crates/zdc-codegen/src/lib.rs:509` (`runtime_files`) | `runtime/rpc.js` is written into *every* bundle unconditionally, including client-only ones that never import it. `dist/hello.zd/runtime/rpc.js` exists though `hello.zd`'s `client.js` imports only `signal.js` and `dom.js`. `zdc-runtime`'s own doc comment on `RPC_JS` says *"A bundle links against this only when the split found a crossing, so a client-only program still ships nothing it does not use (§16.3.1)"* — the import is conditional, the shipped file is not. §14A.1's dead-code claim is about bytes shipped. |
| 2 | **Low** | `crates/zdc-codegen/src/lib.rs:14` | Module doc says *"**What this milestone covers.** `client` placement only — M5a in §16.5"* and *"Everything that would need `zdc-graph` or `zdc-types` emits a diagnostic naming what is missing"*. Both are stale: the crate emits server functions and consumes a `TypeTable`. |
| 3 | **Low** | `crates/zdc-dev/src/lib.rs:13`, `crates/zdc-dev/src/compile.rs:7` | Two doc comments assert `zdc dev` is *"Client-only. `server` and `durable` placements are refused by `zdc-codegen`."* They are not refused; they build. The real limitation is different and worse — they build and are not executed (§4). |
| 4 | **Low** | `crates/zdc-bench/bench/row.zd:9` | Header comment says *"The list this row belongs to is NOT here, because `each` in the view is refused by this compiler (spec §16.5, M5b)."* `each` is not refused; `crates/zdc-bench/tests/fidelity.rs` pins that it compiles. |
| 5 | **Informational** | `README.md`, before this audit | Its front-page example did not parse — the column-aligned `state` declarations read as an indented block, and it used `Int`, which is not a type. Corrected: the example is now `examples/guestbook.zd`, verified to build. |

The three doc-comment defects (2, 3, 4) share a cause worth naming: they are all statements about
what a *different* crate refuses, written when that was true and never rechecked when it stopped
being. A doc comment that describes another crate's behaviour is a claim no compiler checks.

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
| `README.md` | "Name resolution → HIR: ⬜ planned" | `zdc-resolve`, 57 tests. |
| `README.md` | "Type checker (Hindley–Milner): ⬜ planned" | `zdc-types`, 106 tests. |
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
