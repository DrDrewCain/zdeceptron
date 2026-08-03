#!/usr/bin/env bash
# Count `unsafe` in the *dependency graph* and hold it under a ceiling.
#
# `#![forbid(unsafe_code)]` is checked by `check-forbid-unsafe.sh`, and it
# covers first-party code only: every crate in `crates/` is the compiler's
# own, and none of them may write `unsafe`. It says nothing about the 180-
# odd crates the compiler links, which is where essentially all of the
# `unsafe` in a Rust binary lives. `cargo geiger` counts that.
#
# Three assertions, and they do different jobs:
#
#   1. Every first-party crate reports zero in all five columns. This is
#      an *independent* confirmation of the forbid check — geiger counts
#      syntax, the forbid check reads an attribute — so a crate that lost
#      its attribute and gained an `unsafe` fails twice.
#   2. Every first-party crate the scan is supposed to reach is present in
#      the table. A check that measures nothing passes everything, so a
#      report that is empty, or that is missing a workspace member, is a
#      failure and names what is missing.
#   3. The dependency total is under a ceiling. The ceiling is generous on
#      purpose: geiger's count is target-dependent (`libc` alone differs
#      by hundreds of expressions between macOS and Linux) and it moves
#      with every `cargo update`, so pinning it to the exact measured
#      figure would fail on unrelated changes. It is set to catch a *new
#      unsafe-heavy dependency*, which is the event worth a human looking.
#
# Which crates are first-party is decided by `cargo metadata`, never by a
# name prefix in geiger's table. Reading identity out of a third-party
# tool's formatting is what broke this check in CI once already: the
# workflow sets `CARGO_TERM_COLOR: always`, geiger forwards that to
# `colored`, and every row for a crate that forbids `unsafe` came back
# wrapped in green SGR escapes. The escapes defeated the row regex, so the
# only rows that parsed were the uncoloured ones — 58 of 182, none of them
# first-party — and the check failed on a graph that was perfectly
# healthy. The escapes are stripped below, but the durable fix is that the
# set of first-party crates now comes from the resolver.
#
# The measured figure at the time this was written, on the `zdc-cli`
# graph with all features: 23,857 of 29,241 `unsafe` expressions are
# reachable, across 182 crates, of which the 17 first-party ones
# contribute zero. The largest single contributors are
# `intrusive-collections` (3,131), `memchr` (2,440) and the three
# `hashbrown` versions (4,791 between them) — all reached through
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

# The authoritative answer to "which crates are ours", and to "which of
# ours should this scan see". `cargo geiger` refuses to run against a
# virtual manifest, so it is run from the binary crate; the crates it can
# reach are exactly the workspace members in `zdc-cli`'s normal-dependency
# closure, which is what the walk below computes. Members outside that
# closure are named in the output rather than silently tolerated, and are
# covered by `check-forbid-unsafe.sh`, which enumerates every member.
WORKSPACE_JSON=$(cargo metadata --all-features --format-version 1 | python3 -c '
import json, sys

meta = json.load(sys.stdin)
name_of = {pkg["id"]: pkg["name"] for pkg in meta["packages"]}
members = set(meta["workspace_members"])
nodes = {node["id"]: node for node in meta["resolve"]["nodes"]}

roots = [m for m in members if name_of[m] == "zdc-cli"]
if len(roots) != 1:
    sys.exit("expected exactly one zdc-cli workspace member")

reachable, stack = set(), [roots[0]]
while stack:
    current = stack.pop()
    if current in reachable:
        continue
    reachable.add(current)
    for dep in nodes[current]["deps"]:
        if dep["pkg"] not in members:
            continue
        kinds = dep.get("dep_kinds") or [{}]
        if any(kind.get("kind") is None for kind in kinds):
            stack.append(dep["pkg"])

json.dump(
    {
        "members": sorted(name_of[m] for m in members),
        "expected": sorted(name_of[m] for m in reachable),
    },
    sys.stdout,
)
')
export WORKSPACE_JSON

# `cargo geiger` exits non-zero for findings of its own — it ends with
# "Found N warnings" and a failing status while printing the whole table,
# and it reports 284 such warnings on a healthy tree here. Its status is
# therefore not the discriminator; the table is. Both the status and the
# stderr are printed either way, because the warnings are worth reading:
# one of them caught a stale dep-info file naming a deleted test.
GEIGER_STDERR=$(mktemp)
trap 'rm -f "$GEIGER_STDERR"' EXIT

GEIGER_STATUS=0
GEIGER_REPORT=$(cd crates/zdc-cli && cargo geiger --all-features --output-format Ascii 2>"$GEIGER_STDERR") || GEIGER_STATUS=$?
export GEIGER_REPORT

echo "cargo geiger exited $GEIGER_STATUS; its stderr follows"
cat "$GEIGER_STDERR"
echo "end of cargo geiger stderr"

python3 - "$CEILING" <<'PY'
import json
import os
import re
import sys

ceiling = int(sys.argv[1])
report = os.environ["GEIGER_REPORT"]
workspace = json.loads(os.environ["WORKSPACE_JSON"])
members = set(workspace["members"])
expected = set(workspace["expected"])

# Colour is applied per row, so a coloured table is not merely prettier --
# it is unparseable to a regex anchored at the first digit.
sgr = re.compile(r"\x1b\[[0-9;]*m")
row = re.compile(
    r"^(\d+)/(\d+)\s+(\d+)/(\d+)\s+(\d+)/(\d+)\s+(\d+)/(\d+)\s+(\d+)/(\d+)\s+\S+\s+(.*)$"
)
# Both the ASCII vines geiger draws for `--output-format Ascii` and the
# box-drawing ones it draws for every other format.
vines = re.compile(r"^[|`+\-│├└─ ]*")

crates, failed = {}, False
for line in report.splitlines():
    match = row.match(sgr.sub("", line))
    if not match:
        continue
    label = vines.sub("", match.group(11)).strip()
    if not label:
        continue
    # The label is `{name} {version}`; two versions of one crate are two
    # rows and must stay two entries, or the totals undercount.
    crates[label] = (label.split()[0], tuple(int(n) for n in match.groups()[:10]))

if not crates:
    print("::error::cargo geiger produced no rows; the scan did not run")
    sys.exit(1)

first_party = {
    label: counts for label, (name, counts) in crates.items() if name in members
}
if not first_party:
    print(
        "::error::cargo geiger found no first-party crates; "
        "the scan is not measuring this workspace"
    )
    sys.exit(1)

seen = {name for name, _ in crates.values() if name in members}
missing = sorted(expected - seen)
if missing:
    print(
        f"::error::cargo geiger did not report {len(missing)} workspace "
        f"member(s) it should have reached: {', '.join(missing)}"
    )
    failed = True

for label, counts in sorted(first_party.items()):
    if any(counts):
        print(f"::error::{label} reports unsafe code: {counts}")
        failed = True

unreached = sorted(members - expected)
if unreached:
    print(
        "outside the zdc-cli graph, covered by check-forbid-unsafe.sh: "
        f"{', '.join(unreached)}"
    )

used = sum(counts[2] for _, counts in crates.values())
total = sum(counts[3] for _, counts in crates.values())
verdict = "all zero" if not any(map(any, first_party.values())) else "NOT all zero"
print(
    f"unsafe in dependencies: {used}/{total} expressions across {len(crates)} crates "
    f"({len(seen)} of the workspace's {len(members)} first-party crates, {verdict}); "
    f"ceiling {ceiling}"
)
if used > ceiling:
    print(f"::error::{used} reachable unsafe expressions exceeds the ceiling of {ceiling}")
    failed = True

sys.exit(1 if failed else 0)
PY
