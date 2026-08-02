# ZDeceptron

**A reactive dataflow language where placement is a property of state, and the compiler derives the network.**

> ⚠️ Early development. The lexer and expression parser work; the type checker, placement pass, and code generator do not exist yet. See [Status](#status).

---

## The idea

Every piece of state is a signal. Each signal declares *where it lives*:

```
secret state apiKey is server  Text              from environment "STRIPE_KEY"
       state votes  is durable Map of Id to Int  starting empty
       state ranked is server  List of Item      from rank with votes
       state query  is client  Text              starting ""

function rank with votes
    from items
    keep each item where item.live and not item.hidden
    sort each item by votes at item.id
    take first 20

view
    Column
        Input query, hint is "search"

        when ranked is
            Loading           show Spinner
            Failed with error show ErrorBar, message is error.message
            Ready with items
                each item in items
                    Row item.name, padding is 8, weight is bold
                        on click
                            add 1 to votes at item.id
```

The UI is a pure function of the signal graph. The compiler walks that graph, and **any edge crossing a placement boundary becomes transport** — client→server becomes an RPC, anything→durable becomes persistence.

You never write a `fetch`, an API route, a schema, a migration, or a deploy config. You declare where state lives.

### Two rules generate the rest

**Crossing a boundary is visible in the type.** Reading a `server` or `durable` signal from the client yields `Remote of T` — `Loading | Ready | Failed` — not `T`. The network appears in your types exactly where the network is, and nowhere else. You cannot forget a loading state, because you cannot read through the variant without eliminating it.

**Secrecy flows.** A `secret` signal's taint propagates through every derivation. A secret-tainted value reaching client state or the view is a **compile error**. Your API key cannot leak to the browser — not by convention, by type.

## Why

JavaScript doesn't dominate the frontend because it won a design contest. It dominates because it's the only language browsers run natively. Plenty of languages escaped its *syntax* — Elm, ReScript, Gleam, Dart — but they inherited its *deployment model*: you still needed a bundler, a host, a backend, a database client. The language got nicer; the day stayed the same.

ZDeceptron targets the deployment model, the type system's soundness, and the syntax at once. The design decisions are grounded in published evidence rather than taste — including the finding that C-style syntax scores no better with novices than *randomly generated* syntax. See [the research](docs/research/2026-08-02-frontend-language-critique.md); every source URL there was fetched and verified.

## Status

| Component | State |
|---|---|
| Lexer (Unicode identifiers, indentation layout) | ✅ working |
| AST | ✅ working |
| Expression parser | ✅ working |
| Statements, declarations, view parser | 🚧 in progress |
| Diagnostics, CLI | ⬜ planned |
| Name resolution → HIR | ⬜ planned |
| Type checker (Hindley–Milner) | ⬜ planned |
| Placement + information-flow pass | ⬜ planned |
| JS codegen, runtime, dev server | ⬜ planned |
| Dialects, multi-target deploy | ⬜ planned |

Nothing here compiles a ZDeceptron program end to end yet.

## Building

```sh
cargo test --workspace
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

The compiler is written in Rust and emits JavaScript. Rust is the *implementation* language, not the source language — ZDeceptron users download a single static `zdc` binary and never encounter Rust, exactly as Elm users never encounter Haskell.

**Memory safety is mechanically verified.** Every crate carries `#![forbid(unsafe_code)]`, and CI fails if any crate omits it.

## Design docs

- [Language design spec](docs/superpowers/specs/2026-08-02-zdeceptron-design.md) — placements, type system, grammar, dialects, serverless portability
- [Research: what's wrong with frontend languages](docs/research/2026-08-02-frontend-language-critique.md) — the evidence the design is built on
- [Front-end implementation plan](docs/superpowers/plans/2026-08-02-front-end.md)

## License

MIT
