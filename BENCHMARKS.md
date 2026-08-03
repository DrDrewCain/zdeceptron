# Benchmarks

§14A.4 makes this a deliverable rather than an afterthought:

> "Should be faster" is a claim; the writeup needs numbers. A benchmark suite lands with the
> first code generator (M5) and runs in CI thereafter. […] Regressions in these numbers are
> build failures, not observations.

```sh
cargo test -p zdc-bench          # about two minutes; nothing else installed
ZDC_BLESS=1 cargo test -p zdc-bench   # regenerate the table below
```

The workload is the standard js-framework-benchmark one: create 1,000 rows, replace them,
update every 10th, select a row, swap two, remove one, clear, create 10,000, append 1,000
more, clear again.

It is an ordinary workspace test, so CI's existing `cargo test --workspace` step runs it and
fails on it; there is no separate benchmark job to forget to wire up. Dependencies are built at
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
| React and SolidJS | Not measurable. Both need a package manager; CI has no network and §8 forbids a Node dependency. **Nothing here is a measurement against React or Solid.** In their place stand the code-generator design §16.1 rejected, and hand-written JavaScript in two styles. |
| Cold start and latency of an emitted `server` function | Not measurable yet, for a reason that has changed. Server functions **are** emitted now — `zdc build examples/guestbook.zd` writes `functions/greeting.js`, `functions/visits.js` and `functions/visits.incr.js`. What does not exist is anything that runs them: their only free names are `$env` and `$store`, injected by the platform adapter §8.2 describes and no code implements. There is no host to time. |
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

- **There is no standard library** (§14F), so there is still no way to *generate* a thousand
  rows. The pipeline clauses transform a list that already exists.
- **`record … unique` is still not available**, so every list in the repository reconciles
  positionally. The identity-keyed arm below is the same emission with one argument changed; it
  is what the compiler *will* emit, not what it emits today.

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
| swap two rows | 6 | 997 | 6 | 2 | 2 |
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

| Program | client.js | styles.css | index.html | manifest.json | total |
|---|---|---|---|---|---|
| `examples/hello.zd` | 668 | 927 | 236 | 78 | 1909 |
| `examples/counter.zd` | 1006 | 927 | 236 | 98 | 2267 |
| `crates/zdc-bench/bench/row.zd` | 873 | 927 | 236 | 119 | 2155 |

| Runtime file | bytes |
|---|---|
| `runtime/signal.js` | 4815 |
| `runtime/dom.js` | 15141 |
| `runtime/base.css` | 927 |
| `runtime/elements.js (direct emission only)` | 4089 |
<!-- end generated -->

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
| swap, `unique` — 997 `insertBefore` | 997 ✅ |
| remove row, positional — 1 `removeChild`, 1,988 `nodeValue` | 1 `removeChild`, **1,990** `nodeValue`, **plus 995 `setAttribute` the table does not count** |
| remove row, `unique` — 1 `removeChild`, 0 moves | 1 `removeChild`, 0 moves ✅ |

The addition: **replacing all 1,000 rows costs positional keying 3,000 crossings and identity
keying 8,000.** Every key changes, so identity keying tears down and rebuilds the whole list
while positional keying keeps every slot and rewrites its contents. §16.6 presents `unique`
keying as strictly better once it lands; on this operation it is 2.7× worse, and both numbers
should be in the table.

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

`counter.zd` emits 1,006 bytes of JavaScript. The runtime it links against is 18,153 bytes of
unminified, heavily commented source — `signal.js` plus `dom.js`, with no minifier anywhere in
the pipeline, so that is the shipped figure and not a projection. `elements.js` (4,089 bytes)
is *not* shipped: generated code never imports it, which is a placement-independent instance
of the dead-code claim in §14A.1. Direct emission would have shipped it.

## The regression gates

`tests/benchmark.rs` fails the build on all of the following. The counts are deterministic —
there is no timing anywhere — so none of these can flake; the headroom exists so that a
deliberate, harmless change to the row shape does not fail the build, while a change of
architectural significance does. The golden table above is the exact-match gate: **any** change
to **any** number fails until it is regenerated and reviewed.

| Gate | Measured | Threshold | Why that threshold |
|---|---|---|---|
| Every arm renders the same DOM at every step | identical | identical | No headroom is possible or wanted. An arm that is wrong is not fast. |
| Template cloning vs direct emission, crossings | 3.1× | ≥ 2× | The ratio depends on how many holes and handlers a row has. At 2× the architectural claim is still falsifiable, and adding an attribute to the row does not fail the build. |
| Template cloning vs direct emission, effects | 1 per row fewer | ≥ 1 per row fewer | §16.1's own claim, stated per row. |
| Effects per row | 3 | ≤ 3 | The row has three holes. More means a binding is being created the emitter does not need. |
| Hand-tuned vanilla is still the floor | 1.75× | floor < emitted ≤ 2.5× | Two-sided on purpose. If the emitted code ever *beats* the floor, §14A.2 is wrong and the spec needs correcting, not the test relaxing. The 2.5× ceiling catches the loss widening by an order of magnitude without failing on one extra per-row write. |
| Emitted vs node-by-node vanilla | 2.7× | ≥ 2× | Measured margin with room for a row-shape change. |
| Identity-keyed removal | 1 crossing | ≤ 2 | This is what §16.2 R1's two-pass retire bought. |
| Identity-keyed swap | 997 moves | 900–1,100 | §16.6 measures 997, accepts it, and schedules the LIS fix. A different number means the reconciler changed and the spec figure is stale. |
| Positional-keyed removal | 2,986 crossings | 1,000–4,000 | Bounded below as well: if it drops, §16.6's account of positional keying is out of date and this file is wrong. |
| Clearing a list | 11,000 `removeChild` | exactly 11,000 | Pinned so the O(n) teardown stays visible rather than being forgotten. |
| Emitted `client.js` | ≤ 1,006 bytes | ≤ 2,048 | Roughly double, so a code generator that starts emitting a helper per node fails. |
| `signal.js` + `dom.js` | 18,153 bytes | ≤ 24,576 | Not a byte-count contest — a check that no framework has grown inside the runtime. |

## What this suite still cannot tell you

- Whether ZDeceptron is fast **in a browser**. Nothing here is timed, and nothing here runs in
  one. The counts are necessary evidence, not sufficient.
- Whether §14A.2's "against SolidJS and Svelte 5 we expect parity" is true. It is untested and
  untestable in this environment.
- Anything about §14A.1's monomorphic-shape claim. Hidden-class behaviour is a V8 property and
  `boa` does not model it.
- Anything about `server` or `durable` placement. The compiler emits both halves — the client's
  `remote`/`call` and one file per endpoint — but no store, no adapter and no host exists to run
  the server half against, so there is no cold start, no round trip and no persistence to count.
