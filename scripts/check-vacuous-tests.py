#!/usr/bin/env python3
"""Assert no `#[test]` in the workspace can pass without checking anything.

Why this exists: four gates and one test were found in a single night that
passed while inspecting nothing.  The instructive one was a test named
`a_static_initialiser_is_walked_by_the_flow_pass`, which asserted
`contains("E-IFC-02") || split.has_errors()`.  The second disjunct always
held, so the test passed whether or not the property in its own name
worked — and it was masking a real soundness hole.  It was not a weak
test.  It had stopped being a test, while its name went on telling every
reader the property was covered.

Two shapes are rejected, each the mechanical residue of that bug:

  1. `assert!(a || b)` — the exact shape above.  Whether an arm is
     unconditional can only be decided by reading, so the rule is that
     the author writes down why neither one is, on a `// falsifiable:`
     line.  Silence is the failure mode this whole file exists to stop.

  2. A test whose assertions are *all* inside a loop or an iterator
     adaptor over something computed, with nothing asserting how much was
     iterated.  `for x in xs { assert!(..) }` and `xs.iter().all(..)` are
     both vacuously true for an empty `xs`, which is how a gate ends up
     passing over zero inputs.  A loop over a literal `[..]` written in
     the test itself is exempt: it cannot silently become empty, because
     emptying it means deleting text from the test.

A third rule was written and then removed: "every test contains an
assertion".  It cannot be decided from the syntax.  Half this suite
asserts through helpers — `accept(src)`, `only(src)`, `def_named(..)` —
which panic on the failing case, and a rule that cannot see through a
function call reports two dozen sound tests and no defects.  A check that
is mostly false positives is turned off within a week, which leaves the
two rules above unenforced as well.  The narrower pair is the useful one.

The check refuses to pass when it cannot analyse the tree.  The file set
comes from `cargo metadata`, so a new crate is covered without anyone
remembering to add it, and the number of `#[test]` attributes the parser
found is compared against the number in the raw text.  A parser that
desynchronises reports zero findings; that is the failure this comparison
turns into an error.
"""

import json
import pathlib
import re
import subprocess
import sys

# A waived test names the reason it is waived.  A waiver whose test no
# longer trips a rule is an error, so the list cannot outlive its cause.
WAIVED: dict[str, str] = {}

TEST_ATTR = re.compile(r"#\[\s*test\s*\]")
ASSERT = re.compile(r"\bassert(_eq|_ne)?!")
FOR_HEAD = re.compile(r"\bfor\s+[^\n]*?\bin\s+([^\n{]+?)\s*\{")
ADAPTOR = re.compile(r"\.(all|any|for_each|position|find_map)\s*\(")
# An assertion that pins how much was looked at.
COUNTED = re.compile(
    r"assert(_eq|_ne)?!\s*\([^;]*?"
    r"(\.len\s*\(\)|is_empty\s*\(\)|\.count\s*\(\)"
    r"|\bseen\b|\bchecked\b|\bcovered\b|\bscanned\b|\bvisited\b|\bfound\b)"
)
# Looked for in the *unmasked* source: `blank_out` erases comments, so a
# justification searched for in the masked body could never be found.
FALSIFIABLE = re.compile(r"//\s*falsifiable:")
BASE_NAME = re.compile(r"^&?\s*([A-Za-z_][A-Za-z0-9_:]*)")
# `let xs = [..]`, `const XS: &[T] = &[..]`, `let xs = vec![..]`.
def literal_binding(name: str, whole: str) -> bool:
    head = re.escape(name.split("::")[0])
    return bool(
        re.search(
            r"\b(let|const|static)\s+(mut\s+)?" + head + r"\b[^;=]*=\s*(&\s*)?(\[|vec!\[)",
            whole,
        )
    )


def blank(text: str) -> str:
    """Spaces, but newlines kept, so offsets *and* line numbers survive."""
    return "".join("\n" if ch == "\n" else " " for ch in text)


def blank_out(src: str) -> str:
    """Replace comments, string and char literals with spaces of equal length.

    Offsets are preserved, so a position in the result is a position in the
    original.  Braces, `||` and `assert!` inside a string cannot then be
    mistaken for code.
    """
    out: list[str] = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and src[i + 1 : i + 2] == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(blank(src[i:j]))
            i = j
        elif c == "/" and src[i + 1 : i + 2] == "*":
            j = src.find("*/", i)
            j = n if j < 0 else j + 2
            out.append(blank(src[i:j]))
            i = j
        elif c == "r" and src[i + 1 : i + 2] in "#\"":
            k = i + 1
            while k < n and src[k] == "#":
                k += 1
            if k < n and src[k] == '"':
                term = '"' + "#" * (k - i - 1)
                j = src.find(term, k + 1)
                j = n if j < 0 else j + len(term)
                out.append(blank(src[i:j]))
                i = j
            else:
                out.append(c)
                i += 1
        elif c == '"':
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            out.append(blank(src[i:j]))
            i = j
        elif c == "'":
            m = re.match(r"'(\\.|[^\\'])'", src[i:])
            if m:
                out.append(blank(src[i : i + m.end()]))
                i += m.end()
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def test_bodies(masked: str):
    """(name, masked body, line, byte offset) for every `#[test]`."""
    for attr in TEST_ATTR.finditer(masked):
        signature = re.compile(r"\bfn\s+([A-Za-z0-9_]+)").search(masked, attr.end())
        if signature is None:
            continue
        open_brace = masked.find("{", signature.end())
        if open_brace < 0:
            continue
        depth, i = 0, open_brace
        while i < len(masked):
            if masked[i] == "{":
                depth += 1
            elif masked[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        else:
            continue
        yield (
            signature.group(1),
            masked[open_brace : i + 1],
            masked[: attr.start()].count("\n") + 1,
            open_brace,
        )


def span_of(body: str, opener: int, open_ch: str, close_ch: str) -> int:
    """Index just past the delimiter that closes the one at `opener`."""
    depth, i = 0, opener
    while i < len(body):
        if body[i] == open_ch:
            depth += 1
        elif body[i] == close_ch:
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return len(body)


def loop_spans(body: str) -> list[tuple[int, int, str]]:
    """`(start, end, iterable)` for every loop and iterator adaptor.

    The span is the loop's *body*, matched brace to brace, not everything
    after its header: an assertion written below a loop is outside it, and
    a rule that could not tell the difference reported sound tests.
    """
    spans = []
    for head in FOR_HEAD.finditer(body):
        brace = body.find("{", head.end() - 1)
        if brace < 0:
            continue
        spans.append((brace, span_of(body, brace, "{", "}"), head.group(1).strip()))
    for call in ADAPTOR.finditer(body):
        paren = call.end() - 1
        spans.append((paren, span_of(body, paren, "(", ")"), "<adaptor>"))
    return spans


def disjunctive_asserts(body: str) -> bool:
    """True when some `assert!(..)` has a `||` at its own argument level."""
    for start in re.finditer(r"\bassert!\s*\(", body):
        depth, i = 0, start.end() - 1
        while i < len(body):
            ch = body[i]
            if ch == "(":
                depth += 1
            elif ch == ")":
                depth -= 1
                if depth == 0:
                    break
            elif ch == "|" and body[i + 1 : i + 2] == "|" and depth == 1:
                return True
            i += 1
    return False


def crate_files() -> list[pathlib.Path]:
    """Every `.rs` file cargo says belongs to this workspace.

    From `cargo metadata`, not from a glob, so a crate with a non-default
    `path` or an extra `[[bin]]` cannot be skipped silently.
    """
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    packages = meta["packages"]
    if not packages:
        sys.exit("::error::cargo metadata listed no workspace packages")
    roots = {pathlib.Path(pkg["manifest_path"]).parent for pkg in packages}
    files = sorted(
        path
        for root in roots
        for path in root.rglob("*.rs")
        if "target" not in path.parts
    )
    return files


def main() -> int:
    files = crate_files()
    if not files:
        sys.exit("::error::found no Rust sources under the workspace packages")

    findings: list[tuple[str, str, str]] = []
    tripped: set[str] = set()
    attributes_in_text = 0
    tests_parsed = 0

    for path in files:
        raw = path.read_text(encoding="utf-8", errors="replace")
        masked = blank_out(raw)
        attributes_in_text += len(TEST_ATTR.findall(masked))
        for name, body, line, offset in test_bodies(masked):
            tests_parsed += 1
            key = f"{path}::{name}"
            where = f"{path}:{line}"
            justification = raw[offset : offset + len(body)]

            def report(rule: str, why: str) -> None:
                tripped.add(key)
                if key not in WAIVED:
                    findings.append((where, name, f"{rule}: {why}"))

            if disjunctive_asserts(body) and not FALSIFIABLE.search(justification):
                report(
                    "disjunction",
                    "an `assert!(a || b)` with no `// falsifiable:` note saying "
                    "why neither arm is unconditional",
                )

            asserts = list(ASSERT.finditer(body))
            loops = loop_spans(body)
            if asserts and loops:
                inside_only = all(
                    any(start < a.start() < end for start, end, _ in loops)
                    for a in asserts
                )
                iterables = [it for _, _, it in loops]

                def is_literal(iterable: str) -> bool:
                    if iterable.startswith("[") or iterable.startswith("vec!["):
                        return True
                    if re.match(r"^-?\d+\s*\.\.=?", iterable):
                        return True
                    base = BASE_NAME.match(iterable)
                    return bool(base) and literal_binding(base.group(1), raw)

                if (
                    inside_only
                    and not COUNTED.search(body)
                    and not all(is_literal(it) for it in iterables)
                ):
                    report(
                        "unbounded-loop",
                        "every assertion is inside a loop over "
                        f"`{iterables[0]}`, and nothing asserts how much was "
                        "iterated, so an empty one passes",
                    )

    if tests_parsed != attributes_in_text:
        sys.exit(
            f"::error::parsed {tests_parsed} test bodies but the sources hold "
            f"{attributes_in_text} `#[test]` attributes — the parser lost its "
            "place, so this run analysed less than the tree"
        )
    if tests_parsed == 0:
        sys.exit("::error::analysed no tests at all")

    stale = sorted(key for key in WAIVED if key not in tripped)
    for key in stale:
        print(f"::error::the waiver for {key} no longer applies; delete it")

    for where, name, why in findings:
        print(f"::error file={where.rsplit(':', 1)[0]}::{where}: {name}: {why}")

    print(
        f"vacuous tests: {len(findings)} finding(s) across {tests_parsed} tests "
        f"in {len(files)} files, {len(WAIVED)} waived"
    )
    return 1 if findings or stale else 0


if __name__ == "__main__":
    raise SystemExit(main())
