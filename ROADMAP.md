# Roadmap

Remaining work, ranked by **what unblocks the most** — not by what is easiest and not by what is
next in the spec's milestone order. Where a cheap item ranks below an expensive one, that is
deliberate and the reason is stated.

Read [`STATUS.md`](STATUS.md) first; this file assumes it. **Ranked against `feature/docs` @
`f6c6519`**, the same tree `STATUS.md` was measured on.

---

## The ordering principle

ZDeceptron's entire claim is one sentence: *you declare where state lives, and the compiler
derives the network.* **That sentence is now true end to end.** The compiler derives the split,
emits both halves, and `zdc-host` executes the far one — `crates/zdc-host/tests/two_windows.rs`
shows two browser windows moving together over live sync.

That changes what this document is for. The previous ordering was dominated by a single item —
"execute the server half" — which has landed. What remains is ranked by how much of the
*language* it makes usable, and the top of the list is no longer about the network at all.

---

## 1. A value cannot become markup

**Unblocks: every content-shaped program, `blog.zd`, and the milestone-7 target's whole shape.**

This is the largest gap on this branch and the one most likely to stop a real user.

Verified in [`STATUS.md` §4](STATUS.md): there is no `Markup` type and no `Prose` element, and
every value a program computes reaches the DOM through `nodeValue`, `setAttribute`, `.value` or
`.checked` — none of which parses HTML. A value holding `<h1>Hello</h1>` renders as those literal
characters. The single `innerHTML` in `runtime/dom.js:122` takes compile-time literals only and
is not a value path.

**The design that answers it is known and should be followed**, because it is the one that does
not trade a rendering feature for an injection surface:

- A `Markup` type that **no literal spells** and that nothing coerces to or from. Its safety is
  by absence: if it is not `Addable`, then `Text + Markup` does not typecheck in either
  direction, so a program cannot assemble markup out of strings it controls.
- **One** rendering site, reachable from exactly one element, whose argument must be `Markup`.
  That keeps the set of `innerHTML` writes in the runtime auditable by inspection.
- **A producer set that is the trusted base, with as few members as possible** — ideally one: a
  build-time markdown renderer that runs inside the compiler, over a file in the project
  directory, escaping raw HTML spans and rewriting non-`http(s)` URLs.

The reason to write the constraint down before the feature is that "someone reaches for
`innerHTML` under deadline" is exactly how this goes wrong, and a type that cannot be
constructed from user data is the only version of this that survives that.

**Do this with tests that assert against the parsed DOM tree, not the emitted string.** The
`switch`-fallthrough bug ([`STATUS.md` §6](STATUS.md)) is the standing argument for that: it was
invisible to every static pass and to every text-comparing test, and only showed up in the value
the emitted program computed.

---

## 2. Build-time file capabilities, and the call syntax that reaches them

**Unblocks: `blog.zd` — the one example that does not compile — and build-time data generally.**

Two separable pieces, both small, and they must land together to be worth anything:

**2a. A capability for reading the project directory at build time.** `static` placement has
landed and works (`examples/writing.zd` computes its content at build time, inlines it, emits
`rss.xml`, and builds with an empty `PATH`). What it cannot do is *read a directory of files*.
A `build read` / `build list` / `build markdown` family, answered in the compiler's own sandbox,
is the missing half.

The sandbox to answer them in already exists and is the right one:
`crates/zdc-resolve/src/sandbox.rs` bounds every path a `use` can reach, with the project root
fixed once per build rather than re-based at each hop. Build capabilities should go through it
rather than growing a second path policy — two path policies is how the traversal bug happened
the first time.

**2b. A call syntax with a bare argument, or a rewrite of `blog.zd`.** `blog.zd` fails at
`examples/blog.zd:46:54` on `readMarkdown "content/blog"`. Every call in the language is written
`f with a, b`. Either the grammar grows a bare-argument form or the example is rewritten in the
syntax the language has. **The second is almost certainly right** — §4.1's bargain is that the
grammar admits exactly one phrasing per construct, and a second call syntax spends that for one
file's convenience.

Note that 2a without item 1 gets you a `Text` full of HTML that renders as visible angle
brackets. **These two items are one feature in practice**, and item 1 is the half that carries
the safety argument.

---

## 3. `record … unique` — identity keys for lists

**Unblocks: O(1) list mutation. The best effort-to-payoff ratio in this document.**

Still refused, verified by compiling a probe: `unique email is Text` in a record body gives
*"Expected `is` after the field name."* Because no record can declare identity, **every list in
the repository reconciles positionally**, and `BENCHMARKS.md` measures what that costs:

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

It ranks third and not first only because it makes existing programs *faster* rather than making
new programs *possible*. If the question were value per hour it would be first, and it is cheap
enough that it should not wait for items 1 and 2.

The two rows above that are *worse* under identity keying are not a reason to delay: they are a
reason to put both numbers in §16.6's table, which currently presents `unique` as strictly
better.

---

## 4. Close the two known LSP defects

**Unblocks: trusting the editor.** Both are verified and recorded in
[`STATUS.md` §5](STATUS.md); both are small; both make the editor *wrong* rather than merely
incomplete, which is worse.

- **Go-to-definition across a `use` jumps to the wrong offset in the wrong file.**
  `crates/zdc-lsp/src/server.rs:192-207` renders a span against the entry document. The fix
  already exists and is used by the CLI: `Linked::locate`
  (`crates/zdc-resolve/src/modules.rs:86`). The language server should call it.
- **A parse error in an imported file is rendered with no location at all.**
  `crates/zdc-cli/src/main.rs:270-285` renders every load error against the entry file's text;
  the span does not fall inside it, so `ariadne` prints the message with no caret. The reader is
  told what is wrong and not which of their files it is in.

The second is not really an LSP defect — it is in the shared load path — and it degrades the
command-line experience identically. It is the cheaper of the two.

---

## 5. Guard `Whole` overflow on the client path

**Unblocks: the arithmetic meaning what the type says.**

`crates/zdc-codegen/src/expr.rs:1016,1018` emits bare JavaScript `+` and `*` with no guard, so a
`Whole` silently loses integer precision above 2⁵³ and silently becomes `Infinity` above
≈1.7977 × 10³⁰⁸. The narrowing operations are already guarded — `intrinsics.rs:274,279` wrap
`floor of` and `round of` in `Number.isFinite` and give an `Option` — so **the pattern to follow
is in the tree**; it simply does not extend to the arithmetic operators.

The durable path is covered. This is the client path only, which is why it ranks here and not
higher: it is a correctness hole in a corner most programs never reach, and the fix has an
obvious cost in emitted size that should be measured against the runtime size gate before it
lands.

---

## 6. Source maps

**Unblocks: debugging.** Nothing depends on it and no example fails without it. But the first
person to hit a runtime error in generated JavaScript will want it, and now that server and
durable programs actually execute, that person arrives sooner than this ranking suggests.

No `sourceMap` exists anywhere in the tree.

---

## 7. Actually deploy to one platform

**Unblocks: the last third of the pitch — "no deploy config".**

`zdc deploy` writes a complete deployment for four targets and prints a capability report, and
`tests/portability.rs` pins that the handler bodies and router are byte-identical across all
four. **None of it has been run against a real account.** The adapters are checked against
vendor documentation and against each other — never against a vendor.

The valuable step is not a fifth adapter. It is running *one* of the existing four end to end
against a real account, because that is what converts a documented assumption into a tested one,
and whatever it finds will apply to the other three.

---

## 8. Reduce the emitter's quadratic, once it is felt

The emitter is near-quadratic in view size, documented in its own source
(`crates/zdc-codegen/src/analysis.rs:109,116,271`, `lib.rs:300`) and measured in
`BENCHMARKS.md`. It is real and it is not yet felt at present view sizes.

What changes the urgency is that **the editor runs the real passes, so the cost lands per
keystroke.** Optimise the right pass when it comes to that: the measurements say the cost is in
the *split*, and that the information-flow pass is essentially insensitive to root count. Anyone
optimising `ifc` would be optimising the pass that is not the problem.

---

## 9. Dialects (M9) and the writeup (M12)

The spec's §13 already defers dialects, and the M1 enabling structure is in place and correct —
`word_to_kind` is the single keyword table, keyword tokens carry no spelling, and diagnostics are
phrased to take a dialect's word. Nothing needs retrofitting. A second surface should wait until
the language it is a surface *of* has stopped moving.

The writeup is downstream of everything measurable. `BENCHMARKS.md` is the part of it that
already exists and it is the model for the rest: it contradicts several of the spec's own claims
with numbers, which is what makes it worth reading.

---

## 10. The milestone-7 target

### `/Users/msturman00/portfolio`

**The honest position: unverified on this tree, and this document will not rank against a number
it does not have.**

Every figure ever quoted for this target — 0%, 47.6%, 16.4%, 21.3%, 24.2% — was either a
measurement of a commit that had since moved on, or a projection over a union of branches that
was never built. The one genuinely measured increment is **+4.9 points on `feature/numeric` @
`e680dc2`**, and it sits on top of a projection, which does not make the sum a measurement.
[`STATUS.md` §9](STATUS.md) records the provenance of each.

**The single most useful unmeasured number about this project is this target's expressibility on
a tree that exists.** Obtaining it needs the target's feature inventory and a probe corpus,
neither of which lives in this repository, so it is a real piece of analysis rather than a
command.

What can be said without it: the blockers those analyses named were, in order of weight, the
element vocabulary, routing and modules, event payloads, the document head, the `static`
placement, and the standard library. **All six have landed** ([`STATUS.md` §1](STATUS.md)). The
ones that have not are markup (item 1), build-time file reading (item 2), and browser APIs — no
frame loop, no timers, no observers, no storage, no outbound `fetch` on this branch. That last
group is not on this roadmap as a numbered item because nothing has established how much of the
target needs it, which is exactly the thing the measurement would settle.

### JudgeHuman — milestone 12

Milestone 12 is the writeup, so this target is the thing the writeup is *about*. Any application
with users, shared state or a secret needs what item 3 gives — a real list that mutates under
positional keying is O(n) per removal — and it is the target most likely to exercise the
server and durable paths hard enough to find what they still get wrong.

An earlier analysis of this target anchored itself to a commit hash that **does not exist in this
repository**, so its quantitative findings are unanchored and are not repeated here. Its
qualitative findings may well stand; they have not been re-derived.

---

## Not on this roadmap

Deliberate exclusions, so their absence is not read as an oversight.

- **Self-hosting.** §14 makes it a real long-term goal with two named prerequisites and
  explicitly not a near-term milestone. It stays there.
- **Incremental recompilation, cross-file imports beyond `use`, per-user durable scoping,
  authentication, typeclasses, higher-rank types, a general effect system.** All in §13's v1
  non-goals.
- **A React/SolidJS benchmark arm.** Not deferred by choice — it needs a package manager, CI has
  no network, and §8 forbids a Node dependency. `BENCHMARKS.md` states this plainly and it should
  keep stating it rather than quietly dropping the comparison.
- **`insta` snapshot tests.** The spec's testing table asks for them and the project uses
  ordinary assertions instead. The coverage exists; converting it would buy little. Worth noting
  only so the deviation is on the record.
- **Fixing the `#[ignore]`d test by deleting it.** It documents a real disagreement between
  `zdc check` and `zdc build` ([`STATUS.md` §3](STATUS.md)). It should be fixed or kept ignored
  with its reason, never quietly removed.
