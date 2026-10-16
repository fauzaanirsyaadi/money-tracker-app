#!/usr/bin/env python3
"""Fails when a type that holds money, names or notes derives `Debug`.

    check_debug_redaction.py [repo_root]

A derived `Debug` prints every field, so a stray `{:?}` or a failed assertion in
a log could leak a balance or a note. Such types use `mt_core::redacted_debug!`
(or a hand-written `Debug`) instead. A type that merely *looks* sensitive can be
exempted with a `// debug-safe: <reason>` comment on one of the three lines
above its `derive`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

SKIP_DIRS = {"target", "node_modules", ".git", "build", "gen", "tests"}

SENSITIVE_FIELD = re.compile(
    r"\b(amount\w*|\w*_minor|balance\w*|limit\w*|opening\w*|note|notes|name|spent\w*|"
    r"total\w*|income|expense|pin_hash|passphrase|secret\w*|Money)\b"
)
DERIVE = re.compile(r"#\[derive\(([^)]*)\)\]")
ITEM = re.compile(r"\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([a-z]+\))?\s+)?(struct|enum)\s+(\w+)")


def body_after(text: str, start: int) -> str:
    """The `{...}` or `(...)` body following `start`, with nesting respected."""
    opener = {"{": "}", "(": ")"}
    i = start
    while i < len(text) and text[i] not in "{(;":
        i += 1
    if i >= len(text) or text[i] == ";":
        return ""
    close = opener[text[i]]
    depth = 0
    for j in range(i, len(text)):
        if text[j] == text[i]:
            depth += 1
        elif text[j] == close:
            depth -= 1
            if depth == 0:
                return text[i : j + 1]
    return text[i:]


def strip_comments(body: str) -> str:
    return re.sub(r"//[^\n]*", "", body)


def payloads(enum_body: str) -> str:
    """What variants carry: only the `(...)` / `{...}` groups, never their names."""
    inner = enum_body[1:-1]
    found: list[str] = []
    depth = 0
    start = 0
    for index, char in enumerate(inner):
        if char in "({":
            if depth == 0:
                start = index
            depth += 1
        elif char in ")}":
            depth -= 1
            if depth == 0:
                found.append(inner[start : index + 1])
    return " ".join(found)


def check_text(text: str, label: str) -> list[str]:
    problems: list[str] = []
    lines = text.split("\n")
    for match in DERIVE.finditer(text):
        derives = [d.strip() for d in match.group(1).split(",")]
        if "Debug" not in derives:
            continue
        item = ITEM.match(text, match.end())
        if not item:
            continue
        line_no = text.count("\n", 0, match.start())
        context = "\n".join(lines[max(0, line_no - 3) : line_no + 1])
        if "debug-safe" in context:
            continue
        body = strip_comments(body_after(text, item.end()))
        if item.group(1) == "enum":
            body = payloads(body) if body else ""
        if SENSITIVE_FIELD.search(body):
            problems.append(
                f"DEBUG-REDACTION FAIL: {label}:{line_no + 1}: `{item.group(2)}` derives Debug "
                "but has fields that may hold money, names or notes; use mt_core::redacted_debug! "
                "or add `// debug-safe: <reason>`"
            )
    return problems


def check_repo(repo: Path) -> list[str]:
    problems: list[str] = []
    for path in sorted(repo.rglob("*.rs")):
        if any(part in SKIP_DIRS for part in path.relative_to(repo).parts):
            continue
        problems.extend(check_text(path.read_text(), str(path.relative_to(repo))))
    return problems


def main(argv: list[str]) -> int:
    repo = Path(argv[1]) if len(argv) > 1 else Path(__file__).resolve().parent.parent
    problems = check_repo(repo)
    for problem in problems:
        print(problem)
    if not problems:
        print("debug redaction guard: ok")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
