#!/usr/bin/env python3
"""Supply-chain guard for the repository's CI definition.

    check_workflows.py [repo_root]

* Every `uses:` in `.github/workflows/*.yml` must be pinned to a full 40-hex
  commit SHA (a tag or branch can be moved by whoever controls the action's
  repository). Local actions (`./...`) are exempt. Docker images need a digest.
* No `pull_request_target` or `workflow_run` trigger (they run with secrets on
  untrusted code) and every workflow declares `permissions:` at the top level.
* No `curl | sh` style installs in `run:` steps.
* No compiled Python (`*.pyc`), signing key (`*.jks`, `*.keystore`, `*.p12`) or package
  (`*.apk`, `*.aab`) is tracked by git: a tracked `.pyc` is loaded in place of its
  `.py` by the guards themselves, so it could silently neuter one.
* `.github/CODEOWNERS` exists with a rule of its own for each file that defines what CI
  trusts (workflows, composite actions, guard scripts, the exceptions list), so that
  changing them needs the owner's review. A placeholder owner is a warning locally and
  a failure in CI (`CI=true`).
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

SHA = re.compile(r"^[0-9a-f]{40}$")
# `uses` as a YAML key in any spelling: plain, quoted, space before the colon, first in
# a flow mapping (`- {uses: x}`) or after a comma. The value may be empty (it is then
# on the next line).
USES = re.compile(r"""(?:^|[\s{,])["']?uses["']?\s*:\s*(?P<value>[^\s,}#][^,}#]*)?""")
IMAGE = re.compile(r"""(?:^|[\s{,])["']?(?:image|container)["']?\s*:\s*["']?(?P<value>[^\s,}#"']+)""")
DANGEROUS_TRIGGER = re.compile(r"\b(pull_request_target|workflow_run)\b")
# Untrusted text that must never be pasted into a script: it is attacker-controlled
# (PR titles, branch names, issue text, workflow inputs).
UNTRUSTED = re.compile(r"\$\{\{[^}]*\b(github\.event\.|github\.head_ref|inputs\.|github\.actor\b)[^}]*\}\}")
PIPE_TO_SHELL = re.compile(
    r"(curl|wget)\b[^\n|]*\|\s*(sudo\s+)?(ba|z|da|k)?sh\b"
    r"|(curl|wget)\b[^\n|]*\|\s*(sudo\s+)?(python3?|node|perl|ruby)\b"
    r"|\b(ba|z)?sh\s+<\(\s*(curl|wget)\b"
    r"|\b(ba|z)?sh\s+-c\s+[\"']?\$\(\s*(curl|wget)\b"
)
CODEOWNERS_REQUIRED = (
    ".github/workflows/",
    ".github/actions/",
    "scripts/",
    "scripts/guard_exceptions.json",
    ".github/CODEOWNERS",
    "deny.toml",
)


def fail(problems: list[str], message: str) -> None:
    problems.append(f"WORKFLOW-GUARD FAIL: {message}")


def _indent(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def _check_ref(problems: list[str], label: str, number: int, ref: str) -> None:
    ref = ref.strip().strip("\"'")
    if not ref or ref.startswith("${{"):
        return
    if ref.startswith("./"):
        return  # a local action: its own file is scanned too
    if ref.startswith("docker://"):
        if "@sha256:" not in ref:
            fail(problems, f"{label}:{number}: docker image {ref!r} must be pinned by digest")
        return
    _, _, version = ref.partition("@")
    if not SHA.match(version):
        fail(problems, f"{label}:{number}: {ref!r} must be pinned to a full commit SHA, not {version or 'nothing'!r}")


def check_workflow(text: str, label: str) -> list[str]:
    problems: list[str] = []
    lines = text.split("\n")
    code = [line.split(" #", 1)[0] if not line.lstrip().startswith("#") else "" for line in lines]

    for index, line in enumerate(code):
        number = index + 1
        for match in USES.finditer(line):
            value = match.group("value")
            if value is None:  # the value is on the next non-empty line
                follow = next((c for c in code[index + 1 :] if c.strip()), "")
                value = follow.strip()
            _check_ref(problems, label, number, value)
        image = IMAGE.search(line)
        if image and not image.group("value").startswith("${{") and "@sha256:" not in image.group("value"):
            fail(problems, f"{label}:{number}: container image {image.group('value')!r} must be pinned by digest")
        if DANGEROUS_TRIGGER.search(line):
            fail(problems, f"{label}:{number}: pull_request_target / workflow_run triggers are not allowed")

    # Commands: a backslash continues a line, so join before looking for `curl | sh`.
    joined, pending = [], ""
    for line in code:
        if line.rstrip().endswith("\\"):
            pending += line.rstrip()[:-1] + " "
            continue
        joined.append((pending + line).strip())
        pending = ""
    for command in joined:
        if PIPE_TO_SHELL.search(command):
            fail(problems, f"{label}: do not pipe a download into a shell: {command[:70]}")

    # Permissions: declared at the top, and never write-all or a write scope.
    if not re.search(r"^permissions\s*:", text, re.MULTILINE):
        fail(problems, f"{label}: declare top-level `permissions:` (least privilege)")
    for index, line in enumerate(code):
        match = re.match(r"^(\s*)permissions\s*:\s*(\S.*)?$", line)
        if not match:
            continue
        value = (match.group(2) or "").strip()
        if value:
            if value in ("write-all", "write"):
                fail(problems, f"{label}:{index + 1}: permissions must not be {value}")
            continue
        block_indent = len(match.group(1))
        for follower in code[index + 1 :]:
            if follower.strip() and _indent(follower) <= block_indent:
                break
            if re.search(r":\s*write\b", follower):
                fail(problems, f"{label}: write permission granted: {follower.strip()}")

    # Script injection: untrusted expressions inside a `run:` script.
    index = 0
    while index < len(code):
        match = re.match(r"^(\s*(?:-\s+)?)run\s*:\s*(.*)$", code[index])
        if not match:
            index += 1
            continue
        # The key's own column (after any "- "): siblings such as `env:` sit at it.
        base, rest = len(match.group(1)), match.group(2)
        body = [rest]
        end = index + 1
        while end < len(code) and (not code[end].strip() or _indent(code[end]) > base):
            body.append(code[end])
            end += 1
        for offset, script_line in enumerate(body):
            if UNTRUSTED.search(script_line):
                fail(problems, f"{label}:{index + 1 + offset}: untrusted expression in a run script; pass it through env: instead")
        index = max(end, index + 1)
    return problems


def check_codeowners(text: str | None, in_ci: bool = False) -> tuple[list[str], list[str]]:
    """(problems, warnings). In CI a placeholder owner is a failure, not a warning."""
    problems: list[str] = []
    warnings: list[str] = []
    if text is None:
        return ([f"WORKFLOW-GUARD FAIL: .github/CODEOWNERS is missing"], warnings)
    rules: dict[str, list[str]] = {}
    for line in text.split("\n"):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        pattern, *owners = line.split()
        rules[pattern.lstrip("/")] = owners
    for path in CODEOWNERS_REQUIRED:
        # Each protected path needs its own rule: a catch-all `*` must not be able to
        # satisfy this check, because a later, more specific rule can override it.
        owners = rules.get(path) or rules.get(path.rstrip("/"))
        if not owners:
            fail(problems, f".github/CODEOWNERS has no rule of its own for {path}")
    for owners in rules.values():
        if any("REPLACE" in owner.upper() for owner in owners):
            message = (
                ".github/CODEOWNERS still has the placeholder owner; "
                "put your GitHub username there before pushing"
            )
            if in_ci:
                fail(problems, message)
            else:
                warnings.append(f"WORKFLOW-GUARD WARNING: {message}")
            break
    return problems, warnings


FORBIDDEN_TRACKED = re.compile(r"(^|/)__pycache__/|\.(pyc|pyo|jks|keystore|p12|pfx|apk|aab)$", re.IGNORECASE)


def forbidden_tracked(paths: list[str]) -> list[str]:
    return [
        f"WORKFLOW-GUARD FAIL: {path} is tracked by git; compiled Python, signing keys and packages must not be"
        for path in paths
        if FORBIDDEN_TRACKED.search(path)
    ]


def tracked_files(repo: Path) -> list[str] | None:
    """`git ls-files`, or None when this is not a git checkout (nothing to check)."""
    try:
        out = subprocess.run(
            ["git", "-C", str(repo), "ls-files", "-z"], capture_output=True, check=True
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    return [name for name in out.decode("utf-8", "replace").split("\0") if name]


def check_repo(repo: Path) -> tuple[list[str], list[str]]:
    problems: list[str] = []
    workflows = sorted((repo / ".github" / "workflows").glob("*.y*ml"))
    # Composite actions kept in the repository run with the workflow's privileges.
    workflows += sorted((repo / ".github" / "actions").rglob("action.y*ml"))
    if not workflows:
        fail(problems, "no workflow files found")
    for path in workflows:
        problems.extend(check_workflow(path.read_text(), str(path.relative_to(repo))))
    tracked = tracked_files(repo)
    if tracked is not None:
        problems.extend(forbidden_tracked(tracked))
    owners = repo / ".github" / "CODEOWNERS"
    more, warnings = check_codeowners(
        owners.read_text() if owners.exists() else None,
        in_ci=os.environ.get("CI", "").lower() == "true",
    )
    problems.extend(more)
    return problems, warnings


def main(argv: list[str]) -> int:
    repo = Path(argv[1]) if len(argv) > 1 else Path(__file__).resolve().parent.parent
    problems, warnings = check_repo(repo)
    for line in warnings + problems:
        print(line)
    if not problems:
        print("workflow guard: ok")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
