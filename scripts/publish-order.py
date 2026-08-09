#!/usr/bin/env python3
"""Print the workspace crates in an order `cargo publish` can follow.

A crate cannot go to crates.io before the crates it depends on are already
there, so nineteen publishes have to happen in dependency order. That order
was written into `release.yml` by hand once. Hand-written was wrong the
moment anyone added a crate or moved a dependency: the list keeps working
right up until it doesn't, and the place it fails is half way through a
release, with some crates published and some not — and a published version
cannot be taken back.

So it is computed, from the same `cargo metadata` the build itself uses.

**Dev-dependencies are excluded, and that is what makes this possible at
all.** The test-only edges between these crates form cycles — `zdc-host`
is used by `zdc-codegen`'s tests and `zdc-codegen` by `zdc-host`'s — so a
graph including them has no topological order. Cargo omits a version-less
dev-dependency from the published manifest, which is why those edges cost
nothing here and why the path dependencies under `[dev-dependencies]` must
stay version-less.

Ties are broken by name so the output is the same on every machine and in
every run. A release that publishes in a different order each time is one
whose failures cannot be reproduced.

Run it directly to see the order; `--check` exits non-zero if no order
exists, which is the CI gate.
"""

import json
import subprocess
import sys


def workspace() -> dict[str, set[str]]:
    """Each workspace crate, mapped to the workspace crates it needs first."""
    # `--no-deps` keeps this to the crates in this repository, which is all
    # that is being published, and makes it fast enough to run as a gate.
    raw = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    meta = json.loads(raw)

    members = {p["name"] for p in meta["packages"]}
    graph: dict[str, set[str]] = {}
    for package in meta["packages"]:
        graph[package["name"]] = {
            dep["name"]
            for dep in package["dependencies"]
            if dep["name"] in members and dep.get("kind") != "dev"
        }
    return graph


def order(graph: dict[str, set[str]]) -> list[str]:
    """Dependencies first. Raises if the graph has a cycle."""
    done: list[str] = []
    placed: set[str] = set()
    while len(placed) < len(graph):
        # Everything whose dependencies are already placed, in name order.
        ready = sorted(
            name
            for name, needs in graph.items()
            if name not in placed and needs <= placed
        )
        if not ready:
            stuck = sorted(set(graph) - placed)
            raise SystemExit(
                "no publish order exists: these crates depend on one another "
                f"through non-dev dependencies:\n  {', '.join(stuck)}\n"
                "A cycle here cannot be published at all. If the edge is only "
                "needed by tests, move it to [dev-dependencies] and drop its "
                "version, which is how the existing cycles are already handled."
            )
        done.extend(ready)
        placed.update(ready)
    return done


def main() -> None:
    result = order(workspace())
    if "--check" in sys.argv:
        print(f"publish order exists for {len(result)} crates")
        return
    print("\n".join(result))


if __name__ == "__main__":
    main()
