# Contributing

Thank you for looking. This is a compiler for a language nobody is using yet,
so the most useful contribution is usually a program that should compile and
does not, or one that compiles and should not.

## The short version

```sh
git clone https://github.com/DrDrewCain/zdeceptron
cd zdeceptron
cargo build --release
cargo test --workspace --no-fail-fast
```

Branch names are `feature/`, `fix/`, `chore/` or `refactor/`. Commit subjects
are imperative and fit in 72 characters. Run the gates below before opening a
pull request; they run in CI anyway and it is faster to find out locally.

`--no-fail-fast` is not decoration. A bare `cargo test --workspace` stops at
the first failing target, and a run that reports 279 tests and stops looks
almost exactly like a run that passed.

## The gates, and why each one exists

CI runs nine checks. Seven of them are scripts in `scripts/` that enforce
rules you will not have met in another Rust project, and **every one of them
was written after the bug it prevents had already shipped**. None of them is
style. If you hit one, the story below is what it is protecting.

### `cargo fmt` and `cargo clippy -D warnings`

The ordinary two. Nothing surprising.

### Every crate forbids `unsafe`

Each crate root carries `#![forbid(unsafe_code)]`. The check reads crate roots
from `cargo metadata` rather than a glob, so a crate with a non-default `path`,
a second `[[bin]]`, or a build script cannot be skipped silently — and it
asserts the *number* of roots it scanned, so a check that scans nothing fails
rather than passing vacuously.

### No wildcard match arm over a guarded enum

**The story.** This codebase was repeatedly described as having no wildcard
match arms by design. It had dozens. Then `static` was added as the fourth
placement, and the completion engine's `Client | Server | Durable => InType`
arm silently gave it a value position's behaviour. No compile error, no test
failure, wrong output.

So a `match` over one of the guarded enums must list its variants. Adding a
variant is then a compile error at every site that has to think about it,
which is the entire reason for having a closed enum. The guarded set is
deliberately not every enum in the workspace — a wildcard over an open one is
fine.

### No emitter writes its own quotes around a placeholder

**The story.** Three injection holes have been found in this compiler, in
three different emitters — the `import` clause, the generated `class` getter,
and the folded stylesheet. All three had the same shape: a `format!` that
wrote an opening quote, then `{something}`, then a closing quote, with the
something coming from the program being compiled.

`js::string` and `js::json_string` exist so that quoting and escaping are
decided in one place. A site that writes its own quotes has opted out of that,
which is why the check looks for the shape rather than for the bug.

### No test that cannot fail

A `#[test]` that asserts nothing — or asserts something that cannot be false —
is worse than no test, because it reports coverage it does not have. The check
finds them.

If you are tempted to write `assert!(a || b || c)` because you are unsure which
of three shapes the output takes, the test is telling you to go and find out
which one it is. That exact assertion has been caught by this gate.

The gate is static, so it catches the shapes somebody thought to describe. The
harder kind has the shape of a real test and measures something else: a gate
that found a function's end by the first `}` in column 0 until minification
moved the brace; a test that asserted a page mounted, against a page that threw
on its first line, because its harness linked a hand-written module list that
had stopped being the whole list. `crates/zdc-runtime/tests/mutation.rs` is the
answer to that class. It changes the runtime's JavaScript — an equality
reversed, a function made to throw — and asserts that some suite goes red.

Two of its tests run in a plain `cargo test`; the full sweep of 236 mutants is
`#[ignore]`d and has the `mutation` CI job to itself. It prints every surviving
mutant by name and gates the *set* of them against a table with a written
reason per group. **If it fails on your branch, read the names before touching
the numbers.** A mutant that stopped dying means a test stopped checking
something. A mutant that started dying is good news, and recording it — lowering
a count, or deleting a group that has emptied — is how the next reader learns
the hole was filled.

### The editor grammar matches the lexer

The VS Code grammar highlights keywords. The lexer decides what a keyword is.
When they drift, the editor colours a word the compiler will reject, which is
a worse experience than no highlighting.

### The two supply-chain gates agree

`cargo deny` and `cargo audit` read the same RustSec database from two
different config files. If one ignores an advisory the other does not, one of
them is lying about the dependency graph — and which one is lying depends on
which CI job happens to run first.

Every ignored advisory must also carry a comment. An exception with no stated
reason is indistinguishable from an oversight, and it is the thing nobody ever
revisits.

### `unsafe` in the dependency graph

`#![forbid(unsafe_code)]` covers first-party code only. It says nothing about
the ~180 crates the compiler links, which is where essentially all the
`unsafe` in a Rust binary lives. `cargo geiger` counts that and holds it under
a ceiling — and independently confirms that every first-party crate reports
zero, so the forbid check is not marking its own homework.

### The minimum supported Rust version

`rust-version` in `Cargo.toml` is built on exactly that toolchain in CI. An
unenforced MSRV is the same as no MSRV: a consumer finds out it is wrong, not
the project. It is a fact about the dependency graph — currently `redb`'s
floor — so raising it is a decision about which dependency to accept.

## Tests

Test-first. Write the test, watch it fail, then make it pass. A test written
after the code passes immediately, which proves nothing about whether it can
catch the bug.

Two things this project cares about more than most:

**Count, do not time.** A wall-clock assertion is a fact about the machine that
ran it and about what else that machine was running. Where a test needs to show
that something is linear rather than quadratic, instrument the thing and count
it. `crates/zdc-codegen/tests/depth.rs` counts flattens by injecting a counter
into the emitted JavaScript, and it is the model to follow.

**Assert the answer, not just survival.** A test that only checks a program did
not crash will pass for a program that returns the wrong number.

**Linking a module is not testing it.** `markup.js` holds the only assignment
to `innerHTML` in the runtime. Two JavaScript suites load it and neither calls
it, which nobody noticed until something poisoned the module and watched both
suites stay green. If a test loads a thing, make it call the thing.

### Snapshot tests, and the dependency this project does not take

**DECIDED 2026-08-16, closing #157. `insta` is not a dependency of any crate
here and is not going to become one.** The design spec's §11 testing table asks
for snapshot tests of the lexer and parser; `STATUS.md` and `ROADMAP.md` both
recorded the absence, and two documents recording a deviation is one more than
a decision needs. The table's entry is amended to what the tree already does: assertions that name
the property, with a blessed artefact only where nobody could write the
expectation by hand.

**It was tried, and that is the part neither document said.** `insta` was a
dev-dependency of `zdc-lexer`, `zdc-parser` and `zdc-cli` — the two crates
§11's row names, and the one the snapshot lived in — and it produced exactly
one:
`parse_examples__voting_board.snap`, 853 lines of `voting-board.zd`'s
debug-printed AST. Commit `87b1b5d`, *"Replace brittle AST snapshot with
focused coverage"*, deleted it on 2026-08-02, and fifteen packages left
`Cargo.lock` with it. What stands in its place is
`voting_board_exercises_the_front_end` in
`crates/zdc-cli/tests/parse_examples.rs`, which names the five state
declarations in order, matches `rank`'s four pipeline clauses one at a time,
and walks the view down to the click handler's mutation. The snapshot recorded
the whole tree; the test says which parts of it are the front end's job.

**Why it was brittle is why it is not coming back.** A snapshot's expected
value is produced by the code under test: the first run writes it, and every
later run compares the code against what the code said last time. So it cannot
be written before the code and cannot be watched to fail, which is the first
rule of this section — and what it finally asserts is that the output has not
changed, which is not the answer. `nesting_emits_indent_and_dedent`, in
`crates/zdc-lexer/src/layout.rs`, is the indentation edge case §11's table had
in mind, and its expectation is a written-out token vector saying what the
layout algorithm *should* emit. That one can be written first.

**And a red snapshot has a documented repair that a red assertion does not.**
`crates/zdc-diagnostics/tests/caret_labels.rs` asserts on rendered output —
exactly the shape a snapshot covers — and what it asserts is that the caret's
message names the word the user wrote and that no diagnostic says only `here`.
A snapshot of the same report would pin `ariadne`'s box drawing and colour
escapes alongside that sentence, go red for either, and be repaired both times
by accepting the new output. The assertion has to be argued with instead.

**The one blessed artefact this project needs already exists, and it costs no
dependency.** `crates/zdc-bench/tests/benchmark.rs`'s
`the_committed_results_match_the_measurements` regenerates `BENCHMARKS.md`'s
results region under `ZDC_BLESS=1` and exact-match gates it otherwise: a
snapshot in all but name, over a table no one could hand-write, in a document a
reader reads, where blessing arrives as a diff in a pull request and the panic
message says to review it. That is the mechanism to reach for if a second such
artefact appears.

**What would change this.** Not that somebody wants a snapshot. A second and a
third artefact of that kind — the obvious candidate is a whole emitted
`client.js`, asserted today a fact at a time — at which point `ZDC_BLESS` is
being hand-written per test and `insta` is the cheaper of the two. Until then
the dependency buys nothing the tree does not have, and it is not free: the
`msrv` job compiles dev-dependencies with `--all-targets` on the declared
minimum, so a test-only crate's floor becomes the floor a contributor installs.

**No gate enforces this.** Every check in `scripts/` was written after the bug
it prevents had already shipped, and a dependency does not arrive by accident:
adding this one is a line in three manifests and fifteen packages in the lock
file, in front of whoever reviews it. What it does remove is `*.pending-snap` from
`.gitignore`, which outlived the tool that wrote those files by two weeks.

## Documentation goes stale, so check it against the compiler

This is the failure mode this repository actually has. `README.md`,
`STATUS.md`, `ROADMAP.md` and the comments in `examples/` have all, at various
points, described a language that no longer existed — usually because an issue
was closed and the prose that referenced it was not.

Two habits:

- Before writing that something is impossible, try it. Several example
  comments said "no `Map` can be built" for weeks after `mapOf` landed.
- Before citing an issue number, check whether it is still open. Eight of the
  nine issues referenced in `examples/` turned out to be closed.

If you change the CLI, re-run the commands in the README rather than editing
the prose to match what you think it now does.

## Issues

Remaining work lives in the issue tracker, indexed by
[#35](https://github.com/DrDrewCain/zdeceptron/issues/35), not in `ROADMAP.md`
prose. Some issues are labelled `open-decision`; several of those restate
questions the spec has already settled, so it is worth grepping the spec for
`DECIDED` or `WITHDRAWN` before designing against one.

A good bug report is a `.zd` program, what you expected, and what happened.
The program is the important part — it becomes the regression test.

## Releases

**Releases are the owner's call, and that is enforced rather than assumed.**
A repository ruleset restricts creating, moving and deleting `refs/tags/v*` to
administrators, so pushing a tag is not something everyone with write access
can do. Both jobs that publish anything then sit behind environments with a
required reviewer. A pull request cannot start a release, and neither can a
tag on its own.

Tagging `v*` builds binaries for five targets and opens a **draft** release.
Whether a version is ready is a judgement made by a person looking at the
artefacts, not by a tag having been pushed.

The same tag also publishes all twenty crates to crates.io, so
`cargo install zdc-cli` works. No tag has been cut yet, so neither that nor
the installer script does anything today. The build half has been exercised
through `workflow_dispatch`, which runs the five target builds and their
smoke tests without tagging, releasing, or touching crates.io — that is the
way to check a release change before it is irreversible. The order is computed by
`scripts/publish-order.py` rather than written down — a hand-written list
keeps working right up until somebody adds a crate, and the place it fails is
half way through a release with some crates published and no way to unpublish
them. CI checks that an order exists at all, so a dependency edge that makes
one impossible fails on the pull request that adds it. That job waits on a protected
environment: a crates.io version is permanent — it can be yanked but never
deleted — so it does not run off a tag alone.

Two consequences worth knowing before you move a file. A crate may only embed
files inside its own directory, so `include_str!("../../something")` compiles
in a workspace build and breaks the published crate; that is why the runtime
JavaScript lives in `crates/zdc-runtime/runtime/` and not at the repository
root. And every path dependency between these crates carries a version as well
as a path, because the path is what a workspace build uses and the version is
what a published build resolves. Dev-dependencies stay version-less on purpose.

See [`CHANGELOG.md`](CHANGELOG.md).

## Licence

MIT. By contributing you agree your contribution is licensed under it.
