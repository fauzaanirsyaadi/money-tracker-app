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

    def test_biometric_permissions_are_allowed_but_network_still_is_not(self):
        ok = manifest(
            [
                "android.permission.USE_BIOMETRIC",
                "android.permission.USE_FINGERPRINT",
                "com.moneytracker.app.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION",
            ]
        )
        self.assertEqual(self.run_manifest(ok), [])
        mixed = manifest(["android.permission.USE_BIOMETRIC", "android.permission.INTERNET"])
        self.assertTrue(any("INTERNET" in p for p in self.run_manifest(mixed)))

    def test_internet_permission_fails(self):
        problems = self.run_manifest(manifest(["android.permission.INTERNET"]))
        self.assertTrue(any("INTERNET" in p for p in problems), problems)

    def test_unknown_permission_fails(self):
        problems = self.run_manifest(manifest(["android.permission.CAMERA"]))
        self.assertEqual(len(problems), 1)

    def test_a_file_provider_is_refused(self):
        provider = (
            '<provider android:name="androidx.core.content.FileProvider" '
            'android:authorities="x" android:exported="false"/>'
        )
        xml = manifest().replace("<application", "<application", 1).replace("/></manifest>", f">{provider}</application></manifest>")
        problems = self.run_manifest(xml)
        self.assertTrue(any("FileProvider" in p for p in problems), problems)
        # Other providers (androidx startup) are fine.
        other = xml.replace("androidx.core.content.FileProvider", "androidx.startup.InitializationProvider")
        self.assertEqual(self.run_manifest(other), [])

    def test_backup_must_be_disabled(self):
        self.assertTrue(self.run_manifest(manifest(allowBackup="true")))
        self.assertTrue(self.run_manifest(manifest(allowBackup=None)))

    def test_cleartext_debuggable_and_missing_rules_fail(self):
        self.assertTrue(self.run_manifest(manifest(usesCleartextTraffic="true")))
        self.assertTrue(self.run_manifest(manifest(debuggable="true")))
        self.assertTrue(self.run_manifest(manifest(dataExtractionRules=None)))


PERMISSIONS_OK = """package: com.moneytracker.app
uses-permission: name='android.permission.USE_BIOMETRIC'
uses-permission: name='android.permission.USE_FINGERPRINT'
permission: com.moneytracker.app.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION
uses-permission: name='com.moneytracker.app.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION'
"""

TREE_OK = """N: android=http://schemas.android.com/apk/res/android (line=2)
  E: manifest (line=2)
      E: application (line=31)
        A: http://schemas.android.com/apk/res/android:allowBackup(0x01010280)=false
        A: http://schemas.android.com/apk/res/android:usesCleartextTraffic(0x010104ec)=false
        A: http://schemas.android.com/apk/res/android:dataExtractionRules(0x0101063e)=@0x7f130000
"""


class ApkTests(unittest.TestCase):
    libs = {"lib/arm64-v8a/libapp.so": b"\x00some native code\x00"}

    def run_apk(self, permissions=PERMISSIONS_OK, tree=TREE_OK) -> list[str]:
        def fake(aapt2, *args):
            return permissions if "permissions" in args else tree

        return guard.check_apk(Path("app.apk"), run=fake, libs=lambda _: self.libs)

    def test_the_real_release_output_passes(self):
        self.assertEqual(self.run_apk(), [])

    def test_internet_in_the_built_apk_fails(self):
        extra = PERMISSIONS_OK + "uses-permission: name='android.permission.INTERNET'\n"
        problems = self.run_apk(permissions=extra)
        self.assertTrue(any("INTERNET" in p for p in problems), problems)

    def test_implied_and_sdk23_permissions_are_checked_too(self):
        extra = PERMISSIONS_OK + "uses-implied-permission: name='android.permission.INTERNET' reason='x'\n"
        self.assertTrue(self.run_apk(permissions=extra))
        extra = PERMISSIONS_OK + "uses-permission-sdk-23: name='android.permission.CAMERA'\n"
        self.assertTrue(self.run_apk(permissions=extra))

    def test_debuggable_backup_and_cleartext_fail(self):
        for bad in (
            TREE_OK.replace("allowBackup(0x01010280)=false", "allowBackup(0x01010280)=true"),
            TREE_OK + "A: http://schemas.android.com/apk/res/android:debuggable(0x0101000f)=true\n",
            TREE_OK.replace("usesCleartextTraffic(0x010104ec)=false", "usesCleartextTraffic(0x010104ec)=true"),
            TREE_OK.replace("dataExtractionRules", "somethingElse"),
        ):
            self.assertTrue(self.run_apk(tree=bad), bad)

    def test_devtools_compiled_into_a_native_library_fails(self):
        self.libs = {"lib/arm64-v8a/libapp.so": b"\x00setWebContentsDebuggingEnabled\x00"}
        try:
            problems = self.run_apk()
        finally:
            self.libs = {"lib/arm64-v8a/libapp.so": b"\x00ok\x00"}
        self.assertTrue(any("WebView debugging" in p for p in problems), problems)

    def test_a_file_provider_in_the_built_apk_fails(self):
        tree = TREE_OK + '        E: provider\n          A: android:name(0x01010003)="androidx.core.content.FileProvider" (Raw: "androidx.core.content.FileProvider")\n'
        problems = self.run_apk(tree=tree)
        self.assertTrue(any("FileProvider" in p for p in problems), problems)
        ok = TREE_OK + '        E: provider\n          A: android:name(0x01010003)="androidx.startup.InitializationProvider" (Raw: "androidx.startup.InitializationProvider")\n'
        self.assertEqual(self.run_apk(tree=ok), [])

    def test_empty_aapt_output_is_a_failure_not_a_pass(self):
        self.assertTrue(self.run_apk(permissions="", tree=""))


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
