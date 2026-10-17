"""The one place where a CI guard may be told to look away.

Every exception to a guard (a type exempted from the Debug-redaction rule, a
line exempted from the offline rule) lives in `scripts/guard_exceptions.json`,
with a reason, and nowhere else. Inline marker comments in source files are not
honoured: a comment can be slipped into any change unnoticed, whereas this file
is protected by CODEOWNERS (see `.github/CODEOWNERS`) so that every new
exception is a change someone has to approve.

An entry that no longer matches anything is itself a failure, so the list cannot
silently accumulate dead exemptions.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path

EXCEPTIONS_FILE = Path("scripts") / "guard_exceptions.json"

# guard name -> the keys an entry for that guard must have (besides "reason").
SCHEMA = {
    "debug_redaction": ("file", "type"),
    "offline_source": ("file", "contains"),
}

# Inline markers that used to exempt code. They are refused everywhere now.
INLINE_MARKERS = ("debug-safe:", "offline-check: allow")


@dataclass
class Exceptions:
    entries: dict[str, list[dict]] = field(default_factory=dict)
    used: set[tuple[str, int]] = field(default_factory=set)
    problems: list[str] = field(default_factory=list)

    def allows(self, guard: str, **where: str) -> bool:
        """Whether an entry for `guard` matches `where` (all of its keys equal / contained)."""
        for index, entry in enumerate(self.entries.get(guard, [])):
            if all(_matches(guard, key, entry.get(key, ""), where.get(key, "")) for key in SCHEMA[guard]):
                self.used.add((guard, index))
                return True
        return False

    def unused(self, guards: tuple[str, ...]) -> list[str]:
        out = []
        for guard in guards:
            for index, entry in enumerate(self.entries.get(guard, [])):
                if (guard, index) not in self.used:
                    out.append(
                        f"GUARD-EXCEPTIONS FAIL: {EXCEPTIONS_FILE}: unused {guard} exception "
                        f"{_describe(guard, entry)}; remove it"
                    )
        return out


def _matches(guard: str, key: str, expected: str, actual: str) -> bool:
    if guard == "offline_source" and key == "contains":
        return bool(expected) and expected in actual
    return expected == actual


def _describe(guard: str, entry: dict) -> str:
    return ", ".join(f"{key}={entry.get(key)!r}" for key in SCHEMA[guard])


def load(repo: Path) -> Exceptions:
    """Reads and validates the exceptions file. A missing file means no exceptions."""
    result = Exceptions()
    path = repo / EXCEPTIONS_FILE
    if not path.exists():
        return result
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError) as err:
        result.problems.append(f"GUARD-EXCEPTIONS FAIL: {EXCEPTIONS_FILE}: unreadable ({err})")
        return result
    if not isinstance(data, dict):
        result.problems.append(f"GUARD-EXCEPTIONS FAIL: {EXCEPTIONS_FILE}: must be a JSON object")
        return result
    for guard, entries in data.items():
        if guard == "comment":
            continue
        if guard not in SCHEMA:
            result.problems.append(f"GUARD-EXCEPTIONS FAIL: {EXCEPTIONS_FILE}: unknown guard {guard!r}")
            continue
        if not isinstance(entries, list):
            result.problems.append(f"GUARD-EXCEPTIONS FAIL: {EXCEPTIONS_FILE}: {guard} must be a list")
            continue
        for entry in entries:
            keys = SCHEMA[guard] + ("reason",)
            if not isinstance(entry, dict) or any(
                not isinstance(entry.get(key), str) or not entry[key].strip() for key in keys
            ):
                result.problems.append(
                    f"GUARD-EXCEPTIONS FAIL: {EXCEPTIONS_FILE}: every {guard} entry needs non-empty "
                    f"{', '.join(keys)}: {entry!r}"
                )
                continue
            result.entries.setdefault(guard, []).append(entry)
    return result


def inline_marker_problems(text: str, label: str) -> list[str]:
    """Inline exemption comments are refused: the exception must be listed centrally."""
    found = []
    for number, line in enumerate(text.split("\n"), 1):
        for marker in INLINE_MARKERS:
            if marker in line:
                found.append(
                    f"GUARD-EXCEPTIONS FAIL: {label}:{number}: inline exemption {marker!r} is not "
                    f"honoured; list it with a reason in {EXCEPTIONS_FILE}"
                )
    return found
