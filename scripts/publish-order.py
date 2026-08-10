#!/usr/bin/env python3
"""Print the workspace crates in an order `cargo publish` can follow.

A crate cannot go to crates.io before the crates it depends on are already
there, so every publish has to happen in dependency order. That order was
written into `release.yml` by hand once. Hand-written was wrong the
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

**A crate with `publish = false` is left out of the order, and the edges
into it are checked rather than ignored.** `release.yml` feeds this output
straight to `cargo publish -p`, which fails outright on a crate whose
manifest forbids publishing — half way through a release, which is the
failure this script exists to prevent. `zdc-wasm` is the crate in question:
nothing links it, its interface is a JSON document rather than a Rust API,
and its manifest says why at length.

Leaving a crate out is only sound while nothing published depends on it, so
that is asserted rather than assumed. A published crate depending on an
unpublished one is not an error cargo reports at build time; it surfaces on
the day of a release, with the earlier crates already up and unrecallable.
[`unpublishable`] moves that failure to the pull request that introduces it.

Ties are broken by name so the output is the same on every machine and in
every run. A release that publishes in a different order each time is one
whose failures cannot be reproduced.

Run it directly to see the order; `--check` exits non-zero if no order
exists or if an unpublished crate has been made load-bearing, which is the
CI gate.
"""

import json
import subprocess
import sys


def metadata() -> dict:
    # `--no-deps` keeps this to the crates in this repository, which is all
    # that is being published, and makes it fast enough to run as a gate.
    raw = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return json.loads(raw)


def unpublishable(meta: dict) -> set[str]:
    """The crates whose manifests say `publish = false`.

    Cargo reports the field as a list of allowed registries, and `false`
    becomes the empty list. `None` — the field absent — means every
    registry, which is the normal case and the one that must not be
    confused with the empty list: `not package["publish"]` would call both
    unpublishable and drop the whole workspace from the release.
    """
    return {
        package["name"]
        for package in meta["packages"]
        if package.get("publish") == []
    }


def workspace(meta: dict) -> dict[str, set[str]]:
    """Each publishable crate, mapped to the crates it needs first."""
    skipped = unpublishable(meta)
    members = {p["name"] for p in meta["packages"]}
    graph: dict[str, set[str]] = {}
    for package in meta["packages"]:
        if package["name"] in skipped:
            continue
        needs = {
            dep["name"]
            for dep in package["dependencies"]
            if dep["name"] in members and dep.get("kind") != "dev"
        }
        # The assertion that makes the skip above sound. A published crate
        # reaching an unpublished one is unbuildable from crates.io, and
        # dropping the edge silently would hide that until a release.
        reaches = needs & skipped
        if reaches:
            raise SystemExit(
                f"`{package['name']}` is published and depends on "
                f"{', '.join(sorted(reaches))}, which is not. A crate on "
                "crates.io must be buildable from crates.io, so every crate "
                "it links has to be there too. Either publish the dependency "
                "or, if the edge is only needed by tests, move it to "
                "[dev-dependencies] and drop its version."
            )
        graph[package["name"]] = needs - skipped
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
    meta = metadata()
    result = order(workspace(meta))
    if "--check" in sys.argv:
        skipped = sorted(unpublishable(meta))
        # Named, not counted. A crate silently dropped from a release is
        # exactly the thing this line exists to make visible, and "20 of 21"
        # does not say which one.
        held = f"; not publishing {', '.join(skipped)}" if skipped else ""
        print(f"publish order exists for {len(result)} crates{held}")
        return
    print("\n".join(result))


if __name__ == "__main__":
    main()
