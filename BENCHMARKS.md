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
| Cold start and latency of an emitted `server` function | Nothing to measure yet. This compiler refuses every non-`client` placement (§16.5, M6), so no server function is emitted. |
| Bundle size against React and Solid equivalents | Our half is measured below; theirs cannot be fetched. |

## The gap: this workload is not expressible in ZDeceptron

The workload is a list of rows, and **a list cannot be written in ZDeceptron today.** Three
independent reasons, each stated by the compiler itself and each pinned by a test in
`crates/zdc-bench/tests/fidelity.rs` — so on the day any of them stops being true, the build
fails and points at this section:

1. **`each` in the view is refused.** *"It needs the hole machinery and the keying decision of
   milestone M5b (spec §16.5); `zdc build` refuses rather than emitting a list that never
   updates."*
2. **`empty` is refused.** *"whether it is an empty list or an empty map is a question for the
   type checker, which does not exist (spec §16.7)."*
3. **A list literal does not lex.** `starting ["a", "b"]` — §4.4's `listLiteral`, which §14B.4
   records as a closed design gap — is rejected with `` `[` is not valid ZDeceptron ``. No
   example in the repository contains a bracket, which is why this had not surfaced.

Two further reasons are not compiler limitations at all: the benchmark's rows have fields and
`record` declarations do not exist yet (§16.5, M7), and there is no way to *generate* a
thousand rows — the pipeline clauses transform a list that already exists, and §14F records
that there is no standard library.

**So what is the ZDeceptron arm?** `crates/zdc-bench/bench/row.zd` is a real ZDeceptron
program — one benchmark row — compiled by the real pipeline. Its template, the walk to its
holes, and the sequence of bindings attached at them are extracted from the emitted
`client.js` and compared against the row the benchmark renders. Those three things are what a
row costs, and `tests/fidelity.rs` fails the build if they drift apart.

What is written by hand is the plumbing `each` would have supplied: the `eachInto` call, the
key function, and a per-row item getter where the emitted module reads a module-level signal.
§16.6 states exactly what the emitter will produce there —
`eachInto($n7, $n8, list, $byPosition, (player) => …)` — and that is what is written.

This gap is real and it is not small. **The ZDeceptron arm measures the emitter's row and the
runtime's reconciler, joined by hand. It does not measure a ZDeceptron program, because there
is not one to measure.**

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
| `runtime/dom.js` | 14853 |
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

Six of the eleven `.zd` files in the repository build today. Comment and blank lines are
excluded from the line count, because these are teaching files whose prose outnumbers their
code — `hello.zd` is twelve lines of which six are comments — and counting them would halve
the ratio for free.

| Program | file lines | code lines | `client.js` | whole bundle | **bytes/line** | bytes/line charging the whole runtime |
|---|---|---|---|---|---|---|
| `examples/counter.zd` | 28 | 17 | 1,006 | 2,267 | **59** | 1,216 |
| `examples/disclosure.zd` | 48 | 24 | 1,464 | 2,690 | **61** | 880 |
| `examples/guestbook.zd` | 60 | 25 | 2,282 | 3,784 | **91** | 878 |
| `examples/hello.zd` | 12 | 6 | 668 | 1,909 | **111** | 3,389 |
| `examples/todo.zd` | 108 | 62 | 3,491 | 4,805 | **56** | 373 |
| `crates/zdc-bench/bench/row.zd` | 25 | 12 | 873 | 2,155 | **72** | 1,711 |

The runtime is `signal.js` plus `dom.js`, **19,668 bytes**, uncompressed and unminified because
there is no minifier in the pipeline. `elements.js` is not in that sum; generated code never
imports it (§16.3.1).

**Which number is honest.** The marginal one — 56 to 111 bytes per line, and 54 to 56 on
everything larger than a toy. The runtime is one file, byte-identical for every program and
every page, cached once by the browser and shared by an entire application; charging all of it
to whichever program is being measured says more about how many programs you divided by than
about the compiler. The right-hand column is still worth printing, because it is what a
single-page application actually downloads, and because it shows the fixed cost dominating
below about 200 lines. Both numbers beat Swift. The marginal one beats it by **7× to 14×**;
the fixed-cost-included one beats it at every size in the table except `hello.zd`, which is a
six-line file.

The five files that do not build are refused with reasons, not crashes, and the survey prints
them: `blog.zd` and `components.zd` do not parse or resolve, `leaderboard.zd` and
`voting-board.zd` need `at` and the `Option of T` it yields (§16.7 item 5), and `model.zd` has
no `view` by design.

### The empty-program baseline

Swift's is the single most comparable figure in the two systems, because a null program is
almost entirely machinery.

| | Swift | ZDeceptron |
|---|---|---|
| Source | 6 lines | 6 lines |
| Program's own emission | — | **639 bytes** |
| Runtime | — | 19,668 bytes |
| **JavaScript shipped** | **73,000 bytes** | **20,307 bytes** |

**3.6× smaller**, and the shape is different in a way that matters more than the ratio: 97% of
ours is the shared runtime and 3% is the program. Swift's 73 kB was *per program*. Ours is paid
once for a whole application. Extrapolating the measured marginal cost to the size of Swift's
largest application — 1,094 lines — gives roughly 120 kB against Swift's 1.21 MB, a 10×
margin; that is arithmetic on a measured slope rather than a measured application, and
`at_swifts_largest_app_size_the_runtime_is_already_amortised` says so where it asserts it.

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
2. **The information-flow pass is not the problem, and it is not sensitive to roots.** Holding
   definitions fixed and multiplying roots by 64 multiplies `ifc` by 7.7. Holding roots fixed
   and multiplying definitions by 64 multiplies it by 42. §17.3 is driven by how much program
   there is, essentially not by how many pages it is split across — which is precisely the
   opposite of `split`, and worth knowing before anyone optimises the wrong pass.
3. **The constants are small enough that none of this is urgent.** The worst point measured —
   513 definitions and 258 roots, an application with 256 pages — costs 194 ms in `split` and
   70 ms in `ifc`. A 50-page application with 500 definitions lands in the low tens of
   milliseconds. The quadratic is real and it is documented; it is not yet felt.

`splitting_walks_the_product_of_definitions_and_roots` pins the *shape* — root and definition
counts on both sides of the product — as a deterministic gate, since the timing cannot be one.

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
- Anything about `server` or `durable` placement, which this compiler does not emit.
