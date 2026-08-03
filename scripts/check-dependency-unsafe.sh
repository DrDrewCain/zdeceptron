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
# from the binary crate — whose graph is a superset of every other
# crate's, because `zdc-cli` depends on all of them.
GEIGER_REPORT=$(cd crates/zdc-cli && cargo geiger --all-features --output-format Ascii 2>/dev/null || true)
export GEIGER_REPORT

python3 - "$CEILING" <<'PY'
import os
import re
import sys

ceiling = int(sys.argv[1])
report = os.environ["GEIGER_REPORT"]

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
    print("::error::cargo geiger produced no rows; the scan did not run")
    sys.exit(1)

first_party = {name: counts for name, counts in crates.items() if name.startswith("zdc-")}
if not first_party:
    print("::error::cargo geiger found no first-party crates; the scan is not measuring this workspace")
    sys.exit(1)
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
