#!/usr/bin/env bash
# Count `unsafe` in the *dependency graph* and hold it under a ceiling.
#
# `#![forbid(unsafe_code)]` is checked by `check-forbid-unsafe.sh`, and it
# covers first-party code only: every crate in `crates/` is the compiler's
# own, and none of them may write `unsafe`. It says nothing about the 170-
# odd crates the compiler links, which is where essentially all of the
# `unsafe` in a Rust binary lives. `cargo geiger` counts that.
#
# Two assertions, and they do different jobs:
#
#   1. Every first-party crate reports zero in all five columns. This is
#      an *independent* confirmation of the forbid check — geiger counts
#      syntax, the forbid check reads an attribute — so a crate that lost
#      its attribute and gained an `unsafe` fails twice.
#   2. The dependency total is under a ceiling. The ceiling is generous on
#      purpose: geiger's count is target-dependent (`libc` alone differs
#      by hundreds of expressions between macOS and Linux) and it moves
#      with every `cargo update`, so pinning it to the exact measured
#      figure would fail on unrelated changes. It is set to catch a *new
#      unsafe-heavy dependency*, which is the event worth a human looking.
#
# The measured figure at the time this was written, on the `zdc-cli`
# graph with all features: 23,505 of 28,889 `unsafe` expressions are
# reachable, across 174 crates, of which the 13 first-party ones
# contribute zero. The largest single contributors are
# `intrusive-collections` (3,131), `memchr` (1,712) and the three
# `hashbrown` versions (4,146 between them) — all reached through
# `boa_engine`, the JavaScript interpreter this workspace runs its
# emission tests against.
#
# So the memory-safety claim this project can actually make is: the
# compiler contains no `unsafe` of its own, and stands on a dependency
# graph that contains a great deal of it.
set -euo pipefail

# Room for the target-to-target spread and for ordinary version drift,
# without room for a new unsafe-heavy crate to arrive unnoticed.
CEILING="${ZDC_UNSAFE_CEILING:-40000}"

cd "$(dirname "$0")/.."

# `cargo geiger` refuses to run against a virtual manifest, so it is run
# from the binary crate. `zdc-cli` depends on every library crate in the
# workspace, and the one member it does not reach — `zdc-bench` — brings
# in no third-party crate that `zdc-cli` does not already have, so its
# graph is the whole workspace's third-party surface. The first-party
# count below is therefore one short of the workspace's; the missing one
# is covered by `check-forbid-unsafe.sh`, which enumerates from
# `cargo metadata` and misses nothing.
#
# The scan's own failure is reported as its own failure, in geiger's own
# words. This used to read `2>/dev/null || true`, which turned every way
# geiger can fail — not installed, a dependency that will not resolve, a
# stale `.d` file naming a deleted source — into an empty report and the
# single message "produced no rows; the scan did not run". That message
# named nothing, and the one time it fired it pointed nowhere near the
# stale dep-info that caused it. A gate that cannot tell "clean" from
# "could not look" is not a gate.
#
# The exit status alone is not the discriminator, and treating it as one
# was the first attempt at this fix: geiger exits 1 for `error: Found N
# warnings` — 175 of them here, every one an ICU data file it declines to
# parse — while printing a complete table. **The table is what proves the
# scan ran.** So the status never decides anything on its own; it and the
# whole of geiger's stderr are handed to the parser, which fails when
# there is no table and says what geiger said either way.
if ! command -v cargo-geiger >/dev/null 2>&1; then
  echo "::error::cargo-geiger is not installed; run \`cargo install cargo-geiger\`" >&2
  exit 1
fi

set +e
GEIGER_STDERR=$(cd crates/zdc-cli && cargo geiger --all-features --output-format Ascii 2>&1 1>/tmp/zdc-geiger-$$.out)
GEIGER_STATUS=$?
set -e
GEIGER_REPORT=$(cat "/tmp/zdc-geiger-$$.out")
rm -f "/tmp/zdc-geiger-$$.out"
export GEIGER_REPORT GEIGER_STDERR GEIGER_STATUS

python3 - "$CEILING" <<'PY'
import os
import re
import sys

ceiling = int(sys.argv[1])
report = os.environ["GEIGER_REPORT"]
complaints = os.environ["GEIGER_STDERR"]
status = int(os.environ["GEIGER_STATUS"])

row = re.compile(
    r"^(\d+)/(\d+)\s+(\d+)/(\d+)\s+(\d+)/(\d+)\s+(\d+)/(\d+)\s+(\d+)/(\d+)\s+\S+\s+(.*)$"
)

crates, failed = {}, False
for line in report.splitlines():
    match = row.match(line)
    if not match:
        continue
    name = re.sub(r"^[|\`+\- ]*", "", match.group(11)).strip()
    crates[name] = tuple(int(n) for n in match.groups()[:10])

if not crates:
    # No table, so the scan did not measure anything — whether geiger
    # never ran, ran and died, or ran and printed a shape this parser no
    # longer recognises. Which of the three it was is in geiger's own
    # output, so print that rather than a sentence about it.
    print(f"::error::cargo geiger (exit {status}) produced no countable rows, so nothing was scanned.")
    print("--- what cargo geiger said ---")
    print(complaints.strip() or "(nothing on stderr)")
    print("--- what cargo geiger printed ---")
    print(report.strip() or "(nothing on stdout)")
    sys.exit(1)

first_party = {name: counts for name, counts in crates.items() if name.startswith("zdc-")}
if not first_party:
    print("::error::cargo geiger found no first-party crates; the scan is not measuring this workspace")
    print("--- what cargo geiger said ---")
    print(complaints.strip() or "(nothing on stderr)")
    sys.exit(1)

# A table exists, so the scan ran; geiger nevertheless exits non-zero for
# its own `Found N warnings`. That is not a failure to scan, and it is not
# nothing either — printed, so that a *different* complaint arriving here
# one day is read rather than absorbed.
if status != 0:
    lines = [line for line in complaints.splitlines() if line.strip()]
    print(f"note: cargo geiger exited {status} while producing a full table. It said:")
    for line in lines[:4]:
        print(f"  {line}")
    if len(lines) > 4:
        print(f"  ... and {len(lines) - 4} more, ending: {lines[-1]}")
for name, counts in sorted(first_party.items()):
    if any(counts):
        print(f"::error::{name} reports unsafe code: {counts}")
        failed = True

used = sum(counts[2] for counts in crates.values())
total = sum(counts[3] for counts in crates.values())
print(
    f"unsafe in dependencies: {used}/{total} expressions across {len(crates)} crates "
    f"({len(first_party)} first-party, all zero); ceiling {ceiling}"
)
if used > ceiling:
    print(f"::error::{used} reachable unsafe expressions exceeds the ceiling of {ceiling}")
    failed = True

sys.exit(1 if failed else 0)
PY
