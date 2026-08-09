#!/usr/bin/env python3
"""Assert no diagnostic message exceeds the inline budget.

`zdc-diagnostics` budgets what a diagnostic says inline: one message of
at most `INLINE_MESSAGE_BUDGET` characters, its spans, and one help line
that is always the pointer to `zdc explain`.  Barik et al. (ICSE 2017,
n = 56, eye tracking) measured that reading an error message is about as
hard as reading source code and that reading difficulty predicts task
time, so length is a cost the reader pays and the "why" belongs behind
`zdc explain`, where it costs nothing to the reader who does not want it.

`crates/zdc-diagnostics/tests/inline_budget.rs` already holds that line —
but only for the messages a fixture can *provoke*.  It is a corpus, and a
corpus reaches the codes the grammar can reach.  Every diagnostic outside
it — the type errors, the routing errors, the codegen refusals, the
resolver's messages, and the graph findings whose trigger has no syntax
yet — was budgeted by nobody, and that is where the paragraphs are.

This is the other half: a static scan of every message the compiler can
construct, whether or not a test can make it appear.  The two checks
measure different things on purpose.  The corpus measures the message a
reader actually sees, interpolations included.  This measures the message
*as written*, which is a lower bound on it: `{name}` counts as six
characters and expands to however long the program's name is.  A message
that passes here can still be too long at run time, which is what the
corpus is for; a message that fails here is too long however it is
filled in.

Two shapes are read, because the compiler has two ways of building a
diagnostic:

  * a call whose message is a known argument — `GraphError::new(code,
    message, span)`, `ParseError::new(code, message, span)`,
    `self.error(message, span)`, `self.error_with_help(message, span,
    help)`;
  * a struct literal with a `message:` field — `TypeError { message: …,
    … }`, `CodegenError { … }`, `ResolveError { … }`, `LexError { … }`.

Nothing else is measured.  A `help:`, a `.labelled(…)` and a
`.with_help(…)` are deliberately outside the budget: help is one
generated line for a coded diagnostic, and a caret label is words beside
an underline rather than a paragraph.

**The waiver list is how this holds the line without rewriting the
compiler's prose in the same commit.**  Every message that is over budget
today is listed below with the reason, so a *new* message over the budget
fails while the existing ones are recorded rather than hidden.  A waiver
whose message no longer trips the rule is an error, so the list cannot
outlive its cause, and it can only ever shrink: adding to it is a
decision somebody has to write down and defend in review.

Anti-vacuity, because gates in this repository have previously passed
while inspecting nothing:

  1. the scanner is run against a canary containing one over-budget
     message in each supported shape, and one under-budget message in
     each, and the check fails unless every one of them is classified
     correctly;
  2. the file list comes from `git ls-files` rather than a glob, and the
     scan must find at least as many messages as there are crates that
     produce them;
  3. a file the scanner cannot parse to a clean end state is an error
     rather than a file with no findings.
"""

import pathlib
import re
import subprocess
import sys

# Read from the crate rather than restated here, so the gate and the
# tests cannot come to disagree about the number.
BUDGET_DECLARATION = pathlib.Path("crates/zdc-diagnostics/src/explain.rs")
BUDGET_PATTERN = re.compile(r"pub const INLINE_MESSAGE_BUDGET: usize = (\d+);")

# `(head, index)`: in a call to `head`, the argument at `index` is the
# message.  Zero-based.
MESSAGE_ARGUMENTS = {
    "new": 1,  # GraphError::new, ParseError::new
    "warning": 1,  # GraphError::warning
    "error": 0,  # Checker::error
    "error_with_help": 0,  # Checker::error_with_help
}

# The struct field that holds a message, and the types it means something
# on.  Restricted to the diagnostic types on purpose: `let message: String`
# in an unrelated function would otherwise put every string literal after
# it into a message position.
MESSAGE_FIELD = "message"
DIAGNOSTIC_STRUCTS = frozenset(
    {
        "ParseError",
        "ResolveError",
        "TypeError",
        "GraphError",
        "CodegenError",
        "LexError",
        "Diagnostic",
    }
)

# Calls that wrap a message without being one.  A message is written
# `format!("…", name)`, `"…".to_string()` or `Some("…")` about as often as
# it is written bare, and the budget is about the text either way.  Only
# the first argument is transparent: `format!("{a}", b)`'s second argument
# is a value, not the message.
TRANSPARENT = frozenset(
    {"format", "Some", "from", "concat", "into", "to_string", "to_owned"}
)

# Messages over budget on the day this gate was written.  The list may
# shrink and may not grow: adding to it is a decision somebody has to
# write down and defend in review, and a waiver whose message no longer
# trips the rule is an error below.
#
# Keyed by the first sixty characters of the message as written, which is
# enough to identify one and short enough that rewording the tail of a
# message does not silently drop its waiver.
#
# **They all have the same cause, and it is not laziness.**  The budget
# works by moving the "why" out of the message and behind `zdc explain
# <CODE>` — which needs a code.  Every message here belongs to a
# diagnostic that has none: `CodegenError` and `Resolver::error` set
# `code: None` by construction, and `TypeError` has no code field at all
# (#148).  Shortening one of these today would not relocate the rule it
# states, it would delete it, and the reader would be left with a claim
# and nowhere to look it up.  So the honest order is codes first,
# explanations second, shortening third; this gate is what stops the pile
# growing while that happens.
WAIVED: dict[str, str] = {
    # `zdc-codegen`: `CodegenError` carries no code.
    "`Link` takes a route value written where the link is, as in ": "codegen refusal, uncoded",
    "`{}` has no `{name}` argument. It takes {}. The set is close": "codegen refusal, uncoded",
    "`{name}` must be written down. It becomes a rule of its own ": "codegen refusal, uncoded",
    "`{name}` must be written down. Its value is translated into ": "codegen refusal, uncoded",
    "`{}` may not be styled `{name} is x{written}x`. A `{name}` i": "codegen refusal, uncoded",
    "`{}` is what a `PasswordInput` binds, so it can be typed and": "codegen refusal, uncoded",
    # `zdc-resolve`: `Resolver::error` sets `code: None`.
    "This view nests more than {MAX_NODE_DEPTH} levels deep once ": "resolver error, uncoded",
    "This view expands to more than {INSTANCE_BUDGET} component i": "resolver error, uncoded",
    "`{name}` is written to inside this component, so the value p": "resolver error, uncoded",
    "`use x{}x` names a file that {}. A module is read from insid": "resolver error, uncoded",
    "`{}` is `{}` and gives a view. A foreign that gives a view o": "resolver error, uncoded",
    "`{}` is declared `trusted` inside the component `{}`. State ": "resolver error, uncoded",
    "`{name}` places `children` twice. The nodes nested at a call": "resolver error, uncoded",
    "`{DESTINATION_ELEMENT}` takes where it goes as its first arg": "resolver error, uncoded",
    # `zdc-types`: `TypeError` has no code field yet (#148).
    "`{name}` has a parameter that is not enumerable, and `{}` re": "routing type error, uncoded",
    "This program serves `{url}` from the route `{route}`, so the": "routing type error, uncoded",
    "`{}` is `secret`, and a route parameter enumerated over it w": "routing type error, uncoded",
    "`{}` holds `{segment}`, which is not a URL path segment. A r": "routing type error, uncoded",
    "`{}` is initialised from `address`, and a signal initialised": "routing type error, uncoded",
    # Landed after this gate was written, and defended here rather than
    # waved through: the module resolver's refusals (#238) and the number
    # field's (#45) have the same cause as everything above them —
    # `modules.rs` and `CodegenError` both construct with `code: None`, so
    # the "why" has nowhere to move to. They are recorded rather than
    # shortened for the reason the list exists.
    "`{}` imports from `{}`, and {reason} Write a path relative t": "module resolver, uncoded",
    "`{}` imports from `{}`, which names a file that {refusal}. A": "module resolver, uncoded",
    "`{}` imports from `{}`, and a bare specifier names a package": "module resolver, uncoded",
    "`{}` imports from `{}`, and `{manifest}` maps `{}` twice — t": "module resolver, uncoded",
    "`{}` binds an `Option of Whole` or an `Option of Decimal`, a": "codegen refusal, uncoded",
}


class Unreadable(Exception):
    """The scanner lost track of the file and will not guess."""


def budget() -> int:
    text = BUDGET_DECLARATION.read_text()
    found = BUDGET_PATTERN.search(text)
    if not found:
        raise Unreadable(f"no INLINE_MESSAGE_BUDGET in {BUDGET_DECLARATION}")
    return int(found.group(1))


def joined(literal: str) -> str:
    r"""A Rust string literal as the characters it denotes.

    Only the escapes that change the *length* matter here.  A `\` before
    a newline swallows the newline and the following line's indentation,
    which is how nearly every message in this compiler is written; every
    other escape stands for one character.
    """
    without_continuations = re.sub(r"\\\n[ \t]*", "", literal)
    return re.sub(r"\\.", "x", without_continuations)


def messages(text: str):
    """Yield `(line, message)` for every diagnostic message in `text`.

    The walk understands comments, string literals, char literals and
    lifetimes, and tracks one frame per open delimiter so that the
    argument index and the field name are the ones for the *innermost*
    construct.
    """
    identifier = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")
    # A name is at most a few dozen characters, so the walk looks back a
    # bounded distance rather than at the whole prefix.  Unbounded, the
    # scan is quadratic in file size and does not finish on this
    # workspace's larger sources.
    LOOKBACK = 96
    i, line, n = 0, 1, len(text)
    # One frame per open delimiter: [head, argument index, field name].
    stack: list[list] = []

    def in_message_position() -> bool:
        at = len(stack) - 1
        while at >= 0 and stack[at][0] in TRANSPARENT and stack[at][1] == 0:
            at -= 1
        if at < 0:
            return False
        head, argument, field = stack[at]
        if field == MESSAGE_FIELD and head in DIAGNOSTIC_STRUCTS:
            return True
        return MESSAGE_ARGUMENTS.get(head) == argument

    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            end = text.find("\n", i)
            if end == -1:
                return
            i = end
        elif text.startswith("/*", i):
            end = text.find("*/", i + 2)
            if end == -1:
                raise Unreadable(f"unterminated block comment at line {line}")
            line += text.count("\n", i, end)
            i = end + 2
        elif c == "r" and (opener := re.match(r'r(#*)"', text[i:])):
            close = '"' + opener.group(1)
            end = text.find(close, i + len(opener.group(0)))
            if end == -1:
                raise Unreadable(f"unterminated raw string at line {line}")
            body = text[i + len(opener.group(0)) : end]
            if in_message_position():
                yield line, body
            line += text.count("\n", i, end)
            i = end + len(close)
        elif c == '"':
            start, j, body = line, i + 1, []
            while True:
                if j >= n:
                    raise Unreadable(f"unterminated string at line {start}")
                if text[j] == "\\":
                    body.append(text[j : j + 2])
                    if text[j + 1 : j + 2] == "\n":
                        line += 1
                    j += 2
                    continue
                if text[j] == '"':
                    break
                if text[j] == "\n":
                    line += 1
                body.append(text[j])
                j += 1
            if in_message_position():
                yield start, joined("".join(body))
            i = j + 1
        elif c == "'":
            char = re.match(r"'(\\.|[^\\'])'", text[i:])
            i += len(char.group(0)) if char else 1
        elif c in "({[":
            before = text[max(0, i - LOOKBACK) : i].rstrip()
            # A macro invocation is `format!(`, and the `!` is not part of
            # the name.  Missing this is how `format!` stopped counting as
            # a wrapper and every interpolated message went unmeasured.
            if before.endswith("!"):
                before = before[:-1]
            name = identifier.search(before)
            stack.append([name.group(0) if name else "", 0, None])
            i += 1
        elif c in ")}]":
            if not stack:
                raise Unreadable(f"unbalanced `{c}` at line {line}")
            stack.pop()
            i += 1
        elif c == ",":
            if stack:
                stack[-1][1] += 1
                stack[-1][2] = None
            i += 1
        elif c == ":" and not text.startswith("::", i) and text[i - 1 : i] != ":":
            if stack:
                name = identifier.search(text[max(0, i - LOOKBACK) : i])
                stack[-1][2] = name.group(0) if name else None
            i += 1
        else:
            i += 1
    if stack:
        raise Unreadable("file ended with unbalanced delimiters")


def over_budget(text: str, limit: int):
    """Every message in `text` longer than `limit`."""
    return [
        (line, message)
        for line, message in messages(text)
        if len(message) > limit
    ]


def canary_check(limit: int) -> int:
    """Guard 1: the scanner classifies every supported shape correctly."""
    long = "L" * (limit + 1)
    short = "S" * 10
    source = f"""
fn build(&mut self) {{
    self.errors.push(GraphError::new("E0301", "{long}", span));
    self.errors.push(GraphError::warning("W0330", "{short}", span));
    self.error(format!("{long}"), span);
    self.error_with_help("{short}".to_string(), span, "{long}".to_string());
    let a = TypeError {{
        message: format!("{long}"),
        span,
        help: Some("{long}".to_string()),
    }};
    let b = CodegenError {{ message: "{short}".to_string(), span }};
    let c = ParseError::new(codes::PLACEMENT, "{short}", span)
        .labelled("{long}");
    // "{long}" in a comment is not a message
    let d = js::string("{long}");
}}
"""
    found = over_budget(source, limit)
    if len(found) != 3:
        print(
            f"::error::the canary flagged {len(found)} messages, expected exactly 3"
        )
        for line, message in found:
            print(f"::error::line {line}: {message[:40]}")
        return 1

    every = [message for _, message in messages(source)]
    if len(every) != 7:
        print(f"::error::the canary found {len(every)} messages, expected 7")
        for message in every:
            print(f"::error::  read: {message[:30]}")
        return 1
    if not all(message[0] in "LS" for message in every):
        print("::error::the canary read something that is not one of its messages")
        return 1

    # A file the scanner cannot understand must be an error, not a file
    # with no findings.
    try:
        list(messages('fn f() { let s = "unterminated;\n'))
    except Unreadable:
        pass
    else:
        print("::error::the scanner accepts a file it cannot parse")
        return 1
    return 0


def main() -> int:
    limit = budget()

    if canary_check(limit):
        return 1

    tracked = sorted(
        path
        for path in subprocess.run(
            ["git", "ls-files", "crates/*/src/*.rs"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.split()
        if path
    )
    if len(tracked) < 40:
        print(f"::error::the file list found only {len(tracked)} sources")
        return 1

    failed = 0
    counted = 0
    seen_waivers: set[str] = set()
    for path in tracked:
        try:
            found = list(messages(pathlib.Path(path).read_text()))
        except Unreadable as why:
            print(f"::error file={path}::could not analyse this file: {why}")
            failed = 1
            continue
        counted += len(found)
        for line, message in found:
            if len(message) <= limit:
                continue
            key = message[:60]
            if key in WAIVED:
                seen_waivers.add(key)
                continue
            print(
                f"::error file={path},line={line}::this diagnostic message is "
                f"{len(message)} characters as written, over the inline budget "
                f"of {limit}: {message[:80]}…"
            )
            print(
                f"::error file={path},line={line}::the claim goes inline and the "
                "rule goes in `zdc-diagnostics`'s `explain` module, behind "
                "`zdc explain <CODE>`"
            )
            failed = 1

    # Guard 2: a scan that stopped working reports no findings, which is
    # indistinguishable from a compiler with no diagnostics.
    if counted < 100:
        print(
            f"::error::the scan found only {counted} diagnostic messages, which "
            "means it stopped reading them rather than that the compiler "
            "stopped emitting them"
        )
        return 1

    stale = sorted(set(WAIVED) - seen_waivers)
    if stale:
        print("::error::these waivers no longer describe an over-budget message")
        for key in stale:
            print(f"::error::  {key}…")
        print("::error::delete them: a waiver may not outlive its cause")
        failed = 1

    print(
        f"message budget: {counted} diagnostic messages measured against "
        f"{limit} characters, {len(WAIVED)} waived"
    )
    return failed


if __name__ == "__main__":
    sys.exit(main())
