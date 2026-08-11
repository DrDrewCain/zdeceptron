#!/usr/bin/env python3
"""Assert the editor grammar does not highlight keywords the lexer rejects.

The VS Code TextMate grammar is a second copy of the keyword table, which is
duplication the language deliberately accepts (a regular expression cannot
model ZDeceptron's structure, so the grammar is intentionally token-only).
Duplication that is accepted still has to be checked, or it drifts: the
grammar once highlighted `record`, `choice`, `append`, and `remove`, none of
which the compiler recognises, so a program using them looked valid in the
editor and failed to parse.

`word_to_kind` in zdc-lexer is the single source of truth for English
spellings. This script fails if the grammar knows a word it does not.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Words that appear in the grammar as element or pattern names rather than as
# keywords. They are not in `word_to_kind` and correctly so.
NON_KEYWORD = {"text", "row", "column", "heading", "button", "input", "checkbox"}


def lexer_keywords() -> set[str]:
    raw = (ROOT / "crates/zdc-lexer/src/raw.rs").read_text()
    return set(re.findall(r'"([a-z]+)"\s*=>', raw))


def grammar_keywords() -> set[str]:
    grammar = (ROOT / "editors/vscode/syntaxes/zdeceptron.tmLanguage.json").read_text()
    found: set[str] = set()
    # Alternation groups: \\b(a|b|c)\\b — in the JSON source the backslashes
    # are themselves escaped, hence four in this pattern.
    for group in re.findall(r"\\\\b\(([a-z|]+)\)\\\\b", grammar):
        found |= set(group.split("|"))
    # Single-word capture groups: `(test)`, `(on)`, `(route)`. A rule that
    # highlights one word does it this way rather than with an alternation
    # of one, so the pattern above cannot see it — which meant seven of the
    # grammar's keywords were outside the gate that exists to check them,
    # and a `test` rule added in issue #169 was checked by nothing at all.
    # The named list below found five of the seven by having been written
    # down by hand, which is the drift this script exists to stop.
    found |= set(re.findall(r"\(([a-z]+)\)", grammar))
    # Keywords that appear outside any capture group at all.
    for word in ("secret", "view", "is"):
        if re.search(r"\b" + word + r"\b", grammar):
            found.add(word)
    return found - NON_KEYWORD


def main() -> int:
    lexer = lexer_keywords()
    grammar = grammar_keywords()

    unknown = sorted(grammar - lexer)
    if unknown:
        print(
            "::error::the editor grammar highlights keywords the lexer does "
            f"not recognise: {', '.join(unknown)}"
        )
        print("Either add them to word_to_kind or remove them from the grammar.")
        return 1

    print(
        f"grammar drift: {len(grammar)} highlighted keywords, "
        f"all present in word_to_kind ({len(lexer)} total)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
