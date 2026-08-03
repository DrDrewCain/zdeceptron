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
