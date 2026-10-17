#!/usr/bin/env python3
"""Fails when a type that holds money, names or notes derives `Debug`.

    check_debug_redaction.py [repo_root]

Also checked: `#[cfg_attr(..., derive(Debug))]` and hand-written `impl Debug`
(which must say it redacts). A derived `Debug` prints every field, so a stray `{:?}` or a failed assertion in
a log could leak a balance or a note. Such types use `mt_core::redacted_debug!`
(or a hand-written `Debug`) instead. A type that merely *looks* sensitive can be
exempted by an entry (with a reason) in `scripts/guard_exceptions.json`. Inline
`// debug-safe:` comments are not honoured; see `scripts/guard_common.py`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import guard_common

SKIP_DIRS = {"target", "node_modules", ".git", "build", "gen", "tests"}

# Whole words (snake_case and camelCase are split) that mark secret material.
SENSITIVE_WORDS = {
    "key", "keys", "pin", "hash", "blob", "salt", "token", "secret", "passphrase",
    "password", "passwd", "nonce",
}
SENSITIVE_TYPES = re.compile(r"\b(DbKey|Zeroizing|SecretKey|SecretString)\b")

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


IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
CAMEL = re.compile(r"[A-Z]?[a-z0-9]+|[A-Z]+(?![a-z])")


def words_of(identifier: str) -> set[str]:
    """`pin_hash` -> {pin, hash}; `wrappedKey` -> {wrapped, key}."""
    found: set[str] = set()
    for part in identifier.split("_"):
        found.update(word.lower() for word in CAMEL.findall(part))
    return found


def is_sensitive(body: str) -> bool:
    if SENSITIVE_FIELD.search(body) or SENSITIVE_TYPES.search(body):
        return True
    return any(words_of(name) & SENSITIVE_WORDS for name in IDENTIFIER.findall(body))


CFG_ATTR_DERIVE = re.compile(r"#\[cfg_attr\((?:[^\]]*?)derive\(([^)]*)\)[^\]]*\)\]")
MANUAL_DEBUG = re.compile(
    r"impl(?:<[^>]*>)?\s+(?:::?)?(?:(?:std|core)::)?(?:fmt::)?Debug\s+for\s+(\w+)"
)
DEFINITION = r"(?:pub(?:\([a-z]+\))?\s+)?(?:struct|enum)\s+{name}\b"


def type_body(text: str, name: str) -> str | None:
    match = re.search(DEFINITION.format(name=re.escape(name)), text)
    if not match:
        return None
    body = strip_comments(body_after(text, match.end()))
    # An enum is judged by what its variants carry.
    return payloads(body) if "enum" in match.group(0) and body else body


def check_text(
    text: str, label: str, exceptions: guard_common.Exceptions | None = None
) -> list[str]:
    exceptions = exceptions or guard_common.Exceptions()
    problems: list[str] = guard_common.inline_marker_problems(text, label)
    derive_matches = list(DERIVE.finditer(text)) + list(CFG_ATTR_DERIVE.finditer(text))
    for match in derive_matches:
        derives = [d.strip() for d in match.group(1).split(",")]
        if "Debug" not in derives:
            continue
        item = ITEM.match(text, match.end())
        if not item:
            continue
        line_no = text.count("\n", 0, match.start())
        if exceptions.allows("debug_redaction", file=label, type=item.group(2)):
            continue
        body = strip_comments(body_after(text, item.end()))
        if item.group(1) == "enum":
            body = payloads(body) if body else ""
        if is_sensitive(body):
            problems.append(
                f"DEBUG-REDACTION FAIL: {label}:{line_no + 1}: `{item.group(2)}` derives Debug "
                "but has fields that may hold money, names or notes; use mt_core::redacted_debug! "
                "or list the type with a reason in scripts/guard_exceptions.json"
            )
    # Hand-written Debug on a type with sensitive fields must say it redacts.
    for impl in MANUAL_DEBUG.finditer(text):
        name = impl.group(1)
        line_no = text.count("\n", 0, impl.start())
        fields = type_body(text, name)
        if fields is None or not is_sensitive(fields):
            continue
        if exceptions.allows("debug_redaction", file=label, type=name):
            continue
        body = body_after(text, impl.end())
        if "redacted" not in body.lower():
            problems.append(
                f"DEBUG-REDACTION FAIL: {label}:{line_no + 1}: hand-written Debug for `{name}` "
                "prints a type with sensitive fields and does not redact them"
            )
    return problems


def check_repo(repo: Path) -> list[str]:
    exceptions = guard_common.load(repo)
    problems: list[str] = list(exceptions.problems)
    for path in sorted(repo.rglob("*.rs")):
        if any(part in SKIP_DIRS for part in path.relative_to(repo).parts):
            continue
        problems.extend(check_text(path.read_text(), str(path.relative_to(repo)), exceptions))
    problems.extend(exceptions.unused(("debug_redaction",)))
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
