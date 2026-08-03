# Roadmap

Remaining work, ranked by **what unblocks the most** — not by what is easiest and not by what is
next in the spec's milestone order. Where a cheap item ranks below an expensive one, that is
deliberate and the reason is stated.

Read [`STATUS.md`](STATUS.md) first; this file assumes it. Everything here is ranked against
what the compiler and the spec say. Two named targets drive the ordering and are analysed
separately — see [§10](#10-target-driven-work-to-be-filled-in).

---

## The ordering principle

ZDeceptron's entire claim is one sentence: *you declare where state lives, and the compiler
derives the network.* Today the compiler **derives** the network — the split is correct, the
types are right, the secret does not leak, and both halves of the boundary are emitted — and then
**nothing runs the far half**. Every item below is ranked by how much of that sentence it makes
true.

That is why a runtime store outranked a standard library, and why identity keys — the best
effort-to-payoff ratio in this document — rank fifth. Items 2 and 3 have since landed and are
marked as such; item 1 is still the largest gap, and `static` (item 4) is now the cheapest
thing standing between an example and a build.

---

## 1. Execute the server half — `$store`, `$env`, and an RPC route

**Unblocks: M6, M7, M10, M11, the language's central claim, and every end-to-end measurement.**

This is the largest gap in the project and the only one that changes what ZDeceptron *is*. The
compiler already emits a complete, typed, secret-safe client/server split for
`examples/guestbook.zd`. Nothing executes it. Until something does, the three placements are a
static analysis with no observable consequence, and `guestbook.zd` — the file the README calls
"the whole point of ZDeceptron in one file" — renders a spinner forever.

Four pieces, in this order. The order matters because the first is a wire-format decision the
other three build on.

**1a. `runtime/store.js` — decide the command shape first.**
The spec's §18.2 review puts this at rank 1 of its own list, for a reason worth repeating: the
verb contract, the idempotence table, and whether a command carries a write id are all *shape*
decisions in a file that does not exist yet. Deciding them after the file is written is a change
to the wire format, and the emitter already writes five operations where the spec names three.
Settle the op set, settle write ids, then write the file.

**1b. `$env` injection.** Small, and blocked by nothing. The emitted `greeting.js` calls
`$env('GREETING_API_KEY')`; something has to supply it, and it must never be reachable from the
client bundle.

**1c. Route and execute `POST /_zd/<name>` in `zdc dev`.**
`runtime/rpc.js` already posts there; `zdc dev` currently answers *"not part of this bundle"*.
The dev server holds the compiler in-process and already has the manifest that names every
endpoint, its file, and its wire order. It also already has a JavaScript engine available:
`zdc-runtime` embeds `boa` and evaluates JavaScript from Rust, which is how the runtime's own
test suites run inside `cargo test`. The pieces are present; nothing is being invented.

Doing this closes M6 and makes `guestbook.zd` the demo it was written to be.

**1d. Durable persistence and sync (M7).**
With 1a in place: back the store, and push changes to connected clients. The dev server already
runs an SSE stream for live reload, so the transport exists; what is new is the data channel and
what invalidates a client's `Remote` cell.

**Do this with tests.** `crates/zdc-codegen/src/server.rs` has zero unit tests and two
integration tests total ([`STATUS.md` §3](STATUS.md)). Today that gap costs nothing because
nothing runs the output. The moment item 1 lands, it becomes the most expensive gap in the
repository — a wrong endpoint stops being a diff and starts being a bug in production shape.
Write those tests as part of this item, not after it.

---

## 2. A standard library — starting with `Option` elimination and text

> **✅ Landed.** `crates/zdc-lib/prelude/` is seven `.zd` files above a `foreign` primitive
> layer, resolved into the program's own arenas ahead of it (§17.4.1). `leaderboard.zd` checks;
> `voting-board.zd`'s `at` refusal is gone and only the `Row` question in [§6](#6-two-ratified-language-decisions-then-two-small-implementations)
> still blocks its build. The analysis below is kept as the reasoning that produced it.

**Unblocked: `leaderboard.zd`, `voting-board.zd`'s build, and every program that indexes anything.**

§14F records that there is no standard library, and the absence is not cosmetic — it makes a
*correct* language decision unusable. `at` yields `Option of T`, which is the bounds-checked
lookup §5.4 asks for and a genuine improvement on TypeScript's unchecked index. But `Option` can
be eliminated only by `when`, which is a **statement**, so there is no way to unwrap one inside
an expression. A sort key cannot index a map. `leaderboard.zd` fails on precisely this and its
own header says so.

Two decisions, both of which the spec leaves open:

- **How is `Option` eliminated in expression position?** Either expression-position `when`, or
  `Option` helpers in a prelude. This is a language decision and should be made before the
  library is written around it, not after.
- **`$at` in the runtime.** `voting-board.zd`'s build refuses with *"the runtime has no `$at` to
  build one with"*. Straightforward once the representation is settled.

Then the ordinary surface: text operations (`contains`, `length`, `isEmpty`), which `blog.zd`
invents three of because none exist. A content-shaped program needs text operations before it
needs almost anything else.

---

## 3. Components and modules — `component`, `use`, `children`

> **✅ Landed.** `component`, `children`, `use … for …` and view-position `if` all parse,
> resolve and emit. Components are inlined and monomorphised before the graph passes run, so a
> component stays colorless. `components.zd` and `model.zd` check, `disclosure.zd` builds.
> `blog.zd` is now blocked only by `static` — [§4](#4-static-placement-and-the-ffi).

**Unblocked: `components.zd`, `blog.zd`, and any program longer than one screen.**

Designed in §14D; not in the grammar. `use` does not parse, which is the *first* error in both
aspirational examples. Without components a program is one flat file with no way to name a
repeated shape, and every multi-page or multi-section application is out of reach regardless of
what else lands.

`components.zd` is already written as the design's acceptance test and states the hard parts:
component-local state must be `client` (a component instance is a browser-side thing), and a
component is **colorless** — it runs wherever its inputs are, and passing a `durable` signal
through one cannot launder the obligation to handle `Remote`. That second property is the
interesting one, because it is where §14D meets `zdc-graph`, and it should be pinned by a
negative test the way the leak suite is.

Ranked below the standard library because a component that can only compute what the language can
already compute is worth less than making indexing usable.

---

## 4. `static` placement and the FFI

> **◐ Half landed.** The FFI arrived with the prelude: `foreign f is anywhere` parses,
> typechecks and emits, and §17.4.10's primitives are declared with it. `static` did not —
> it remains the single reason `blog.zd` does not parse, and it is now the highest-value
> unlanded item in this document.

**Unblocks: `blog.zd`, build-time data, and the whole content-site shape.**

Half of this already exists where it is hardest. `zdc-graph` has a `Region::Static` and a `BUILD`
root, the split walks a durable-or-static initialiser into the build region, and the analysis
knows that a value computed at build time crosses no boundary at runtime — so it is `List of Post`
and not `Remote of List of Post`. Rule 1 is satisfied rather than excepted, which is the point.

What is missing is the surface and the evaluation: `static` is not a placement the lexer knows,
and nothing runs a build-time derivation. The FFI (`foreign … is anywhere`, §14E) travels with it,
because build-time data has to come from somewhere and `blog.zd` reads Markdown.

This is the item most likely to move on target analysis — a content site is exactly the shape
`static` serves, and it may outrank item 3 once §10 is filled in.

---

## 5. `record … unique` — identity keys for lists

**Unblocks: O(1) list mutation. The best effort-to-payoff ratio in this document.**

`unique` does not parse: `unique email is Text` in a record body is rejected with *"Expected `is`
after the field name."* Because no record can declare identity, **every list in the repository
reconciles positionally**, and `BENCHMARKS.md` measures what that costs:

| Operation, N=1,000 | positional (today) | identity (`unique`) |
|---|---|---|
| remove a row | 2,986 crossings, 2,986 effect runs | **1 crossing, 1 effect run** |
| swap two rows | 6 | 997 |
| replace all rows | 3,000 | 8,000 |

The removal row is the headline: identity keying is the difference between O(1) and O(n) on the
most common list operation there is. The work is small — one parser rule, one entry in the type
table, one argument changed at the `eachInto` call site, and the emitter's key function. The
benchmark harness already has the identity-keyed arm wired up and measured, so the payoff is
known before the work starts.

It ranks fifth and not first only because it makes existing programs *faster* rather than making
new programs *possible*, and the ranking rule here is what unblocks the most. If the question were
value per hour, it would be first — and it is cheap enough that it should not wait for items 1–4
to finish.

The two rows above that are *worse* under identity keying are not a reason to delay: they are a
reason to put both numbers in §16.6's table, which currently presents `unique` as strictly better.

---

## 6. Two ratified language decisions, then two small implementations

Both of these block a checked-in example, and both are stalled on a decision rather than on work.
They are cheap once decided and should be decided together.

- **A leading argument for `Row` and `Column`.** Four checked-in examples write `Row item.name`;
  `elements.js` has no such slot. §16.3.6 recommends giving `Row` and `Column` a leading text slot
  as `Button` already has, and the compiler refuses rather than inventing the semantics — the right
  call, but it has to be ratified in §4.4 before `voting-board.zd` can build.
- **`if` in view position.** §4.4 puts `if` under `stmt`, not under `node`; `blog.zd` uses it in
  view position. Either the grammar or `blog.zd` is wrong, and the emitter's plan (a two-armed
  hole, exactly like `when`) is already written down.

---

## 7. Source maps

**Unblocks: debugging.** Nothing depends on it and no example fails without it, which is why it is
here and not higher — but the first person to hit a runtime error in generated JavaScript will
want it, and that person arrives the moment item 1 lands.

---

## 8. Deploy targets (M11)

**Unblocks: the last third of the pitch — "no deploy config".**

Deliberately after item 1. A deploy adapter and the dev server's executor are the *same* interface
seen twice: both have to supply `$env` and `$store` to a function bundle that imports nothing.
Building the local one first means the adapter contract is discovered against something that runs,
rather than designed against a document. Vercel, AWS Lambda, Cloudflare and a hosted KV then differ
in their bindings and not in their shape.

---

## 9. Dialects (M9) and the writeup (M12)

The spec's §13 already defers dialects, and the M1 enabling structure is in place and correct —
`word_to_kind` is the single keyword table, keyword tokens carry no spelling, and diagnostics are
phrased to take a dialect's word. Nothing needs retrofitting. A second surface should wait until
the language it is a surface *of* has stopped moving, which means after items 1–6.

The writeup is downstream of everything measurable. `BENCHMARKS.md` is the part of it that already
exists and it is the model for the rest: it contradicts three of the spec's own claims with
numbers, which is what makes it worth reading.

---

## 10. Target-driven work — to be filled in

Two named targets drive the priorities above, and both are being analysed in separate passes. Their
findings slot in here, and they may reorder items 2–5.

### `/Users/msturman00/portfolio` — milestone 7

> **To be filled in by the portfolio analysis.**

What is known from this side without looking at that repository: `examples/blog.zd` was written as
the milestone-7 rehearsal and its header records what it needed and could not have — `static`
placement (item 4), the FFI (item 4), `record` declarations (landed), and three text operations
that exist in no spec (item 2). It notably does **not** need `server` or `durable` placement, no
identity, no database, no API. If that shape holds for the real target, it is reachable on
items 2, 3 and 4 alone, without item 1 — which would make it the *first* target that can ship and
a reason to raise item 4 above item 3.

### JudgeHuman — milestone 12

> **To be filled in by the JudgeHuman analysis.**

What is known from this side: milestone 12 is the writeup, so this target is the thing the writeup
is *about*. Any application with users, shared state or a secret needs item 1 in full — the store,
the executor, and durable sync — and probably item 5 as well, because a real list that mutates
under positional keying is O(n) per removal. It is the target most likely to make item 1
non-negotiable and to expose whatever `crates/zdc-codegen/src/server.rs` gets wrong.

---

## Not on this roadmap

Deliberate exclusions, so their absence is not read as an oversight.

- **Self-hosting.** §14 makes it a real long-term goal with two named prerequisites and explicitly
  not a near-term milestone. It stays there.
- **Incremental recompilation, cross-file imports beyond `use`, per-user durable scoping,
  authentication, typeclasses, higher-rank types, a general effect system.** All in §13's v1
  non-goals.
- **A React/SolidJS benchmark arm.** Not deferred by choice — it needs a package manager, CI has no
  network, and §8 forbids a Node dependency. `BENCHMARKS.md` states this plainly and it should keep
  stating it rather than quietly dropping the comparison.
- **`insta` snapshot tests.** The spec's testing table asks for them and the project uses ordinary
  assertions instead. The coverage exists; converting it would buy little. Worth noting only so the
  deviation is on the record.
