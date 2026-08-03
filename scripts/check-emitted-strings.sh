#!/usr/bin/env bash
# Assert no emitter builds a quoted literal by interpolating around the
# quotes.
#
# Three injection holes have been found in this compiler, in three
# different emitters — the `import` clause, the generated `class` getter,
# and the folded stylesheet — and all three had the same shape: a `format!`
# that wrote an opening quote, then `{something}`, then a closing quote,
# with the something coming from the program being compiled. `js::string`
# and `js::json_string` exist precisely so that the quotes and the escaping
# are decided by one function; a site that writes its own quotes has opted
# out of it.
#
# So the rule is mechanical: inside `zdc-codegen`, a quote character may
# not sit immediately beside a format placeholder *within a string literal
# that becomes output*. `js.rs` is the one exempt file, because writing
# those quotes is its whole job — and the exemption is asserted rather than
# assumed, so deleting or renaming the file fails this check rather than
# silently widening it. Diagnostic messages are excluded too, because they
# are prose that quotes the program rather than output that runs it; they
# are handled instead by `zdc-diagnostics`, which replaces every control
# character before anything reaches a terminal.
#
# Telling those two apart needs to know which string literals sit inside a
# call to `error(` or inside a `CodegenError`, which needs a scan that
# understands Rust's comments, string literals and nesting. That scan is
# below, and it is deliberately fragile in the safe direction: if it ends a
# file inside a string or with unbalanced delimiters, it has failed to
# understand the file and says so rather than reporting no findings.
#
# Three anti-vacuity guards, because gates in this repository have
# previously passed while inspecting nothing:
#
#   1. the scanner is run against a canary containing one of each forbidden
#      form, and the check fails unless every one of them is flagged — and
#      unless the two forms that must *not* be flagged are not;
#   2. the file list comes from `git ls-files` and is compared against the
#      directory listing, so a glob matching nothing, or missing a newly
#      added module, fails rather than passing;
#   3. a file the scanner cannot parse to a clean end state is an error.
set -euo pipefail

cd "$(dirname "$0")/.."

python3 - js.rs crates/zdc-codegen/src <<'PY'
import pathlib
import re
import subprocess
import sys

EXEMPT, EMITTER_DIR = sys.argv[1], sys.argv[2]

# Inside a string literal, as the literal is *written in Rust source*.
# `\"` is how a double quote reaches the emitted text; `'` needs no escape.
FORBIDDEN = [
    (re.compile(r"'\{"), "an apostrophe opening a literal around `{...}`"),
    (re.compile(r"\}'"), "an apostrophe closing a literal after `{...}`"),
    (re.compile(r'\\"\{'), "a double quote opening a literal around `{...}`"),
    (re.compile(r'\}\\"'), "a double quote closing a literal after `{...}`"),
]

# A string literal enclosed by one of these is prose, not output.
DIAGNOSTIC_CALLS = ("error", "refuse")
DIAGNOSTIC_TYPES = ("CodegenError",)

IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")


class Unreadable(Exception):
    """The scanner lost track of the file and will not guess."""


def string_literals(text):
    """Yield `(line, content, diagnostic)` for every string literal.

    `content` is the literal's bytes as written in the source, escapes
    included. `diagnostic` is true when the literal is somewhere inside a
    call to `error(...)` or inside a `CodegenError { ... }`.
    """
    i, line, n = 0, 1, len(text)
    # One entry per open delimiter; true when it introduced a diagnostic.
    stack = []
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            i = text.find("\n", i)
            if i == -1:
                return
        elif text.startswith("/*", i):
            end = text.find("*/", i + 2)
            if end == -1:
                raise Unreadable(f"unterminated block comment at line {line}")
            line += text.count("\n", i, end)
            i = end + 2
        elif c == "r" and (m := re.match(r'r(#*)"', text[i:])):
            hashes = m.group(1)
            close = '"' + hashes
            end = text.find(close, i + len(m.group(0)))
            if end == -1:
                raise Unreadable(f"unterminated raw string at line {line}")
            start = line
            body = text[i + len(m.group(0)) : end]
            yield start, body, any(stack)
            line += text.count("\n", i, end)
            i = end + len(close)
        elif c == '"':
            start, j, body = line, i + 1, []
            while True:
                if j >= n:
                    raise Unreadable(f"unterminated string at line {start}")
                if text[j] == "\\":
                    body.append(text[j : j + 2])
                    j += 2
                    continue
                if text[j] == '"':
                    break
                if text[j] == "\n":
                    line += 1
                body.append(text[j])
                j += 1
            yield start, "".join(body), any(stack)
            i = j + 1
        elif c == "'":
            # A char literal, or a lifetime. Neither can contain a quote we
            # care about, so both are skipped whole.
            m = re.match(r"'(\\.|[^\\'])'", text[i:])
            i += len(m.group(0)) if m else 1
        elif c in "({[":
            before = text[:i].rstrip()
            name = IDENT.search(before)
            word = name.group(0) if name else ""
            introduces = (c == "(" and word in DIAGNOSTIC_CALLS) or (
                c == "{" and word in DIAGNOSTIC_TYPES
            )
            stack.append(introduces)
            i += 1
        elif c in ")}]":
            if not stack:
                raise Unreadable(f"unbalanced `{c}` at line {line}")
            stack.pop()
            i += 1
        else:
            i += 1
    if stack:
        raise Unreadable("file ended with unbalanced delimiters")


def violations(text):
    """Every forbidden adjacency in an output-bound string literal."""
    found = []
    for line, body, diagnostic in string_literals(text):
        if diagnostic:
            continue
        probe = body.replace("{{", "\x00\x00").replace("}}", "\x01\x01")
        for pattern, what in FORBIDDEN:
            if pattern.search(probe):
                found.append((line, what, body[:90]))
    return found


# --- guard 1: the scanner catches what it claims to, and only that ---------
CANARY = """
fn emit(&mut self) -> String {
    let a = format!("on({target}, '{event}', {handler});");
    let b = format!("() => '{base} ' + x");
    let c = format!("{{\\"{name}\\":1}}");
    // format!("commented('{name}')") is not code
    let d = format!("{pad}bindAttr({target}, {name}, {getter});");
    self.error(format!("`{name}` is not `\\"{key}\\"` here."), span);
    self.errors.push(CodegenError {
        message: format!("`{}` names `\\"{export}\\"`.", name),
        span,
    });
    a
}
"""
caught = {what for _, what, _ in violations(CANARY)}
missing = [what for _, what in FORBIDDEN if what not in caught]
if missing:
    print("::error::the scanner failed its own canary; it would pass vacuously")
    for what in missing:
        print(f"::error::never fired: {what}")
    sys.exit(1)
flagged = {body for _, _, body in violations(CANARY)}
for must_not in ("commented(", "is not `", "names `"):
    if any(must_not in body for body in flagged):
        print(f"::error::the scanner flags {must_not!r}; it would be unusable")
        sys.exit(1)
lines = {line for line, _, _ in violations(CANARY)}
if len(lines) != 3:
    print(f"::error::canary flagged {len(lines)} lines, expected exactly 3")
    sys.exit(1)
try:
    list(string_literals('fn f() { let s = "unterminated;\n'))
except Unreadable:
    pass
else:
    print("::error::the scanner accepts a file it cannot parse")
    sys.exit(1)

# --- guard 2: the file list is the whole directory --------------------------
tracked = sorted(
    line.strip()
    for line in subprocess.run(
        ["git", "ls-files", f"{EMITTER_DIR}/*.rs"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    if line.strip()
)
on_disk = sorted(str(p) for p in pathlib.Path(EMITTER_DIR).glob("*.rs"))
if not tracked:
    print(f"::error::no emitter sources found under {EMITTER_DIR}")
    sys.exit(1)
if tracked != on_disk:
    print("::error::tracked emitter sources differ from what is on disk")
    print(f"::error::tracked: {tracked}")
    print(f"::error::on disk: {on_disk}")
    sys.exit(1)
if f"{EMITTER_DIR}/{EXEMPT}" not in tracked:
    print(f"::error::the exempt file {EMITTER_DIR}/{EXEMPT} does not exist")
    sys.exit(1)

# --- guard 3, and the check itself ------------------------------------------
failed = 0
scanned = 0
for path in tracked:
    if path.endswith(f"/{EXEMPT}"):
        continue
    scanned += 1
    try:
        found = violations(pathlib.Path(path).read_text())
    except Unreadable as why:
        print(f"::error file={path}::could not analyse this file: {why}")
        failed = 1
        continue
    for line, what, body in found:
        print(f"::error file={path},line={line}::{what}: {body}")
        print(
            f"::error file={path},line={line}::"
            "use js::string or js::json_string, which own both the quotes and the escaping"
        )
        failed = 1

if scanned + 1 != len(tracked):
    print(f"::error::scanned {scanned} of {len(tracked)} emitter sources")
    failed = 1

print(f"emitted strings: {scanned} emitter sources scanned, {EXEMPT} exempt")
sys.exit(failed)
PY
