#!/usr/bin/env bash
# Assert `scripts/install.sh` parses under a POSIX shell, and that no
# variable is written immediately against a non-ASCII character.
#
# Why this exists: `install.sh` could not be run end to end until there
# was a published release to install from, so its first real execution was
# after `v0.1.0` went out — and it failed on line 94.
#
#     say "Installing zdc $VERSION for $target…"
#
# macOS ships bash 3.2 as `/bin/sh`, and it reads the multi-byte `…` as
# part of the variable name: it looks up `target…`, finds nothing, and
# `set -u` ends the script. The fix is one pair of braces. The bug was
# invisible to every check this repository had, because nothing ran the
# installer and a shell parse alone does not catch it — `$target…` is
# syntactically fine, it just names a different variable.
#
# So two things are checked. The first is the specific shape that bit us.
# The second is that the script parses at all under `sh -n`, which catches
# the ordinary syntax errors a rewrite might introduce.
set -euo pipefail

cd "$(dirname "$0")/.."

script=scripts/install.sh
failed=0

# --- a variable pressed against a non-ASCII byte -----------------------------
#
# `${name}` is fine and `$name` followed by ASCII is fine. What is not is
# `$name` followed immediately by a byte outside ASCII, because where the
# name ends is then up to the shell.
# python3 rather than `grep -P`, which BSD grep does not have — the first
# draft of this check used it, errored on macOS, and therefore passed with
# the bug still in place. A gate that cannot fail is worse than none.
if ! python3 - "$script" <<'PYTHON'
import re, sys

pattern = re.compile(r"\$[A-Za-z_][A-Za-z0-9_]*[^\x00-\x7F]")
offenders = []
with open(sys.argv[1], encoding="utf-8") as handle:
    for number, line in enumerate(handle, 1):
        if pattern.search(line):
            offenders.append(f"  {number}: {line.rstrip()}")

if offenders:
    print("::error::a variable is written directly against a non-ASCII character.")
    print("Bash 3.2 — which is /bin/sh on macOS — reads the character as part of")
    print("the name. Write ${name} instead:")
    print("\n".join(offenders))
    sys.exit(1)
PYTHON
then
    failed=1
fi

# --- it parses under a POSIX shell -------------------------------------------
for shell in sh bash; do
    if ! command -v "$shell" >/dev/null; then
        continue
    fi
    if ! "$shell" -n "$script" 2>/tmp/installer-parse.$$; then
        echo "::error::$script does not parse under $shell:"
        sed 's/^/  /' /tmp/installer-parse.$$
        failed=1
    fi
    rm -f /tmp/installer-parse.$$
done

if [ "$failed" -eq 0 ]; then
    echo "installer: no variable against a non-ASCII byte, parses under sh and bash"
fi
exit "$failed"
