# Decisions

Questions this project has answered, with the answer, the reasoning, **what the answer rules
out**, and what would have to change for it to be reopened. An entry with no rejected
alternative is not a decision and does not belong here.

**Why this file exists at the root.** These are cross-cutting: one is about the integrity
lattice, one about the benchmark suite, one about the emitted page shell. Scattering them meant
[`ROADMAP.md`](ROADMAP.md) and [`STATUS.md`](STATUS.md) each carrying half of the same
position — which is the complaint that opened #157, *"two documents recording a deviation is
one more than a decision needs."* Those files now point here rather than restate. `docs/` is
not a candidate: it is `.gitignore`d, so a decision written there is not in the repository.

Each entry names the issue it closes, so the discussion is one `gh issue view` away. Every file
reference was checked against the tree at the commit that added the entry; line numbers move,
the names do not.

---

## 1. The integrity lattice carries no robustness claim (#212)

**Decided 2026-08-09.**

**The question.** The polarity is settled — default-closed, `Trusted ⊑ Untrusted`, a value is
Untrusted unless a grant in the closed set says otherwise (§21.7, and
`crates/zdc-graph/src/integrity.rs`'s module doc is the statement of it). What was left open is
whether, standing on that settled direction, a *robustness* property can now be claimed at all,
even a bounded one.

**The answer: it stays withdrawn, and this is the reason rather than the absence of one.**
§21.8.8's four untouched residual risks were re-argued against the default-closed direction,
and the direction does not reach any of them:

- **R3 / #29 — nothing bounds cumulative disclosure.** `limit` is per declaration and per
  anonymous session; two declarations compose to twice the budget and nothing counts. Polarity
  is a property of the *integrity* lattice; a disclosure budget is a quantity on the
  *confidentiality* one. The two are independent axes, so inverting one moved this not at all.
- **R5 / #30 — `gives trusted T`, `gives pure T` and `is anywhere` are asserted about
  third-party JavaScript and checked by nobody.** Default-closed makes this *more* load-bearing,
  not less: under a default-open lattice a false assertion could only fail to raise a flag,
  whereas under a closed grant set an assertion is the *only* way into the Trusted half, so the
  set's soundness rests entirely on unverifiable human claims. Purity of arbitrary JavaScript is
  undecidable and §14E.4's dev-mode check validates the shape of a return value and nothing
  else. No mechanism inside this compiler can discharge it.
- **R6 / #31 — a purity grant has no argument chain for an attacker-reachability walk.** The
  `report.json` walk is the one readability mechanism the design has, and it does not reach the
  grants the direction now depends on. Same direction as R5: worse under a closed set, because
  those grants matter more.
- **R7 N2 / #32 — one visitor reading another's row is a leak that compiles.** This is a
  relation between two principals. A two-point lattice has no principal to name, so no
  strengthening of it can express the property, let alone check it.

Three of the four are outside what a two-point, declaration-level lattice can *express*; the
fourth is unverifiable in principle. A bounded claim would therefore have to be bounded to
programs that declare no `foreign` grant, disclose at most once ever, and read at most one
visitor's rows — and **`grep -rl 'release\|gives pure\|gives trusted' examples/` returns
nothing across all 27 checked-in programs**, so the sentence would describe a set no example is
in. A guarantee that covers none of the code you can read is worse than no guarantee, because
it will be quoted without its qualifications.

**What this rules out.** No statement in `report.json`, in diagnostic help text, in this
repository's prose, or in §21.7.10's sentence may tell a user that a program is robust, free of
laundering, or that `limit` bounds what leaves. The rules built on §21.8.8 option 2's terms —
the declaration shape, the report, `limit`, REL-PLACE′, REL-CLOSED, REL-PURE, REL-ARG — stay
built and stay **review aids**. `crates/zdc-graph/src/integrity.rs` says so in prose and that
prose is normative.

**What is not decided here, because it is not this file's to decide.** §21.8.8 option 3 —
abandoning the two-point lattice for per-principal labels — is the project owner's call and
nothing above forecloses it. It is the mechanism every predecessor used and every predecessor
died of, and §21.8.5 measures the annotation burden of not having it at 38% self-endorsement
noise. This entry decides only that the claim stays withdrawn *while the lattice is what it is*.

**What would reopen it.** Any one of: (a) the owner spending option 3; (b) `limit` becoming a
budget that composes across declarations, which closes R3; (c) a way to check a purity or trust
assertion about third-party JavaScript, which closes R5 and R6 together. Until one of those
lands, re-argue nothing — the answer is here.

---

## 2. A `model` placement stays possible; here is exactly what it would touch (#210)

**Decided 2026-08-09.** Audited against the tree, by grep, not by assumption.

**The question.** §15.1 records `model` — an LLM call as a further placement — as a
designed-for extension. Is "kept possible" still true, and does the exhaustiveness mechanism
still force every site that enumerates the placements to be updated?

**The answer: yes for `match`, no for comparison, and the gap is fourteen sites.**

*The mechanism holds where it was claimed.* `Placement` has four variants
(`crates/zdc-ast/src/lib.rs`), `Placement::ALL` is a four-element array and `Placement::index`
is a total `match` whose doc says why. Both are pinned by
`crates/zdc-ast/src/lib.rs`'s `ALL.len() == 4` assertion and by
`crates/zdc-ast/tests/public_contract.rs`. Adding a fifth variant is a compile error at every
exhaustive match, and there are thirteen: `Placement::index` and `Placement::word` (`zdc-ast`);
`SignalPlacement::from_ast` and the placement word (`zdc-types/src/placement.rs`); the keyword
mapping (`zdc-parser/src/decl.rs`); the per-instance-state explanation
(`zdc-resolve/src/resolve.rs`); `region_of` (`zdc-graph/src/root.rs`); the two-way-binding
writer set and the client-signal rule (`zdc-graph/src/integrity.rs` — which already carries the
comment *"a fifth placement must be ruled on here rather than defaulting into the grant"*); and
four in the language server (`complete.rs`, `tokens.rs`, `hover.rs`, `server.rs`).

*It is enforced, not merely present.* `scripts/check-wildcard-arms.sh` names `Placement`,
`SignalPlacement`, `Region` and `RootKind` in its guarded set and fails the build on a wildcard
arm over any of them. That script exists because the *fourth* placement, `static`, was added and
a `Client | Server | Durable => …` arm in the completion engine silently gave it a value
position's behaviour. This is the one mechanism in the repository written after the exact
mistake it prevents.

*Where it does not reach, and this is the finding.* Clippy's `wildcard_enum_match_arm` sees
`match`. It does not see `==`, `!=` or `matches!`, each of which carries an implicit "everything
else" branch that no gate can inspect. **Fifteen sites in `crates/*/src` test a placement
against a single variant. One of them is immediately followed by an exhaustive `match` and is
therefore caught (`zdc-resolve/src/resolve.rs`). The other fourteen would hand a fifth placement
the other branch with no compile error and no test failure:**

| Site | Test | What a fifth placement silently becomes |
|---|---|---|
| `zdc-graph/src/split.rs` (E0313) and `zdc-graph/src/ifc.rs` (E-IFC-01) | `matches!(Client \| Static)` | **permitted to be `secret`** |
| `zdc-graph/src/ifc.rs` (`is_client_state`) | `matches!(Client)` | not client state |
| `zdc-graph/src/split.rs` (E0314) | `!= Static` | forbidden to be `emitting` |
| `zdc-types/src/routing.rs` (×2) | `!= Static` | not a prerender source |
| `zdc-types/src/infer.rs`, `zdc-codegen/src/view.rs` | `!= Client` | refused a two-way binding |
| `zdc-codegen/src/analysis.rs` (`is_reactive_signal`) | `!= Static` | a reactive cell |
| `zdc-codegen/src/analysis.rs` (seed set) | `== Client` | not a client seed |
| `zdc-codegen/src/expr.rs`, `zdc-codegen/src/lib.rs` (×2) | `== Static` / `!= Static` | not build-inlined |
| `zdc-codegen/src/lib.rs` | `== Durable` | no store endpoint |

The first row is the one that matters. `secret` is admitted for anything that is not client-side
or build-time, so a `model` placement would inherit permission to hold an API key without anyone
ruling on it. That is the *right* answer — a model call is exactly where a key belongs — but it
would be right **by accident**, which is precisely the defect §21.8.4 diagnosed and named: *a
rule stated over a classifier built to answer a different question.* The audit's practical output
is that these fourteen are the hand-review list, and no gate will produce it for you.

**Two things a fifth placement would *not* need, and both are load-bearing for "it falls out".**
`Region` stays at three: a model call is issued from the server, so `region_of` maps `model` to
`Region::Server` exactly as `durable` does, and no new execution region is introduced. And the
default-closed lattice already gives the right answer for free — a model's output derives from
no grant in `Grant`, so it is Untrusted by construction with no new rule. §15.1's claim that
the machinery is already there survives the audit.

**What this rules out.** Building `model` now — §15's own prerequisite is that the placement
pass, IFC and runtime exist first, and they do, but the four residual risks of entry 1 are open
and a model call is precisely a place where an unchecked foreign grant would matter. It also
rules out treating the exhaustiveness mechanism as complete: it is complete for `match` and
silent for comparison.

**What would break "kept possible".** (a) Removing `Placement` or `SignalPlacement` from the
guarded set in `scripts/check-wildcard-arms.sh`. (b) Growing the table above. The count of
unguarded single-variant tests is the metric — fourteen today, by
`grep -rn 'Placement::' crates/*/src` — and a change that raises it is spending the extension's
budget without saying so. (c) Collapsing `SignalPlacement` into a client/server boolean
anywhere, which would erase the distinction a fifth placement needs.

**What would reopen the build decision.** Entry 1's R5 gaining a check, or a decision that a
`model` result's untrustworthiness is sufficient without one.

---

## 3. A program depends on another program by containing it (#174)

**Decided 2026-08-09.**

**The question.** There is no mechanism for depending on code outside the project, and §13 lists
cross-file modules among the v1 non-goals while `use` exists. What is the position?

**The answer: the unit of dependency is the file, and the boundary is the project sandbox.**
`use "./x"` is the only import there is. The project root is the entry file's parent directory,
canonicalised and **fixed once for the whole build**
(`zdc_hir::sandbox::project_root`, called once in `zdc-resolve/src/modules.rs`), and every path
the build may open — module imports and the `build read` / `build list` / `build markdown`
capabilities alike — passes `zdc_hir::sandbox::refuse` *before* the read, through a single entry
point that no caller can half-adopt. Symbolic links are caught, because the check
canonicalises rather than inspecting the string.

**So the answer to "how do I use someone else's ZDeceptron code today" is: copy it into your
project.** `use "./vendor/thing/model"` is an ordinary in-sandbox path and needs no new
mechanism, no manifest and no compiler change. That is a real answer rather than a placeholder:
it is `examples/blog.zd`'s own `use "./layout" for PageShell, PostCard` with a longer path.

**What this rules out.**

- **URL imports** (`use "https://…"`, the Deno shape). They would make the set of bytes entering
  a compilation depend on the network. `examples/blog.zd` and `examples/writing.zd` are verified
  to build with an empty `PATH`; a fetching import gives that up for every program, permanently,
  in exchange for convenience in a language with no users yet.
- **A registry and a lockfile.** Same objection plus a second: the sandbox is a *security*
  property, and a registry moves the decision about which bytes are compiled from a path the
  program wrote to a name a server resolves.
- **Deriving the root from the working directory.** `sandbox.rs` records why: the same program
  would compile or fail depending on where `zdc` was run from, and running it from `/` switches
  the sandbox off entirely.

**The constraint on any future answer**, which is the reason this is written down at all. A
package mechanism may arrive; it must (a) fix one root per build before the first read rather
than re-basing per file — the boundary used to be recomputed per module, which is not a boundary
at all, because a chain of imports could walk anywhere one `..` at a time; (b) route every path
through `sandbox::refuse`'s single entry point; and (c) leave the byte set reproducible without
a network.

**What would reopen it.** A manifest is the shape the eventual answer takes, and `sandbox.rs`
already says why it was deferred rather than rejected: *"requiring a manifest would settle the
question, but v1 has no manifest and inventing a file format is a language decision, not a fix
to this one."* The trigger is a program for which the entry file's parent is the wrong root —
an entry in `src/` with modules in `lib/` — or a second project that actually wants to share
code and has measured the cost of copying it.

---

## 4. `insta` is not adopted; the deviation is permanent (#157)

**Decided 2026-08-09.**

**The question.** The spec's testing table asks for snapshot tests via `insta`. `insta` is a
dependency of no crate and the coverage exists as ordinary assertions. Does the spec change or
does the dependency land?

**The answer: the spec's testing table is superseded on this row. `insta` does not land.**

**Why.** The coverage the table was buying already exists in a better shape. `zdc-lexer` has 63
tests, `zdc-parser` 149 and `zdc-ast` 4, and the codegen suite is built on *parity* rather than
on accepted output: `dom_parity.rs`, `element_parity.rs`, `page_parity.rs` and
`component_parity.rs` compare the emitted tree against what the real runtime constructs. That is
a comparison between two implementations, and it is strictly stronger than a comparison against
a blob someone approved — a snapshot cannot tell you the emitter and the runtime agree, only
that the emitter has not changed.

The second reason is the repository's own testing rule. `scripts/check-vacuous-tests.py` is a CI
gate whose entire subject is *a test that passes while inspecting nothing*, written after a
test named `a_static_initialiser_is_walked_by_the_flow_pass` passed unconditionally while
masking a soundness hole. `cargo insta accept` is the ergonomic that manufactures that failure
mode at scale: it turns a review into a keystroke, and a snapshot that was accepted without
being read is a test that has stopped being one while its name still claims coverage. Adopting a
tool whose happy path is the thing another gate exists to catch is not a cost this project should
pay for output it can assert about directly.

**What this rules out.** Converting existing assertions to snapshots, and adding `insta` for new
work. It does *not* rule out golden artefacts in general — `zdc-bench` already commits a
generated region of `BENCHMARKS.md` and fails the build when it drifts, with
`ZDC_BLESS=1` as the regeneration path. That pattern is fine and is the one to copy if a large
expected output ever needs committing; it costs no dependency.

**Consequence, applied.** `.gitignore` no longer carries `*.pending-snap`, which was an
`insta` artefact pattern for a dependency that does not exist.

**What would reopen it.** A test that genuinely needs a multi-kilobyte expected value, where the
`ZDC_BLESS` pattern above proves insufficient — for instance a second surface dialect (M9)
whose whole point is that the same program prints differently.

---

## 5. The React and SolidJS benchmark arms stay unbuilt, and the stated reason is corrected (#158)

**Decided 2026-08-09.**

**The question.** §14A.4 asks for a `js-framework-benchmark` comparison against React and
SolidJS. `BENCHMARKS.md` says they are not measurable. Is that a decision or an omission?

**The answer: a decision. The arms are not built, and the comparison keeps being stated as
missing rather than quietly dropped.**

**The reason had to be corrected first, because the one on record is not true.**
`BENCHMARKS.md` said *"CI has no network"*. CI plainly has a network — `.github/workflows/ci.yml`
fetches actions and crates on every run. The true constraints are two, and neither is about
connectivity:

1. **The project's own claim is that it installs nothing.** `.github/workflows/release.yml`
   opens with *"no Node, no npm"*, `README.md` says the same, and `zdc` is one static binary. A
   benchmark arm that needs `npm install` makes the claim conditional.
2. **The harness runs in `boa`, the compiler's embedded JavaScript engine, against a counting
   DOM shim.** Getting `react-dom` to run there means a shim faithful enough for its scheduler,
   its synthetic event system and `MessageChannel` — a second, much larger DOM implementation
   whose fidelity nothing would check, in a suite whose entire value is that its counts are
   exact.

**The alternative that was actually available, and why it is rejected.** A package manager is
not strictly required: React and SolidJS both publish single-file browser bundles that could be
vendored into the repository, and the metric this suite reports — DOM crossings, effect runs,
text writes — is *engine-independent*, so counts taken under `boa` would be comparable in a way
timings never could be. This is the strongest case for building the arms and it is worth stating
before refusing it. It is refused because the cost is ~150 kB of minified third-party JavaScript
committed to a workspace that runs `cargo deny` and `cargo audit` over every dependency it has,
plus the shim in point 2, plus a vendored version that ages silently — bought for a number that
§14A.2 already predicts (*"against SolidJS and Svelte 5 we expect parity, not victory"*) and
whose falsification would not change any decision in the language.

**What this rules out.** Adding `npm`, `node_modules`, or a package manager to CI for benchmark
purposes; and quietly deleting the row from `BENCHMARKS.md`'s "what §14A.4 asks for and this
cannot give" table. **Nothing in this repository is a measurement against React or Solid**, and
that sentence stays.

**What would reopen it.** The DOM shim becoming faithful enough to host `react-dom` for some
other reason — server rendering (#138) and hydration (#208) both push in that direction — at
which point the crossing counts become nearly free and the arm should be built. Also: a real
browser harness, which would make the timing half meaningful and change the calculus entirely.

---

## 6. The wire format is unversioned by rule; a mismatch is named at the decode site (#144)

**Decided 2026-08-09.**

**The question.** `crates/zdc-runtime/runtime/wire.js` is a tagged codec. What happens when the
two ends disagree about the format?

**The answer: nothing negotiates, because within a build there are not two ends.** One
`zdc build` emits the client bundle, the server handlers and the store adapter from one compiler
run over one program, and all three link the same `wire.js`. The compatibility rule is therefore
an obligation on the *compiler*, not a field in the payload:

1. **The encoding of a shape that has ever been persisted may not change.** Durable values are
   stored encoded, so a change to how a shape is written silently reinterprets data written by
   an older build. This is not hypothetical: #204 stored a `[1]` as
   `{"base":[],"item":1,"flat":null}` because `encode` walked past a `toJSON`, and no error was
   raised at either end.
2. **New shapes take a new `$`-prefixed marker.** `$` is outside `XID_Start` and
   `XID_Continue`, so no ZD record field can collide with one — the same argument that makes
   `$map` unambiguous by construction rather than by convention.
3. **A disagreement is a named failure at the decode site, never a coercion.** `decode` throws on
   a `$map` carrying sibling keys, on a non-array `$map`, and on a malformed pair. `rpc.js`
   classifies a decoder rejection as `Failed(Rejected)`, one of the three closed `FailureCode`
   variants, so it reaches the program as a language-level value rather than as a console
   message. Every one of those used to be a silent conversion, and silence is the one thing a
   persistence format may not do.

**What this rules out.** A version integer in an envelope around every request and every stored
value. It was the obvious answer and it is rejected on three counts: the envelope would have to
be added to `rpc.js`, `store.js`, every emitted endpoint and every deploy adapter, so it is the
expensive kind of change; it protects the case that cannot happen (two halves of one build
disagreeing) and not the case that can (an older build's *stored* bytes); and a version number
answers "are these the same format" when the question a stale durable value actually poses is
"is this value the shape this program's type says it is" — which is a digest over the declared
shape, and belongs in the store, not in the codec. It also rules out lenient or best-effort
decoding of anything.

**The gap this leaves, stated rather than hidden.** Of the two places two builds genuinely meet,
one is named and one is not. A stale browser tab posting to a newly deployed server gets
`Failed(Rejected)` — correct, if unspecific. A durable value written by an older build whose
encoding changed still decodes into the wrong ZD value with no error, and rule 1 above is a rule
a human keeps, not one the compiler checks. The mechanism that would check it is a digest of the
declared shape stored beside the value, refused on mismatch.

**What would reopen it.** Client and server bundles becoming independently deployable, which
would make an envelope version buy something; or the codec gaining a second marker, at which
point rule 2's forward-compatibility story needs a test rather than an argument.

---

## 7. No `noscript` fallback is emitted (#141)

**Decided 2026-08-09.**

**The question.** The emitted `index.html` is a shell — `<div id="app"></div>` and a module
script (`zdc-codegen/src/lib.rs`'s `index_html`). With scripting off, it renders nothing. Should
the compiler emit a fallback?

**The answer: no. The answer to a page without scripting is server rendering (#138) and
hydration (#208), not a placeholder — and until those land the shell's requirement is documented
in [`README.md`](README.md) rather than apologised for in the page.**

**Why.** The only thing the compiler could emit today is a sentence, and it has no sentence to
emit. **`index.html` currently contains no compiler-authored prose at all**: the `<title>` comes
from the program's metadata and `<html lang=…>` from the program's declared language. A
`<noscript>This page requires JavaScript</noscript>` would be the first natural-language string
the compiler ever put in a user-visible document, in English, inside a document whose `lang` the
program chose — so a French program would ship an English apology. A message catalogue to fix
that is a language feature (M9's dialects are about the *source* surface, not the output) and is
far more machinery than the problem justifies.

The second reason is that the placeholder is a thing that would have to be removed again. Once
#138 puts the first paint in the document, a page *does* have content without scripting, and a
`<noscript>` saying otherwise becomes a lie the compiler emits. Shipping text whose planned
lifetime is "until the real fix" is how a codebase accumulates apologies.

**What this rules out.** Emitting `<noscript>` content, and emitting the page title or any other
program string into a `<noscript>` block as a compromise — the second is worse than the first,
because a bare title with no explanation tells a reader nothing about why the page is empty.

**What would reopen it.** #138 landing without covering every route: if server rendering is
per-page and some pages stay client-only, those pages need something, and at that point the
compiler knows *which* pages and can say so. Also: any decision to give the compiler a message
catalogue for emitted output, which removes the language objection above entirely.
