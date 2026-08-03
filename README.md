# ZDeceptron

**A reactive dataflow language where placement is a property of state, and the compiler derives the network.**

> ⚠️ Early development. The compiler is real — lexer, parser, name resolution, Hindley–Milner
> type checker, tier split, information-flow pass, JavaScript code generator, durable store,
> platform adapter, dev server and language server all exist and are tested. **`client`,
> `server`, `durable` and `static` programs all build, and the server half executes**: two
> browser windows move together over live sync, proven by
> `crates/zdc-host/tests/two_windows.rs`. What it still cannot do is turn a computed value into
> markup, read a directory at build time, or deploy to a real account. See
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

**1360 tests pass across 18 crates**, with 0 failures and 4 deliberate `#[ignore]`s — three
that print a scaling survey rather than gating on it, and one that records an unfixed
disagreement between `zdc check` and `zdc build`. The full picture, with the evidence behind
each row, is in [`STATUS.md`](STATUS.md).

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
| Routing — declared routes, one bundle per URL | ✅ working |
| Element vocabulary — 36 built-ins | ✅ working |
| Event payloads on handlers | ✅ working |
| `static` placement, build-time evaluation, file emission | ✅ working |
| Standard library (prelude, 7 modules over 21 primitives) | ✅ working |
| FFI (`foreign`) — declared, resolved, typechecked, lowered | ✅ working |
| Multi-target deploy (Cloudflare, Lambda, Vercel, Deno) | ✅ generates, ⬜ never invoked |
| Computed values rendered as markup | ⬜ not started |
| Reading files at build time | ⬜ not started |
| Source maps | ⬜ not started |
| `record … unique` — identity keys for lists | ⬜ not started |
| Dialects | ⬜ not started |

## Where it stops

The honest boundary, stated once so nothing below oversells:

- **A computed value cannot become markup.** There is no `Markup` type and no element that
  renders one. Every value a program computes reaches the DOM through `nodeValue`,
  `setAttribute`, `.value` or `.checked`, none of which parses HTML — so a string holding
  `<h1>Hello</h1>` renders as those literal characters. This is the biggest gap for
  content-shaped programs.
- **Nothing reads files at build time.** `static` placement evaluates at build time and inlines
  the result, but there is no capability for reading a directory, so a blog cannot load its
  posts.
- **`zdc deploy` generates a deployment; it never performs one.** It writes the files and
  prints a capability report. Nothing here has been run against a real Cloudflare, Lambda,
  Vercel or Deno account — the adapters are checked against vendor documentation and against
  each other, never against a vendor.
- **`Whole` overflow is uncaught on the client path.** `+` and `*` emit bare JavaScript
  operators, so a `Whole` silently loses precision above 2⁵³ and becomes `Infinity` above
  ≈1.8 × 10³⁰⁸. The narrowing operations *are* guarded; the arithmetic is not.
- **Two known language-server defects.** Go-to-definition across a `use` jumps to the wrong
  offset in the wrong file, and a parse error in an imported file is reported with no location
  at all. Both are recorded in [`STATUS.md`](STATUS.md) with the fix each needs.
- **No source maps, no dialects, no `record … unique`.**

Of the eighteen programs in [`examples/`](examples/), **seventeen pass `zdc check` and
seventeen produce a bundle from `zdc build`.** The one failure is
[`examples/blog.zd`](examples/blog.zd), which is deliberately aspirational — it is kept as a
record of what a real content program needs. The per-file table, with the exact error, is in
[`STATUS.md`](STATUS.md).

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
# 1360 tests. Test execution is about 3.5 minutes; a cold compile dominates the wall clock.
# Worth splitting — the benchmark suite is 157s of the 210s of execution.
cargo test --workspace --exclude zdc-bench --no-fail-fast   # 1328 passed, 1 ignored
cargo test -p zdc-bench --no-fail-fast                      #   32 passed, 3 ignored

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
