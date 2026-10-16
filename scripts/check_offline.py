#!/usr/bin/env python3
"""Offline-mode guard for Money Tracker (docs/ARCHITECTURE.md, rule set 5.1).

    check_offline.py manifest <AndroidManifest.xml>   release manifest rules
    check_offline.py apk <app.apk> [aapt2]            the same rules on the built APK
    check_offline.py config   [repo_root]             CSP and Tauri capabilities
    check_offline.py source   [repo_root]             no network APIs or URLs in our code

Exits 1 with one "OFFLINE-GUARD FAIL" line per violation. Network-capable
crates in the dependency graph are enforced separately by `cargo deny`
(see [bans] in deny.toml).
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path

ANDROID = "{http://schemas.android.com/apk/res/android}"

# Every permission a *release* build may declare. INTERNET must never be here.
# Add an entry only together with the feature that needs it and a note in
# docs/ARCHITECTURE.md.
ALLOWED_PERMISSIONS = {
    # Added by the AndroidX core library for non-exported dynamic receivers.
    "DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION",  # matched by suffix, any package
    # Added by androidx.biometric for the app-lock fingerprint/face prompt. Local
    # hardware only; neither permission grants network or storage access.
    "android.permission.USE_BIOMETRIC",
    "android.permission.USE_FINGERPRINT",
}

# CSP source expressions that cannot reach a remote origin.
SAFE_CSP_SOURCES = {"'self'", "'none'", "data:", "ipc:", "http://ipc.localhost"}
UNSAFE_INLINE_ONLY_IN = {"style-src", "style-src-elem", "style-src-attr"}

FORBIDDEN_CAPABILITY_PREFIXES = ("http:", "websocket:", "shell:", "opener:", "updater:")

SOURCE_RULES = {
    ".rs": re.compile(
        r"std::net\b|\bTcpStream\b|\bTcpListener\b|\bUdpSocket\b|\breqwest\b|\bhyper\b"
        r"|\bureq\b|\bsurf\b|\bisahc\b|\bcurl\b|\btokio::net\b|tungstenite"
    ),
    ".kt": re.compile(r"HttpURLConnection|java\.net\.(URL|Socket)|okhttp|\bSocket\("),
    ".svelte": re.compile(r"\bfetch\(|XMLHttpRequest|\bWebSocket\(|sendBeacon|EventSource\("),
    ".ts": re.compile(r"\bfetch\(|XMLHttpRequest|\bWebSocket\(|sendBeacon|EventSource\("),
    ".js": re.compile(r"\bfetch\(|XMLHttpRequest|\bWebSocket\(|sendBeacon|EventSource\("),
}
URL = re.compile(r"https?://", re.IGNORECASE)
URL_ALLOWED = ("http://www.w3.org/",)  # XML/SVG namespaces, not requests
URL_SCANNED = {".svelte", ".ts", ".js", ".html", ".css", ".svg", ".json"}
SKIP_DIRS = {"node_modules", "target", "build", ".svelte-kit", "gen", ".git", "dist"}
ALLOW_MARK = "offline-check: allow"


def fail(problems: list[str], message: str) -> None:
    problems.append(f"OFFLINE-GUARD FAIL: {message}")


def check_manifest(path: Path) -> list[str]:
    problems: list[str] = []
    root = ET.parse(path).getroot()

    for tag in ("uses-permission", "uses-permission-sdk-23"):
        for node in root.iter(tag):
            name = node.get(f"{ANDROID}name", "")
            if not any(name.endswith(allowed) for allowed in ALLOWED_PERMISSIONS):
                hint = " (INTERNET is forbidden in release builds)" if name.endswith("INTERNET") else ""
                fail(problems, f"{path}: permission {name!r} is not allowed{hint}")

    app = root.find("application")
    if app is None:
        fail(problems, f"{path}: no <application> element")
        return problems
    for provider in app.iter("provider"):
        if provider.get(f"{ANDROID}name", "").endswith("FileProvider"):
            fail(problems, f"{path}: a FileProvider shares files with other apps and is not used here")
    if app.get(f"{ANDROID}allowBackup") != "false":
        fail(problems, f"{path}: android:allowBackup must be \"false\"")
    if app.get(f"{ANDROID}usesCleartextTraffic") == "true":
        fail(problems, f"{path}: android:usesCleartextTraffic must not be \"true\"")
    if app.get(f"{ANDROID}debuggable") == "true":
        fail(problems, f"{path}: release build must not be debuggable")
    if not app.get(f"{ANDROID}dataExtractionRules"):
        fail(problems, f"{path}: android:dataExtractionRules is missing")
    return problems


def _aapt2(aapt2: str, *args: str) -> str:
    result = subprocess.run([aapt2, *args], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"aapt2 {' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout


DEVTOOLS_MARKER = b"setWebContentsDebuggingEnabled"


def _native_libs(apk: Path) -> dict[str, bytes]:
    with zipfile.ZipFile(apk) as archive:
        return {
            name: archive.read(name)
            for name in archive.namelist()
            if name.startswith("lib/") and name.endswith(".so")
        }


def check_apk(apk: Path, aapt2: str = "aapt2", run=_aapt2, libs=_native_libs) -> list[str]:
    """Checks the *built* APK, the only manifest that is certainly the shipped one.

    Intermediate Gradle manifests can omit library manifests or merge later, so
    the permission list and application flags are read from the final artifact.
    """
    problems: list[str] = []
    permissions = run(aapt2, "dump", "permissions", str(apk))
    declared = re.findall(
        r"^uses-(?:implied-)?permission(?:-sdk-23)?: name='([^']+)'", permissions, re.MULTILINE
    )
    if not declared and "package:" not in permissions:
        fail(problems, f"{apk}: aapt2 produced no manifest information")
        return problems
    for name in declared:
        if not any(name.endswith(allowed) for allowed in ALLOWED_PERMISSIONS):
            hint = " (INTERNET is forbidden in release builds)" if name.endswith("INTERNET") else ""
            fail(problems, f"{apk}: permission {name!r} is not allowed{hint}")

    tree = run(aapt2, "dump", "xmltree", "--file", "AndroidManifest.xml", str(apk))

    def flag(attribute: str) -> str | None:
        match = re.search(rf":{attribute}\(0x[0-9a-fA-F]+\)=(\w+)", tree)
        return match.group(1) if match else None

    if flag("allowBackup") != "false":
        fail(problems, f"{apk}: android:allowBackup must be \"false\"")
    if flag("usesCleartextTraffic") == "true":
        fail(problems, f"{apk}: android:usesCleartextTraffic must not be \"true\"")
    if flag("debuggable") == "true":
        fail(problems, f"{apk}: release build must not be debuggable")
    if not re.search(r":dataExtractionRules\(", tree):
        fail(problems, f"{apk}: android:dataExtractionRules is missing")
    if re.search(r'FileProvider"', tree):
        fail(problems, f"{apk}: a FileProvider shares files with other apps and is not used here")

    # wry only emits this JNI call when devtools are compiled in (debug build or
    # the `devtools` feature). A release APK must not expose the inspector.
    for name, blob in libs(apk).items():
        if DEVTOOLS_MARKER in blob:
            fail(problems, f"{apk}: {name} enables WebView debugging (devtools compiled in)")
    return problems


def check_csp(csp: object) -> list[str]:
    problems: list[str] = []
    if not isinstance(csp, str) or not csp.strip():
        return ["app.security.csp must be a non-empty string (null disables the CSP)"]
    directives: dict[str, list[str]] = {}
    for part in csp.split(";"):
        tokens = part.split()
        if tokens:
            directives[tokens[0]] = tokens[1:]
    if "default-src" not in directives:
        problems.append("CSP has no default-src")
    for name, sources in directives.items():
        for source in sources:
            if source in SAFE_CSP_SOURCES:
                continue
            if source == "'unsafe-inline'" and name in UNSAFE_INLINE_ONLY_IN:
                continue
            problems.append(f"CSP {name} allows {source!r}")
    for name in ("default-src", "script-src", "connect-src"):
        if "'unsafe-inline'" in directives.get(name, []):
            problems.append(f"CSP {name} must not allow 'unsafe-inline'")
    return problems


def check_config(repo: Path) -> list[str]:
    problems: list[str] = []
    conf_path = repo / "src-tauri" / "tauri.conf.json"
    conf = json.loads(conf_path.read_text())
    for message in check_csp(conf.get("app", {}).get("security", {}).get("csp")):
        fail(problems, f"{conf_path}: {message}")

    for cap in sorted((repo / "src-tauri" / "capabilities").glob("*.json")):
        data = json.loads(cap.read_text())
        for permission in data.get("permissions", []):
            name = permission if isinstance(permission, str) else permission.get("identifier", "")
            if name.startswith(FORBIDDEN_CAPABILITY_PREFIXES):
                fail(problems, f"{cap}: capability {name!r} grants network or process access")
        if data.get("remote"):
            fail(problems, f"{cap}: remote capabilities are not allowed")
    return problems


def iter_source_files(repo: Path):
    roots = [repo / "crates", repo / "src-tauri" / "src", repo / "src-tauri" / "plugins",
             repo / "src", repo / "static"]
    for root in roots:
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if path.is_file() and not (set(path.relative_to(repo).parts) & SKIP_DIRS):
                yield path


def check_source(repo: Path) -> list[str]:
    problems: list[str] = []
    for path in iter_source_files(repo):
        suffix = path.suffix.lower()
        if suffix not in SOURCE_RULES and suffix not in URL_SCANNED:
            continue
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except UnicodeDecodeError:
            continue
        rule = SOURCE_RULES.get(suffix)
        for number, line in enumerate(lines, 1):
            if ALLOW_MARK in line:
                continue
            if rule and rule.search(line):
                fail(problems, f"{path.relative_to(repo)}:{number}: network API in code: {line.strip()[:80]}")
            if suffix in URL_SCANNED and URL.search(line):
                if not any(allowed in line for allowed in URL_ALLOWED):
                    fail(problems, f"{path.relative_to(repo)}:{number}: remote URL: {line.strip()[:80]}")
    return problems


def main(argv: list[str]) -> int:
    if len(argv) < 2 or argv[1] not in {"manifest", "apk", "config", "source"}:
        print(__doc__)
        return 2
    command = argv[1]
    if command == "manifest":
        if len(argv) != 3:
            print("usage: check_offline.py manifest <AndroidManifest.xml>")
            return 2
        problems = check_manifest(Path(argv[2]))
    elif command == "apk":
        if len(argv) not in (3, 4):
            print("usage: check_offline.py apk <app-release.apk> [path/to/aapt2]")
            return 2
        problems = check_apk(Path(argv[2]), argv[3] if len(argv) == 4 else "aapt2")
    else:
        repo = Path(argv[2]) if len(argv) > 2 else Path(__file__).resolve().parent.parent
        problems = check_config(repo) if command == "config" else check_source(repo)
    for problem in problems:
        print(problem)
    if not problems:
        print(f"offline guard ({command}): ok")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
