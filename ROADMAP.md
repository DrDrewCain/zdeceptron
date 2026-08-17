# Roadmap

Remaining work, ranked by **what unblocks the most** — not by what is easiest and not by what is
next in the spec's milestone order. Where a cheap item ranks below an expensive one, that is
deliberate and the reason is stated.

Read [`STATUS.md`](STATUS.md) first; this file assumes it. **Ranked against the merged
`feature/front-end` tree**, the same tree `STATUS.md` was measured on.

---

## The ordering principle

ZDeceptron's entire claim is one sentence: *you declare where state lives, and the compiler
derives the network.* **That sentence is now true end to end.** The compiler derives the split,
emits both halves, and `zdc-host` executes the far one — `crates/zdc-host/tests/two_windows.rs`
shows two browser windows moving together over live sync.

That changes what this document is for. The previous ordering was dominated by a single item —
"execute the server half" — which has landed. What remains is ranked by how much of the
*language* it makes usable, and the top of the list is no longer about the network at all.

**Items 1 and 2 have since landed too**, and are kept struck through rather than deleted: both
were written as constraints before the feature existed, and both shipped in the shape the entry
specified. That is worth leaving visible. The first item still open is item 3.

---

## 1. ~~A value cannot become markup~~ — **landed**

**Was: the largest gap on this branch and the one most likely to stop a real user.**

It shipped in the shape this section specified, which is why the section is kept rather than
deleted — the constraint was written down before the feature, and the feature matches it:

- `Markup` is a type **no literal spells** and nothing coerces to or from. It is not `Addable`,
  so `Text + Markup` does not typecheck in either direction and a program cannot assemble markup
  out of strings it controls.
- **One** rendering site — `Slot::Rendered`, which only `Prose` has — whose argument must be
  `Markup`. The runtime's assignments to `innerHTML` are three and are held to three by
  `crates/zdc-codegen/tests/markup.rs`: `template`, which takes compile-time literals only, and
  `markup`/`bindMarkup`, which the emitter reaches only from that one site.
- **A producer set of exactly one:** `build markdown`, which runs inside the compiler over a
  file in the project directory, escapes every raw HTML span, and rewrites every non-`http(s)`
  URL before returning.

The tests assert against the parsed DOM tree rather than the emitted string, as this section
asked: `the_blog_renders_its_posts_as_headings_and_paragraphs` reads the headings and paragraphs
back out of the rendered document.

---

## 2. ~~Build-time file capabilities, and the call syntax that reaches them~~ — **landed**

**Was: the one example that does not compile, and build-time data generally.**

Both halves landed, and the second landed the way this section predicted it should.

**2a.** `build read`, `build list` and `build markdown` are answered in the compiler's own
sandbox — `crates/zdc-hir/src/sandbox.rs`, the same one that bounds `use`, rather than a
second path policy. `examples/content/blog/` is what `blog.zd` reads.

**2b.** The grammar did **not** grow a bare-argument call form. `blog.zd` was rewritten in the
syntax the language already has, which is what this section argued was almost certainly right:
§4.1's bargain is one phrasing per construct, and a second call syntax would have spent that for
one file's convenience.

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

## 8. ~~Reduce the emitter's quadratic, once it is felt~~ — **landed**

It was felt, at 18.8 ms per keystroke against a 10 ms budget, and it is now 7.2 ms.
`BENCHMARKS.md` has the before and after; what belongs here is the part that was wrong, because
this entry was written to make sure the right pass got optimised and it named the wrong one.

**"The measurements say the cost is in the *split*"** — they did not, and could not have. The
survey that says so varies definitions and roots and holds a view at one `Text` per root, so it
never measured the axis this entry is about. The cost was in the emitter's *path scheduling*,
which was cubic in view size and invisible to every gate in the suite, because the walk it
schedules comes out identical however long the scheduling takes. `split` was 0.3 ms of the 18.8.

**"Anyone optimising `ifc` would be optimising the pass that is not the problem"** — `ifc` was
two thirds of a keystroke. The finding that entry rests on says `ifc` is insensitive to *root
count*, which is true and is not the same claim. What made it slow is a phase that walks the
whole program once per function parameter, over a prelude compiled with every program.

The lesson for the next entry of this kind: an entry that names the pass to optimise is only as
good as the axis the measurement varied, and this one named a pass from a survey that was
holding the interesting variable fixed.

What remains, and is not this item: **six of the 7.2 ms is flat in the size of the file** —
`hello.zd` and a sixty-kilobyte program cost the same — because §17.4.1's prelude is resolved,
split, typechecked and flow-analysed from nothing on every keystroke. No further work on the
emitter reaches it. What reaches it is somewhere to keep an answer between keystrokes.

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

**Measured 2026-08-16, and this document now ranks against the number.**
[`EXPRESSIBILITY.md`](EXPRESSIBILITY.md) has the method; the result is **65.5% of the target's
14,499 non-test TypeScript lines**, measured by porting it — `~/zdc-portfolio` @ `8a79990`, 21
modules, 12,515 lines, which compiles with zero diagnostics on `363b9e7` and emits 33 documents.

Every figure quoted before that — 0%, 47.6%, 16.4%, 21.3%, 24.2% — was either a
measurement of a commit that had since moved on, or a projection over a union of branches that
was never built. The one genuinely measured increment is **+4.9 points on `feature/numeric` @
`e680dc2`**, and it sits on top of a projection, which does not make the sum a measurement.
[`STATUS.md` §9](STATUS.md) records the provenance of each.

**The measurement's first finding outranks everything on this list.** On `origin/main` the tree
does not compile at all — 358 diagnostics, every one of them the same rule: a line ended where
the writer meant it to continue. `ee63fe5` closes all 358 and is on four branches and not on
`main`. It is worth nothing on a feature list and 100% of this target in practice, and nothing
below should be ranked above landing it.

What the earlier analyses said, and what has since landed: the blockers they named were, in order
of weight, the
element vocabulary, routing and modules, event payloads, the document head, the `static`
placement, and the standard library. **All six have landed** ([`STATUS.md` §1](STATUS.md)).

**Corrected 2026-08-07.** This paragraph used to add markup and build-time file reading to the
list of what had not landed, while items 1 and 2 of this same document were already struck
through as landed. Both shipped: `Markup`, `Prose` and `build markdown` exist, and `build read`
and `build list` reach the project directory inside the compiler's own sandbox.

**The named blocker was browser APIs — and the measurement dissolves four fifths of it.**
[#19](https://github.com/DrDrewCain/zdeceptron/issues/19) grouped a frame loop, timers, observers,
storage and outbound `fetch`, and said it could not be ranked until something established how much
of the target needed each. Measured on the port:

| | sites in the original | verdict |
|---|---:|---|
| frame loop | 11 | **answered.** 4 `every frame` declarations, and the clock fold makes them game loops rather than tick sources |
| timers | 18 | **answered.** 3 `every "…"` declarations at 60, 90 and 110 ms; `after` unused |
| storage | 34 `localStorage` | **answered.** All 34 collapse to 4 `remembered` declarations |
| storage | 6 `sessionStorage` | **confirmed refused.** All six hold one OAuth verifier, which is a `secret`, which no browser store may hold — §14's ruling, independently reproduced |
| observers | 2 | **rank first.** Both `IntersectionObserver`, both per-element, neither expressible by `from scroll`; 353 lines blocked |
| outbound `fetch` | 5 | **declined.** 4 of 5 want a bearer token in a header, which `request` refuses by design; the 5th is a build-time read wearing a fetch |

So the group is not one item. **Observers are the one to specify** — and scoped to the
*visibility* question rather than the observer API, because `ResizeObserver` has zero sites here
and `blocks.zd` shows why it may never get one: a `Scene` that scales itself deleted the resize
handling rather than needing an observer for it. The mechanism is #19's: `foreign` against the
platform, which §14E already governs. That mechanism did not work until
[#223](https://github.com/DrDrewCain/zdeceptron/issues/223) — a `foreign` was emitted and never
linked — so the remaining blocker was, until then, blocked on a defect rather than on a design.

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
- **`insta` snapshot tests.** ~~The spec's testing table asks for them and the project uses
  ordinary assertions instead. The coverage exists; converting it would buy little. Worth noting
  only so the deviation is on the record.~~ **DECIDED 2026-08-16 (#157): not adopted, and the
  spec's §11 entry is amended rather than owed.** It was tried and removed — one 853-line AST
  snapshot, deleted with the dependency in `87b1b5d` — and the reason it was brittle is the
  reason it stays out: a snapshot's expected value is written by the code under test, so it
  cannot be written first and cannot be watched to fail, which is the rule `CONTRIBUTING.md`
  opens its test section with.
  [*Snapshot tests, and the dependency this project does not take*](CONTRIBUTING.md#snapshot-tests-and-the-dependency-this-project-does-not-take)
  carries the argument, the bless mechanism this repository uses instead, and what reverses it.
- **Fixing an `#[ignore]`d test by deleting it.** The one this entry used to name — a
  disagreement between `zdc check` and `zdc build` — was fixed, and its `#[ignore]` removed
  rather than the test. The two that remain document open language decisions
  ([`STATUS.md` §3](STATUS.md)). Either should be decided or kept ignored with its reason, never
  quietly removed.
