#!/usr/bin/env bash
# Assert every crate root in the workspace carries #![forbid(unsafe_code)].
#
# Crate roots come from `cargo metadata`, not from a glob, so a crate with a
# non-default `path`, a second [[bin]], or a build script cannot be skipped
# silently. The scan count is checked against the number of crates so that a
# check which scans nothing fails rather than passing vacuously.
set -euo pipefail

cd "$(dirname "$0")/.."

roots=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c '
import json, sys
meta = json.load(sys.stdin)
for pkg in meta["packages"]:
    for target in pkg["targets"]:
        if {"lib", "bin"} & set(target["kind"]):
            print(target["src_path"])
')

scanned=0
missing=0
for root in $roots; do
  scanned=$((scanned + 1))
  if ! grep -q '#!\[forbid(unsafe_code)\]' "$root"; then
    echo "::error file=$root::missing #![forbid(unsafe_code)]"
    missing=1
  fi
done

crates=$(find crates -maxdepth 1 -mindepth 1 -type d | wc -l | tr -d ' ')
if [ "$scanned" -lt "$crates" ]; then
  echo "::error::scanned $scanned crate roots but the workspace has $crates crates"
  missing=1
fi

echo "forbid(unsafe_code): $scanned crate roots scanned across $crates crates"
exit $missing
