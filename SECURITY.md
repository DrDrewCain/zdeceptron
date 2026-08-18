# Security policy

## Supported versions

ZDeceptron has not had a release. Until `0.1.0` is tagged, the only
supported version is the current `main` branch, and the only fix that will
be offered for any report is a commit on `main`.

| Version | Supported | Notes |
| --- | --- | --- |
| `main` | Yes | Fixes land here. |
| Any published tag | Not yet | There are none. |

When the first tag ships, this table becomes the record of which tags
still receive fixes, and no version will be dropped from it without a
deprecation notice in the release that drops it.

## Reporting a vulnerability

**Do not open a public issue.**

Report privately through GitHub's private vulnerability reporting:
<https://github.com/DrDrewCain/zdeceptron/security/advisories/new>. That
opens a draft advisory visible only to you and the maintainers, and it is
the preferred route because it carries the disclosure conversation, the
CVE request and the eventual publication in one place.

If that form is unavailable to you, open a public issue containing the
single word `security` and no details, and a maintainer will open a
private channel. Please do not include the vulnerability in that issue.

A useful report has: the commit hash, the platform, the smallest input
that reproduces the problem, what you expected, and what happened
instead. A proof of concept is welcome and is never required.

### What to expect, and when

| Stage | Target |
| --- | --- |
| Acknowledgement that a human has read the report | 3 working days |
| An initial assessment — accepted, not a vulnerability, or need more information | 10 working days |
| A fix on `main`, or a written plan with a date if the fix is not simple | 30 days from acceptance |
| Public advisory | Within 7 days of the fix landing |

These are targets for a project maintained by volunteers, not a
contractual SLA. If a deadline is going to be missed you will be told
before it is missed, not after.

Coordinated disclosure is the default: the advisory is published once a
fix is available, and the reporter is credited unless they ask not to be.
If a report is still unfixed 90 days after acknowledgement, the reporter
is free to disclose it, and we would rather they did than that it sat.

### CWE for every CVE

Every advisory published for this project will carry a CWE identifier,
not only a CVE identifier and a prose description. This is *The Case for
Memory Safe Roadmaps* (CISA/NSA/FBI and five partner agencies, December
2023) asking manufacturers to "publicly commit to supplying CWEs for 100
percent of CVEs in a timely manner", and it costs a maintainer one field.
It is what makes the resulting record aggregable by anyone studying
classes of defect rather than individual products.

## What counts as a vulnerability

The compiler's job is to refuse programs that leak. So the interesting
report is usually not a crash.

**In scope, and serious:**

- **A program that compiles but leaks.** Any input where a value declared
  `secret` reaches the browser — the view, client state, the build
  artefact, a response body, a platform log, or the live-sync stream —
  without the information-flow pass rejecting it. These are the six sinks
  the pass is built around, and a hole in any of them is the highest
  severity this project has.
- **A placement violation that survives the split**: server-only code
  emitted into the client bundle, or `environment` reachable outside
  server context.
- **Generated code that is unsafe in the browser**: an injection through
  a text value, an emitted endpoint that omits a check the source asked
  for, or a bundle that reaches state its own program cannot name.
- **Anything the compiler does to a machine it runs on** that a source
  file should not be able to cause: writing outside the output directory,
  reading a file the program did not name, executing anything.
- A denial of service in the compiler or the language server that a
  *small* input can trigger — the language server re-analyses on every
  keystroke, so a non-terminating pass is a real availability bug rather
  than a curiosity.

**Out of scope:**

- A crash, panic or hang on input the compiler already rejects, unless it
  is reachable from a plausible source file. Report it as a normal bug;
  it will still be fixed.
- Vulnerabilities in a dependency with no path from this workspace. Send
  those upstream; `cargo audit` and `cargo deny` run in CI here and we
  will pick them up.
- Vulnerabilities in the JavaScript engine that runs the emitted output.
  Report those to the engine.
- Findings from a scanner with no demonstrated impact on this code.

## What this project does and does not claim

Stated precisely, because the difference matters to anyone assessing it:

- **The compiler contains no `unsafe` code.** All fourteen crates carry
  `#![forbid(unsafe_code)]` as the first item in the crate root, and CI
  fails if any of them loses it (`scripts/check-forbid-unsafe.sh`). This
  is a mechanical property, not an aspiration. It matters here more than
  it does for most programs because a compiler and a language server
  ingest untrusted source continuously.
- **The compiler's dependencies contain a great deal of `unsafe` code.**
  Measured with `cargo geiger`: roughly 23,500 of 28,900 `unsafe`
  expressions in the dependency graph are reachable, spread over about
  170 crates, none of them first-party.
  `scripts/check-dependency-unsafe.sh` measures this in CI and holds it
  under a ceiling.
- **A ZDeceptron program cannot express a memory-unsafe operation.** The
  language has no raw pointers, no manual deallocation and no unchecked
  indexing.
- **The memory safety of the process that runs a compiled program is not
  this project's to claim.** The output is JavaScript, and the engine
  executing it is a C++ program. Chromium reports that around 70% of its
  high-severity security bugs are memory-unsafety problems. That is
  outside this project's control and is not improved by anything here.
- **A build reads the project it is building and nothing else.** Every
  path a program can make the compiler open — a `use` specifier, a
  `build read` or `build list` path, a `foreign … from "./x.js"`, and a
  `[packages]` target in `zd.toml` — goes through one rule
  (`zdc_hir::sandbox`), which is applied to the *resolved* path, so a `..`
  and a symbolic link planted inside the project are refused alike. A
  specifier that leaves the project is a compile error; the file is never
  opened and its bytes never enter the compilation.
- **A remote module is fetched by the browser, never by the build.** A
  `foreign` may name an `http:`/`https:` URL, and since #238 that is
  allowed rather than pushed into a hand-written `.js` file that imported
  the same URL out of the compiler's sight. `zdc` does not resolve it, does
  not download it, and does not execute it: the specifier is written into
  the emitted `import`, and every origin the bundle will fetch from is
  listed under `origins` in `manifest.json` so that a reader and a
  Content-Security-Policy can enumerate them without running the compiler.
  Pinning a remote module to a hash is not implemented; a CDN that serves
  different bytes tomorrow is a risk this accepts and reports rather than
  one it prevents.

## The generated endpoints are not a public API

**DECIDED 2026-08-16, closing #38. The endpoints under `/_zd/` are the
private calling convention between one compiled program and the one client
that same compiler run emitted. They are not a supported surface, nothing
else may depend on them, and neither mechanism #38 offered is adopted:
there is no way to declare an endpoint public, and there is no manifest
that pins a derived name.**

This is a decision about what is promised, not a new restriction. Nothing
here stops anyone sending a request; what it settles is that doing so buys
no guarantee, and that the project owes such a caller nothing.

#38's third step asks that the outcome reach the design spec's ordering
table, which is local to the author's machine and not published here. Its
entry, stated in the place a reader can actually see it: **the public API
surface is decided rather than ordered — it is not built, it blocks nothing,
and the three items it would have depended on (authentication, an inbound
construct, and a version of its own) keep their existing rows.** Recording it
here is the same route #157 and #158 took on 2026-08-16.

### There is no name to preserve

An endpoint's name is a function of the program's text. Measured on
`examples/guestbook.zd`, which emits `greeting`, `visits` and `visits.incr`
— one edit per row, each built with `zdc build` and the output listed:

| the edit | what happened |
| --- | --- |
| renamed the signal `visits` to `visitCount` | `visits` and `visits.incr` became `visitCount` and `visitCount.incr`, and the durable key moved with them |
| `add 1 to visits` → `subtract 1 from visits` | `visits.incr` became `visits.decr` |
| renamed the `client` signal `name` to `who` | `greeting` kept its name and its declared inputs went from `["name"]` to `["who"]` |
| deleted the seven view lines that display `visits` | `functions/visits.js` stopped being emitted |

None of those four is a change to an interface as the author would describe
it: two are renames, one is a different operator in a click handler, and one
is markup. The third settles it. The `client` signal `name` appears nowhere
in `greeting`'s declaration — it is lifted into the endpoint's signature
because the split found a `client` read under a `server` root — and its
identifier is the parameter name on the wire. There is no spelling in this
language for *the name this argument has on the wire*, and a language whose
claim is that you declare where state lives and the compiler derives the
network cannot grow one without the derivation becoming a second thing to
maintain.

### Why a pinning manifest is refused, which is the harder half

A pinned `visits.incr` would be an entry asserting a name whose meaning is
*(the signal `visits`, the operator `incr`, the empty path)*. Rendering that
triple injectively is what makes two writes to one signal two distinct
endpoints; it is the property the endpoint scheme is built to have. Change
any component and the build has three options and no fourth:

- **Fail.** Then renaming a signal is a breaking change to an external
  contract, and the coupling this language exists to delete is back, wearing
  a manifest.
- **Keep the old name pointing at the new key.** That asserts the old name's
  meaning survived an edit whose meaning the compiler cannot see. `incr` and
  `decr` differ by one word in a handler body.
- **Emit both.** The surface doubles and the injectivity obligation now
  ranges over the union of the pinned names and the derived ones.

Only the first is honest, and it prices an ordinary rename at a deprecation
cycle. That is the question #38 asks in its step 2 — *what happens when the
signal graph changes underneath a pinned name* — and refusing the pin is the
answer, because keeping both halves has none.

### Why a declared surface cannot be had yet

A public endpoint needs authentication, a version of its own, and a
deprecation policy. Authentication is one of §13's v1 non-goals, and its
prerequisites are only partly discharged: it depends on the
externally-initiated effect construct, whose design was decided on
2026-08-16 (#211) and whose implementation has not started, and separately
on `release` not yet declassifying, so a session token derived from a
`secret` cannot reach the browser at all (#26, #29, #30, #31). A keyword
that published an endpoint before any of that existed would publish an
unauthenticated write surface and call it a feature.

### What follows for the wire format

Nothing changes, and the rule gains an argument it did not have. #144 asks
what compatibility the wire format promises across versions, and the work
answering it settles on none — a mismatch is refused, and the refusal names
both versions. That rule is affordable only because the two ends ship
together, and this section is why they ship together. A client built by an
older compiler is not merely encoding a `Map` the wrong way; it may be
calling a name the current build does not emit. The version refusal is
therefore the mechanism that enforces this decision rather than an incidental
transport check: a second client fails loudly at the first compiler upgrade
instead of quietly at the first renamed signal.

### What follows for authentication

§13's non-goal stands, and this decision is what lets it stand. The
endpoints are unauthenticated: `crates/zdc-deploy/js/router.js` checks the
method and the argument shape and nothing else, and no file under
`crates/zdc-deploy/js/` or `crates/zdc-dev/src/` reads an inbound
`Authorization` header. That is defensible for exactly as long as the only
thing meant to speak to them is a bundle the same deployment serves. It stops
being defensible on the day the surface is called public — so authentication
is not a feature a public API could ship without, it is the first
prerequisite of one.

Two things that are not authentication and should not be proposed as it:

- **An origin check.** CORS is a browser's rule about who may read a
  response. A script or a mobile application is not a browser and does not
  consult it, and those are exactly the second clients #38 names.
- **An unguessable name.** `manifest.json` is served beside `client.js` and
  lists every endpoint's name, file, kind and input names, and every durable
  key. The deployment publishes its own description on purpose, and it should
  keep doing so; a decision that depended on nobody reading it would be worth
  nothing.

### What follows for a second client

A mobile application or a script can speak to a deployment today. What this
decision does is say what one is owed, which is nothing: no endpoint name
survives a rename, no argument name survives a rename, the wire version
refuses it at the first compiler upgrade, and none of that will appear in
`CHANGELOG.md`, because a changelog records changes to what was promised.

What a second client should wait for is a declared surface, and its entry
conditions are named here rather than left to be inferred: authentication
implemented rather than designed; a version for the surface that is not the
compiler's version; and a decision about inbound webhook receipt, which is
the same question arriving from the other direction. Until all three, the
answer to *is there a public API* is no.

### One consequence the tree does not yet honour

If the endpoints are one client's private convention, the deployed surface
should be exactly what that client uses. It is not. `zdc dev` narrows a
live-sync subscription to the keys the compiled program declares, and says
why: *"a request for a key the program never declared would otherwise be a
way to read any value in the store by guessing its name"*
(`crates/zdc-dev/src/server.rs:340`). The generated router does not narrow —
`crates/zdc-deploy/js/router.js` hands the `?keys=` list straight to
`store.get` in `once` and to `store.watch` in `watch`, and both are reachable
with `GET`, because the `POST` check comes after those two branches. The key
list is not missing from the deployment: `Program::durable` is *"every
durable key the program touches"* (`crates/zdc-deploy/src/lib.rs:334`), read
at two places, neither of which is a generated file.

A key the program does not declare is reachable rather than hypothetical: a
durable key is the signal's name, the first row of the table above renames
one, and nothing removes the old cell (#37). This is recorded and not fixed
here — the same file is edited by two open pull requests, and half-fixing a
transport in a third is how the two halves come to disagree.

It is recorded here rather than reported privately because no leak has been
exhibited. The obvious attempt does not compile: `secret state ledger is
durable Whole starting 41`, written from a click, is rejected at
**E-IFC-08**, which names the response body. What is demonstrated is a
divergence between the two runners and a surface wider than the emitted
client uses — a defect, and not a hole in the information-flow pass that
anyone has shown how to reach.

### What would reverse this

Not that somebody wants to call an endpoint, and not a second client
appearing — one can be written today and its existence is what this section
is about. Three things together, in this order: authentication implemented,
so that a caller can be told apart from a stranger; a surface version that
moves independently of the compiler's, so that the wire refusal stops being
the whole compatibility story; and a place to declare that a name is part of
the surface, which is where #38's first option comes back and is answered on
evidence rather than in advance. The design spec's own re-entry conditions
for the externally-initiated effect name *"a second client or a public API
surface (#38)"* as the fact that reopens them; this section is the answer
they were waiting on, and it is no rather than not yet.

## Supply-chain controls in CI

Every one of these fails the build rather than warning:

| Gate | What it enforces |
| --- | --- |
| `scripts/check-forbid-unsafe.sh` | `#![forbid(unsafe_code)]` in every crate root, enumerated from `cargo metadata` |
| `cargo deny check` (`deny.toml`) | Advisories, licence allow-list, duplicate and wildcard bans, crates.io as the only source |
| `cargo audit --deny warnings` | RustSec advisories, including unmaintained and yanked crates |
| `scripts/check-advisory-exceptions.sh` | The two ignore lists agree, and every exception states its reason |
| `scripts/check-dependency-unsafe.sh` | `unsafe` in the dependency graph, measured and capped |

Exceptions are listed in `deny.toml` and `.cargo/audit.toml`, each with
the reason it exists and what would let it be removed. There is currently
one: `RUSTSEC-2024-0436`, the unmaintained `paste` crate, a build-time
proc-macro reached only through `boa_engine` with no upgrade available.
