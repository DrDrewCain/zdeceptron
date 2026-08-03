# Status

Where ZDeceptron actually stands, established by running the compiler rather than by reading
prose. Every claim below has a command, a test name, or a file behind it.

**Measured at commit `e1ce421` on `feature/front-end`** (138 commits), after components,
modules and the prelude were merged into it. `cargo test --workspace` passes with **798 tests,
0 failures**, in about four minutes — the benchmark suite is 134s of that and is not hung.

Anything that moved after `e1ce421` is not in this file.

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
`e1ce421`. **Nine of ten check; five of ten build.** Two of the ten — `disclosure.zd` and
`model.zd` — arrived with components and modules.

| File | `check` | `build` | If it fails, why |
|---|---|---|---|
| `examples/hello.zd` | ✅ | ✅ | — |
| `examples/counter.zd` | ✅ | ✅ | — |
| `examples/todo.zd` | ✅ | ✅ | — |
| `examples/guestbook.zd` | ✅ | ✅ | Builds all three placements. Emits three server function files. Does not run — see §4. |
| `examples/voting-board.zd` | ✅ | ❌ | One codegen refusal now, not two: *"`Row` has no leading argument in `elements.js`, yet four checked-in examples write one. §16.3.6 recommends giving `Row` and `Column` a leading text slot as `Button` already has; until that is ratified in §4.4 the compiler refuses rather than inventing the semantics."* The second — `at` having no `$at` to build an `Option` with — is gone: the prelude supplies it. |
| `examples/leaderboard.zd` | ✅ | ❌ | Checks since the prelude landed: `at` and the `Option` it yields are library operations now, not missing ones. `build` still refuses on the `Row` leading argument, exactly as `voting-board.zd` does. |
| `examples/blog.zd` | ❌ | ❌ | *"Expected a placement after `is`, found a name."* at line 38 — the file asks for `static`, which is the one placement §14C.3b names and the grammar still does not have. `use` and `foreign` parse now; `static` is what is left. |
| `examples/components.zd` | ✅ | ❌ | Checks since components and modules landed. `build` refuses on the `Row` leading argument, as above. |
| `examples/disclosure.zd` | ✅ | ✅ | A component with its own `state`, rendered. |
| `examples/model.zd` | ✅ | ❌ | A module, imported by `blog.zd`. `build` correctly refuses a file with no `view`; `check` is the command for one. |

Not in `examples/`, but compiled by the test suite:

| File | `check` | `build` |
|---|---|---|
| `crates/zdc-bench/bench/row.zd` | ✅ | ✅ |

---

## 3. Tests

**798 passing, 0 failing, 0 ignored.** By crate:

| Crate | Tests | Note |
|---|---|---|
| `zdc-types` | 127 | Largest suite; `tests/checking.rs` alone is 67, and `tests/prelude.rs` adds 17. |
| `zdc-parser` | 112 | Split across boundary-focused files. |
| `zdc-resolve` | 90 | |
| `zdc-codegen` | 88 | See the coverage note below. |
| `zdc-lsp` | 87 | |
| `zdc-dev` | 80 | Four modules with self-contained unit suites plus three integration files. |
| `zdc-graph` | 59 | 20 of them the information-flow negative suite. |
| `zdc-lexer` | 53 | |
| `zdc-cli` | 45 | End-to-end over the real binary. |
| `zdc-bench` | 21 | Includes the exact-match `BENCHMARKS.md` gate. |
| `zdc-diagnostics` | 11 | |
| `zdc-runtime` | 11 | Two of these run the JavaScript suites — 47 further assertions the count above does not see. |
| `zdc-hir` | 8 | |
| `zdc-ast` | 3 | |
| `zdc-lib` | 3 | The prelude's surface, pinned so an operation cannot stop being declared unnoticed. |

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

### It has a standard library, and it is young

§14F's gap is closed: `crates/zdc-lib/prelude/` is seven `.zd` files — text, list, map, number,
option, remote, time — written in ZDeceptron above a `foreign` primitive layer, resolved into
the program's own arenas ahead of it (§17.4.1). `length of`, `at`, `contains` and the `Option`
helpers exist, which is why `leaderboard.zd` now checks. `crates/zdc-lib/src/lib.rs` pins the
library's whole surface so an operation cannot silently stop being declared.

Text is the part that has been taken past the examples, because a content site is what needed
it: `before`, `after`, `beforeLast`, `afterLast`, `withoutPrefix`, `withoutSuffix`, `replace`,
`indexOf`, `lines` and `unlines` join `contains`, `slice`, `startsWith` and `endsWith`. All ten
are written over `split`, which is the one primitive that walks a whole `Text` in a single step,
so all ten are linear in the input — a title comes out of a ten-thousand character markdown
document, and `crates/zdc-codegen/tests/library.rs` runs one to prove it.

What it is not yet is *complete*. The rest of the surface is what the checked-in examples
needed, and **twenty-one** `foreign` declarations — counted in
`crates/zdc-lib/src/lib.rs::most_of_the_library_is_written_in_zdeceptron`, which is the
authority — are the whole of what cannot be written in the language. §17.4.10's table of
"eighteen" is neither this number nor its own, and its header says seventeen; read the test.

`newline` is no longer among them. It was a primitive for the reason §17.4.10(e) gives — the
lexer's string rule was `"[^"\n]*"`, so the line separator was a `Text` constant the language
could not write — and the `"""` block literal is what closed that without the string escapes
§17.4.10(e) costed as the alternative: a block takes its lines from the source, so a body of
two empty lines is one line break, and `prelude/text.zd` writes exactly that.
`examples/terminal-help.zd` is the same literal in a program. `trim` did **not** follow it out:
a block literal can hold a line break, but `trim` has to name every whitespace character
Unicode has, and a source file can only hold some of them.

The builders that are *not* linear are named at their definitions: `slice`, `dropFirst`,
`startsWith` and `endsWith` concatenate one code point at a time, because there are no local
bindings, so they cost O(k²) and O(k) stack depth in the characters they copy. They are for
affixes, which are literals in practice. §17.4.10 already names the fix and calls it the single
change with the largest return.

### The following syntax does not parse

| Construct | Status |
|---|---|
| `state x is static …` | `static` is not a placement the lexer knows; only `client`, `server`, `durable`. (`zdc-graph` has a `Region::Static` and a `BUILD` root internally, unreachable from source.) This is the one thing still blocking `blog.zd`. |
| `unique` in a record field | *"Expected a line break after the field."* This is why every list reconciles positionally — see `BENCHMARKS.md`. |
| `Row item.name` — a leading argument to `Row`/`Column` | Parses; refused at codegen pending a §4.4 decision. It is the only thing standing between three examples and a successful `build`. |

Landed since this section was first written: `use "./m" for X`, `component X with …` and
`children`, `foreign f is anywhere`, and `if` in view position. Each has an example that
exercises it — `model.zd`, `disclosure.zd`, the prelude, and `disclosure.zd` again.

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
