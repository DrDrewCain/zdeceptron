#!/usr/bin/env python3
"""Write the front page of the rustdoc site.

`cargo doc` on a workspace of twenty-one crates writes twenty-one
directories and no `index.html` above them, so the root of a site built
from `target/doc` is a 404. Every page rustdoc does write carries the full
crate list in its sidebar; the one URL a reader is given — the one in
`README.md` — is the only one that has nothing.

So this writes that page, from `cargo metadata`, rather than by hand. The
list and the one-line descriptions are read from the manifests, which are
already the place those sentences have to be correct: `description` is
required for `cargo publish`, so a crate whose description is wrong here is
a crate whose crates.io page is wrong too, and there is only one of them to
fix. A hand-written list would be right on the day it was written and
silently short by one the day a crate was added — which is this
repository's characteristic failure, and the reason `publish-order.py`
computes its answer instead of storing it.

**A package with no documented directory is an error, not a row omitted.**
The failure this guards against is `cargo doc -p something` leaving a
stale tree behind, which would produce a page that looks complete and
links four crates. Being loud costs a re-run of `cargo doc`; being quiet
costs a reader who believes the list.

A package is represented by its library, or by its binaries when it has no
library — `zdc-cli` is the whole compiler and ships as `zdc`, with no lib
target at all, so keying on libraries alone would drop the one crate a
user has actually run.
"""

import html
import json
import subprocess
import sys
from pathlib import Path

# Kinds rustdoc emits a directory for. A test, a benchmark or an example
# target is compiled but never documented, and asking for its directory
# would fail this script over something rustdoc was never going to write.
DOCUMENTED_KINDS = {"lib", "rlib", "dylib", "cdylib", "proc-macro", "bin"}


def metadata() -> dict:
    # `--no-deps`: this page lists the crates in this repository. The
    # dependencies are not documented either (`cargo doc --no-deps`), so
    # listing them would link nothing.
    raw = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return json.loads(raw)


def doc_names(package: dict) -> list[str]:
    """The directory names rustdoc writes for one package.

    Rustdoc replaces `-` with `_`, and skips a binary whose name collides
    with the package's own library — `zdc-wasm` is both — so the library
    is preferred and the binaries are the fallback rather than an addition.
    """

    def named(kinds: set[str]) -> list[str]:
        return [
            target["name"].replace("-", "_")
            for target in package["targets"]
            if kinds.intersection(target["kind"])
        ]

    libs = named(DOCUMENTED_KINDS - {"bin"})
    return libs or named({"bin"})


def rows(meta: dict, doc_dir: Path) -> list[tuple[str, str, str]]:
    """`(link, crate name, description)` for every workspace package."""
    members = set(meta["workspace_members"])
    found, missing = [], []
    for package in meta["packages"]:
        if package["id"] not in members:
            continue
        description = package.get("description") or ""
        for name in doc_names(package):
            if (doc_dir / name / "index.html").is_file():
                found.append((f"{name}/index.html", package["name"], description))
                break
        else:
            missing.append(package["name"])
    if missing:
        sys.exit(
            "no documentation under {} for: {}\n"
            "run `cargo doc --workspace --no-deps --document-private-items` "
            "first".format(doc_dir, ", ".join(sorted(missing)))
        )
    # By name. Any ordering that carried meaning — the order of the passes,
    # say — would be a judgement written down in a third place and left to
    # drift; this one is a fact about the names.
    return sorted(found, key=lambda row: row[1])


def described(description: str) -> str:
    """A `description` field as HTML.

    The manifests write code spans in Markdown, because crates.io renders
    Markdown. Backticks left alone here would be shown as backticks, so
    the one construct those sentences actually use is translated and the
    rest is escaped.
    """
    return "".join(
        f"<code>{html.escape(part)}</code>" if index % 2 else html.escape(part)
        for index, part in enumerate(description.split("`"))
    )


def page(entries: list[tuple[str, str, str]]) -> str:
    listing = "\n".join(
        "      <tr>\n"
        f'        <td><a href="{html.escape(link)}"><code>{html.escape(name)}</code></a></td>\n'
        f"        <td>{described(description)}</td>\n"
        "      </tr>"
        for link, name, description in entries
    )
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>ZDeceptron crate documentation</title>
<style>
:root {{ color-scheme: light dark; --fg: #1a1a1a; --bg: #fff; --dim: #555; --rule: #ddd; --link: #0b5ed7; }}
@media (prefers-color-scheme: dark) {{
  :root {{ --fg: #e6e6e6; --bg: #16181d; --dim: #a0a4ad; --rule: #33363d; --link: #7fb0ff; }}
}}
* {{ box-sizing: border-box; }}
body {{ margin: 0 auto; padding: 2.5rem 1.25rem 4rem; max-width: 46rem; background: var(--bg); color: var(--fg);
  font: 16px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; }}
h1 {{ font-size: 1.6rem; margin: 0 0 .5rem; }}
p {{ color: var(--dim); }}
a {{ color: var(--link); }}
code {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .95em; }}
table {{ border-collapse: collapse; width: 100%; margin-top: 2rem; }}
td {{ border-top: 1px solid var(--rule); padding: .6rem .75rem .6rem 0; vertical-align: top; }}
td:first-child {{ white-space: nowrap; }}
footer {{ margin-top: 3rem; border-top: 1px solid var(--rule); padding-top: 1rem; color: var(--dim); font-size: .9rem; }}
</style>
</head>
<body>
<h1>ZDeceptron crate documentation</h1>
<p>API documentation for the {len(entries)} crates the compiler is built from, with
private items included — most of the reasoning in this workspace is written
down beside items no consumer can call.</p>
<p>The place to start is
<a href="zdc_graph/integrity/index.html"><code>zdc_graph::integrity</code></a>,
which states what the integrity lattice does <em>not</em> claim, and
<a href="zdc_graph/index.html"><code>zdc_graph</code></a> itself, whose two passes are
what make ZDeceptron a language rather than a framework.</p>
<table>
  <tbody>
{listing}
  </tbody>
</table>
<footer>
Generated from <code>cargo metadata</code> by <code>scripts/doc-index.py</code>.
Source, issues and the rest of the documentation:
<a href="https://github.com/DrDrewCain/zdeceptron">github.com/DrDrewCain/zdeceptron</a>.
</footer>
</body>
</html>
"""


def main() -> None:
    meta = metadata()
    doc_dir = Path(meta["target_directory"]) / "doc"
    if not doc_dir.is_dir():
        sys.exit(f"no {doc_dir}; run `cargo doc --workspace --no-deps` first")
    entries = rows(meta, doc_dir)
    (doc_dir / "index.html").write_text(page(entries), encoding="utf-8")
    print(f"{doc_dir / 'index.html'}: {len(entries)} crates")


if __name__ == "__main__":
    main()
