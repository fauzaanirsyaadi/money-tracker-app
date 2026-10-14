#!/usr/bin/env node
// On-device smoke test: installs the debug APK, then drives the quick-add flow
// with real adb taps (FAB -> type 25000 -> Simpan) and asserts the saved row.
//
//   node scripts/android_smoke.mjs [--apk PATH] [--no-install] [--serial ID]
//
// Needs a debug build (the release WebView is not inspectable, by design), an
// emulator or device on adb, and Node 22+ (global fetch and WebSocket).
// Taps are real input events, so this exercises focus, the keyboard and the
// pinned Simpan button the way a user would; the DevTools channel is used only
// to read element positions and to assert the result.

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PACKAGE = "com.moneytracker.app";
const ACTIVITY = `${PACKAGE}/.MainActivity`;
const PORT = 9222;

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};

const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT;
const adbBin = process.env.ADB ?? (sdk ? path.join(sdk, "platform-tools", "adb") : "adb");
const serial = option("--serial");
const apk =
  option("--apk") ??
  path.join(root, "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const adb = (...a) =>
  execFileSync(adbBin, [...(serial ? ["-s", serial] : []), ...a], { encoding: "utf8" }).trim();

const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  -> ${detail}` : ""}`);
  if (!ok) throw new Error(`check failed: ${name}`);
}

async function until(what, fn, ms = 60_000) {
  const start = Date.now();
  for (;;) {
    try {
      const value = await fn();
      if (value) return value;
    } catch {
      // not ready yet
    }
    if (Date.now() - start > ms) throw new Error(`timeout waiting for ${what}`);
    await sleep(500);
  }
}

// --- DevTools channel ------------------------------------------------------

let ws;
let nextId = 0;
const pending = new Map();

async function connect() {
  const pid = adb("shell", "pidof", PACKAGE);
  if (!pid) throw new Error("app is not running");
  adb("forward", `tcp:${PORT}`, `localabstract:webview_devtools_remote_${pid}`);
  const targets = await (await fetch(`http://localhost:${PORT}/json`)).json();
  const target = targets.find((t) => t.url?.includes("tauri.localhost")) ?? targets[0];
  if (!target) throw new Error("no inspectable page");
  ws = new WebSocket(target.webSocketDebuggerUrl);
  ws.onmessage = (event) => {
    const m = JSON.parse(event.data);
    if (m.id && pending.has(m.id)) {
      pending.get(m.id)(m);
      pending.delete(m.id);
    }
  };
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = reject;
  });
}

async function evaluate(expression) {
  const message = await new Promise((resolve) => {
    const id = ++nextId;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method: "Runtime.evaluate", params: { expression, awaitPromise: true, returnByValue: true } }));
  });
  if (message.result?.exceptionDetails) {
    throw new Error(message.result.exceptionDetails.exception?.description ?? "page exception");
  }
  return message.result?.result?.value;
}

const invoke = (command, payload = {}) =>
  evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(payload)})`);

// --- Real taps -------------------------------------------------------------

/** Top of the WebView in device pixels (status bar inset), from the view hierarchy. */
function webViewTop() {
  adb("shell", "uiautomator", "dump", "/sdcard/mt-ui.xml");
  const xml = adb("shell", "cat", "/sdcard/mt-ui.xml");
  const match = /class="android\.webkit\.WebView"[^>]*bounds="\[\d+,(\d+)\]/.exec(xml);
  if (!match) throw new Error("WebView bounds not found");
  return Number(match[1]);
}

async function tapElement(selector, top, dpr) {
  const rect = await evaluate(
    `(() => { const e = document.querySelector(${JSON.stringify(selector)}); if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`,
  );
  if (!rect) throw new Error(`no element ${selector}`);
  adb("shell", "input", "tap", String(Math.round(rect.x * dpr)), String(Math.round(rect.y * dpr) + top));
}

// --- Flow ------------------------------------------------------------------

async function main() {
  if (!flag("--no-install")) {
    if (!existsSync(apk)) throw new Error(`APK not found: ${apk} (pnpm tauri android build --apk --debug --target aarch64)`);
    try {
      adb("uninstall", PACKAGE);
    } catch {
      // not installed
    }
    adb("install", apk);
  }
  adb("shell", "am", "force-stop", PACKAGE);
  adb("shell", "am", "start", "-n", ACTIVITY);

  await until("the web view", async () => {
    try {
      await connect();
      return await evaluate("typeof window.__TAURI_INTERNALS__ !== 'undefined'");
    } catch {
      return false;
    }
  });
  check("app started with an inspectable web view", true);

  const note = `smoke-${Date.now()}`;
  const accounts = await invoke("accounts_list", { includeArchived: false });
  let account = accounts[0];
  if (!account) {
    account = await invoke("account_create", { input: { name: "Dompet", kind: "cash", openingBalanceMinor: "0" } });
  }
  await evaluate("document.querySelector('dialog[open] button[aria-label=Tutup]')?.click()");
  await evaluate("[...document.querySelectorAll('nav a')].find(a => a.innerText.trim() === 'Transaksi')?.click()");
  await until("transactions screen", () => evaluate("!!document.querySelector('.fab')"));

  const dpr = await evaluate("devicePixelRatio");
  const top = webViewTop();

  // Tap 1: the FAB opens the quick-add sheet with the amount field focused.
  await tapElement(".fab", top, dpr);
  await until("sheet open", () => evaluate("!!document.querySelector('dialog[open]')"));
  check("tap 1 opens the quick-add sheet", true);
  await until("amount focused", () => evaluate("document.activeElement?.id === 'quick-amount'"));
  check("amount field is focused", true);

  // Typing is keyboard entry, not a tap.
  adb("shell", "input", "text", "25000");
  await until("amount shown", async () => (await evaluate("document.getElementById('quick-amount')?.value")) === "25.000");
  check("typed digits are grouped as 25.000", true);

  await evaluate(
    "(() => { const i = [...document.querySelectorAll('dialog[open] input')].find(e => e.maxLength === 500); i.focus(); return true; })()",
  );
  adb("shell", "input", "text", note);
  await sleep(500);

  // Tap 2: Simpan, pinned above the keyboard.
  await tapElement("dialog[open] footer button[type=submit]", top, dpr);
  await until("sheet closed", async () => !(await evaluate("!!document.querySelector('dialog[open]')")));
  check("tap 2 saves and closes the sheet", true);

  const page = await invoke("transactions_list", { filter: { search: note }, cursor: null, limit: 10 });
  const row = page.items[0];
  check("exactly one row was saved", page.items.length === 1, String(page.items.length));
  check("amount is -25000 minor units", row.amountMinor === "-25000", row.amountMinor);
  check("it is an expense", row.kind === "expense", row.kind);
  check("it used the only account", row.accountId === account.id, row.accountName);
  check("it has a category", typeof row.categoryName === "string" && row.categoryName.length > 0, row.categoryName);

  const dialogs = adb("shell", "dumpsys", "window", "windows");
  check("no ANR dialog on screen", !/Application Not Responding/i.test(dialogs));
}

try {
  await main();
  console.log(`\nSMOKE OK (${results.length} checks)`);
} catch (error) {
  console.error(`\nSMOKE FAILED: ${error.message}`);
  process.exitCode = 1;
} finally {
  try {
    ws?.close();
    adb("forward", "--remove", `tcp:${PORT}`);
  } catch {
    // best effort
  }
}

// The DevTools socket would otherwise keep the event loop alive.
process.exit(process.exitCode ?? 0);
