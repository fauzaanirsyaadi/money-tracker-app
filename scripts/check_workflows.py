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
* `.github/CODEOWNERS` exists and covers the files that define what CI trusts:
  the workflows, the guard scripts and the exceptions list, so that changing
  them needs the owner's review. A placeholder owner is allowed but reported.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

SHA = re.compile(r"^[0-9a-f]{40}$")
USES = re.compile(r"^\s*(?:-\s+)?uses:\s*([^\s#]+)")
CODEOWNERS_REQUIRED = (
    ".github/workflows/",
    "scripts/",
    "scripts/guard_exceptions.json",
    ".github/CODEOWNERS",
    "deny.toml",
)
PIPE_TO_SHELL = re.compile(r"(curl|wget)\b[^\n|]*\|\s*(sudo\s+)?(ba|z)?sh\b")


def fail(problems: list[str], message: str) -> None:
    problems.append(f"WORKFLOW-GUARD FAIL: {message}")


def check_workflow(text: str, label: str) -> list[str]:
    problems: list[str] = []
    for number, line in enumerate(text.split("\n"), 1):
        match = USES.match(line)
        if match:
            ref = match.group(1).strip("\"'")
            if ref.startswith("./"):
                continue
            if ref.startswith("docker://"):
                if "@sha256:" not in ref:
                    fail(problems, f"{label}:{number}: docker image {ref!r} must be pinned by digest")
                continue
            _, _, version = ref.partition("@")
            if not SHA.match(version):
                fail(problems, f"{label}:{number}: {ref!r} must be pinned to a full commit SHA, not {version or 'nothing'!r}")
        if re.match(r"^\s*(pull_request_target|workflow_run)\s*:", line) or re.match(
            r"^\s*-\s*(pull_request_target|workflow_run)\b", line
        ):
            fail(problems, f"{label}:{number}: pull_request_target / workflow_run triggers are not allowed")
        if PIPE_TO_SHELL.search(line):
            fail(problems, f"{label}:{number}: do not pipe a download into a shell")
    if not re.search(r"^permissions\s*:", text, re.MULTILINE):
        fail(problems, f"{label}: declare top-level `permissions:` (least privilege)")
    return problems


def check_codeowners(text: str | None) -> tuple[list[str], list[str]]:
    """(problems, warnings)."""
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
        owners = rules.get(path) or rules.get(path.rstrip("/")) or rules.get("*")
        if not owners:
            fail(problems, f".github/CODEOWNERS has no owner for {path}")
    for owners in rules.values():
        if any("REPLACE" in owner.upper() for owner in owners):
            warnings.append(
                "WORKFLOW-GUARD WARNING: .github/CODEOWNERS still has the placeholder owner; "
                "put your GitHub username there before pushing"
            )
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
    if not workflows:
        fail(problems, "no workflow files found")
    for path in workflows:
        problems.extend(check_workflow(path.read_text(), str(path.relative_to(repo))))
    tracked = tracked_files(repo)
    if tracked is not None:
        problems.extend(forbidden_tracked(tracked))
    owners = repo / ".github" / "CODEOWNERS"
    more, warnings = check_codeowners(owners.read_text() if owners.exists() else None)
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
