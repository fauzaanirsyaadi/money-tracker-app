#!/usr/bin/env node
// Checks the *release* build on a device: properties that a debug build cannot
// prove and that a code comment cannot guarantee.
//
//   node scripts/android_release_check.mjs [--apk PATH] [--serial ID]
//
// The release APK is unsigned, so a copy is test-signed with the debug key
// (never use that copy for distribution). It then verifies, after launching
// with the debug-only `mt_insecure` extra, that:
//   - FLAG_SECURE is still set (the extra is ignored),
//   - the window does not keep the screen on,
//   - the app is not debuggable and its WebView is not inspectable.

import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PACKAGE = "com.moneytracker.app";
const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT;
if (!sdk) throw new Error("set ANDROID_HOME");

const args = process.argv.slice(2);
const option = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const serial = option("--serial");
const apk =
  option("--apk") ??
  path.join(root, "src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk");
const adbBin = path.join(sdk, "platform-tools", "adb");
const adb = (...a) =>
  execFileSync(adbBin, [...(serial ? ["-s", serial] : []), ...a], { encoding: "utf8" }).trim();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function latestBuildTool(name) {
  const dir = path.join(sdk, "build-tools");
  const versions = execFileSync("ls", [dir], { encoding: "utf8" }).trim().split("\n").sort();
  return path.join(dir, versions.at(-1), name);
}

let failed = false;
function check(name, ok, detail = "") {
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  -> ${detail}` : ""}`);
  if (!ok) failed = true;
}

if (!existsSync(apk)) throw new Error(`release APK not found: ${apk}`);
const keystore = path.join(homedir(), ".android", "debug.keystore");
if (!existsSync(keystore)) throw new Error("no ~/.android/debug.keystore (build a debug APK once)");

const signed = path.join(mkdtempSync(path.join(tmpdir(), "mt-release-")), "release-test-signed.apk");
execFileSync(latestBuildTool("apksigner"), [
  "sign", "--ks", keystore, "--ks-pass", "pass:android", "--key-pass", "pass:android",
  "--out", signed, apk,
]);

try {
  adb("uninstall", PACKAGE);
} catch {
  // not installed
}
adb("install", signed);
adb("shell", "am", "force-stop", PACKAGE);
adb("shell", "am", "start", "-n", `${PACKAGE}/.MainActivity`, "--ez", "mt_insecure", "true");
await sleep(8000);

const windows = adb("shell", "dumpsys", "window", "windows");
const main = /Window #\d+ Window\{[^}]*com\.moneytracker\.app\/[^}]*\}:\n\s+(?:.*\n)*?\s+fl=([^\n]*)/.exec(windows);
const flags = main?.[1] ?? "";
check("main window found", main !== null);
check("FLAG_SECURE is set although mt_insecure was passed", /\bSECURE\b/.test(flags), flags.slice(0, 80));
check("the window does not keep the screen on", !/KEEP_SCREEN_ON/.test(flags));

const pkg = adb("shell", "dumpsys", "package", PACKAGE);
check("the app is not debuggable", !/pkgFlags=\[[^\]]*\bDEBUGGABLE\b/.test(pkg));

// The inspector socket is checked statically: the release native library must not
// contain the call that enables it (scripts/check_offline.py apk does the same in CI).
const lib = execFileSync("unzip", ["-p", apk, "lib/arm64-v8a/libmoney_tracker_app_lib.so"], {
  maxBuffer: 256 * 1024 * 1024,
});
check("the release library does not enable WebView debugging", !lib.includes("setWebContentsDebuggingEnabled"));

// On a userdebug/eng image the WebView itself opens the socket for every app
// (ro.debuggable=1), so the runtime check only means something on a user build.
if (adb("shell", "getprop", "ro.debuggable") === "1") {
  console.log("SKIP  runtime DevTools socket check: this device image is debuggable");
} else {
  const sockets = adb("shell", "cat", "/proc/net/unix");
  check("no WebView DevTools socket is exposed", !/webview_devtools_remote_/.test(sockets));
}

adb("shell", "am", "force-stop", PACKAGE);
process.exit(failed ? 1 : 0);
