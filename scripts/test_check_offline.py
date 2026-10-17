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


# The directives that never fall back to default-src, so every policy must name them.
REQUIRED = "base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'"


class CspTests(unittest.TestCase):
    GOOD = (
        "default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; "
        "img-src 'self' data:; connect-src ipc: http://ipc.localhost; base-uri 'none'; "
        "form-action 'none'; object-src 'none'; frame-ancestors 'none'"
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
        self.assertEqual(guard.check_csp(f"default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; {REQUIRED}"), [])

    def test_a_repeated_directive_is_refused_because_the_browser_obeys_the_first(self):
        # The first script-src is the remote one; a guard that kept the last would have passed this.
        evil_first = "default-src 'none'; script-src https://evil.example; script-src 'self'"
        problems = guard.check_csp(evil_first)
        self.assertTrue(any("repeats script-src" in p for p in problems), problems)
        self.assertTrue(any("evil.example" in p for p in problems), problems)
        # Even a harmless repeat is refused: the policy must say each thing once.
        self.assertTrue(guard.check_csp(self.GOOD + "; img-src 'self'"))

    def test_directive_names_and_keywords_are_case_insensitive(self):
        self.assertTrue(guard.check_csp("default-src 'none'; Script-Src https://evil.example"))
        self.assertTrue(guard.check_csp("DEFAULT-SRC 'none'; script-src 'SELF' 'Unsafe-Eval'"))
        self.assertEqual(guard.check_csp(f"Default-Src 'NONE'; Script-Src 'Self'; {REQUIRED}"), [])

    def test_the_map_form_is_judged_like_the_string_form(self):
        good = {"default-src": "'none'", "script-src": ["'self'"], "connect-src": "ipc: http://ipc.localhost",
                "base-uri": "'none'", "form-action": "'none'", "object-src": ["'none'"], "frame-ancestors": "'none'"}
        self.assertEqual(guard.check_csp(good), [])
        self.assertTrue(guard.check_csp({"default-src": "'none'", "script-src": ["https://evil.example"]}))

    def test_data_and_ipc_sources_only_where_they_belong(self):
        # data: in a script-src would allow scripts from data URLs.
        self.assertTrue(any("script-src" in p and "data:" in p for p in guard.check_csp(self.GOOD.replace("script-src 'self'", "script-src 'self' data:"))))
        self.assertTrue(guard.check_csp(self.GOOD.replace("default-src 'none'", "default-src 'none' data:")))
        self.assertTrue(guard.check_csp(self.GOOD.replace("img-src 'self' data:", "img-src 'self' ipc:")))
        # ...and where they belong they are fine.
        self.assertEqual(guard.check_csp(self.GOOD.replace("font-src", "font-src")), [])
        self.assertEqual(guard.check_csp(self.GOOD + "; font-src 'self' data:"), [])

    def test_directives_that_do_not_fall_back_must_be_present(self):
        for name in ("base-uri 'none'", "form-action 'none'", "object-src 'none'", "frame-ancestors 'none'"):
            weakened = self.GOOD.replace("; " + name, "")
            self.assertNotEqual(weakened, self.GOOD, name)
            self.assertTrue(any(name.split()[0] in p for p in guard.check_csp(weakened)), name)

    def test_unsafe_eval_fails(self):
        self.assertTrue(guard.check_csp("default-src 'self'; script-src 'self' 'unsafe-eval'"))


class RepoTests(unittest.TestCase):
    def make_repo(self, tmp: str, csp: str = CspTests.GOOD, permissions=("core:default",), **conf_security) -> Path:
        repo = Path(tmp)
        (repo / "src-tauri" / "capabilities").mkdir(parents=True)
        (repo / "scripts").mkdir()
        (repo / "src-tauri" / "tauri.conf.json").write_text(
            json.dumps({"app": {"security": {"csp": csp, **conf_security}}})
        )
        (repo / "src-tauri" / "capabilities" / "default.json").write_text(
            json.dumps({"windows": ["main"], "permissions": list(permissions)})
        )
        (repo / "src").mkdir()
        (repo / "crates" / "core" / "src").mkdir(parents=True)
        return repo

    def cap(self, repo: Path, name: str, text: str) -> None:
        (repo / "src-tauri" / "capabilities" / name).write_text(text)

    def test_config_ok(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(guard.check_config(self.make_repo(tmp)), [])

    def test_network_capability_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp, permissions=("core:default", "http:default"))
            self.assertEqual(len(guard.check_config(repo)), 1)

    def test_the_allow_list_refuses_plugins_nobody_thought_of(self):
        # A deny-list of "http:", "shell:" ... would pass every one of these.
        for name in ("fs:default", "dialog:default", "clipboard-manager:default", "os:default",
                     "process:allow-exit", "core:webview:default", "my-plugin:default"):
            with tempfile.TemporaryDirectory() as tmp:
                repo = self.make_repo(tmp, permissions=("core:default", name))
                self.assertTrue(guard.check_config(repo), name)

    def test_object_permissions_and_scopes_are_checked(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            self.cap(repo, "scoped.json", json.dumps({
                "windows": ["main"],
                "permissions": [{"identifier": "core:default", "allow": [{"path": "$HOME/**"}]}],
            }))
            self.assertTrue(any("scope" in p for p in guard.check_config(repo)))
            self.cap(repo, "scoped.json", json.dumps({
                "windows": ["main"], "permissions": [{"identifier": "fs:read-all"}],
            }))
            self.assertTrue(any("fs:read-all" in p for p in guard.check_config(repo)))

    def test_remote_capabilities_and_wide_windows_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            self.cap(repo, "remote.json", json.dumps({
                "windows": ["main"], "permissions": ["core:default"], "remote": {"urls": ["https://x.example"]},
            }))
            self.assertTrue(any("remote" in p for p in guard.check_config(repo)))
        for windows in (["*"], ["other"], [], None):
            with tempfile.TemporaryDirectory() as tmp:
                repo = self.make_repo(tmp)
                data = {"permissions": ["core:default"]}
                if windows is not None:
                    data["windows"] = windows
                self.cap(repo, "wide.json", json.dumps(data))
                self.assertTrue(guard.check_config(repo), windows)

    def test_json5_and_toml_capability_files_are_read_too(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            self.cap(repo, "a.json5", '// comment\n{ "windows": ["main"], /* c */ "permissions": ["core:default",], }\n')
            self.assertEqual(guard.check_config(repo), [])
            self.cap(repo, "a.json5", '{ "windows": ["main"], "permissions": ["core:default", "fs:default",], }\n')
            self.assertTrue(any("fs:default" in p for p in guard.check_config(repo)))
            (repo / "src-tauri" / "capabilities" / "a.json5").unlink()
            self.cap(repo, "b.toml", 'identifier = "b"\nwindows = ["main"]\npermissions = ["core:default"]\n')
            self.assertEqual(guard.check_config(repo), [])
            self.cap(repo, "b.toml", 'identifier = "b"\nwindows = ["main"]\npermissions = ["core:default", "shell:default"]\n')
            self.assertTrue(any("shell:default" in p for p in guard.check_config(repo)))

    def test_what_cannot_be_parsed_cannot_be_verified_and_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            self.cap(repo, "odd.json5", "{ windows: ['main'], permissions: ['fs:default'] }")  # unquoted keys
            self.assertTrue(any("cannot be parsed" in p for p in guard.check_config(repo)))
            (repo / "src-tauri" / "capabilities" / "odd.json5").unlink()
            self.cap(repo, "notes.yaml", "permissions: [fs:default]")
            self.assertTrue(any("unrecognised" in p for p in guard.check_config(repo)))

    def test_inline_capabilities_in_the_config_are_checked(self):
        with tempfile.TemporaryDirectory() as tmp:
            ok = self.make_repo(tmp, capabilities=["default", {
                "identifier": "inline", "windows": ["main"], "permissions": ["core:default"]}])
            self.assertEqual(guard.check_config(ok), [])
        with tempfile.TemporaryDirectory() as tmp:
            bad = self.make_repo(tmp, capabilities=[{
                "identifier": "inline", "windows": ["main"], "permissions": ["core:default", "http:default"]}])
            self.assertTrue(any("inline capability" in p for p in guard.check_config(bad)))

    def test_capability_files_in_subdirectories_are_read_too(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            nested = repo / "src-tauri" / "capabilities" / "x" / "y"
            nested.mkdir(parents=True)
            (nested / "evil.json").write_text(json.dumps({"windows": ["main"], "permissions": ["fs:default"]}))
            self.assertTrue(any("fs:default" in p for p in guard.check_config(repo)))

    def test_a_capabilities_list_is_judged_by_what_is_inside_it(self):
        # Tauri ignores the other top-level fields of such a file; a benign top level
        # must not stand in for the list.
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            self.cap(repo, "named.json", json.dumps({
                "windows": ["main"], "permissions": ["core:default"],
                "capabilities": [{"identifier": "evil", "windows": ["main"], "permissions": ["shell:default"]}],
            }))
            self.assertTrue(any("shell:default" in p for p in guard.check_config(repo)))
            self.cap(repo, "named.json", json.dumps({"capabilities": [
                {"identifier": "ok", "windows": ["main"], "permissions": ["core:default"]}, "default"]}))
            self.assertEqual(guard.check_config(repo), [])
            self.cap(repo, "named.json", json.dumps({"capabilities": "oops"}))
            self.assertTrue(guard.check_config(repo))

    def test_configs_that_tauri_merges_over_the_main_one_are_refused(self):
        for name in ("tauri.android.conf.json", "tauri.conf.json5", "Tauri.toml", "tauri.linux.conf.json", "Tauri.android.toml"):
            with tempfile.TemporaryDirectory() as tmp:
                repo = self.make_repo(tmp)
                (repo / "src-tauri" / name).write_text('{"app": {"security": {"csp": null}}}')
                self.assertTrue(any(name.lower() in p.lower() for p in guard.check_config(repo)), name)

    def test_runtime_capability_loading_is_refused_in_build_scripts_and_code(self):
        for rel, text in (
            ("src-tauri/build.rs", 'tauri_build::try_build(Attributes::new().capabilities_path_pattern("x/**/*"));'),
            ("src-tauri/src/lib.rs", "app.add_capability(r#\"{}\"#)?;"),
        ):
            with tempfile.TemporaryDirectory() as tmp:
                repo = self.make_repo(tmp)
                path = repo / rel
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
                self.assertTrue(any("capabilit" in p for p in guard.check_config(repo)), rel)

    def test_window_and_webview_labels_must_be_names_not_patterns(self):
        for key, value in (("webviews", ["*"]), ("webviews", ["**"]), ("webviews", ["?*"]), ("webviews", ["[!z]*"]),
                           ("windows", ["m*"]), ("windows", ["main", "{a,b}"])):
            with tempfile.TemporaryDirectory() as tmp:
                repo = self.make_repo(tmp)
                data = {"windows": ["main"], "permissions": ["core:default"], key: value}
                self.cap(repo, "w.json", json.dumps(data))
                self.assertTrue(guard.check_config(repo), (key, value))
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            self.cap(repo, "w.json", json.dumps({"windows": ["main"], "webviews": ["main-view"], "permissions": ["core:default"]}))
            self.assertEqual(guard.check_config(repo), [])

    def test_a_file_that_is_not_utf8_is_reported_not_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            (repo / "src" / "a.ts").write_bytes(b"// \xff\nfetch('/x')\n")
            problems = guard.check_source(repo)
            self.assertTrue(any("not valid UTF-8" in p for p in problems), problems)

    def test_the_other_script_suffixes_and_kotlin_in_gen_are_scanned(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            (repo / "src" / "a.mjs").write_text("fetch('/x')\n")
            (repo / "src" / "b.tsx").write_text("new WebSocket('x')\n")
            kt = repo / "src-tauri" / "gen" / "android" / "app" / "src" / "main" / "java"
            kt.mkdir(parents=True)
            (kt / "MainActivity.kt").write_text("val c: HttpsURLConnection\n")
            problems = guard.check_source(repo)
            self.assertEqual(len([p for p in problems if "network API" in p]), 3, problems)
            # Build output under gen/ is still skipped.
            out = repo / "src-tauri" / "gen" / "android" / "app" / "build"
            out.mkdir(parents=True)
            (out / "Gen.kt").write_text("val c: HttpsURLConnection\n")
            self.assertEqual(len([p for p in guard.check_source(repo) if "network API" in p]), 3)

    def test_flags_that_widen_the_webview_are_refused(self):
        for flag in ("dangerousDisableAssetCspModification", "dangerousRemoteDomainIpcAccess"):
            with tempfile.TemporaryDirectory() as tmp:
                repo = self.make_repo(tmp, **{flag: [{"domain": "x.example"}] if "Remote" in flag else True})
                self.assertTrue(any(flag in p for p in guard.check_config(repo)), flag)

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

    def test_source_allows_namespaces(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            (repo / "src" / "icon.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg"/>\n')
            self.assertEqual(guard.check_source(repo), [])

    def exceptions(self, repo: Path, entries) -> None:
        (repo / "scripts" / "guard_exceptions.json").write_text(json.dumps({"offline_source": entries}))

    def test_a_listed_exception_allows_the_line_and_an_inline_comment_does_not(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            line = "// reqwest is mentioned only in docs\n"
            (repo / "crates" / "core" / "src" / "lib.rs").write_text(line)
            self.assertEqual(len(guard.check_source(repo)), 1)
            self.exceptions(repo, [{"file": "crates/core/src/lib.rs", "contains": "reqwest is mentioned only in docs",
                                    "reason": "prose, not a dependency"}])
            self.assertEqual(guard.check_source(repo), [])
            # The old inline marker no longer exempts anything, and is itself reported.
            (repo / "scripts" / "guard_exceptions.json").unlink()
            (repo / "crates" / "core" / "src" / "lib.rs").write_text(line.rstrip() + "  // offline-check: allow\n")
            problems = guard.check_source(repo)
            self.assertTrue(any("inline exemption" in p for p in problems), problems)
            self.assertTrue(any("network API" in p for p in problems), problems)

    def test_an_exception_that_matches_nothing_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            self.exceptions(repo, [{"file": "crates/core/src/lib.rs", "contains": "no such line", "reason": "stale"}])
            self.assertTrue(any("unused" in p for p in guard.check_source(repo)))

    def test_an_exception_for_another_file_does_not_apply_here(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.make_repo(tmp)
            (repo / "crates" / "core" / "src" / "lib.rs").write_text("use std::net::TcpStream;\n")
            self.exceptions(repo, [{"file": "crates/core/src/other.rs", "contains": "std::net", "reason": "x"}])
            problems = guard.check_source(repo)
            self.assertTrue(any("network API" in p for p in problems))
            self.assertTrue(any("unused" in p for p in problems))


if __name__ == "__main__":
    unittest.main()
