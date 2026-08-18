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
- **Incremental recompilation, per-user durable scoping, authentication, typeclasses,
  higher-rank types, a general effect system.** All in §13's v1 non-goals.
- **A package manager, and every other way one program might depend on another.** This used to
  be deferred to the same non-goal list, which is not something it can be deferred to: §13 names
  cross-file modules as a v1 non-goal and `use` compiles, so the list has already been overtaken
  on exactly this point and settles nothing. It is now decided in
  [the reference §14](docs/reference.md#depending-on-another-program) — a dependency is a file
  inside the project, the sandbox that bounds `use` is the reason, and the three observations
  that would reopen it are written down beside it (#174).
- **A React/SolidJS benchmark arm.** Not deferred by choice — it needs a package manager, CI has
  no network, and §8 forbids a Node dependency. `BENCHMARKS.md` states this plainly and it should
  keep stating it rather than quietly dropping the comparison.
- **`insta` snapshot tests.** The spec's testing table asks for them and the project uses
  ordinary assertions instead. The coverage exists; converting it would buy little. Worth noting
  only so the deviation is on the record.~~ **DECIDED 2026-08-16 (#157): not adopted, and the
  spec's §11 entry is amended rather than owed.** It was tried and removed — one 853-line AST
  snapshot, deleted with the dependency in `87b1b5d` — and the reason it was brittle is the
  reason it stays out: a snapshot's expected value is written by the code under test, so it
  cannot be written first and cannot be watched to fail, which is the rule `CONTRIBUTING.md`
  opens its test section with.
  [*Snapshot tests, and the dependency this project does not take*](CONTRIBUTING.md#snapshot-tests-and-the-dependency-this-project-does-not-take)
  carries the argument, the bless mechanism this repository uses instead, and what reverses it.
  only so the deviation is on the record.
- **A robustness claim on the integrity lattice.** **DECIDED 2026-08-16 (#212): it stays
  withdrawn, and it is now withdrawn for a reason rather than pending one.** Robust
  declassification's rule has two conjuncts — the value released must be high integrity, and the
  *decision* to release it must be — and this compiler enforces the first and has never written
  the second. `zdc-graph`'s walk carries a program counter and no release rule reads it, so a
  text box may choose which of two releases runs and the pass reports nothing
  (`a_browser_chosen_branch_chooses_which_release_runs`). The second conjunct also cannot be
  discharged the textbook way here, because a `release` is called from a browser and the browser
  decides when: what stands in for it is `limit N per visitor`, which is read by four consumers
  and enforced by none — verified by building a program with `limit 20 per visitor` and reading
  the emitted handler, which contains no counter. So [#29](https://github.com/DrDrewCain/zdeceptron/issues/29)
  is not a risk beside the question; in this language it is the question.
  `crates/zdc-graph/src/integrity.rs`'s module doc carries the argument, re-argues
  [#30](https://github.com/DrDrewCain/zdeceptron/issues/30),
  [#31](https://github.com/DrDrewCain/zdeceptron/issues/31) and
  [#32](https://github.com/DrDrewCain/zdeceptron/issues/32) against the settled default-closed
  direction, and names the three things that would change the answer.

- **A public API surface, and the second client that would use one.** **DECIDED
  2026-08-16 (#38): there is none, and the generated endpoints are not one.** They are
  the private calling convention between one compiled program and the client that same
  compiler run emitted — an implementation detail, named from the program's own text,
  renamed by an ordinary rename and deleted by a change to markup. Neither option #38
  offered is taken: no declaration makes an endpoint public, and no manifest pins a
  derived name, because pinning one cannot say what a pinned name means after the
  signal graph moves under it.
  [`SECURITY.md`](SECURITY.md#the-generated-endpoints-are-not-a-public-api) carries the
  argument, the four measurements behind it, and what reverses it — authentication
  implemented rather than designed, a surface version independent of the compiler's,
  and a place to declare that a name is part of the surface. Until all three, a mobile
  client or a script may call a deployment and is owed nothing when it breaks.
- **Fixing an `#[ignore]`d test by deleting it.** The one this entry used to name — a
  disagreement between `zdc check` and `zdc build` — was fixed, and its `#[ignore]` removed
  rather than the test. The two that remain document open language decisions
  ([`STATUS.md` §3](STATUS.md)). Either should be decided or kept ignored with its reason, never
  quietly removed.

---

## A sixth placement, and `model` — DECIDED

**Date:** 2026-08-16. **Status: DECIDED. Closes [#210](https://github.com/DrDrewCain/zdeceptron/issues/210).**
A sixth placement is **not** on this roadmap; keeping it *possible* is, and this entry is the
check that it still is. No compiler behaviour changes here. What changes is the tests, which had
stopped covering the placement set they claim to cover.

§15.1 records `model`, an LLM call as a placement beside `client`, `static`, `server`, `durable`
and `remembered`, as deliberately kept possible and explicitly not v1. The question is not
whether to build it. It is whether the tree has quietly foreclosed it, and whether the mechanism
that was supposed to prevent that still works.

The question is already a placement behind where #210 left it, and that is part of the answer:
`Placement::ALL` was four elements when the issue was filed and is five now (`lib.rs:265`),
because `remembered` landed in between. The mechanism has been run once for real, so what it
does and does not reach can be read off the tree rather than reasoned about.

**A `model` placement is not foreclosed, and that was measured rather than argued.** A sixth
variant was added to `zdc_ast::Placement` and to `zdc_types::SignalPlacement`, and the workspace
was compiled until it was green again. Nothing had to be undone. Every failure was a `match`
asking to be told what the new placement is, which is what `Placement::index`'s doc comment
promises. The interesting finding is not that it held — it held at every `match` — but where it
does not reach.

### What a sixth placement touches

Measured on `origin/main` at `8a99ff9`. **Twenty-two `match` sites in seven crates refuse to
compile until the new placement is classified**, and once they were answered the workspace built
with no error and no warning — so this is the whole list, not a sample of it. Line numbers are
this branch's; the sites are the ones main has, and this branch adds a twenty-third of its own,
`SignalPlacement::index`.

| Crate | Site |
|---|---|
| `zdc-ast` | `lib.rs:280` `Placement::index`, `lib.rs:292` `Placement::word` |
| `zdc-types` | `placement.rs:103` `from_ast`, `:113` `describe`, `:153` `may_be_secret`, `:190` `is_externally_written`, `:241` `read_kind`; `infer.rs:2195` the two-way binding rule |
| `zdc-resolve` | `resolve.rs:1456` — why state inside a `component` must be `client` |
| `zdc-graph` | `root.rs:125` `region_of`; `split.rs:385` `classify`, `:489` `clock_placement_refusal`, `:569` `classify_write`, `:1122` `walks_its_body`, `:1157` `form_of`, `:1968` `unread_warnings`; `integrity.rs:963` `int_01` |
| `zdc-codegen` | `lib.rs:1522` — which constructor makes the cell |
| `zdc-doc` | `prose.rs:180` `placement_sentence` |
| `zdc-lsp` | `complete.rs:471` `placement_token`, `tokens.rs:294` `placement_bit`, `server.rs:990` `detail` |

**Seven tests fail, in six binaries across four crates** — and that number is the interesting
one, because every single one of them iterates or sizes `Placement::ALL`, and no test that
writes its placements out by hand noticed anything: `zdc-ast`'s
`all_lists_every_placement_exactly_once`,
`placements_have_one_stable_word_and_index_each` and `placement_words_and_indices_are_unique`;
`zdc-doc`'s `every_placement_has_a_sentence_and_no_two_are_the_same`; `zdc-graph`'s
`static_is_the_one_placement_that_reaches_the_build_artefact_sink`; and `zdc-lsp`'s
`every_placement_keyword_opens_a_type` and `completion_works_before_the_source_can_parse`.

**The grammar costs nothing.** `remembered` is a soft keyword — `zdc_lexer::SoftKeyword`,
matched in `Parser::placement` (`crates/zdc-parser/src/decl.rs:124`) before the four hard tokens
— so the fifth placement reserved no word and invalidated no program that already used it as a
name. A sixth would be spelled the same way.

### What is load-bearing, and what is incidental

The inventory is twenty-two sites rather than two hundred because **code generation is keyed on
`MemberForm` and `Region`, not on `Placement`**. `MemberForm` appears outside `split.rs` in three
source files (`zdc-codegen/src/build.rs`, `zdc-codegen/src/server.rs`, `zdc-graph/src/ifc.rs`),
and a placement reaches the emitter only by having been turned into one of its forms. That
indirection is the load-bearing part. Two consequences fall out of it:

- **No fourth region.** `Region` is three (`root.rs:17`), and `durable` already demonstrates a
  placement that is storage rather than a machine. A model call runs where the API key is
  allowed to be, which is `Region::Server`, so `region_of` has an answer to give and the region
  set does not grow.
- **No new `Crossing` and no new `MemberForm`** were needed to make the tree compile. That is a
  fact about the inventory, not a design claim: the experiment proves which sites must answer,
  not that the answers given to them were the right ones.

Two things a `model` placement would need that no existing placement needed:

- **An eighth sink.** `Sink::CLOSED_LIST` has seven (`zdc-graph/src/ifc.rs:99`) and every one of
  them names a value reaching a browser, a build artefact or a log. None names a value leaving
  the *server* for a third party, and the reason is that nothing in the language does that on
  its own account: `request`, the only egress the grammar spells, is refused outside the client
  region by **E0363** in `split.rs`'s `Site::Outbound` arm. Server egress goes through a
  `foreign … is server`, where the author has taken the responsibility — checked by running the
  compiler, not inferred: a `secret` handed to one compiles with no error, which is §8's audit
  surface working as designed rather than a hole. A `model` placement is the compiler taking
  that responsibility back, and taking it back means a sink to attach *"a secret must not reach
  a prompt"* to. `Sink` is deliberately not `#[non_exhaustive]`, so adding the eighth breaks
  every consumer — the extension point working.
- **A runtime.** The fifth placement's is `crates/zdc-runtime/runtime/remembered.js`, 129 lines
  over a browser API that already existed. A model call has no such API to wrap.

### The integrity half, which #210 asks about specifically

A model answer is attacker-influenced by construction, and the closed lattice already treats it
that way **without anything being added**. `Grant::CLOSED_LIST`
(`crates/zdc-graph/src/integrity.rs:201`) has eight grants and none of them awards Trusted to a
`foreign` on the strength of `is anywhere`: G-FGN-A was deleted and replaced by G-FGN-P,
conditional on a `gives pure` declaration. A model call written as an ordinary `foreign` is
Untrusted, and so is everything derived from it.

What survives is **R5**, and it is the reason to prefer a placement over a library. `gives
trusted` and `gives pure` are, in `Grant::ForeignTrusted`'s own words, *asserted by a human and
checked by nobody* — so a `foreign` wrapping a model call and declared `gives trusted Text`
launders the model's answer into the Trusted half in one line, and nothing in the compiler
objects. A placement's classification is the compiler's and cannot be annotated away, which is
an argument for §15.1's shape that §15.1 does not make.

### What would break it

1. **Replacing an exhaustive `match` with `matches!` or `!=`.** Seven such predicates on a
   placement exist today — `zdc-graph/src/ifc.rs:801`, `zdc-types/src/routing.rs:317,460`,
   `zdc-codegen/src/analysis.rs:43,577`, `zdc-codegen/src/view.rs:2443`,
   `zdc-codegen/src/expr.rs:545`. Each is correct for the placements that exist, and each is a
   site a sixth would pass through with an answer nobody wrote.
   `scripts/check-wildcard-arms.sh` cannot see them: it reports clippy's
   `wildcard_enum_match_arm`, and a `matches!` has no wildcard arm to report.
   `SignalPlacement::may_be_secret`'s doc comment already says this about the two sites it
   replaced; these are what is left.
2. **A hand-written list of variants in a test.** This is the one that had already gone wrong,
   so it is repaired here rather than only recorded. `remembered` reached every `match` and none
   of the placement lists the tests iterate; three test files carried them and every one still
   described a five-placement world, including two tests named *total over every context and
   placement* that never passed `Remembered` to either classifier. Both failed the moment it was
   passed in — their expected tables ended in wildcards answering `remote` and `command` where
   the classifiers answer `direct` and `local` — so the combinations they skipped were exactly
   the ones they would have caught. The lists now come from `Placement::ALL` and the new
   `SignalPlacement::ALL`, whose completeness `SignalPlacement::index` enforces the way
   `Placement::index` already enforced the other. See `CONTRIBUTING.md`.
3. **Marking `Sink` or `Grant` `#[non_exhaustive]`.** Both are closed on purpose, and both say
   so in the same words: a new variant must break every downstream `match`.
4. **Emitting per placement instead of per `MemberForm`.** That is what turns twenty-two sites
   into however many places the emitter asks where a value lives.
