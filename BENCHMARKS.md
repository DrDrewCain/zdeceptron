# Benchmarks

§14A.4 makes this a deliverable rather than an afterthought:

> "Should be faster" is a claim; the writeup needs numbers. A benchmark suite lands with the
> first code generator (M5) and runs in CI thereafter. […] Regressions in these numbers are
> build failures, not observations.

```sh
cargo test -p zdc-bench          # six to eight minutes; nothing else installed
ZDC_BLESS=1 cargo test -p zdc-bench   # regenerate the table below

# The one gate that is a clock rather than a count (#165). Release only,
# `#[ignore]`d, and run by CI's `asymptotics` job.
cargo test -p zdc-bench --release --test asymptotics -- --ignored --nocapture
```

The workload is the standard js-framework-benchmark one: create 1,000 rows, replace them,
update every 10th, select a row, swap two, remove one, clear, create 10,000, append 1,000
more, clear again.

It is an ordinary workspace test, so CI's existing `cargo test --workspace` step runs it and
fails on it; there is no separate benchmark job to forget to wire up. The one exception is the
timed sweep added by #165, which needs a release build to be measuring the emitter rather than
the compiler's own missing inlining, and therefore does have a job of its own — see **What a
count cannot see** below. Dependencies are built at
`opt-level = 2` even in debug (`[profile.dev.package."*"]` in the root `Cargo.toml`) because
the interpreter is otherwise slow enough that the suite could not be a gate at all.

## What is counted, and what is not

**Operations, not time.** The workload runs in `boa`, a JavaScript interpreter written in
Rust, embedded in a `cargo test`. A wall-clock number from there describes `boa` and not a
browser, so none is reported. What is reported is how many times each arm calls into the DOM,
how many nodes it allocates, how many effects it creates, and how often those effects re-run.
Those are properties of the emitted code rather than of the engine, so they are the same
numbers V8 would produce.

Two quantities are kept per operation, and the distinction carries the whole argument:

- **crossings** — calls made from JavaScript into the DOM. This is what a browser charges for.
- **work** — nodes actually created, linked, unlinked and written, *including* work performed
  inside one call. `cloneNode(true)` is one crossing that allocates a subtree; inserting a
  fragment is one crossing that links every child.

Reporting only crossings would flatter template cloning; reporting only work would hide why it
wins. Both are below, and the per-row table shows the point directly: every arm allocates the
same seven nodes per row, and they differ by more than 5× in how many calls that takes.

**What §14A.4 asks for and this cannot give:**

| Asked for | Status |
|---|---|
| React and SolidJS | **Not built, and decided rather than deferred — [the two arms that are not here](#the-two-arms-that-are-not-here) states the decision and retires the reason this row used to give. Nothing here is a measurement against React or Solid.** In their place stand the code-generator design §16.1 rejected, and hand-written JavaScript in two styles. |
| Cold start and latency of an emitted `server` function | Not measurable here, and the reason has changed again. **A host now exists**: `zdc-host` is §8.2's platform adapter, it binds `$env` and `$store`, and it executes the emitted handler — so the claim that "there is no host to time" is out of date and has been removed. What is still missing is a *representative* thing to time. `zdc-host` runs handlers in the compiler's own `boa` interpreter, so a latency number from it describes `boa` rather than a serverless platform, exactly as the wall-clock caveat above says of the DOM workload. **Cold start in particular is a property of the platform, and `zdc deploy` has never been run against one.** |
| Bundle size against React and Solid equivalents | Our half is measured below; theirs cannot be fetched. |

## The gap, mostly closed

**This section used to say the workload was inexpressible. That is no longer true**, and
`crates/zdc-bench/tests/fidelity.rs` is what says so — the tests that once pinned each refusal
now pin the opposite, so the build fails if any of them regresses.

Three things blocked writing the workload's list in ZDeceptron, and all three have landed:

1. **`each` in the view.** Emitted as an anchored hole reconciled by `eachInto`.
   `fidelity.rs::the_workloads_list_is_expressible` compiles a list in the view and asserts the
   emitted module reaches the runtime's reconciler.
2. **`empty`.** The type checker decides which collection it is;
   `zdc-types` pins this as `empty_knows_which_collection_it_is`.
3. **The list literal.** `starting ["a", "b"]` lexes, parses and typechecks;
   `fidelity.rs::a_list_literal_parses` asserts the two elements.

`record` declarations have landed too, so the benchmark's row fields are now expressible —
`examples/todo.zd` declares a `record`, holds a populated list literal, and builds. Two gaps
remain and they are the reason the arm is still joined by hand:

- **The prelude generates nothing** (§14F). It has landed — `crates/zdc-lib/prelude/` declares
  `first`, `sumOf`, `join`, `slice`, `atOr` and the rest — but every one of them *transforms* a
  list that already exists. There is no range, no repeat, and no way to say "a thousand rows",
  so the workload's data still has to arrive from outside the program.
- **`record … unique` is available now**, so a list of a record that declares an identity
  reconciles on it and everything else still reconciles positionally. The two arms below are
  the same emission with one argument changed — `(item) => item.id` against `$byPosition` —
  and the compiler emits whichever the program's own types call for. `examples/todo.zd` declares one, so its list is
  keyed and every other example's is positional — which is the distribution a real project
  has, rather than one arm being hypothetical.

**So what is the ZDeceptron arm?** `crates/zdc-bench/bench/row.zd` is a real ZDeceptron
program — one benchmark row — compiled by the real pipeline. Its template, the walk to its
holes, and the sequence of bindings attached at them are extracted from the emitted
`client.js` and compared against the row the benchmark renders. Those three things are what a
row costs, and `tests/fidelity.rs` fails the build if they drift apart.

What is still written by hand is the surrounding list: the `eachInto` call, the key function,
and a per-row item getter where `bench/row.zd`'s module reads a module-level signal. That is
now a property of the harness rather than of the compiler — the emitter would supply all three
— but until the harness is rewritten around a real `each`, the honest statement is unchanged:
**the ZDeceptron arm measures the emitter's row and the runtime's reconciler, joined by hand.**

## The five arms

| Arm | What it is |
|---|---|
| **ZDeceptron (positional keys)** | What `zdc build` emits today: one cloned template per row, bindings at the holes, reconciled by `eachInto` with the interim positional key function (§16.6). |
| **ZDeceptron (identity keys)** | The same emission with the key function `record … unique` will supply. Exactly one argument differs. |
| **Direct emission** | What a naive code generator would have produced: nested `elements.js` calls, one runtime call per node. This is the design §16.1 rejected, and it is here so the rejection is measured rather than asserted. |
| **Vanilla JS (node by node)** | Hand-written, `document.createElement` per node, one listener per row — the style js-framework-benchmark's own `vanillajs` entry uses. |
| **Vanilla JS (hand-tuned)** | Hand-written and hand-tuned: one parsed template cloned per row, one delegated listener for the entire list, and a direct DOM edit per operation. This is the floor §14A.2 says we lose to. |

## Fairness rules

- **Every arm must render the same DOM.** After each step the driver digests the element tree,
  its text and its attributes, and the test fails if any two arms disagree. Without this an arm
  could look fast by being wrong — which is precisely the failure §16.6 found in the previous
  reconciler, where update and swap were silent no-ops.
- Every arm is handed the same row objects and performs the same transitions.
- **The vanilla arms are told what changed; the reactive arms are not.** The vanilla arms
  receive the operation (`swap 1 and 998`); the reactive arms receive only the new list and
  must work it out. That is the real difference between hand-written code and a framework, and
  it is deliberately not equalised.
- `class` is compared as the unordered token set a browser treats it as, so the emitter's
  `"zd-row "` and a hand-written `"zd-row"` compare equal.
- One-time template parsing is charged to `mount and render one row`, per arm, so it is not
  billed to the first list operation.

## The two arms that are not here

**DECIDED 2026-08-16, closing #158. §14A.4's React and SolidJS arms are not built. This is a
decision and not a deferral, and the first thing it has to do is retire the reason this file has
been giving, because two of that reason's three clauses are false.**

What was on the record: *"Both need a package manager; CI has no network and §8 forbids a Node
dependency."*

- **CI has a network.** `.github/workflows/ci.yml` installs `cargo-deny`, `cargo-audit` and
  `cargo-geiger` over it in three separate jobs, and the `browser` job drives Chrome. And
  `actions/checkout@v4`, which every job here begins with, is a JavaScript action: the runner
  executes it with a Node of its own. A workflow that wanted a package manager would add a line.
- **§8 does not forbid one here.** Its rule 1 governs *emitted functions* — no `fs`, `path`,
  `Buffer` or `require`, so that one artefact runs on Lambda, Workers, Deno and Vercel. It says
  nothing about what the compiler's own test suite may install. Citing it against a benchmark
  harness was reaching for the nearest rule rather than the governing one.
- **The third clause is the real constraint, and it was already written in the right place.**
  `crates/zdc-bench/Cargo.toml` says the suite "must run under `cargo test` with nothing else
  installed (spec §14A.4), which rules out Node, a browser, and therefore React and SolidJS."
  Being an ordinary workspace test is what makes a regression a build failure with no separate
  job to forget to wire up; the price is that everything the suite needs has to be a crate. An
  arm requiring a package manager makes `cargo test --workspace` conditional on software this
  project does not ship — on three operating systems in CI, and on the machine of anyone who
  followed `CONTRIBUTING.md`'s four lines.

**The toolchain is not the binding constraint anyway, and this is the part that decides it.**
Suppose the bundles were vendored and the build step gone. The arms would still run in `boa`
against `crates/zdc-runtime/runtime/dom-shim.js`, which is 506 lines and says of itself that it
implements *exactly* the surface `dom.js` and `elements.js` touch, and throws on anything else —
deliberately, so that a failure there means the runtime is wrong rather than the shim being
incomplete. React's reconciler and Solid's runtime ask a DOM a great deal more than our two
files do, and every method added to keep an arm running is a DOM this repository invented for
the arm it is measuring. The counts this file reports are offered as properties of the emitted
code that V8 would reproduce; a React number obtained that way would be a property of the shim,
published under React's name. #205 is the standing evidence that the shim and the emitter can agree on a
tree no browser builds.

**The strongest case for building them is Solid's, and it belongs here rather than left for a
reader to make.** Template cloning is Solid's model and §16.1 adopts it as such, so a Solid arm
would measure the distance between the model and this implementation of it — which no arm here
measures. That case is granted. It is not a case for building the arm on this harness: the
distance it would measure is measured in crossings and work, and those cannot be counted
honestly against a shim built to answer only our own two files. It is a reason to build the arm
somewhere that can hold it.

**Set against the five arms that are here, the two that are missing are the two that would change
the least.** The design question §16.1 settled is measured directly by the direct-emission arm,
and the floor §14A.2 concedes we lose to is measured directly by the hand-tuned vanilla arm.
React and Solid land between two arms that are already in the table. The comparison that moves a
decision in this compiler is measured; the two that are missing are the two a reader would quote.

**What would change this answer: a browser-hosted counter, and nothing short of it.** `ci.yml`
already runs Chrome for `crates/zdc-cli/tests/browser.rs`, and the thirteen DOM counters in
`crates/zdc-bench/js/instrument.js` are not tied to the shim in principle — the same operations
are wrappable on the real prototypes before an arm loads, which counts the same things in the
engine the numbers are claimed for. On that harness the two arms are worth their cost and this
decision reverses, under two conditions carried over from here: the bundles are vendored with
their versions and the command that built them recorded, because a checked-in artefact nobody
can rebuild is a number nobody can reproduce; and the wall clock stays out of it, because what
this file reports is counts.

Until then the comparison keeps being stated rather than quietly dropped, which is what
`ROADMAP.md` asked for and is now the smaller half of a decision rather than the whole of one.

## Results

<!-- generated: benchmark results -->

### DOM crossings per operation

Calls from JavaScript into the DOM. Work performed *inside* one call — the subtree `cloneNode(true)` allocates, the children inserting a fragment links, the removals `replaceChildren()` performs — is not a further crossing; it is counted as work below.

| Operation | ZDeceptron (positional keys, today) | ZDeceptron (identity keys, with `unique`) | Direct emission (rejected design) | Vanilla JS (node by node) | Vanilla JS (hand-tuned) |
|---|---|---|---|---|---|
| mount and render one row | 32 | 32 | 28 | 22 | 27 |
| create 1,000 rows | 7000 | 7000 | 22000 | 19003 | 4003 |
| replace 1,000 rows | 3000 | 8000 | 3000 | 19003 | 4003 |
| update every 10th row | 200 | 200 | 300 | 100 | 100 |
| select a row | 1 | 1 | 3 | 1 | 1 |
| swap two rows | 6 | 2 | 6 | 2 | 2 |
| remove a row | 2986 | 1 | 2986 | 1 | 1 |
| clear 999 rows | 999 | 999 | 999 | 1 | 1 |
| create 10,000 rows | 70000 | 70000 | 220000 | 190003 | 40003 |
| append 1,000 to 10,000 | 7000 | 7000 | 22000 | 19002 | 4002 |
| clear 11,000 rows | 11000 | 11000 | 11000 | 1 | 1 |

### Effect runs per operation

A binding re-running. Zero for the vanilla arms, which have no bindings. This is the number that says whether a list operation touched only what changed.

| Operation | ZDeceptron (positional keys, today) | ZDeceptron (identity keys, with `unique`) | Direct emission (rejected design) | Vanilla JS (node by node) | Vanilla JS (hand-tuned) |
|---|---|---|---|---|---|
| mount and render one row | 5 | 5 | 6 | 0 | 0 |
| create 1,000 rows | 3001 | 3001 | 4001 | 0 | 0 |
| replace 1,000 rows | 3001 | 3001 | 3001 | 0 | 0 |
| update every 10th row | 301 | 301 | 301 | 0 | 0 |
| select a row | 4 | 4 | 4 | 0 | 0 |
| swap two rows | 7 | 1 | 7 | 0 | 0 |
| remove a row | 2986 | 1 | 2986 | 0 | 0 |
| clear 999 rows | 1 | 1 | 1 | 0 | 0 |
| create 10,000 rows | 30001 | 30001 | 40001 | 0 | 0 |
| append 1,000 to 10,000 | 3001 | 3001 | 4001 | 0 | 0 |
| clear 11,000 rows | 1 | 1 | 1 | 0 | 0 |

### Text-node writes per operation

`nodeValue` writes that actually reached a text node. `bindText` compares before writing (§16.2 R7), so a re-run that computes the same string costs an effect run and no write.

| Operation | ZDeceptron (positional keys, today) | ZDeceptron (identity keys, with `unique`) | Direct emission (rejected design) | Vanilla JS (node by node) | Vanilla JS (hand-tuned) |
|---|---|---|---|---|---|
| mount and render one row | 2 | 2 | 3 | 0 | 2 |
| create 1,000 rows | 2000 | 2000 | 3000 | 0 | 2000 |
| replace 1,000 rows | 2000 | 2000 | 2000 | 0 | 2000 |
| update every 10th row | 100 | 100 | 200 | 100 | 100 |
| select a row | 0 | 0 | 2 | 0 | 0 |
| swap two rows | 4 | 0 | 4 | 0 | 0 |
| remove a row | 1990 | 0 | 1990 | 0 | 0 |
| clear 999 rows | 0 | 0 | 0 | 0 | 0 |
| create 10,000 rows | 20000 | 20000 | 30000 | 0 | 20000 |
| append 1,000 to 10,000 | 2000 | 2000 | 3000 | 0 | 2000 |
| clear 11,000 rows | 0 | 0 | 0 | 0 | 0 |

### Moves per reorder

`insertBefore` calls one reorder makes. Every row in this measurement has exactly one root, so a move is one call and the count is the size of the move set rather than a proxy for it. **`cursor walk`** is the placement pass `eachInto` used before the longest-increasing-subsequence reconciler landed; it is kept as an arm so that the change is measured rather than remembered, and the two arms are checked for having produced the same order.

| Reorder | moves, LIS reconciler | moves, cursor walk (before) | rows retired |
|---|---|---|---|
| swap two rows at N=100 | 2 | 97 | 0 |
| move the last row to the front at N=100 | 1 | 1 | 0 |
| remove one, add one, swap two at N=100 | 4 | 98 | 1 |
| reverse the whole list at N=100 | 99 | 99 | 0 |
| swap two rows at N=1000 | 2 | 997 | 0 |
| move the last row to the front at N=1000 | 1 | 1 | 0 |
| remove one, add one, swap two at N=1000 | 4 | 998 | 1 |
| reverse the whole list at N=1000 | 999 | 999 | 0 |
| swap two rows at N=5000 | 2 | 4997 | 0 |
| move the last row to the front at N=5000 | 1 | 1 | 0 |
| remove one, add one, swap two at N=5000 | 4 | 4998 | 1 |
| reverse the whole list at N=5000 | 4999 | 4999 | 0 |

### What one row costs, at `create 10,000 rows`

| Per row | ZDeceptron (positional keys, today) | ZDeceptron (identity keys, with `unique`) | Direct emission (rejected design) | Vanilla JS (node by node) | Vanilla JS (hand-tuned) |
|---|---|---|---|---|---|
| DOM crossings | 7 | 7 | 22 | 19 | 4 |
| nodes allocated | 7 | 7 | 7 | 7 | 7 |
| effects created | 3 | 3 | 4 | 0 | 0 |
| event listeners | 2 | 2 | 2 | 2 | 0 |
| attribute writes | 1 | 1 | 3 | 3 | 0 |
| text writes | 2 | 2 | 3 | 0 | 2 |

### `create 10,000 rows` — every counter

| Counter | ZDeceptron (positional keys, today) | ZDeceptron (identity keys, with `unique`) | Direct emission (rejected design) | Vanilla JS (node by node) | Vanilla JS (hand-tuned) |
|---|---|---|---|---|---|
| cloneNode | 10000 | 10000 | 0 | 0 | 10000 |
| createElement | 0 | 0 | 40000 | 40000 | 0 |
| createTextNode | 0 | 0 | 30000 | 30000 | 0 |
| createComment | 0 | 0 | 0 | 0 | 0 |
| insertBefore | 10000 | 10000 | 70000 | 70001 | 10001 |
| removeChild | 0 | 0 | 0 | 0 | 0 |
| replaceChildren | 0 | 0 | 0 | 1 | 1 |
| setAttribute | 10000 | 10000 | 30000 | 30000 | 0 |
| addEventListener | 20000 | 20000 | 20000 | 20000 | 0 |
| text writes | 20000 | 20000 | 30000 | 0 | 20000 |
| **crossings, total** | 70000 | 70000 | 220000 | 190003 | 40003 |
| nodes allocated | 70000 | 70000 | 70000 | 70000 | 70000 |
| effects created | 30000 | 30000 | 40000 | 0 | 0 |
| effect runs | 30001 | 30001 | 40001 | 0 | 0 |
| signals created | 10000 | 10000 | 10000 | 0 | 0 |

### `update every 10th row` — every counter

| Counter | ZDeceptron (positional keys, today) | ZDeceptron (identity keys, with `unique`) | Direct emission (rejected design) | Vanilla JS (node by node) | Vanilla JS (hand-tuned) |
|---|---|---|---|---|---|
| cloneNode | 0 | 0 | 0 | 0 | 0 |
| createElement | 0 | 0 | 0 | 0 | 0 |
| createTextNode | 0 | 0 | 0 | 0 | 0 |
| createComment | 0 | 0 | 0 | 0 | 0 |
| insertBefore | 0 | 0 | 0 | 0 | 0 |
| removeChild | 0 | 0 | 0 | 0 | 0 |
| replaceChildren | 0 | 0 | 0 | 0 | 0 |
| setAttribute | 100 | 100 | 100 | 0 | 0 |
| addEventListener | 0 | 0 | 0 | 0 | 0 |
| text writes | 100 | 100 | 200 | 100 | 100 |
| **crossings, total** | 200 | 200 | 300 | 100 | 100 |
| nodes allocated | 0 | 0 | 0 | 0 | 0 |
| effects created | 0 | 0 | 0 | 0 | 0 |
| effect runs | 301 | 301 | 301 | 0 | 0 |
| signals created | 0 | 0 | 0 | 0 | 0 |

### Bundle size, in bytes

The per-file columns are what the compiler emitted. **`shipped`** is the same bundle after minification, which is what `zdc build` writes and what a browser downloads (#135) — comments and formatting removed, no identifier renamed. `index.html` and `manifest.json` are counted unchanged in both, because neither is minified.

| Program | client.js | boot.js | styles.css | index.html | manifest.json | emitted | shipped | saved |
|---|---|---|---|---|---|---|---|---|
| `examples/hello.zd` | 708 | 208 | 3641 | 748 | 122 | 5427 | 2659 | 2768 |
| `examples/counter.zd` | 1046 | 208 | 3641 | 846 | 142 | 5883 | 3103 | 2780 |
| `crates/zdc-bench/bench/row.zd` | 913 | 208 | 3641 | 740 | 163 | 5665 | 2879 | 2786 |

**`shipped`** is the file a reader downloads — `// $dev` assertions stripped (#140), then minified (#135). **`source`** is the file a contributor opens. The gap between them is how much of this runtime is prose.

| Runtime file | shipped | source |
|---|---|---|
| `runtime/signal.js` | 2647 | 8430 |
| `runtime/dom.js` | 5278 | 13670 |
| `runtime/foreign.js (a gives-view foreign only)` | 2541 | 9434 |
| `runtime/markup.js (a program with Prose only)` | 408 | 2686 |
| `runtime/list.js (a program with an each only)` | 2544 | 10952 |
| `runtime/base.css` | 1088 | 3641 |
| `runtime/elements.js (direct emission only)` | 10625 | 29133 |
<!-- end generated -->

**Two transformations stand between a runtime file and a reader, and the survey measures all
three points rather than subtracting the ends.** Since #140 the runtime carries assertions
inside `// $dev` … `// $end` markers, which `zdc build` strips and `zdc dev` serves. Since #135
what survives is minified — comments and indentation removed, **no identifier renamed**. The
middle column below is the linked set minified but not stripped, so the assertions' cost is
measured in the units that matter, which is bytes a reader would have downloaded:

| Linked set | shipped | + assertions | source | the assertions |
|---|---|---|---|---|
| `signal.js` + `dom.js` + `list.js` (a program with an `each`) | 10,424 | 11,208 | 32,853 | 784 |
| `+ wire.js`, `rpc.js`, `store.js` (a `durable` program) | 20,038 | 21,814 | 68,794 | 1,776 |

Read across a row: about two thirds of what is written is comments and formatting, and the
`// $dev` assertions are a further 784 bytes on top of the smaller set. `list.js` is in both
rows on purpose — #207 moved the reconciler, and the larger of the two assertions, out of
`dom.js` into a module a bundle links only for a program that writes an `each`, so a set
without it measures the mechanism as costing far less than it does.

Every figure in this file, and every size gate in `crates/zdc-bench/tests/`, is the `shipped`
column. Neither change is allowed to move it by accident: a release build of the runtime is a
*subsequence of the lines* a developer reads (`stripping_only_ever_removes_whole_lines`) and a
*subsequence of the characters* of those (`minifying_only_ever_removes_characters`), both in
`crates/zdc-runtime/src/`. What makes that safe rather than merely tidy is that the minified
release build is **executed**: `tests/render.rs` runs it through the same suites as the
development build.

## What the numbers say

### §16.1's headline number is not reproduced

> "Measured on `counter.zd`, 13 effects → 2 […] On the js-framework-benchmark row shape at
> N=1000 it is **4.2× fewer DOM API crossings** and 1,000 fewer effect allocations."

Measured on this row shape: **3.1× fewer crossings** — 7,000 against 22,000 at N=1,000, and
70,000 against 220,000 at N=10,000 — and **exactly 1,000 fewer effect allocations**, 3,000
against 4,000.

The effect claim is exact. The crossing claim is out by a third. Nothing about the decision
changes: 3.1× is a large architectural margin and template cloning is still the right choice.
But 4.2× is not what this row shape produces, and §16.1 should either correct the figure or
state the row it was measured on.

### Direct emission allocates an effect for a constant

Its fourth effect per row is `Button('x', …)`: `text('x')` creates a text node, an effect and a
write for a string that cannot change. Template cloning cannot make this mistake, because a
literal is markup. This is §16.1's "collapse a constant thunk" one-line rule, and it is worth
one effect and one text write per row — 10,000 of each at N=10,000.

### §14A.2's concession holds. Its qualifier does not.

> "A hand-tuned vanilla-JS micro-app will beat us. A fifty-line counter with direct DOM calls
> beats any runtime, including ours. This is the one comparison we lose, **and it does not
> generalise past toy size.**"

The concession holds: hand-tuned vanilla makes 4 crossings per row against the emitted code's
7 — **1.75× at 10,000 rows**.

The qualifier does not. Ten thousand rows is not toy size, and the gap does not fade with
scale; it is constant per row. It is also fully accounted for, and not by the render strategy:
the emitted code and the hand-tuned floor clone the same template and insert it the same way.
The entire difference is **two `addEventListener` calls and one `setAttribute` per row**. A
person writes one delegated listener for the whole list and does not write a class attribute
that never changes; the emitter writes a listener per handler and a binding per hole.

That makes the remaining gap an emission decision rather than an architectural limit, which is
a more useful thing to know than "we lose to vanilla".

### What §14A.2 does not claim, and the measurement supports

Emitted code beats hand-written vanilla that has *not* been hand-tuned — 7 crossings per row
against 19, a **2.7× win** over the node-by-node style js-framework-benchmark's own `vanillajs`
entry uses. §14A.2 concedes the vanilla comparison without qualification; the concession is
owed only to vanilla written by someone who knows this trick.

### §14A.1's "fine-grained updates" holds, until a row moves

> "A signal write touches only the DOM nodes that read it — no virtual DOM diff, no component
> re-render."

Selecting a row costs **1 DOM crossing** — the same as hand-written vanilla, out of 1,000
mounted rows. Updating every 10th row of 1,000 costs 100 text writes, again matching vanilla
exactly, plus 100 class-attribute writes that changed nothing (defect 2 below). That claim is
delivered.

It stops being delivered when rows *shift*. Under the positional keying every list uses today,
removing one row of 1,000 costs **2,986 crossings and 2,986 effect runs** — one `removeChild`
and then 995 rows rewritten in place, because identity is the slot. Under identity keying the
same removal costs **1 crossing**. `record … unique` is not a convenience feature; it is the
difference between O(1) and O(n) on the most common list operation there is.

### §16.6's cost table is corroborated, with one correction and one addition

| §16.6, N=1,000 | measured here |
|---|---|
| update every 10th, positional — 100 `nodeValue` | 100 ✅ |
| update every 10th, `unique` — 100 `nodeValue` | 100 ✅ |
| swap, positional — 4 `nodeValue`, 0 moves | 4 `nodeValue`, 0 moves ✅ |
| swap, `unique` — 997 `insertBefore` | **2** — §16.10's longest-increasing-subsequence reconciler has landed (#207), and 997 was the number it was scheduled against |
| remove row, positional — 1 `removeChild`, 1,988 `nodeValue` | 1 `removeChild`, **1,990** `nodeValue`, **plus 995 `setAttribute` the table does not count** |
| remove row, `unique` — 1 `removeChild`, 0 moves | 1 `removeChild`, 0 moves ✅ |

The addition: **replacing all 1,000 rows costs positional keying 3,000 crossings and identity
keying 8,000.** Every key changes, so identity keying tears down and rebuilds the whole list
while positional keying keeps every slot and rewrites its contents. §16.6 presents `unique`
keying as strictly better once it lands; on this operation it is 2.7× worse, and both numbers
should be in the table.

### Reordering costs the fewest moves it can (§16.10, #207)

§16.10 recorded the reconciler as an outstanding debt: *"Identity-keyed reordering is O(n) moves
until the LIS reconciler lands. Measured, stated, scheduled, and invisible to codegen."* It has
landed. `eachInto`'s placement pass no longer walks the list left to right reinserting every row
it finds out of place; it computes a longest increasing subsequence of the positions the
surviving rows occupy now, leaves every row in it alone, and moves the rest.

The generated table above has the whole grid. The shape of it:

| Reorder of N rows | before | after |
|---|---|---|
| swap two rows, N=1,000 | 997 moves | **2** |
| swap two rows, N=5,000 | 4,997 moves | **2** |
| remove one, add one, swap two, N=5,000 | 4,998 moves | **4** |
| reverse the whole list, N=5,000 | 4,999 moves | 4,999 |

Three things are worth reading off it rather than out of the headline.

**The count stopped depending on the list.** Two moves at N=100, at N=1,000 and at N=5,000 —
which is the only form a claim about an order of growth can honestly take here. One size cannot
tell 2-out-of-1,000 from O(n); three sizes spanning 50× can, and
`the_cost_of_a_reorder_no_longer_grows_with_the_list` is where that is asserted rather than
described.

**The reversal row does not improve, and that is the point.** A reversed list has no increasing
subsequence longer than one row, so n − 1 moves is already minimal and there is nothing to save.
An implementation that reported fewer would be wrong, not fast. The same is true in the other
direction of *move the last row to the front*: one move before and one move after, because the
cursor walk happened to be optimal on that shape. A reconciler that is minimal is minimal
against the best case as well as the worst.

**The before column is measured, not remembered.** `crates/zdc-bench/js/reorder.js` carries the
previous placement pass as a second arm, copied unchanged, for the reason `benchmark.js` carries
the direct-emission arm: a comparison whose "before" is a number in a commit message stops being
checkable the moment anything else changes. Both arms are digested after every shape and the
build fails if they disagree, so neither can be fast by being wrong.

**What it cost in bytes.** About 2,900, and paying for them is why `runtime/list.js` exists —
see the size gate at the bottom of this file, which had five bytes of headroom before this
change and has 4,460 after it.

### Components inline, and the bill is linear (§16.10, #209)

§16.10 also states a dilemma about components and does not say which side this compiler is on:
*"either the compiler inlines bodies into the parent's template, multiplying template bytes and
destroying per-component incremental compilation, or a call site becomes a dynamic hole with its
own clone, degrading toward one clone per component."*

It is the first. Instantiation copies a component's body into the parent's template, so a view
full of components is still one `template()` and one `cloneNode`. Measured over a chain of
components `depth` deep instantiated `count` times, with every argument a hole so that all
`count` copies are the same string:

| markup, in bytes | count = 1 | count = 5 | count = 20 | per instantiation |
|---|---|---|---|---|
| depth 1 | 96 | 376 | 1,426 | 70 |
| depth 2 | 156 | 676 | 2,626 | 130 |
| depth 4 | 276 | 1,276 | 5,026 | 250 |

And the emitted module, which also carries the walk to each hole and the bindings attached
there: 520, 1,232 and 5,634 bytes at depth 1, and 733, 2,297 and 9,894 at depth 4.

**Linear in both, and "multiplying" overstates it.** The marginal cost of one more
instantiation is flat in the count and rises by a constant 60 bytes per level of nesting. That
is the best an inlining strategy can do, and there is no compounding anywhere in the grid.

**A component costs nothing over writing its body out.** At every one of the eighteen points in
that grid, the emission for `k` instantiations is byte-identical to the emission for the same
tree typed out `k` times, up to the four bytes by which the two source paths differ. There is no
per-component wrapper, no anchor pair and no second clone — a call site is not a hole. The
source, meanwhile, is 2.4× shorter at depth 1 and count 20 (26 lines against 62). So the trade
§16.10 describes as a loss is, on this measurement, source compression at zero emitted cost.

**What it does still waste, exactly.** When every copy is the same string — which is what the
table above measures, since every argument there is a hole — the emitter writes all of them. At depth 4 and twenty instantiations that is **4,750 of
5,026 bytes of markup, 95%,** which a shared-template emission would not have needed. That is
not a defect and it is not fixed here: sharing the string trades those bytes for either a second
clone per instantiation, which is the other horn of §16.10's dilemma and costs a DOM crossing,
or a concatenation at module load, and neither has been measured. What is settled is the size of
what is on the table, and it is pinned by
`identical_component_bodies_are_each_written_out_in_full` so that it cannot change in either
direction without this section changing with it.

**The two costs §16.10 names beside the bytes are not measured here.** Per-component incremental
compilation does not exist to be destroyed — there is no incremental pipeline — and the
resolver's per-call-site body copy is the mechanism behind the span-aliasing family in
`STATUS.md` §7, which is a correctness matter rather than a size one.

### Three defects the counts found

1. **`text()` does not compare before writing; `bindText()` does.** §16.2 R7 added the guard to
   one of the two. `text()` is what `elements.js` and the demo pages use, and it is why direct
   emission writes 3 text nodes per row where template cloning writes 2, and 200 on an update
   where template cloning writes 100. One line, in `runtime/dom.js`.
2. **Attribute writes are unconditional except for `value`.** A re-run binding writes the class
   attribute whether or not it changed: 100 needless `setAttribute` calls when updating every
   10th row, 995 when removing one row under positional keys. `setAttribute` already compares
   for `value`; extending that to every attribute is the same one line.
3. **Clearing a list is O(n) crossings.** Every reactive arm retires 11,000 rows with 11,000
   `removeChild` calls; both vanilla arms use one `replaceChildren()`. When a list empties
   entirely, `eachInto` could take the same shortcut.

Defect 2 costs the emitted arm about 1,100 needless attribute writes across this workload and
nothing on create, where the first write is a real one. Defect 1 costs the direct-emission arm
and every consumer of `elements.js`, including the two demo pages, and costs the emitted arm
nothing. Neither is fixed here: this branch adds the benchmark, and changing the runtime under
it would mean shipping a measurement of code that had just been changed to look good.

### Bundle size

`counter.zd` emits 1,006 bytes of JavaScript, and ships 915 of them: since #135 `zdc build`
minifies what it writes — comments and formatting out, no identifier renamed. The runtime it
links against is **7,880 bytes**, `signal.js` plus `dom.js`, as shipped rather than as written;
the two files are 21,901 bytes of heavily commented source, and the gap between those numbers
is the point of the minifier. `counter.zd` has no list, so it does not link `runtime/list.js`;
one that did would add 2,544 bytes. `elements.js` (8,973 bytes shipped, 22,844 written) is
*not* shipped at all: generated code never imports it, which is a placement-independent
instance of the dead-code claim in §14A.1. Direct emission would have shipped it.

Whole bundle, `counter.zd`, everything a first visit downloads: **10,717 bytes** against the
27,022 the same build wrote before #135 — **16,305 bytes saved, 60% of the page**. Almost all
of that is the runtime, because that is where the prose is.

## Against Swift: what this approach costs per line

Swift (SOSP'07, Best Paper) implemented ZDeceptron's exact thesis — security labels driving
automatic client/server partitioning — and the number that made it a warning rather than a
technique is **~800 bytes of JavaScript per line of application source**. A 6-line null program
emitted 73 kB; its largest application, 1,094 lines, emitted 1.21 MB. That is the documented
failure mode of this entire design: the machinery that makes the network boundary invisible
ends up in the bundle.

This section is ZDeceptron measured in Swift's units. `crates/zdc-bench/src/scaling.rs` produces
the numbers; `tests/scaling.rs` gates the ones that can be gated and prints the rest:

```sh
cargo test -p zdc-bench --test scaling                                  # the gates, under a second
cargo test -p zdc-bench --release --test scaling -- --ignored --nocapture   # the surveys below
```

The gates run inside the ordinary `cargo test --workspace` and add well under a second to it.
The surveys are `#[ignore]`d because they are wall-clock and wall-clock is not a build failure.

### Bytes of JavaScript per line of ZDeceptron

Seventeen files are in this table. Comment and blank lines are excluded from the line count,
because these are teaching files whose prose outnumbers their code — `hello.zd` is twelve lines
of which six are comments — and counting them would halve the ratio for free.

**These are not the files `zdc build` accepts.** `zdc build` accepts the examples this survey
refuses. It compiles each file *standalone* — `Resolver::new`, with no prelude and no `use`
linking — because it is measuring what one file's own emission costs. A file that calls a
prelude function or imports a module is refused by the harness and left out of the table, which
is why seventeen of the thirty-four are missing from it and why that is not a statement about
the language.

**Regenerated from `survey_bytes_per_line`.** Two things moved since the last regeneration.
Three examples are new — `keys.zd`, `preferences.zd` and `quote.zd` — and the **right-hand
column fell across every row**, because #135 minifies what a build ships and that column is the
only one measuring shipped bytes. The `client.js` and `bytes/line` columns are deliberately
*not* minified: Swift's 800 bytes per line was its compiler's output, and measuring ours after
a minifier the other side never had would be winning the comparison by changing it. Those two
columns are what the argument below rests on, and they are unaffected.

| Program | file lines | code lines | `client.js` | whole bundle | **bytes/line** | bytes/line charging the whole runtime |
|---|---|---|---|---|---|---|
| `examples/content.zd` | 32 | 13 | 502 | 4,266 | **38** | 31 |
| `examples/counter.zd` | 28 | 17 | 1,006 | 5,407 | **59** | 517 |
| `examples/disclosure.zd` | 52 | 20 | 1,108 | 5,474 | **55** | 444 |
| `examples/events.zd` | 71 | 34 | 1,932 | 6,401 | **56** | 284 |
| `examples/gauge.zd` | 68 | 23 | 1,505 | 5,904 | **65** | 513 |
| `examples/guestbook.zd` | 83 | 29 | 3,113 | 7,859 | **107** | 696 |
| `examples/hello.zd` | 12 | 6 | 668 | 5,049 | **111** | 1,411 |
| `examples/keys.zd` | 109 | 47 | 2,531 | 6,983 | **53** | 232 |
| `examples/layout.zd` | 40 | 9 | 61 | 3,809 | **6** | 0 |
| `examples/model.zd` | 19 | 6 | 767 | 4,515 | **127** | 112 |
| `examples/page.zd` | 93 | 52 | 2,438 | 7,012 | **46** | 245 |
| `examples/preferences.zd` | 71 | 27 | 2,084 | 6,529 | **77** | 489 |
| `examples/quote.zd` | 26 | 14 | 1,344 | 5,747 | **96** | 852 |
| `examples/tally.zd` | 31 | 13 | 1,696 | 6,325 | **130** | 1,461 |
| `examples/todo.zd` | 119 | 65 | 4,566 | 9,134 | **70** | 224 |
| `examples/voting-board.zd` | 40 | 27 | 1,853 | 6,658 | **68** | 660 |
| `crates/zdc-bench/bench/row.zd` | 25 | 12 | 873 | 5,295 | **72** | 721 |

The runtime a rendering program links is `signal.js` plus `dom.js`, **7,880 bytes** shipped,
uncompressed — 21,901 bytes as the two files are written, and the difference is #140's
assertions and #135's comments and indentation. `elements.js` is not in that sum; generated
code never imports it (§16.3.1).

**It is not one number for every program, and the right-hand column above no longer pretends it
is.** The runtime is several modules and a bundle links a subset, computed once as
`Bundle::runtime` and used both to write the import list and to decide which files are copied:

| Module | Linked when |
|---|---|
| `signal.js` | always, by anything that reaches any of the others |
| `dom.js` | the emission reached a rendering helper — every program with a `view` does |
| `list.js` | the program has an `each`, so the emission reached the reconciler |
| `foreign.js` | the program writes a `foreign … gives view` (§14E.1) |
| `rpc.js`, `wire.js` | the split found a crossing |
| `store.js` | the split found a `durable` key |

So the right-hand column differs row by row: `gauge.zd` is charged the 2,541 bytes of foreign
lifecycle that nothing else pays for; `tally.zd` and `guestbook.zd` are charged the ~20,000
bytes of RPC, wire and live-sync they reach; and a module reaching no runtime symbol at all is
charged nothing, which is why `layout.zd` and `content.zd` sit at the bottom. Previously every
row was charged a flat `signal.js + dom.js` whether it linked them or not, which overstated
some rows and understated others by more.

**Which number is honest.** The marginal one — 38 to 130 bytes per line across the table, and a
steady 54 to 56 in the growth series once a program is larger than a toy. (`layout.zd`'s 6 is
not a counter-example so much as a reminder of what the column measures: it is a module of
components with no view of its own, so almost nothing is emitted for it.) The runtime is one file, byte-identical for every program and
every page, cached once by the browser and shared by an entire application; charging all of it
to whichever program is being measured says more about how many programs you divided by than
about the compiler. The right-hand column is still worth printing, because it is what a
single-page application actually downloads, and because it shows the fixed cost dominating
below about 200 lines. The marginal one beats Swift by **6× to 21×** across the table, and by
about **14×** at the scale the growth series reaches.

The right-hand column is the one to state carefully. Before #135 it was above Swift's 800 on
seven of fourteen rows; minifying the runtime moved it under 800 on all but **three of
seventeen** — `tally.zd` at 1,461 and `hello.zd` at 1,411, each a program of a dozen lines or
fewer charged an entire runtime, and `quote.zd` at 852. That is the fixed cost dominating,
which is exactly what the column is for showing, and it is why the marginal number is still the
one the comparison rests on: a smaller fixed cost moves this column and says nothing about what
a line of ZDeceptron costs. The rows with enough source to amortise the runtime are now far
clear of it — `todo.zd` at 224, `keys.zd` at 232, `page.zd` at 245, `events.zd` at 284.

The seventeen files that do not build **in this survey** are refused with reasons, not crashes,
and the survey prints every one: `blog.zd`, `booking.zd`, `components.zd`, `dungeon.zd`,
`edit-distance.zd`, `graph-traversal.zd`, `knapsack.zd`, `leaderboard.zd`, `poker.zd`,
`queens.zd`, `shortest-path.zd`, `site.zd`, `sorting.zd`, `sorting.test.zd`,
`terminal-help.zd`, `timers.zd` and `writing.zd`. Almost all of them refuse only here: the
survey compiles each file on its own, without the prelude beneath it, so anything reaching
`atOr`, `listAt`, `split`, `quotient` or `valueOr` reports an undefined name — `zdc check` and
`zdc build`, which resolve against the prelude the way §17.4.1 says to, accept them.
`writing.zd` is refused for a different reason worth naming: its `static` state needs the build
host, and this harness does not run one. `blog.zd` is the one that does not parse anywhere, and
it is the one example excluded by name in `crates/zdc-cli/tests/resolve_examples.rs`.

### What a routed program duplicates across its pages (#136)

`site.zd` is missing from the table above because the survey compiles each file alone and it
imports a module. It is also the only routed example, so the one program whose cost is *per
document* is the one the per-line survey cannot see. Issue #136 asked what that cost is; nothing
had measured it.

Measured on `site.zd` (5 documents) and on a synthetic ten-route site (12 documents: a home
page, ten parameterised posts, and the not-found page), counting every byte a build writes:

| | `site.zd`, before | `site.zd`, after | ten routes, before | ten routes, after |
|---|---|---|---|---|
| `pages/*.js` | 4,400 | 4,400 | 10,350 | 10,350 |
| `pages/*.css` | 18,625 | 420 | 44,665 | 973 |
| `runtime/base.css` | — | 3,641 | — | 3,641 |
| documents | 3,272 | 3,527 | 7,817 | 8,429 |
| `pages/*.boot.js` | 1,086 | 1,086 | 2,587 | 2,587 |
| **total written** | **27,383** | **13,074** | **65,419** | **25,980** |

**The duplication was the stylesheet, and it was most of the build.** `base.css` is 3,641 bytes
and identical for every document of a site, and it was inlined into every one of them: four
redundant copies in `site.zd` and eleven in the ten-route site, which is 61% of everything that
build wrote. It is now written once and linked, which is what the `runtime/` row is.

**No first visit got slower.** A reader landing on any page still fetches 3,641 + 79 bytes of
CSS. The second page fetches 79, because the base is already in the cache — a 98% drop on every
navigation after the first. The documents grew 51 bytes each, for the second `<link>`, and that
is the whole of what the split costs.

**An unrouted program is untouched**, deliberately: it has one document, so there is nothing to
share the sheet with, and splitting would buy it a second round trip and no bytes. The
null-program figures below, and the generated `styles.css` column above, are the single file
they have always measured.

**What is left is JavaScript, and the fold is what makes it.** After the split, 5,359 of the
ten-route site's 10,350 emitted bytes are still text some other page also carries — 928 of
`site.zd`'s 4,400. Per-page specialisation is the cause rather than an oversight: `/post/one`
and `/post/two` differ only in one folded literal, so ten enumerated values emit ten
near-identical modules that share a template, two helpers and most of `main`. Hoisting those
into a module the pages import would trade §16.3.1's per-document dead-code claim — pinned by
`one_pages_code_is_not_in_another_pages_bundle` — for cache reuse across navigations. That is a
decision about the claim, not an optimisation of it, so it is left to the issue that owns the
claim rather than folded in here.

### The empty-program baseline

Swift's is the single most comparable figure in the two systems, because a null program is
almost entirely machinery.

| | Swift | ZDeceptron |
|---|---|---|
| Source | 6 lines | 6 lines |
| Program's own emission | — | 639 bytes, **570 shipped** |
| Runtime linked (`signal.js` + `dom.js`) | — | 7,880 bytes |
| **JavaScript shipped** | **73,000 bytes** | **8,450 bytes** |

**8.6× smaller**, and the shape is different in a way that matters more than the ratio: 93% of
ours is the shared runtime and 7% is the program. Swift's 73 kB was *per program*. Ours is paid
once for a whole application.

**It was 19,873 bytes and 3.67× until #135 minified what a build writes.** Two thirds of that
figure was comments and indentation in the runtime, and no reader ever needed a byte of it.
Nothing about the code changed: `minifying_only_ever_removes_characters` holds the output to a
subsequence of the input, and the release runtime is executed by the same suites as the
development one. The program's share of the total rose from 3% to 7% for the same reason — the
runtime is prose-heavy and generated code is not — and
`the_null_program_is_a_fraction_of_swifts` now compares the two shipped lengths rather than one
emitted against one shipped, which was two units.

This table also said 3.6× and 20,307 bytes until 2026-08-03, when it was wrong for a duller
reason: the figures were written when the runtime was 19,668 bytes and were never regenerated
as it grew. Unlike the generated section above, this paragraph is prose and no test compares it
to anything, which is exactly why it is worth restating with the date it was measured.

Extrapolating the measured marginal cost to the size of Swift's largest application — 1,094
lines — gives roughly **150 kB** against the 875 kB that 800 bytes per line implies at that
size, a 5.8× margin, or 8.1× against the 1.21 MB `Shop` actually emitted. That is arithmetic on
a measured slope rather than a measured application, and
`at_swifts_largest_app_size_the_runtime_is_already_amortised` says so where it asserts it; it
requires 5×. The margin is a little wider than it was, and the reason is worth being honest
about: minification moved the fixed term, not the slope, so at 1,094 lines almost all of the
projection is still the marginal cost.

**What a `foreign … gives view` costs.** The lifecycle that drives one lives in its own module
(`runtime/foreign.js`, 2,541 bytes shipped, 9,434 written) precisely so that the figures above
stay true of a program that does not use it — §16.3.1's "a bundle ships nothing it does not
use", applied to a feature most programs never write. Charged in full to a program that does
write one, the same null-program comparison is 10,899 bytes, or **6.7× smaller** than
Swift's. That number is
asserted too, by `a_foreign_view_program_links_the_lifecycle_and_still_beats_swift`, so the
split cannot become a way of making the headline smaller than the truth: a null program's
linked set is pinned by name, and a program with a foreign is required to link the module,
import it, and still clear 2×.

The module was 3,424 bytes of source until the contract check landed (#239), and that is the
largest single jump any runtime file has taken here. It is spent on prose: three refusals that
name the declaration, state `mount(node, props) -> { update(props), destroy() }`, and say what
arrived instead. The margin over Swift narrowed from 2.64× to 2.17× to buy it, which was a
deliberate trade and not drift — the alternative was an engine `TypeError` raised inside a
runtime file, for a contract the compiler cannot check and no library satisfies. It recovered
to 2.50× when the reconciler moved to `runtime/list.js` (#207), and #135 took it to 6.7× by
removing what a reader never read. **That last one bought no room for the next feature**: the
comments are gone once, and the file that gets longer next still gets longer. The gate below is
2×.

The smallest program the compiler will accept at all — a `view` and one `Text` — emits **232
bytes**. The program's name is part of that: the emitter writes it into `client.js`, so the
same file measured under two spellings differs by the difference in their lengths. Everything
above is named by repository-relative path, the same way the bundle-size table is.

### Growth is linear

`n` client signals, each declared once and read once in the view, out to a thousand of them:

| signals | code lines | `client.js` | bytes/line | ratio to previous |
|---|---|---|---|---|
| 8 | 18 | 1,138 | 63 | — |
| 16 | 34 | 1,984 | 58 | 1.74 |
| 32 | 66 | 3,696 | 56 | 1.86 |
| 64 | 130 | 7,120 | 54 | 1.93 |
| 128 | 258 | 14,138 | 54 | 1.99 |
| 256 | 514 | 28,602 | 55 | 2.02 |
| 512 | 1,026 | 57,530 | 56 | 2.01 |
| 1,024 | 2,050 | 115,532 | 56 | 2.01 |

Doubling the program doubles the output, to three significant figures, across seven doublings.
The marginal cost per line is **flat at 54–56 bytes** — it does not drift upward at any size
measured. Nesting does not compound either: quadrupling a view's nesting depth from 12 to 48
multiplies the emission by 2.6× — 846 bytes to 2,178 — not 16×. (The parser refuses an indented block nested more
than 64 levels deep, which is its own answer to how deep this can go.)

**Nothing in the emitter is superlinear.** This is the result that would have threatened the
design, and it does not hold.

### Compiler asymptotics: tier splitting *is* the product

§17.2 makes tier splitting reachability over the product of the definition set and the root
set. Routing multiplies the roots — one per page — so whether that product is real is a
question for the routing work, not a theoretical one.

It is real. The generator (`program_with_roots`) gives every root the same chain of definitions
to walk: `defs` chained functions and `roots` server-placed signals each rooted at the head of
the chain, so the source is O(defs + roots) lines and the reachable set is `defs × roots` pairs.
Server placement is what mints a root; `zdc build` refuses to emit a server function (§16.5,
M6), so `split` and `ifc` are timed directly rather than through the whole pipeline.

**Definitions fixed at 32, roots doubling:**

| roots | pairs | `split` | `ifc` |
|---|---|---|---|
| 6 | 222 | 0.08 ms | 0.26 ms |
| 10 | 410 | 0.18 ms | 0.26 ms |
| 18 | 882 | 0.32 ms | 0.28 ms |
| 34 | 2,210 | 0.75 ms | 0.36 ms |
| 66 | 6,402 | 1.73 ms | 0.50 ms |
| 130 | 20,930 | 4.44 ms | 0.83 ms |
| 258 | 74,562 | **16.11 ms** | 2.02 ms |

**Roots fixed at 32, definitions doubling:**

| definitions | pairs | `split` | `ifc` |
|---|---|---|---|
| 37 | 1,258 | 0.14 ms | 0.19 ms |
| 41 | 1,394 | 0.24 ms | 0.22 ms |
| 49 | 1,666 | 0.49 ms | 0.29 ms |
| 65 | 2,210 | 1.00 ms | 0.48 ms |
| 97 | 3,298 | 2.22 ms | 0.90 ms |
| 161 | 5,474 | 4.70 ms | 2.28 ms |
| 289 | 9,826 | **11.15 ms** | 7.86 ms |

**Both doubling:**

| definitions | roots | pairs | `split` | `ifc` |
|---|---|---|---|---|
| 17 | 10 | 170 | 0.04 ms | 0.07 ms |
| 33 | 18 | 594 | 0.16 ms | 0.17 ms |
| 65 | 34 | 2,210 | 0.76 ms | 0.37 ms |
| 129 | 66 | 8,514 | 3.97 ms | 1.11 ms |
| 257 | 130 | 33,410 | 24.58 ms | 5.35 ms |
| 513 | 258 | 132,354 | **194.21 ms** | **69.88 ms** |

Three findings, in order of how much they matter.

1. **`split` is superlinear in roots with definitions held fixed**, and superlinear in
   definitions with roots held fixed. Doubling either factor alone more than doubles the time
   — the last doubling of roots costs 3.6×, the last doubling of definitions 2.4×. Doubling
   both multiplies the time by 4 to 8. It is at least the product §17.2 describes, and above
   about 100 roots it is worse than the product: cost per `(definition, root)` pair rises from
   0.7 µs at 34 roots to 2.9 µs at 258.
2. **The information-flow pass is not sensitive to roots.** Holding
   definitions fixed and multiplying roots by 64 multiplies `ifc` by 7.7. Holding roots fixed
   and multiplying definitions by 64 multiplies it by 42. §17.3 is driven by how much program
   there is, essentially not by how many pages it is split across — which is precisely the
   opposite of `split`.

   This finding used to open "the information-flow pass is not the problem", and the two
   sections below are what that sentence cost. It is a true statement about *roots* and it was
   read as a statement about `ifc`, which is how the pass that was two thirds of a keystroke
   came to be the one nobody looked at. What this survey holds fixed — the prelude, and the
   size of a view — is where `ifc`'s cost actually was.
3. **The constants are small enough that none of this is urgent.** The worst point measured —
   513 definitions and 258 roots, an application with 256 pages — costs 194 ms in `split` and
   70 ms in `ifc`. A 50-page application with 500 definitions lands in the low tens of
   milliseconds. The quadratic is real and it is documented; it is not yet felt.

   Still true of `split`, and it is worth being precise about what "not yet felt" turned out to
   mean, because issue #8 was written to wait for exactly that trigger. It was felt — by the
   language server, at 18.8 ms per keystroke — and when it was, `split` was 0.3 ms of it. The
   pass named here is not the pass that got slow.

`splitting_walks_the_product_of_definitions_and_roots` pins the *shape* — root and definition
counts on both sides of the product — as a deterministic gate, since the timing cannot be one.

### The emitter was the quadratic, and it was not in this table (#8)

Issue #8 asks for the emitter's near-quadratic split to be reduced *once it is felt*, and says
plainly which pass to optimise: the one the measurements name. It named `split`, because
`split` and `ifc` are the two passes the survey above measures and `split` is the one that
grows. Both halves of that turned out to be wrong, and the reason is worth stating before the
numbers, because it is the same reason the cost went unnoticed for so long.

**The survey above varies the wrong axis for this question.** `program_with_roots` varies
definitions and roots, which is what §17.2's product is made of. It holds the *view* at one
`Text` per root. Nothing in this file varied the size of one view — the shape a person
actually types, and the shape a language server re-analyses on every keystroke — so the pass
whose cost is a function of that had no column anywhere. `survey_emitter_growth` is that
column, and it is the survey that found this.

**No byte count could have found it either.** The emitter's walk scheduling produces the same
walk however long it takes to schedule, so every size gate in `crates/zdc-bench/tests/` passed
throughout, `emitted_size_grows_linearly_with_source_size` included. The emission is
byte-identical before and after this change on all thirty-four example programs. There was
never anything wrong with the answer.

What it was doing: `firstChild` and `nextSibling` are the left-child and right-sibling edges,
so a region's node graph *is* a binary tree, and a binary tree has exactly one path between any
two nodes it connects. The scheduler ran a breadth-first *search* for the shortest one — over a
set with a single element in it — allocating two vectors the size of the whole region per call,
and it ran that search once per node already named. Naming a region cost the square of its node
count in searches and the cube of it in allocation. Recording each node's parent and depth once
at build time answers all three questions the scheduler asks: reachability is ancestry, a route
is the parent chain reversed, and the nearest named base is the first named node on the chain
upward.

`survey_emitter_growth`, timing `zdc_codegen::compile` alone with every earlier pass outside
the timed region:

| view nodes | `client.js` | emit, before | emit, after | µs/node before | µs/node after |
|---|---|---|---|---|---|
| 9 | 1,138 | 0.100 ms | 0.100 ms | 11.1 | 11.2 |
| 17 | 1,984 | 0.081 ms | 0.050 ms | 4.8 | 2.9 |
| 33 | 3,696 | 0.235 ms | 0.089 ms | 7.1 | 2.7 |
| 65 | 7,120 | 0.942 ms | 0.145 ms | 14.5 | 2.2 |
| 129 | 14,138 | 5.062 ms | 0.271 ms | 39.2 | 2.1 |
| 257 | 28,602 | 32.907 ms | 0.491 ms | 128.0 | 1.9 |
| 513 | 57,530 | 234.231 ms | 1.065 ms | 456.6 | 2.1 |
| 1,025 | 115,532 | **1,866.575 ms** | **2.048 ms** | 1821.1 | 2.0 |

**It was cubic, not quadratic.** Each doubling of the view multiplied the time by six to eight
over the last four rows — 5.06, 32.9, 234, 1867 — and cubed is what the two factors multiply
out to: naming the k-th node ran k searches, and every search allocated two vectors the size of
the whole region. Issue #8 and the emitter's own source comments both call it quadratic. The
error is the same in both places, and it is a substitution: the *reachability* walk above this
one really is the product §17.2 describes, and the path scheduling underneath it was never that
walk at all.

**After, the marginal cost is flat at 2 µs per node** from 33 nodes to 1,025 — a 31-fold span
with no drift, which is the same form the byte-per-line claim takes above and the only form a
claim about an order of growth can honestly take against a clock. The nine-node row is 11 µs
per node in both columns and is the fixed cost of compiling anything, not a counter-example.

At the largest point measured the emitter is **911× faster**, and a program with 1,025 view
nodes is not a stress test: it emits 115 kB of JavaScript, about twenty-five times the largest
checked-in example.

### What a keystroke costs, and what issue #8 got wrong about it

The trigger for #8 was per-keystroke latency: `crates/zdc-lsp/tests/latency.rs` asserts that
analysing a six-kilobyte file — twice the largest checked-in example — stays inside a ten
millisecond editor budget, and it was failing at 18.8 ms. That test prints three columns, and
its `codegen` column is `full` minus `front end`.

**The subtraction was comparing two different programs.** `Analysis::of` resolves against the
prelude, because the editor has to give the answers `zdc check` gives. `front_end_only` called
`Resolver::new`, which compiles the file alone. §17.4.1's library — 150 definitions and 1,200
expressions, there whatever the file says — was therefore inside one measurement and outside
the other, and the whole of that difference came out attributed to the emitter. It read 98%.

Both sides now compile the same program. With that one line corrected and nothing else changed
— this is what `main` was actually doing:

| rows | bytes | full | front end | codegen | codegen's share |
|---|---|---|---|---|---|
| 1 | 128 | 17.663 ms | 17.227 ms | 0.437 ms | 2.5% |
| 10 | 1,136 | 17.473 ms | 17.057 ms | 0.416 ms | 2.4% |
| 50 | 5,776 | **18.616 ms** | 17.474 ms | 1.141 ms | 6.1% |
| 200 | 23,576 | 26.091 ms | 19.144 ms | 6.947 ms | 26.6% |
| 500 | 59,576 | 94.958 ms | 21.857 ms | 73.100 ms | 77.0% |

Two costs, and they are different in kind. The emitter really was superlinear, and by the
500-row row it is three quarters of the time — #8's premise is sound. But **17 ms of every
keystroke was flat**: the same on a 128-byte file as on a 60-kilobyte one, because what is
being re-analysed is the prelude. Fixing the emitter alone would have moved the row the budget
is asserted on from 18.6 ms to 17.5 ms.

Ten of those seventeen milliseconds were one phase of the flow pass: §17.3.4's witness
reconstruction, which re-walks every function once per parameter to record how a secret would
reach the result, and does it with *traced* walks that re-solve every callee three levels deep.
Its only reader, `witness_for`, returns before consulting a single reconstructed path unless
the argument in hand is concretely secret — and exactly two constructs make a value secret
without being handed one, a `secret` declaration and `environment`. The prelude contains
neither, and nor does the program being edited unless someone wrote one. So on the programs
people mostly edit, all of that work was computed in full and read zero times. It is now run
only where a secret exists to explain; a program that has one still builds every path it did
before.

After both changes:

| rows | bytes | full | front end | codegen | codegen's share |
|---|---|---|---|---|---|
| 1 | 128 | 6.744 ms | 6.081 ms | 0.663 ms | 9.8% |
| 10 | 1,136 | 6.586 ms | 6.061 ms | 0.525 ms | 8.0% |
| 50 | 5,776 | **7.196 ms** | 6.398 ms | 0.798 ms | 11.1% |
| 200 | 23,576 | 9.355 ms | 7.796 ms | 1.559 ms | 16.7% |
| 500 | 59,576 | 14.486 ms | 10.568 ms | 3.918 ms | 27.0% |

**The ten-millisecond budget is met at 7.2 ms**, and
`analysing_a_realistic_file_stays_inside_an_editors_budget` passes rather than being relaxed. A
sixty-kilobyte file went from 95 ms to 14.5 ms, and both columns now grow linearly.

**What is left is the next thing to fix, and it is neither of these passes.** Six of the 7.2 ms
is still flat — the `front end` column reads 6.08 ms on a 128-byte file and 6.40 ms on a
six-kilobyte one — and it is the prelude being resolved, split, typechecked and flow-analysed
from nothing on every keystroke. Broken down by pass with temporary instrumentation, on the
one-row program: `ifc` 5.0 ms, `zdc-types` 1.4 ms, `split` 0.2 ms, resolve 0.3 ms. That last
breakdown is the one figure in this section no checked-in test reproduces; the 6.08 ms it adds
up to is printed by `latency.rs` on every run. No further work on the emitter can reach any of
it. What reaches it is somewhere to keep an answer between keystrokes, which this compiler does
not have.

### The fold ceiling

There are no local bindings in ZDeceptron, so a fold cannot carry an accumulator through a
loop; §17.4.9's technique is index recursion and stack depth is therefore linear in the input
(§17.4.10). Measured by bisection against the same interpreter the rest of this suite uses:

**510 elements fold; 511 do not.** The failure is `RuntimeLimit: exceeded maximum number of
recursive calls` — a stated failure with the recursing function named, not a wrong answer and
not a silent one.

Two things that number is not. It is not the language's limit: it is `boa`'s recursion budget,
and a browser's is roughly an order of magnitude larger. And it is not measured through the
prelude, which does not exist on this base — `sumOf` and the rest live on `feature/prelude`,
where a `depth` test records 200 elements folding and 4,000 exhausting the interpreter. What is
measured here is the emitted *shape*, one self-call per element, in isolation; a real `sumOf`
carries more frames per element, so 510 is a ceiling on the ceiling. Both measurements agree on
the only part that is a property of the language: **the depth grows with the input at all.**

### Template cloning against `elements.js`: the counts stand, the clock cannot

§16 chose template cloning over calling `elements.js`, and `runtime/elements.js` still exists
and is still built (it is not shipped — generated code never imports it). The crossing counts
above already measure the decision and support it: 3.1× fewer DOM crossings and one fewer
effect per row. **The wall-clock comparison was attempted and is reported here as not
measurable, which is a more useful result than a number would have been.**

Timing 1,000 rows through both paths in `boa`, five samples each, each in a fresh context, gave
`elements.js` at 0.80× the cost of template cloning on one run and 1.25× on the next, from
byte-identical code. Two reasons, and the second is structural:

- The spread between samples is comparable to the difference between the arms.
- **The DOM shim implements `cloneNode(true)` in JavaScript** — a recursive walk calling
  `createElement` and `setAttribute` per node. In a browser it is one native call into C++.
  Template cloning's entire advantage is moving work across that boundary, so a shim that has
  no such boundary cannot show it. This is not `boa` being slow; it is the measurement being
  structurally incapable of the comparison, and running it faster would not fix it.

The counts are what this environment can honestly say about §16's central decision, and they
say it clearly.

### One more thing the environment cannot do

Signal fan-out could not be timed at all. Above roughly ten to twenty-five effects subscribed
to one signal, `boa` aborts the *process* with a Rust-level `BorrowMutError` inside its own
`Set` builtin, non-deterministically and whether or not the writes are batched. That is an
engine defect rather than a JavaScript exception, so it cannot be caught or worked around from
the harness, and no propagation timing appears above. The effect *counts* in the tables at the
top of this file are unaffected — they come from a workload that does not hit it.

## The regression gates

`tests/benchmark.rs` fails the build on all of the following. The counts are deterministic —
there is no timing in this table — so none of these can flake; the headroom exists so that a
deliberate, harmless change to the row shape does not fail the build, while a change of
architectural significance does. The golden table above is the exact-match gate: **any** change
to **any** number fails until it is regenerated and reviewed.

There is one gate that is *not* in this table and is not a count, and the reason for it is
below under **What a count cannot see**.

| Gate | Measured | Threshold | Why that threshold |
|---|---|---|---|
| Every arm renders the same DOM at every step | identical | identical | No headroom is possible or wanted. An arm that is wrong is not fast. |
| Template cloning vs direct emission, crossings | 3.1× | ≥ 2× | The ratio depends on how many holes and handlers a row has. At 2× the architectural claim is still falsifiable, and adding an attribute to the row does not fail the build. |
| Template cloning vs direct emission, effects | 1 per row fewer | ≥ 1 per row fewer | §16.1's own claim, stated per row. |
| Effects per row | 3 | ≤ 3 | The row has three holes. More means a binding is being created the emitter does not need. |
| Hand-tuned vanilla is still the floor | 1.75× | floor < emitted ≤ 2.5× | Two-sided on purpose. If the emitted code ever *beats* the floor, §14A.2 is wrong and the spec needs correcting, not the test relaxing. The 2.5× ceiling catches the loss widening by an order of magnitude without failing on one extra per-row write. |
| Emitted vs node-by-node vanilla | 2.7× | ≥ 2× | Measured margin with room for a row-shape change. |
| Identity-keyed removal | 1 crossing | ≤ 2 | This is what §16.2 R1's two-pass retire bought. |
| Identity-keyed swap | 2 moves | exactly 2 | Two rows change places, so two rows move. Pinned exactly: a minimal move set is a fact about the permutation and not about the row shape, so there is nothing for headroom to absorb. §16.6's 997 was the cursor walk this replaced, and it is still measured beside it. |
| A reorder's move count against the list's length | 2 moves at N=100, 1,000 and 5,000 | equal at all three | The order-of-growth claim, in the only form a benchmark can state one. One size cannot tell O(1) from O(n); three spanning 50× can. |
| The cursor walk is still the linear arm | 97, 997 and 4,997 moves | 50× over 50× | The before column has to keep measuring the algorithm that was replaced, or the comparison drifts into measuring two versions of the same thing. |
| Positional-keyed removal | 2,986 crossings | 1,000–4,000 | Bounded below as well: if it drops, §16.6's account of positional keying is out of date and this file is wrong. |
| Clearing a list | 11,000 `removeChild` | exactly 11,000 | Pinned so the O(n) teardown stays visible rather than being forgotten. |
| Emitted `client.js` | ≤ 1,006 bytes | ≤ 2,048 | Roughly double, so a code generator that starts emitting a helper per node fails. |
| `signal.js` + `dom.js` | 19,234 bytes | ≤ 24,576 | Not a byte-count contest — a check that no framework has grown inside the runtime. It fell by 4,455 bytes when the reconciler moved to `list.js`, which a program with no `each` no longer downloads. |

The binding constraint is not the row above but
`scaling.rs::the_null_program_is_a_fraction_of_swifts`, which asserts `shipped * 3 <  73,000`
where `shipped` is the null program's `client.js` plus the runtime. Measured: 639 + 19,234 =
19,873, and 19,873 × 3 = 59,619 against 73,000. **The gate passes with about 4,460 bytes of
headroom in shipped JavaScript.**

**It had five.** Before the reconciler moved out of `dom.js`, the same sum was 639 + 23,689 =
24,328 against a ceiling of 24,333 — the tightest this figure has ever been, and tight enough
to be load-bearing on prose: adding the safe-markup path to `runtime/dom.js` spent most of what
was left, and a ten-line doc comment added to that file during integration was by itself enough
to fail the gate. The longest-increasing-subsequence reconciler (#207) is about 2,900 bytes of
source, so it could not have landed at all without either failing this gate or being written
without comments to fit under it.

What paid for it is the split the runtime already uses twice: `foreign.js` and `markup.js` are
separate files because a DOM-owning foreign and a `Prose` are optional, and a list is optional
in exactly the same sense. `runtime/list.js` holds `each`, `eachInto` and the key function, and
`Bundle::runtime` links it only for a program that emits an `eachInto` —
`a_null_program_links_two_runtime_files` still pins the null program's set to `signal.js` and
`dom.js`, which is what makes this shipping less rather than moving the measurement. A program
*with* a list is 4,314 bytes larger than before, which is the reconciler's real cost and is
charged to the programs that use one.

**Measure a runtime addition against this gate before it lands, not after** — and that includes
comments, because nothing in this pipeline strips them.

### What a count cannot see (#165)

Every gate above weighs output, and **a pass can become arbitrarily slower without changing one
byte of it**. Issue #8 is the worked example rather than the hypothetical one: the emitter's
path scheduler ran a breadth-first *search* for the shortest walk between two nodes of a binary
tree — over a set with one element in it — once per node already named, which made scheduling
cubic in the size of a region. Scheduling 1,025 view nodes took **1,866 ms**. The walk it
scheduled came out byte-identical however long the scheduling took, so every gate in the table
above passed, on every run, for as long as the code was in the tree.

Nothing in this repository measured time in a way that could fail. `crates/zdc-bench/tests/
asymptotics.rs` is that gate, and `.github/workflows/ci.yml`'s `asymptotics` job is what runs
it.

**It does not assert a time.** A wall-clock threshold on a runner nobody controls is a coin
flip, and a flaky gate is worse than no gate — it teaches people to re-run until green, and
then the run that was telling the truth gets re-run too. The evidence for that is in this
repository: `crates/zdc-lsp/tests/latency.rs` holds analysis of a six-kilobyte file to ten
milliseconds, concedes in its own comment that a ratio "would fail on a loaded machine and
teach everyone to ignore it", and then guards the assertion with an early return on a debug
build. CI builds in debug. **That budget has never once been enforced by CI.**

What is asserted is the **shape**: over a 16× sweep of program size, how much the cost *per
view node* changes between the small end and the large end. Call it the inflation.

| what the pass is | inflation over a 16× sweep |
|---|---|
| linear | 1 |
| `n log n` | 2.7 |
| quadratic | 16 |
| cubic | 256 |

A runner that is uniformly twice as slow multiplies both ends by two, and two cancels in a
ratio of two samples taken in one process on one machine seconds apart. What does not cancel is
a pass whose cost per node grows with the program — which is exactly the class #8 belongs to
and exactly the class no count here can see. The estimator is the **least** of several runs
rather than the mean or the median, because interference on a shared machine only ever *adds*
time to a sample: the uncontended cost is the minimum, and everything above it is that sample's
noise rather than the pass's cost.

Measured on this base, with `program_with_signals` — one `Column` of `n` `Text` nodes, which is
the generator `survey_growth` above already sweeps to 1,024:

| pass | 64 nodes | 1,024 nodes | inflation | recorded | fails past |
|---|---|---|---|---|---|
| front end | 3.5 µs/node | 5.3 µs/node | **1.5×** | 1.45 | 4.35 |
| emitter | 13.7 µs/node | 1,752 µs/node | **127×** | 125 | 375 |

The front-end row is what a healthy axis looks like. **The emitter row is a defect, recorded.**
#8 is open on this base and `Graph::route` is still a breadth-first search; a gate that refused
to write the number down would only be a permanently red build. What the recorded number buys
today is that the emitter cannot get *worse* unnoticed. What it buys on the day #8 lands is a
linearity gate, obtained by editing one number — and the gate fails from *below* as well, so
that editing it is part of the change that earned it rather than something to get round to.
That is the same discipline as regenerating the golden table.

Five consecutive runs on one machine gave the emitter 122, 130, 138, 115 and 140, and the front
end 1.44, 1.43, 1.44, 1.49 and 1.25 — about ±12%. The 3× band is not that spread with a safety
factor bolted on; it is the smallest step that cannot be anything except a change in the order
of growth, given that the readings a pass can have are 1, 2.7, 16 and 256.

**The claim that a slow runner cancels is measured rather than argued.** The first CI run of
this gate on `ubuntu-latest` was 1.7× slower in absolute terms than the machine the table above
was recorded on — 7.6 µs a node against 3.5 for the front end, 23.6 against 13.7 for the
emitter, and 2,845 ms against 1,793 at the largest point. The inflations it reported were
**1.20 and 117.9**, both inside the local spread. The constant factor is precisely what a ratio
of two samples divides out, which is what makes this a gate rather than a survey.

## What this suite still cannot tell you

- Whether ZDeceptron is fast **in a browser**. Nothing *emitted* is timed, and nothing here
  runs in one. The counts are necessary evidence, not sufficient. The one timed gate here
  (#165) measures the compiler, which is Rust, and it asserts an order of growth rather than a
  duration even then.
- Whether §14A.2's "against SolidJS and Svelte 5 we expect parity" is true. It is untested and
  untestable in this environment.
- Anything about §14A.1's monomorphic-shape claim. Hidden-class behaviour is a V8 property and
  `boa` does not model it.
- **The cost in time of `server` or `durable` placement.** The *size* of the emitted server half
  is now counted — see "What one bundle per root duplicates" above, which measures what the
  endpoints of a program weigh and how much of that weight is the same text twice. What is still
  missing is everything with a clock in it. `zdc-host` executes the emitted handlers, but it does
  so in the compiler's own `boa` interpreter, so a latency from it describes `boa`; and cold
  start is a property of a platform `zdc deploy` has never been run against. There is still no
  round trip and no persistence to time.
