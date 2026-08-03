# ZDeceptron

**A reactive dataflow language where placement is a property of state, and the compiler derives the network.**

> ⚠️ Early development. The compiler is real — lexer, parser, name resolution, Hindley–Milner
> type checker, tier split, information-flow pass, JavaScript code generator, dev server and
> language server all exist and are tested. **Client-only programs build and run.** Programs
> with `server` or `durable` state compile and emit both halves, but **nothing executes the
> server half yet** — there is no runtime store and no platform adapter. See
> [`STATUS.md`](STATUS.md) for the milestone-by-milestone truth and
> [`ROADMAP.md`](ROADMAP.md) for what is next.

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

That program compiles today. `zdc build` on it emits `client.js`, `styles.css`, `index.html`,
`manifest.json`, and one file per derived server endpoint — `functions/greeting.js` and
`functions/visits.incr.js` — and `GREETING_API_KEY` appears in none of the client output. It is
[`examples/guestbook.zd`](examples/guestbook.zd) with the comments removed. What it does *not*
do is run: see [Where it stops](#where-it-stops).

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

798 tests pass across 15 crates. The full picture, with the evidence behind each row, is in
[`STATUS.md`](STATUS.md).

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
| `zdc parse`, `check`, `build`, `dev`, `deploy`, `explain`, `lsp` | ✅ working |
| Server function emission + RPC client | ✅ emitted **and** executed |
| Durable store, persistence, live sync | ✅ working |
| Components (`component`, `use`, `children`) | ✅ working |
| `static` placement, build-time file emission | ✅ working |
| Standard library (prelude) | ✅ working |
| FFI (`foreign`) | ◐ parses, not lowered |
| Multi-target deploy (Cloudflare, Lambda, Vercel, Deno) | ✅ generates, ⬜ never invoked |
| Dialects | ⬜ not started |

## Where it stops

The honest boundary, stated once so nothing below oversells:

- **`zdc deploy` generates a deployment; it never performs one.** It writes the files and
  prints a capability report. Running the platform's own command is a separate, deliberate
  act, and nothing here has been run against a real Cloudflare, Lambda, Vercel or Deno
  account.
- **`Row` and `Column` take no leading argument**, so `Row item.name` is refused pending a
  language decision.
- **`foreign` parses but is not lowered**, so FFI declares nothing that codegen can call.
- **No source maps, no dialects.**

Of the twelve programs in [`examples/`](examples/), **eleven pass `zdc check` and seven
produce a bundle from `zdc build`.** The per-file table, with the exact error for each
failure, is in [`STATUS.md`](STATUS.md).

## Try it

```sh
cargo build --release

./target/release/zdc parse   examples/hello.zd      # syntax tree, exit 0
./target/release/zdc check   examples/guestbook.zd  # resolves, splits, typechecks, exit 0
./target/release/zdc build   examples/writing.zd    # writes dist/, no toolchain needed
./target/release/zdc dev     examples/counter.zd    # http://127.0.0.1:4321
./target/release/zdc deploy  examples/tally.zd --target cloudflare
./target/release/zdc explain E-IFC-05               # the rule behind a code
```

A rejection states the claim, shows the spans, and ends with
`run 'zdc explain E-IFC-05' for the rule`. The rule itself — why it exists
and a worked repair — is one command away rather than in every message,
because reading a diagnostic costs measurable time (Barik et al., ICSE
2017: 13–25% of fixations, and reading difficulty predicts how long the
fix takes).

Feed it something wrong and the compiler names the one valid phrasing:

```
Error: Expected a placement after `is`, found a name. Write `client` for browser
memory, `server` for a serverless invocation, or `durable` for persistent storage.
   ╭─[bad.zd:1:16]
 1 │ state votes is Map of Id to Int starting empty
   │                ─┬─
   │                 ╰─── here
```

That is the language's central bargain (§4.1): the grammar admits exactly one phrasing per
construct, so the compiler always tells you what it is.

`zdc dev` watches the file, rebuilds on save, reloads the browser, and — when the program does
not compile — puts the diagnostic on the page instead of the app. No Node, no npm, no bundler:
the HTTP server, the file watcher and the JavaScript runtime are all inside the one `zdc`
binary. It is useful for `client` programs; for a `server` or `durable` program it will serve
the client half and the RPC calls will fail.

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
cargo test --workspace     # 798 tests; allow about five minutes, the benchmark suite is in it
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

CI runs all three, plus a scan asserting every crate root carries `#![forbid(unsafe_code)]` and
a check that the editor grammar highlights no keyword the lexer rejects.

The compiler is written in Rust and emits JavaScript. Rust is the *implementation* language,
not the source language — ZDeceptron users download a single static `zdc` binary and never
encounter Rust, exactly as Elm users never encounter Haskell. The JavaScript runtime's own test
suites (`runtime/signal.test.js`, `runtime/dom.test.js`) run inside `cargo test` through an
embedded pure-Rust engine, so verifying the runtime installs nothing either.

Performance is measured, not asserted: see [`BENCHMARKS.md`](BENCHMARKS.md). Its numbers are
regenerated from the suite and an exact-match test fails the build if the file drifts.

## License

MIT
