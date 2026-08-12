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
# `TokenKind` — every token the lexer can produce — is the right code and
# stays. Guarded here are the closed domains this compiler owns and
# reasons about exhaustively: placement, the split's crossings, member
# forms and boundary edges, the regions and roots code runs in, the
# information-flow lattice and sink list, and `Type`.
#
# `Type` was added by #280, and it is the one entry that needs an
# exception, so the exception is a mechanism rather than a footnote. A
# match on `Type` comes in two shapes and only one of them is a hazard:
#
#   * **Dispatch.** Some variants are named and the rest get one answer.
#     A variant added later inherits that answer silently, which is how
#     #277 happened: `unify`'s scalar arm listed six scalars, `Code` was
#     not among them, and two `Code`s fell through to a shape mismatch —
#     the checker refusing a type against itself, in a message that named
#     `Code` twice. These are written out.
#
#   * **Identity and recursion.** `settled => settled` returns what it
#     matched, and `occurs`'s base case is "nothing to walk into". A
#     variant added later is handled *correctly* by both, and clippy's
#     suggestion for a binding arm is thirteen repetitions of
#     `other @ Type::…` that say nothing and rot on the next variant.
#
# So an arm may be waived in the source with `// wildcard-ok: <reason>`,
# on the arm or the line above it. A reason is required and an empty one
# fails the check, because the point of the marker is the sentence rather
# than the silence.
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
Type
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
waivers = 0

MARKER = "// wildcard-ok:"


# Whether the arm reported at `line` carries a reasoned waiver.
#
# The marker is looked for on the arm itself and then upward through the
# comment block immediately above it, stopping at the first line that is
# not a comment. A reason worth writing usually runs to several lines, and
# requiring the marker on the last of them would put it where it reads
# worst — so it goes wherever it reads best and this walks to find it.
#
# Read from the source rather than from a list of line numbers in this
# script, so a waiver moves with the code it excuses instead of drifting
# onto whatever ends up at that line later.
def waived(path, line):
    try:
        with open(path, encoding="utf-8") as handle:
            lines = handle.readlines()
    except OSError:
        return False

    candidates = [line - 1]
    above = line - 2
    while 0 <= above < len(lines) and lines[above].lstrip().startswith("//"):
        candidates.append(above)
        above -= 1

    for candidate in candidates:
        if not (0 <= candidate < len(lines)) or MARKER not in lines[candidate]:
            continue
        # The reason is everything after the marker *and* whatever the rest
        # of the block says, so a marker that trails off onto the next line
        # still counts. Only a marker with nothing after it at all fails.
        reason = lines[candidate].split(MARKER, 1)[1].strip()
        if reason:
            return True
        print(f"::error file={path},line={candidate + 1}::"
              f"a `{MARKER}` with no reason after it. The marker is the "
              f"sentence, not the silence.", file=sys.stderr)
        sys.exit(2)
    return False


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

    if waived(span["file_name"], span["line_start"]):
        waivers += 1
        continue

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
      f"{waivers} waived, across {len(checked)} crates", file=sys.stderr)

expected = len([name for name in os.listdir("crates") if not name.startswith(".")])
if len(checked) < expected:
    print(f"::error::clippy checked {len(checked)} of {expected} crates, so this "
          f"check did not run over the workspace", file=sys.stderr)
    sys.exit(2)

sys.exit(1 if bad else 0)
' 2>&1
