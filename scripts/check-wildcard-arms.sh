#!/usr/bin/env bash
# Assert no `match` in the workspace covers a *guarded* enum with a wildcard
# arm, so that adding a variant to one of them is a compile error at every
# site that has to consider it.
#
# Why this exists: this codebase was repeatedly described as having no
# wildcard match arms by design. It has dozens. `static` was then added as
# the fourth placement, and the completion engine's
# `Client | Server | Durable => InType` arm silently gave it a value
# position's behaviour — no compile error, no test failure, wrong output.
# This is the check that makes the claim true where it matters.
#
# The guarded set is deliberately not every enum. A wildcard over
# `TokenKind` — every token the lexer can produce — or over `Type`, where
# the wildcard is the base case of a structural recursion, is the right
# code and stays. Guarded here are the closed domains this compiler owns
# and reasons about exhaustively: placement, the split's crossings, member
# forms and boundary edges, the regions and roots code runs in, and the
# information-flow lattice and sink list.
#
# The detector is clippy's own `wildcard_enum_match_arm`, whose suggestion
# names the variants the arm swallows together with their enum paths. That
# is a fact about the type being matched, resolved by the compiler, so a
# rename, an alias or a re-export cannot hide a violation from it.
#
# Cargo replays cached diagnostics, so a warm target directory reports the
# same findings a cold one does. The number of crates clippy actually
# checked is asserted anyway, so a run that analyses nothing fails rather
# than passing vacuously.
set -euo pipefail

cd "$(dirname "$0")/.."

GUARDED='
Placement
SignalPlacement
ReadContext
ReadKind
Crossing
MutCrossing
MemberForm
BoundaryEdge
EndpointKind
RootOrigin
Region
RootKind
Sink
SinkSite
FailureCode
Secrecy
Obs
DefKind
HirNode
Site
'
export GUARDED

# `--message-format=json`, because `suggested_replacement` — the list of
# variants the wildcard covers — appears only there.
cargo clippy --workspace --all-targets --all-features --message-format=json \
  -- -W clippy::wildcard_enum_match_arm 2>/dev/null \
  | python3 -c '
import json, os, re, sys

guarded = {name for name in os.environ["GUARDED"].split() if name}
seen, bad, checked = set(), [], set()

for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue

    if message.get("reason") == "compiler-artifact":
        path = message.get("manifest_path", "")
        parts = path.split("/crates/")
        if len(parts) == 2:
            checked.add(parts[1].split("/")[0])
        continue

    body = message.get("message")
    if not isinstance(body, dict):
        continue
    if (body.get("code") or {}).get("code") != "clippy::wildcard_enum_match_arm":
        continue

    span = body["spans"][0]
    where = (span["file_name"], span["line_start"], span["column_start"])
    if where in seen:
        continue
    seen.add(where)

    covered = ""
    for child in body.get("children", []):
        for child_span in child.get("spans", []):
            if child_span.get("suggested_replacement"):
                covered = child_span["suggested_replacement"]
    # Every segment before a `::`, so `Foo::Bar`, `other @ Foo::Bar` and
    # `ast::Placement::Static` all yield the name the guarded list is
    # written in. A lookahead rather than a consuming match: `ast::` and
    # `Placement::` overlap, and consuming the first hides the second.
    enums = set(re.findall(r"([A-Za-z_][A-Za-z0-9_]*)(?=::)", covered))
    hit = sorted(enums & guarded)
    if hit:
        bad.append((where, ", ".join(hit), " ".join(covered.split())[:110]))

for (path, line, column), enums, covered in bad:
    print(f"::error file={path},line={line},col={column}::"
          f"wildcard arm over {enums}. Write the variants out so the next one "
          f"is a compile error: {covered}")

print(f"wildcard arms: {len(seen)} inspected, {len(bad)} over a guarded enum, "
      f"across {len(checked)} crates", file=sys.stderr)

expected = len([name for name in os.listdir("crates") if not name.startswith(".")])
if len(checked) < expected:
    print(f"::error::clippy checked {len(checked)} of {expected} crates, so this "
          f"check did not run over the workspace", file=sys.stderr)
    sys.exit(2)

sys.exit(1 if bad else 0)
' 2>&1
