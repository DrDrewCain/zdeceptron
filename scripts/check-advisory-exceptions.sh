#!/usr/bin/env bash
# Assert the two supply-chain gates agree about what they ignore.
#
# `cargo deny` and `cargo audit` read the same RustSec database from two
# different config files. If one ignores an advisory the other does not,
# one of the two gates is lying about the state of the dependency graph —
# and the one that is lying is whichever CI happens to run first.
#
# Also asserts that every ignored advisory carries a comment. An exception
# with no stated reason is indistinguishable from an oversight, and it is
# the thing nobody ever revisits.
set -euo pipefail

cd "$(dirname "$0")/.."

python3 - <<'PY'
import re
import sys
import tomllib


def ignored(path):
    """The advisory ids a config ignores, and which of them are preceded
    by a comment in the file itself."""
    raw = open(path, "rb").read()
    config = tomllib.loads(raw.decode("utf-8"))
    ids = [
        entry if isinstance(entry, str) else entry.get("id", "")
        for entry in config.get("advisories", {}).get("ignore", [])
    ]

    commented, pending = set(), False
    for line in raw.decode("utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("#"):
            pending = True
            continue
        found = re.findall(r"RUSTSEC-[0-9]{4}-[0-9]{4}", stripped)
        if found and pending:
            commented.update(found)
        # `pending` means "the line immediately above was a comment", so it
        # is cleared by any line that is not one — including a blank line
        # or a section header, which would otherwise let a comment at the
        # top of the file vouch for an id much further down.
        pending = False
    return ids, commented


deny, deny_commented = ignored("deny.toml")
audit, audit_commented = ignored(".cargo/audit.toml")

failed = False
if sorted(deny) != sorted(audit):
    print(
        f"::error::deny.toml ignores {sorted(deny)} but .cargo/audit.toml "
        f"ignores {sorted(audit)}"
    )
    failed = True

for path, ids, commented in (
    ("deny.toml", deny, deny_commented),
    (".cargo/audit.toml", audit, audit_commented),
):
    for advisory in ids:
        if advisory not in commented:
            print(f"::error file={path}::{advisory} is ignored with no reason given")
            failed = True

if not failed:
    print(
        f"advisory exceptions: {len(deny)} ignored, agreed by both gates, all explained"
    )
sys.exit(1 if failed else 0)
PY
