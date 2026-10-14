"""Tests for check_offline.py. Run: python3 -m unittest scripts/test_check_offline.py"""

import json
import tempfile
import unittest
from pathlib import Path

import check_offline as guard

NS = 'xmlns:android="http://schemas.android.com/apk/res/android"'


def manifest(permissions=(), **app_attrs) -> str:
    attrs = {
        "allowBackup": "false",
        "usesCleartextTraffic": "false",
        "dataExtractionRules": "@xml/rules",
        **app_attrs,
    }
    attr_text = " ".join(f'android:{k}="{v}"' for k, v in attrs.items() if v is not None)
    perms = "".join(f'<uses-permission android:name="{p}"/>' for p in permissions)
    return f"<manifest {NS}>{perms}<application {attr_text}/></manifest>"


class ManifestTests(unittest.TestCase):
    def run_manifest(self, xml: str) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "AndroidManifest.xml"
            path.write_text(xml)
            return guard.check_manifest(path)

    def test_clean_release_manifest_passes(self):
        xml = manifest(["com.moneytracker.app.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION"])
        self.assertEqual(self.run_manifest(xml), [])

    def test_internet_permission_fails(self):
        problems = self.run_manifest(manifest(["android.permission.INTERNET"]))
        self.assertTrue(any("INTERNET" in p for p in problems), problems)

    def test_unknown_permission_fails(self):
        problems = self.run_manifest(manifest(["android.permission.CAMERA"]))
        self.assertEqual(len(problems), 1)

    def test_backup_must_be_disabled(self):
        self.assertTrue(self.run_manifest(manifest(allowBackup="true")))
        self.assertTrue(self.run_manifest(manifest(allowBackup=None)))

    def test_cleartext_debuggable_and_missing_rules_fail(self):
        self.assertTrue(self.run_manifest(manifest(usesCleartextTraffic="true")))
        self.assertTrue(self.run_manifest(manifest(debuggable="true")))
        self.assertTrue(self.run_manifest(manifest(dataExtractionRules=None)))


class CspTests(unittest.TestCase):
    GOOD = (
        "default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; "
        "img-src 'self' data:; connect-src ipc: http://ipc.localhost"
    )

    def test_good_policy_passes(self):
        self.assertEqual(guard.check_csp(self.GOOD), [])

    def test_null_or_empty_fails(self):
        self.assertTrue(guard.check_csp(None))
        self.assertTrue(guard.check_csp("  "))

    def test_remote_hosts_and_wildcards_fail(self):
        for bad in (
            "default-src 'self'; img-src https://example.com",
            "default-src 'self'; connect-src *",
            "default-src 'self'; script-src https:",
            "default-src 'self'; font-src https://fonts.gstatic.com",
        ):
            self.assertTrue(guard.check_csp(bad), bad)

    def test_unsafe_inline_only_for_styles(self):
        self.assertTrue(guard.check_csp("default-src 'self'; script-src 'self' 'unsafe-inline'"))
        self.assertTrue(guard.check_csp("default-src 'unsafe-inline'"))
        self.assertEqual(guard.check_csp("default-src 'none'; style-src 'self' 'unsafe-inline'"), [])

    def test_unsafe_eval_fails(self):
        self.assertTrue(guard.check_csp("default-src 'self'; script-src 'self' 'unsafe-eval'"))


class RepoTests(unittest.TestCase):
    def make_repo(self, tmp: str, csp: str = CspTests.GOOD, permissions=("core:default",)) -> Path:
        repo = Path(tmp)
        (repo / "src-tauri" / "capabilities").mkdir(parents=True)
        (repo / "src-tauri" / "tauri.conf.json").write_text(
            json.dumps({"app": {"security": {"csp": csp}}})
        )
        (repo / "src-tauri" / "capabilities" / "default.json").write_text(
            json.dumps({"permissions": list(permissions)})
        )
        (repo / "src").mkdir()
        (repo / "crates" / "core" / "src").mkdir(parents=True)
        return repo

    def test_config_ok(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(guard.check_config(self.make_repo(tmp)), [])

    def test_network_capability_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp, permissions=("core:default", "http:default"))
            self.assertEqual(len(guard.check_config(repo)), 1)

    def test_source_flags_network_code_and_urls(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            (repo / "crates" / "core" / "src" / "lib.rs").write_text(
                "use std::net::TcpStream;\nfn ok() {}\n"
            )
            (repo / "src" / "App.svelte").write_text(
                '<img src="https://cdn.example.com/a.png">\n<script>fetch("/x")</script>\n'
            )
            problems = guard.check_source(repo)
            self.assertEqual(len(problems), 3, problems)

    def test_source_allows_namespaces_and_marked_lines(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            (repo / "src" / "icon.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg"/>\n')
            (repo / "crates" / "core" / "src" / "lib.rs").write_text(
                "// reqwest is mentioned only in docs  // offline-check: allow\n"
            )
            self.assertEqual(guard.check_source(repo), [])


if __name__ == "__main__":
    unittest.main()
