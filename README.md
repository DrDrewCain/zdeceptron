# ZDeceptron

[![CodeRabbit Pull Request Reviews](https://img.shields.io/coderabbit/prs/github/DrDrewCain/zdeceptron?utm_source=oss&utm_medium=github&utm_campaign=DrDrewCain%2Fzdeceptron&labelColor=171717&color=FF570A&link=https%3A%2F%2Fcoderabbit.ai&label=CodeRabbit+Reviews)](https://coderabbit.ai)

**A reactive dataflow language where placement is a property of state, and the compiler derives the network.**

> ⚠️ Early development. The compiler is real — lexer, parser, name resolution, Hindley–Milner
> type checker, tier split, information-flow pass, JavaScript code generator, durable store,
> platform adapter, dev server and language server all exist and are tested. **`client`,
> `server`, `durable` and `static` programs all build, and the server half executes**: two
> browser windows move together over live sync, proven by
> `crates/zdc-host/tests/two_windows.rs`. Two of the three gaps this note used to name have
> closed: `build read`, `build list` and `build markdown` read the project directory inside the
> compiler's own sandbox, and `examples/blog.zd` renders real markdown off disk into the bundle.
> What is left is narrower and worth stating exactly: **only the compiler can make a `Markup`**,
> from a file, at build time, so a value a program computes still cannot become one. And
> **`zdc deploy` has never been run against a real account**: it writes a complete deployment for
> four targets and has been checked against vendor documentation, never against a vendor. See
> [`STATUS.md`](STATUS.md) for the milestone-by-milestone truth and
> [`ROADMAP.md`](ROADMAP.md) for what is next.

---

## Documentation

**[zdeceptron.marksturman.com](https://zdeceptron.marksturman.com)**

| | |
|---|---|
| [Getting started](https://zdeceptron.marksturman.com/docs/getting-started) | Build the compiler, write a first program, run it. |
| [How it works](https://zdeceptron.marksturman.com/docs/language-tour) | The whole model on one page, with the placements you can poke. |
| [The tutorial](docs/tutorial.md) | One program in five steps, where the step that matters is one word. |
| [The language reference](docs/reference.md) | Every declaration, placement and type, systematically — and what is not implemented. |
| [The examples](https://zdeceptron.marksturman.com/docs/examples) | All twenty-eight programs, ordered by what they teach. |
| [The standard library](https://zdeceptron.marksturman.com/docs/standard-library) | The prelude, module by module. |

This `README` states the idea and the current boundary. The documentation is
the part you read to *use* the thing. To work on the compiler itself, see
[`CONTRIBUTING.md`](CONTRIBUTING.md) — it explains what each CI gate is
protecting, and every one of them was written after the bug it prevents had
already shipped. What changed between versions is in
[`CHANGELOG.md`](CHANGELOG.md).

---

## The idea

Every piece of state is a signal. Each signal declares *where it lives*:

```
secret state apiKey is server Text from environment "GREETING_API_KEY"
state visits is durable Whole starting 0
state name is client Text starting ""
state greeting is server Text from politeGreeting with name, apiKey

function politeGreeting with who, key
    if who is ""
        give "Hello, stranger."
    give "Hello, " + who + "."

view
    Column
        Heading "Guestbook"

        Input name, hint is "your name"

        when greeting
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with text   show Text text

        Button "sign the guestbook"
            on click
                add 1 to visits
```

That program compiles today, and it runs. `zdc build` on it emits `client.js`, `styles.css`,
`index.html`, `manifest.json`, and one file per derived server endpoint —
`functions/greeting.js`, `functions/visits.js` and `functions/visits.incr.js`. `apiKey` and
`GREETING_API_KEY` appear in none of the client output; that is checked by grepping the built
bundle, not asserted. It is [`examples/guestbook.zd`](examples/guestbook.zd) with the comments
removed. For what it still cannot do, see [Where it stops](#where-it-stops).

The UI is a pure function of the signal graph. The compiler walks that graph, and **any edge
crossing a placement boundary becomes transport** — client→server becomes an RPC,
anything→durable becomes a store command.

### Two rules generate the rest

**Crossing a boundary is visible in the type.** Reading a `server` or `durable` signal from the
client yields `Remote of T` — `Loading | Ready | Failed` — not `T`. The network appears in your
types exactly where the network is, and nowhere else. You cannot forget a loading state,
because the checker will not let you read through the variant without eliminating it.

**Secrecy flows.** A `secret` signal's taint propagates through every derivation, through data
*and through control*. A secret-tainted value reaching client state or the view is a compile
error. This is enforced by the information-flow pass in `zdc-graph`, whose negative test suite
is the crate's largest.

## Why

JavaScript doesn't dominate the frontend because it won a design contest. It dominates because
it's the only language browsers run natively. Plenty of languages escaped its *syntax* — Elm,
ReScript, Gleam, Dart — but they inherited its *deployment model*: you still needed a bundler,
a host, a backend, a database client. The language got nicer; the day stayed the same.

ZDeceptron targets the deployment model, the type system's soundness, and the syntax at once.
The design decisions are grounded in published evidence rather than taste — including Stefik &
Siebert's finding (*ACM Transactions on Computing Education* 13(4), 2013) that C-style syntax
scores no better with novices than *randomly generated* syntax.

## Status

**2200 tests pass across 18 crates**, with 0 failures and 5 deliberate `#[ignore]`s — three
that print a scaling survey rather than gating on it, and two that hold a known defect open:
a `give` after a pipeline run is emitted as unreachable code, and `Input` cannot bind a
component's own `state` though a handler can write it. The full picture, with the evidence
behind each row, is in [`STATUS.md`](STATUS.md).

Reproduce the count with `cargo test --workspace --no-fail-fast`. The flag matters: a bare
`cargo test --workspace` stops at the first failing target, and #192's wall-clock ratio test
fails often enough that a bare run reports 279 tests and stops — about an eighth of the suite,
with a tail that reads like an ordinary summary.

| Component | State |
|---|---|
| Lexer — Unicode identifiers, indentation layout | ✅ working |
| Parser, AST — expressions, statements, declarations, views | ✅ working |
| Diagnostics — names the valid phrasing, points at the span | ✅ working |
| Name resolution → HIR | ✅ working |
| Type checker — Hindley–Milner, `Remote of T`, records, choices | ✅ working |
| Tier split + information-flow pass | ✅ working |
| JavaScript codegen + runtime, `client` programs | ✅ working |
| Scoped CSS generation | ✅ working |
| `zdc new`, `parse`, `check`, `build`, `dev`, `deploy`, `explain`, `lsp` | ✅ working |
| Server function emission + RPC client | ✅ emitted **and** executed |
| Durable store, persistence, live sync | ✅ working |
| Components (`component`, `use`, `children`) | ✅ working |
| Routing — declared routes, one bundle per URL | ✅ working |
| Element vocabulary — 66 built-ins | ✅ working |
| Event payloads on handlers | ✅ working |
| `static` placement, build-time evaluation, file emission | ✅ working |
| Standard library (prelude, 8 modules over 28 primitives) | ✅ working |
| FFI (`foreign`) — declared, resolved, typechecked, lowered | ✅ working |
| Multi-target deploy (Cloudflare, Lambda, Vercel, Deno) | ✅ generates, ⬜ never invoked |
| Markup — `Markup` type, `Prose` element, `build markdown` | ✅ working, ⬜ only the compiler can make one |
| Reading files at build time — `build read`, `build list`, `build markdown` | ✅ working |
| Source maps | ⬜ not started |
| `record … unique` — identity keys for lists | ⬜ parsed, then refused: *"`unique` is not implemented past the parser yet (#2)"* |
| Dialects | ⬜ not started, beyond the M1 enabling structure |

## Where it stops

The honest boundary, stated once so nothing below oversells:

- **Only the compiler can make a `Markup`.** The type exists, `Prose` renders one, and
  `build markdown` produces one from a file on disk at build time. What a program computes
  still cannot become one: every other value reaches the DOM through `nodeValue`,
  `setAttribute`, `.value` or `.checked`, none of which parses HTML, so a string holding
  `<h1>Hello</h1>` renders as those literal characters. The runtime's `innerHTML` path is
  reachable only from `Slot::Rendered`, which only a `Markup` can occupy — that is the
  property that makes the narrow version safe, and it is tested rather than asserted.
- **`zdc deploy` generates a deployment; it never performs one.** It writes the files and
  prints a capability report. Nothing here has been run against a real Cloudflare, Lambda,
  Vercel or Deno account — the adapters are checked against vendor documentation and against
  each other, never against a vendor.
- **`Whole` overflow is uncaught on the client path.** `+` and `*` emit bare JavaScript
  operators, so a `Whole` silently loses precision above 2⁵³ and becomes `Infinity` above
  ≈1.8 × 10³⁰⁸. The narrowing operations *are* guarded; the arithmetic is not.
- **One known language-server defect**, and one that is not one. Go-to-definition across a
  `use` jumps to the wrong offset in the wrong file. A parse error in an imported file is
  reported with no file, no line and no caret — that one is on the **shared load path**, not
  the language server, so it is what `zdc check` prints too (#4). Now that every other
  diagnostic carries a code, a caret label and a suggested repair, it is the worst output the
  compiler produces. Both are recorded in [`STATUS.md`](STATUS.md) with the fix each needs.
- **No source maps, no dialects, no `record … unique`.**

All twenty-eight programs in [`examples/`](examples/) **pass `zdc check` and produce a bundle
from `zdc build`.** [`examples/blog.zd`](examples/blog.zd) was the last aspirational one; it now
reads its posts off disk at build time, renders the markdown in the compiler, and is verified to
build with an empty `PATH`. The per-file table is in [`STATUS.md`](STATUS.md).

## Try it

### Install it

**Today, from source.** A stable Rust toolchain, 1.89 or later, is the only
prerequisite — there is no Node, no npm and no bundler anywhere in this.

```sh
git clone https://github.com/DrDrewCain/zdeceptron && cd zdeceptron
cargo build --release
./target/release/zdc --version        # zdc 0.1.0
```

`cargo install --git https://github.com/DrDrewCain/zdeceptron zdc-cli` also
works and puts `zdc` on your `PATH`.

**At the first tagged release**, two more, and neither works before then
because there is nothing published yet:

```sh
cargo install zdc-cli
curl -fsSL https://raw.githubusercontent.com/DrDrewCain/zdeceptron/main/scripts/install.sh | sh
```

The machinery for both is in `.github/workflows/release.yml` — five targets, a
checksum per artefact, and a `sh` installer that verifies it. It has been run:
all five targets build and each native one smoke-tests the binary it produced.
What is missing is a tag, not a mechanism. The rest of this section writes `zdc` for whichever of
those you used.

### Start a project

```sh
zdc new notes
zdc dev notes/main.zd                 # http://127.0.0.1:4321
```

Two files, and no manifest — there is nothing for one to configure, because the
entry file named on the command line is the whole project model. `notes/main.zd`
is one signal, one derived from it and one event handler, so the first edit is a
change rather than a deletion; `notes/assets/style.css` is linked after the
stylesheet the compiler generates, which is why its rules win without an
`!important`. A directory that already contains anything is refused.

### Compile and run one of the examples

The fastest loop is `zdc dev`: it compiles, serves, watches the file, rebuilds
on save and reloads the browser. It works for **every** example, including the
ones with server and durable state, because it runs the emitted server
functions too.

```sh
zdc dev examples/counter.zd           # http://127.0.0.1:4321
zdc dev examples/shortest-path.zd     # Dijkstra, stepped one pop at a time
zdc dev examples/guestbook.zd         # durable + server, end to end, no cloud account
```

To produce files rather than serve them, `zdc build`:

```sh
zdc build examples/hello.zd -o dist
```

`dist/` is the whole program: `index.html`, `client.js`, `styles.css`,
`manifest.json` and a `runtime/`. A program with `server` or `durable` state
gets a `functions/` directory as well — one module per endpoint the program
implies, which you never wrote. A program that asks for more gets more:
`zdc build examples/writing.zd` also emits the `rss.xml` that file declares.

Serve it over HTTP. The document loads ES modules, so opening `index.html` as a
`file://` URL will not work:

```sh
python3 -m http.server 8000 --directory dist     # then http://localhost:8000
```

That is enough for a client-only program. A program with `durable` or `server`
state needs the endpoints running too, which is what `zdc dev` is for.

### The examples worth starting with

| | |
|---|---|
| [`hello.zd`](examples/hello.zd) | The smallest program. One signal, one view. |
| [`counter.zd`](examples/counter.zd) | Events and mutation. |
| [`tally.zd`](examples/tally.zd) | Change one word, `client` to `durable`, and the transport is generated. |
| [`guestbook.zd`](examples/guestbook.zd) | `server`, `durable` and `client` in one program. |
| [`shortest-path.zd`](examples/shortest-path.zd) | Dijkstra, and an honest note on what the language costs it. |
| [`sorting.zd`](examples/sorting.zd) | Three sorts, and the comparison counts to tell them apart. |

All twenty-eight are listed with what each one teaches in
[`STATUS.md`](STATUS.md), and every one of them `check`s and `build`s.

### Every command

```sh
zdc new     notes                  # writes notes/, then tells you to run zdc dev
zdc check   examples/guestbook.zd  # resolves, splits, typechecks, exit 0
zdc build   examples/writing.zd    # writes dist/, no toolchain needed
zdc dev     examples/counter.zd    # http://127.0.0.1:4321
zdc doc     examples/guestbook.zd  # writes doc/, Markdown, placements first
zdc doc     --prelude              # the same, for the standard library
zdc deploy  examples/tally.zd --target cloudflare   # writes a deployment, performs none
zdc explain E-IFC-05               # the rule behind a code
zdc parse   examples/hello.zd      # syntax tree, exit 0
zdc lsp                            # spoken to by the editor, not by you
```

`--no-color` is global and every command honours it; `NO_COLOR` in the
environment does the same without the flag.

A rejection states the claim, shows the spans, and ends with
`run 'zdc explain E-IFC-05' for the rule`. The rule itself — why it exists
and a worked repair — is one command away rather than in every message,
because reading a diagnostic costs measurable time (Barik et al., ICSE
2017: 13–25% of fixations, and reading difficulty predicts how long the
fix takes).

Feed it something wrong and the compiler names the one valid phrasing:

```
Error: Expected a placement after `is`, found `Map`. A `state` declaration says
where its value lives.
   ╭─[bad.zd:1:16]
   │
 1 │ state votes is Map of Id to Int starting empty
   │                ─┬─
   │                 ╰─── `Map` is the type, and a placement goes before it
   │
   │ Help: run 'zdc explain E0101' for the rule
   │
   │ Note: the line as it would be accepted: state votes is client Map of Id to Int starting empty
───╯
```

That is the language's central bargain (§4.1): the grammar admits exactly one phrasing per
construct, so the compiler always tells you what it is. The caret says what it is pointing
at rather than that it is pointing, the message names the word you wrote, and the repair is
printed as the line you would have written rather than described. Every parse error carries
a code, so `zdc explain E0101` has the other three placements and the reason the compiler
will not choose one for you.

`zdc dev` watches the file, rebuilds on save, reloads the browser, and — when the program does
not compile — puts the diagnostic on the page instead of the app. No Node, no npm, no bundler:
the HTTP server, the file watcher and the JavaScript runtime are all inside the one `zdc`
binary.

It is not client-only. `zdc dev` runs the emitted server functions through `zdc-host`, which
binds `$env` and `$store` and executes the handler, so a `server` or `durable` program works
end to end locally — `POST /_zd/greeting` returns a value. `crates/zdc-dev/tests/endpoints.rs`
drives the running server over real HTTP.

## In your editor

```sh
./target/release/zdc lsp                          # spoken to by the editor, not by you
```

Those same diagnostics arrive inline as you type, along with hover, go to definition, semantic
highlighting, and completion — all computed by running the compiler's real passes, so the
editor and `zdc check` cannot disagree. The language server is a subcommand rather than a
second binary, and like everything else it needs nothing installed alongside it.

The hover worth seeing is on a `server` or `durable` signal read from the view. It says the
read is `Remote of T` and that it crosses the network, because that is what the type checker
decided (§5.2). Semantic tokens carry the same information as a modifier, so client, server and
durable state can be told apart at a glance.

See [`editors/vscode/README.md`](editors/vscode/README.md) to set it up.

## Building

```sh
# 2200 tests. Test execution is a few minutes; a cold compile dominates the wall clock.
# Worth splitting — the benchmark suite dominates execution, at about seven
# minutes of the total in one target.
cargo test --workspace --exclude zdc-bench --no-fail-fast   # 2135 passed, 2 ignored
cargo test -p zdc-bench --no-fail-fast                      #   40 passed, 3 ignored

cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

CI runs all three, plus eight scripted gates: every crate root carries `#![forbid(unsafe_code)]`;
no wildcard match arm over a closed compiler enum; no test that cannot fail; no emitter that
writes its own quotes around a placeholder; the editor grammar highlights no keyword the lexer
rejects; advisory exceptions agree and are explained; `cargo deny`; and `cargo audit`. The last
two of those and a `cargo-geiger` scan of the dependency graph are why the dependency list stays
short.

Three of those gates exist because a bug got through: the vacuous-test check, the wildcard-arm
check and the emitted-string check were each written after something they would have caught.

The compiler is written in Rust and emits JavaScript. Rust is the *implementation* language,
not the source language — ZDeceptron users download a single static `zdc` binary and never
encounter Rust, exactly as Elm users never encounter Haskell. The JavaScript runtime's own test
suites (`runtime/signal.test.js`, `runtime/dom.test.js`) run inside `cargo test` through an
embedded pure-Rust engine, so verifying the runtime installs nothing either.

Performance is measured, not asserted: see [`BENCHMARKS.md`](BENCHMARKS.md). Its numbers are
regenerated from the suite and an exact-match test fails the build if the file drifts.

## License

MIT
