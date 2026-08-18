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
| [The examples](https://zdeceptron.marksturman.com/docs/examples) | All thirty-seven programs, ordered by what they teach. |
| [The standard library](https://zdeceptron.marksturman.com/docs/standard-library) | The prelude, module by module. |

This `README` states the idea and the current boundary. The documentation is
the part you read to *use* the thing. To work on the compiler itself, see
[`CONTRIBUTING.md`](CONTRIBUTING.md) — it explains what each CI gate is
protecting, and every one of them was written after the bug it prevents had
already shipped. What changed between versions is in
[`CHANGELOG.md`](CHANGELOG.md).

The compiler's own crates are documented at
**[drdrewcain.github.io/zdeceptron](https://drdrewcain.github.io/zdeceptron/)**,
rebuilt from `main` on every merge. It is rustdoc over every crate in the
workspace, built with private items included because that is where most of the
reasoning is written down. `zdc_graph::integrity` is the page to read first: it
sets out, beside the code that enforces it, what the integrity lattice does
**not** claim — which adversarial pass broke which earlier claim, and which
risks are left standing — and nothing in `docs/` states that as precisely.

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

That program compiles today, and it runs. `zdc build` on it emits `client.js`, a stylesheet,
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

**Over 2600 tests pass across 21 crates**, with 0 failures and 15 deliberate `#[ignore]`s — the
surveys that print a measurement rather than gating on it, the browser tests CI runs with
`--ignored` because they need a real browser, and the two that hold a known defect open: a `give` after a
pipeline run is emitted as unreachable code, and `Input` cannot bind a component's own
`state` though a handler can write it. The full picture, with the evidence behind each row
and a per-crate table CI compares against the tree, is in [`STATUS.md`](STATUS.md).

The figure is written as a floor rather than an exact count, and that is deliberate: an exact
one is a fact about the tree restated in prose, and it was wrong here for months — 2358
against a tree of 2662, across 20 crates against 21. A floor is checked by
`the_readme_does_not_overstate_its_own_test_count` and stays true as the suite grows, so it
rots in the safe direction only.

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
| `zdc new`, `parse`, `check`, `build`, `dev`, `deploy`, `explain`, `fmt`, `lsp` | ✅ working |
| Server function emission + RPC client | ✅ emitted **and** executed |
| Durable store, persistence, live sync | ✅ working |
| Components (`component`, `use`, `children`) | ✅ working |
| Routing — declared routes, one bundle per URL | ✅ working |
| Element vocabulary — 69 built-ins | ✅ working |
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

- **`durable` state is global, and there is no cross-visitor confidentiality.** There is one
  store per program, not one per visitor: every request reads and writes the same rows, and a
  durable row is visible to any request that computes its key. Scoping data to the visitor it
  belongs to is the *program's* job and **nothing checks that you did it** — a forgotten owner
  filter is a multi-tenant data leak that compiles clean. The `secret` and `trusted` lattices
  are over placements, not principals, so neither has anything to say about it. `durable per
  visitor` is refused outright (`E0107`): partitioning by principal needs a principal, and
  nothing authenticates a request — there are no headers, no cookies and no session. Per-user
  scoping and authentication are one v1 non-goal, not two.
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

All thirty-seven programs in [`examples/`](examples/) **pass `zdc check` and produce a bundle
from `zdc build`.** [`examples/blog.zd`](examples/blog.zd) was the last aspirational one; it now
reads its posts off disk at build time, renders the markdown in the compiler, and is verified to
build with an empty `PATH`. The per-file table is in [`STATUS.md`](STATUS.md).

## Try it

### In a browser, installing nothing

The compiler is built to WebAssembly and runs in a tab. A program you type is
lexed, parsed, resolved, split, typechecked, flow-checked and emitted in the
page, and the emitted bundle runs in the page beside it. Nothing is uploaded
and no server takes part in either half.

```sh
rustup target add wasm32-wasip1
cargo build --release --target wasm32-wasip1 -p zdc-wasm
python3 -m http.server 8000        # from the repository root
#  then open http://localhost:8000/playground/
```

That is the whole build. No `wasm-pack`, no `wasm-bindgen`, no generated glue
— [`playground/wasi.js`](playground/wasi.js) is thirteen host functions and it
says why the conventional route was rejected. To deploy the page anywhere,
copy `playground/` and drop `zdc-wasm.wasm` beside `index.html`.

Four things it can show, which is most of what this language is:

| | |
|---|---|
| a `client` program **running** | pick `counter` or type your own |
| the diagnostics, drawn by `ariadne` exactly as `zdc check` draws them | pick `a secret in the view` |
| the **placement split** — what became client, what became server, and the endpoints nobody wrote | pick `guestbook` |
| a `secret` refused before it can reach a browser | pick `a secret in the view` |

And three things it cannot, each refused by name rather than failing quietly:
a `server` or `durable` program compiles and is *not run*, because running it
needs a host and a store this page does not have — the split is shown instead;
a `static` signal is computed at build time by a JavaScript engine this build
deliberately does not carry, so it is refused and every `static` is named; and
`use` of another module, `build read` and `build list` all want a filesystem,
which a tab does not have. [`crates/zdc-wasm/src/lib.rs`](crates/zdc-wasm/src/lib.rs)
is the full account, including which dependency stood in the way and why the
fix was a feature seam rather than a flag.

### Install it

**From crates.io.** A stable Rust toolchain, 1.89 or later, is the only
prerequisite — there is no Node, no npm and no bundler anywhere in this.

```sh
cargo install zdc-cli
zdc --version                         # zdc 0.1.0
```

The crate is `zdc-cli` and the binary is `zdc`. Twenty crates go up with
each release, because reaching the binary from crates.io means everything
it links is there too; you install one and cargo fetches the rest.
[#300](https://github.com/DrDrewCain/zdeceptron/issues/300) is the open
question of renaming the crate to `zdeceptron`, so that the install name
is the project's name.

**From source**, which is the same compiler and no slower to use:

```sh
git clone https://github.com/DrDrewCain/zdeceptron && cd zdeceptron
cargo build --release
./target/release/zdc --version        # zdc 0.1.0
```

**A prebuilt binary**, once a release is published rather than drafted:

```sh
curl -fsSL https://raw.githubusercontent.com/DrDrewCain/zdeceptron/main/scripts/install.sh | sh
```

`install.sh` resolves `releases/latest` and verifies the checksum beside
the artefact, so it works from the moment a draft is published and not
before — a draft is deliberately invisible to that endpoint.
`.github/workflows/release.yml` builds five targets and generates each
checksum on the machine that built the thing it describes.

The rest of this section writes `zdc` for whichever of those you used.

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

`dist/` is the whole program: `index.html`, `client.js`, `styles.<hash>.css`,
`manifest.json`, `_headers` and a `runtime/`. A program with `server` or
`durable` state gets a `functions/` directory as well — one module per endpoint
the program implies, which you never wrote. A program that asks for more gets
more: `zdc build examples/writing.zd` also emits the `rss.xml` that file
declares.

The stylesheets carry a content hash in their names, and `_headers` is what
tells a static host that a name like that may be cached for a year and never
revalidated. Nothing else is renamed: an image under `assets/` is named by the
program's own text, and a compiler may only rename a file it can prove it
named.

`--report` adds one more file, `report.json`, and it is for a reader rather
than for a browser. A `foreign` whose `gives` line says `pure` or `trusted` is
making a claim about somebody's JavaScript that the compiler cannot check and
the program's integrity rests on; the report lists every one of them, where it
is declared, and which `release` bodies depend on it. It is an enumeration and
not a verdict, and its own `notClaimed` array says what it is not telling you.

No example in this repository writes either word, so every example's report has
an empty `asserted` list — which is the answer, not the absence of one.

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
| [`sorting.test.zd`](examples/sorting.test.zd) | What the file above claims about itself, as six `test` declarations `zdc test` checks. |
| [`scene.zd`](examples/scene.zd) | One drawing, as DOM nodes and as pixels: `Svg` and `Scene` side by side. |
| [`timers.zd`](examples/timers.zd) | A timer, a frame loop and a delay, none of them a callback. |
| [`tree-webgl/`](examples/tree-webgl/) | Real three.js, driven from the language with no hand-written JavaScript. |

All thirty-seven are listed with what each one teaches in
[`STATUS.md`](STATUS.md), and every one of them `check`s and `build`s.
`sorting.test.zd` is a thirty-fifth file and is not a thirty-fifth
program: it declares no view of its own and exists to be run by `zdc test`.

### Every command

```sh
zdc new     notes                  # writes notes/, then tells you to run zdc dev
zdc check   examples/guestbook.zd  # resolves, splits, typechecks, exit 0
zdc build   examples/writing.zd    # writes dist/, no toolchain needed
zdc build   examples/writing.zd --report  # + dist/report.json, the grants nobody checks
zdc test    examples/sorting.test.zd  # checks each `test`, exit 1 if one is false
zdc dev     examples/counter.zd    # http://127.0.0.1:4321
zdc doc     examples/guestbook.zd  # writes doc/, Markdown, placements first
zdc doc     --prelude              # the same, for the standard library
zdc deploy  examples/tally.zd --target cloudflare   # writes a deployment, performs none
zdc explain E-IFC-05               # the rule behind a code
zdc fmt     examples/todo.zd       # rewrites it in the one canonical layout
zdc fmt --check examples/*.zd      # exit 1 if any would change; what CI runs
zdc parse   examples/hello.zd      # syntax tree, exit 0
zdc lsp                            # spoken to by the editor, not by you
```

`zdc fmt` normalises the *vertical* layout — four spaces a level, no
trailing whitespace, one line break at the end, one blank line at most, a
comment at the indentation of the line it introduces, and a `"""` block's
closing delimiter one level inside the line that opens it. It refuses any
file the compiler will not parse rather than guessing where the blocks
were, and it works on the source text rather than on the syntax tree —
comments are discarded by the lexer, so a formatter that round-tripped
through the tree would delete every one of them.

It deliberately does **not** touch the spacing within a line, so the
aligned columns the examples use survive:

```
state count   is client Whole starting 0
state doubled is client Whole from count * 2
```

There is one legal program it refuses: a second `"""` literal opened on the
line that closes the first. That line's indentation is part of a value and
part of the block structure at once, so no single answer for it is right,
and guessing would change what the program says. It is reported with a
caret at the line, and nothing else in the file is rewritten. Nothing in
`examples/` is written that way.

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

One server, so every editor that speaks the protocol gets all of it:
[`editors/`](editors/README.md) has the configuration for
[VS Code](editors/vscode/README.md), [Neovim](editors/neovim/README.md) and
[Helix](editors/helix/README.md), the four facts any other LSP client needs, and — per
editor — what was actually run against the compiler rather than only written down.

Highlighting is the one thing that does not travel, because every editor wants a different
artefact for it. VS Code has the TextMate grammar in `editors/vscode/syntaxes/`; Neovim needs
no grammar at all, because it colours a `.zd` file from the language server's semantic tokens;
Helix implements neither a grammar for this language nor semantic tokens, so it runs the server
and shows the file in one colour. There is no tree-sitter grammar, which is also why there is
no Zed extension — `editors/README.md` has the argument.

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
