#!/usr/bin/env python3
"""Offline-mode guard for Money Tracker (docs/ARCHITECTURE.md, rule set 5.1).

    check_offline.py manifest <AndroidManifest.xml>   release manifest rules
    check_offline.py apk <app.apk> [aapt2]            the same rules on the built APK
    check_offline.py config   [repo_root]             CSP and Tauri capabilities (allow-list)
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
import tomllib
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path

import guard_common

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

# The only permissions a capability may grant. An allow-list on purpose: a
# deny-list of "dangerous" prefixes passes every plugin nobody thought of. Adding
# an entry needs the feature that uses it and a note in docs/ARCHITECTURE.md.
ALLOWED_CAPABILITY_PERMISSIONS = {"core:default"}
ALLOWED_CAPABILITY_WINDOWS = {"main"}
# Tauri reads capability files in these formats; each one is parsed here.
CAPABILITY_SUFFIXES = {".json", ".json5", ".toml"}
# Settings in tauri.conf.json that widen what the WebView may reach.
FORBIDDEN_SECURITY_FLAGS = ("dangerousDisableAssetCspModification", "dangerousRemoteDomainIpcAccess")

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


def _csp_text(csp: object) -> str | None:
    """The policy as one string. Tauri also accepts a map of directive -> sources."""
    if isinstance(csp, str):
        return csp
    if isinstance(csp, dict):
        parts = []
        for name, value in csp.items():
            sources = value if isinstance(value, str) else " ".join(map(str, value)) if isinstance(value, list) else ""
            parts.append(f"{name} {sources}")
        return "; ".join(parts)
    return None


def check_csp(csp: object) -> list[str]:
    problems: list[str] = []
    text = _csp_text(csp)
    if text is None or not text.strip():
        return ["app.security.csp must be a non-empty string (null disables the CSP)"]
    # Directive names and source keywords are case-insensitive. When a directive is
    # repeated the browser obeys the FIRST one and ignores the rest, so a repeat is
    # refused outright: judging only the last one would approve a policy the
    # browser reads differently.
    directives: dict[str, list[str]] = {}
    for part in text.split(";"):
        tokens = part.split()
        if not tokens:
            continue
        name = tokens[0].lower()
        if name in directives:
            problems.append(f"CSP repeats {name}; browsers obey only the first, so repeats are refused")
            continue
        directives[name] = [token.lower() for token in tokens[1:]]
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


def _json5_lite(text: str) -> object:
    """Strict JSON plus the two JSON5 conveniences we tolerate: comments and trailing commas.

    Anything else JSON5 allows (unquoted keys, single quotes, ...) is not parsed:
    the file is reported as unverifiable, which fails the guard rather than
    letting a capability through unread.
    """
    out, i, in_string = [], 0, False
    while i < len(text):
        ch = text[i]
        if in_string:
            out.append(ch)
            if ch == "\\":
                out.append(text[i + 1 : i + 2])
                i += 1
            elif ch == '"':
                in_string = False
        elif ch == '"':
            in_string = True
            out.append(ch)
        elif text.startswith("//", i):
            while i < len(text) and text[i] != "\n":
                i += 1
            continue
        elif text.startswith("/*", i):
            end = text.find("*/", i + 2)
            i = len(text) if end < 0 else end + 1
        else:
            out.append(ch)
        i += 1
    return json.loads(re.sub(r",(\s*[}\]])", r"\1", "".join(out)))


def _load_capability(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    if path.suffix == ".toml":
        return tomllib.loads(text)
    if path.suffix == ".json5":
        return _json5_lite(text)
    return json.loads(text)


def check_capability(label: str, data: object) -> list[str]:
    """One capability (a file's contents, or an inline object in tauri.conf.json)."""
    problems: list[str] = []
    if not isinstance(data, dict):
        return [f"{label}: a capability must be an object"]
    permissions = data.get("permissions")
    if not isinstance(permissions, list):
        problems.append(f"{label}: capability has no permissions list to verify")
        permissions = []
    for permission in permissions:
        name = permission if isinstance(permission, str) else (
            permission.get("identifier", "") if isinstance(permission, dict) else ""
        )
        if name not in ALLOWED_CAPABILITY_PERMISSIONS:
            problems.append(f"{label}: permission {name!r} is not on the allow-list")
        if isinstance(permission, dict) and ("allow" in permission or "deny" in permission):
            problems.append(f"{label}: permission {name!r} carries a scope; scopes are not allowed")
    for key in ("remote", "urls"):
        if data.get(key):
            problems.append(f"{label}: remote capabilities ({key}) are not allowed")
    windows = data.get("windows", [])
    if not isinstance(windows, list) or not windows or not set(windows) <= ALLOWED_CAPABILITY_WINDOWS:
        problems.append(f"{label}: windows must be an explicit subset of {sorted(ALLOWED_CAPABILITY_WINDOWS)}")
    webviews = data.get("webviews")
    if webviews is not None and (not isinstance(webviews, list) or "*" in webviews):
        problems.append(f"{label}: webviews must be explicit, not a wildcard")
    return problems


def check_config(repo: Path) -> list[str]:
    problems: list[str] = []
    conf_path = repo / "src-tauri" / "tauri.conf.json"
    conf = json.loads(conf_path.read_text())
    security = conf.get("app", {}).get("security", {})
    for message in check_csp(security.get("csp")):
        fail(problems, f"{conf_path}: {message}")
    for flag in FORBIDDEN_SECURITY_FLAGS:
        if security.get(flag):
            fail(problems, f"{conf_path}: app.security.{flag} must not be set")

    # Inline capabilities in tauri.conf.json: objects are checked like files;
    # strings name a capability file, which must exist.
    cap_dir = repo / "src-tauri" / "capabilities"
    for index, entry in enumerate(security.get("capabilities", []) or []):
        if isinstance(entry, dict):
            for message in check_capability(f"{conf_path} inline capability #{index}", entry):
                fail(problems, message)
        elif not isinstance(entry, str):
            fail(problems, f"{conf_path}: inline capability #{index} is neither a name nor an object")

    for cap in sorted(cap_dir.glob("*")) if cap_dir.exists() else []:
        if not cap.is_file():
            continue
        label = str(cap.relative_to(repo))
        if cap.suffix not in CAPABILITY_SUFFIXES:
            fail(problems, f"{label}: unrecognised capability file type; cannot verify it")
            continue
        try:
            data = _load_capability(cap)
        except (ValueError, OSError) as err:
            fail(problems, f"{label}: cannot be parsed, so it cannot be verified ({err})")
            continue
        for message in check_capability(label, data):
            fail(problems, message)
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
    exceptions = guard_common.load(repo)
    problems: list[str] = list(exceptions.problems)
    for path in iter_source_files(repo):
        suffix = path.suffix.lower()
        if suffix not in SOURCE_RULES and suffix not in URL_SCANNED:
            continue
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except UnicodeDecodeError:
            continue
        label = str(path.relative_to(repo))
        problems.extend(guard_common.inline_marker_problems("\n".join(lines), label))
        rule = SOURCE_RULES.get(suffix)
        for number, line in enumerate(lines, 1):
            if exceptions.allows("offline_source", file=label, contains=line):
                continue
            if rule and rule.search(line):
                fail(problems, f"{label}:{number}: network API in code: {line.strip()[:80]}")
            if suffix in URL_SCANNED and URL.search(line):
                if not any(allowed in line for allowed in URL_ALLOWED):
                    fail(problems, f"{label}:{number}: remote URL: {line.strip()[:80]}")
    problems.extend(exceptions.unused(("offline_source",)))
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
