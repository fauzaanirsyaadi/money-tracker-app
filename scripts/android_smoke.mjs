#!/usr/bin/env node
// On-device smoke test: installs the debug APK, then drives the quick-add flow
// with real adb taps (FAB -> type 25000 -> Simpan) and asserts the saved row.
//
//   node scripts/android_smoke.mjs [--apk PATH] [--no-install] [--serial ID] [--lock] [--biometric]
//
// With --lock it also sets a PIN and checks the app lock end to end: locked on
// cold start and after backgrounding, wrong and right PIN, the delay after five
// misses, and that data commands fail while locked.
//
// With --biometric it also exercises the fingerprint prompt. That needs an
// emulator with a fingerprint enrolled (device PIN set, then
// `am start -a android.settings.BIOMETRIC_ENROLL` and `adb emu finger touch 1`).
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

/** Polls every 500 ms for at most `ms` of *polling* (counted in tries, so a stalled host cannot trip it). */
async function until(what, fn, ms = 60_000) {
  const tries = Math.ceil(ms / 500);
  for (let attempt = 0; attempt < tries; attempt += 1) {
    try {
      const value = await fn();
      if (value) return value;
    } catch {
      // not ready yet
    }
    await sleep(500);
  }
  throw new Error(`timeout waiting for ${what}`);
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

/** Like invoke, but resolves to the error code instead of throwing. */
const invokeCode = (command, payload = {}) =>
  evaluate(
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(payload)}).then(() => 'ok', (e) => e?.code ?? 'error')`,
  );

// --- Real taps -------------------------------------------------------------

/** Top of the WebView in device pixels (status bar inset), from the view hierarchy. */
function webViewTop() {
  // `uiautomator dump` can fail while the screen is animating; retry briefly.
  for (let attempt = 0; ; attempt += 1) {
    try {
      adb("shell", "uiautomator", "dump", "/sdcard/mt-ui.xml");
      break;
    } catch (error) {
      if (attempt >= 4) throw error;
    }
  }
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

  const note = `smoke-${Date.now()}`; // typed after the amount; may arrive truncated under load
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

  // A fresh install has no other rows, so the saved one is found without the
  // note: under load `adb input text` can drop characters of the typed note,
  // which says nothing about the app.
  const page = await invoke("transactions_list", { filter: {}, cursor: null, limit: 10 });
  if (page.items.length !== 1) {
    // Say what was saved instead, so a typing glitch is told apart from a lost write.
    const all = await invoke("transactions_list", { filter: {}, cursor: null, limit: 5 });
    console.log(`INFO  rows in the database: ${JSON.stringify(all.items.map((t) => [t.amountMinor, t.note]))}`);
  }
  const row = page.items[0];
  check("exactly one row was saved", page.items.length === 1, String(page.items.length));
  check("amount is -25000 minor units", row.amountMinor === "-25000", row.amountMinor);
  check("it is an expense", row.kind === "expense", row.kind);
  check("it used the only account", row.accountId === account.id, row.accountName);
  check("it has a category", typeof row.categoryName === "string" && row.categoryName.length > 0, row.categoryName);

  const dialogs = adb("shell", "dumpsys", "window", "windows");
  check("no ANR dialog on screen", !/Application Not Responding/i.test(dialogs));

  const windows = adb("shell", "dumpsys", "window", "windows");
  const main = /Window #\d+ Window\{[^}]*com\.moneytracker\.app\/[^}]*\}:\n\s+(?:.*\n)*?\s+fl=([^\n]*)/.exec(windows);
  check("FLAG_SECURE is set on the main window", /\bSECURE\b/.test(main?.[1] ?? ""), main?.[1]?.slice(0, 60));

  if (flag("--lock")) await lockFlow(top, dpr);
  if (flag("--biometric")) await biometricFlow();
}

const PIN = "493017";

async function startApp() {
  adb("shell", "am", "force-stop", PACKAGE);
  adb("shell", "am", "start", "-n", ACTIVITY);
  await until("the web view", async () => {
    try {
      ws?.close();
      await connect();
      return await evaluate("typeof window.__TAURI_INTERNALS__ !== 'undefined'");
    } catch {
      return false;
    }
  });
}

const finger = (id) => adb("emu", "finger", "touch", String(id));

async function biometricFlow() {
  const lockScreenShown = () => evaluate("!!document.querySelector('#lock-pin')");
  const first = await invoke("lock_setup", { pin: PIN });
  check("a fingerprint is enrolled on this device", first.biometric === "available", first.biometric);

  // Enabling proves the prompt works: it resolves only after a touch.
  const enabling = invoke("lock_biometric_set", { enabled: true });
  await sleep(2500);
  finger(1);
  const enabled = await enabling;
  check("a touch enables biometric unlock", enabled.biometricEnabled === true);

  // Cold start: the prompt appears by itself; the enrolled finger unlocks.
  await startApp();
  await until("lock screen", lockScreenShown);
  await sleep(2500);
  finger(1);
  await until("unlocked by fingerprint", async () => !(await lockScreenShown()));
  check("the enrolled finger unlocks", true);
  check("data commands work after a fingerprint unlock", (await invokeCode("accounts_list", { includeArchived: false })) === "ok");

  // Back cancels the prompt and leaves the PIN path open, without an error.
  adb("shell", "input", "keyevent", "KEYCODE_HOME");
  await sleep(1500);
  adb("shell", "am", "start", "-n", ACTIVITY);
  await until("lock screen", lockScreenShown);
  await sleep(2500);
  adb("shell", "input", "keyevent", "KEYCODE_BACK");
  await sleep(1500);
  check("cancelling the prompt stays locked", await lockScreenShown());
  check("cancelling shows no error", !(await evaluate("document.body.innerText.includes('Biometrik tidak berhasil')")));

  // A finger that is not enrolled does not unlock.
  await evaluate("document.querySelector('button.btn.ghost')?.click()");
  await sleep(2000);
  finger(2);
  await sleep(1500);
  check("an unenrolled finger does not unlock", await lockScreenShown());
  finger(1);
  await until("unlocked", async () => !(await lockScreenShown()));
  check("the enrolled finger still works afterwards", true);

  check("the lock can be removed", (await invokeCode("lock_remove", { current: PIN })) === "ok");
}

async function lockFlow(top, dpr) {
  const typeInto = async (text) => {
    await tapElement("#lock-pin", top, dpr);
    adb("shell", "input", "text", text);
    await sleep(300);
    adb("shell", "input", "keyevent", "KEYCODE_BACK"); // hide the keyboard
    await sleep(500);
    await tapElement("form button[type=submit]", top, dpr);
  };
  const lockScreenShown = () => evaluate("!!document.querySelector('#lock-pin')");
  const status = () => invoke("lock_status");

  const first = await invoke("lock_setup", { pin: PIN });
  check("setting a PIN leaves the app unlocked", first.configured && !first.locked);
  check("a trivial PIN is refused", (await invokeCode("lock_setup", { pin: "123456" })) === "invalid_input");

  // Cold start: the database must not be opened until the PIN is entered.
  await startApp();
  await until("lock screen", lockScreenShown);
  check("cold start shows the lock screen", true);
  check("data commands fail with `locked` while locked", (await invokeCode("accounts_list", { includeArchived: false })) === "locked");

  // Wrong PIN, by real taps.
  await typeInto("000001");
  await until("error shown", () => evaluate("document.body.innerText.includes('PIN salah')"));
  check("a wrong PIN is refused and explained", true);
  check("the miss was counted", (await status()).failedAttempts === 1);

  // Right PIN.
  await typeInto(PIN);
  await until("unlocked", async () => !(await lockScreenShown()));
  check("the right PIN unlocks", true);
  check("the miss counter reset", (await status()).failedAttempts === 0);
  check("data commands work again", (await invokeCode("accounts_list", { includeArchived: false })) === "ok");

  // Backgrounding locks (grace is immediate by default).
  adb("shell", "input", "keyevent", "KEYCODE_HOME");
  await sleep(1500);
  adb("shell", "am", "start", "-n", ACTIVITY);
  await until("lock screen after resume", lockScreenShown);
  check("leaving the app locks it", true);
  check("data commands fail again after backgrounding", (await invokeCode("accounts_list", { includeArchived: false })) === "locked");

  // Five misses start a delay that even the right PIN must wait out.
  for (let i = 0; i < 5; i += 1) {
    check(`miss ${i + 1} is wrong_pin`, (await invokeCode("lock_unlock_pin", { pin: "000001" })) === "wrong_pin");
  }
  const delayed = await status();
  check("the fifth miss starts a delay", delayed.retryAfterMs > 25_000 && delayed.retryAfterMs <= 30_000, String(delayed.retryAfterMs));
  check("the right PIN is refused during the delay", (await invokeCode("lock_unlock_pin", { pin: PIN })) === "locked_out");

  // A restart must not shorten it.
  await startApp();
  await until("lock screen", lockScreenShown);
  const afterRestart = await status();
  check("restarting keeps the miss count", afterRestart.failedAttempts >= 5, String(afterRestart.failedAttempts));
  check("restarting keeps a delay", afterRestart.retryAfterMs > 0, String(afterRestart.retryAfterMs));

  console.log("waiting out the delay...");
  await sleep(Math.min(afterRestart.retryAfterMs, 60_000) + 1500);
  check("the right PIN works after the delay", (await invokeCode("lock_unlock_pin", { pin: PIN })) === "ok");

  // Clean up so the device is left without a lock.
  check("the lock can be removed with the PIN", (await invokeCode("lock_remove", { current: PIN })) === "ok");
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
