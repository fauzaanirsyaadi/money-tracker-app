#!/usr/bin/env node
// End-to-end backup, restore and recovery on a device or emulator, through the
// real system file pickers:
//
//   seed data -> set PIN -> export (picker) -> check the file ->
//   uninstall -> reinstall -> restore (picker) -> compare everything ->
//   damage the installation (database gone, key damaged, key gone, lock only)
//   and check that nothing is created silently and each way out works.
//
//   node scripts/android_backup_e2e.mjs [--apk PATH] [--serial ID]
//
// Needs a debug build (the DevTools channel reads and asserts state; the
// pickers are driven with real input events). Uses the system "Files" UI, so
// selectors are looked up by text and retried.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PACKAGE = "com.moneytracker.app";
const ACTIVITY = `${PACKAGE}/.MainActivity`;
const PORT = 9222;
const PASSPHRASE = "kopi susu gula aren enak";
const FILE = "money-tracker-e2e.mtbak";
const PIN = "493017";

const args = process.argv.slice(2);
const option = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT;
if (!sdk) throw new Error("set ANDROID_HOME");
const adbBin = path.join(sdk, "platform-tools", "adb");
const serial = option("--serial");
const apk =
  option("--apk") ??
  path.join(root, "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk");
const adb = (...a) =>
  execFileSync(adbBin, [...(serial ? ["-s", serial] : []), ...a], { encoding: "utf8", maxBuffer: 64 << 20 }).trim();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let checks = 0;
function check(name, ok, detail = "") {
  checks += 1;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  -> ${detail}` : ""}`);
  if (!ok) throw new Error(`check failed: ${name}`);
}

/** Polls every 500 ms for at most `ms` of *polling* (counted in tries, not wall clock). */
async function until(what, fn, ms = 60_000) {
  const tries = Math.ceil(ms / 500);
  for (let attempt = 0; attempt < tries; attempt += 1) {
    try {
      const value = await fn();
      if (value) return value;
    } catch {
      // not ready
    }
    await sleep(500);
  }
  throw new Error(`timeout waiting for ${what}`);
}

// --- DevTools -----------------------------------------------------------

let ws;
let nextId = 0;
const pending = new Map();

async function connect() {
  const pid = adb("shell", "pidof", PACKAGE);
  adb("forward", `tcp:${PORT}`, `localabstract:webview_devtools_remote_${pid}`);
  const targets = await (await fetch(`http://localhost:${PORT}/json`)).json();
  const target = targets.find((t) => t.url?.includes("tauri.localhost")) ?? targets[0];
  try {
    ws?.close();
  } catch {
    // already closed
  }
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

function evaluate(expression) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, (m) =>
      m.result?.exceptionDetails
        ? reject(new Error(m.result.exceptionDetails.exception?.description ?? "page exception"))
        : resolve(m.result?.result?.value),
    );
    ws.send(
      JSON.stringify({ id, method: "Runtime.evaluate", params: { expression, awaitPromise: true, returnByValue: true } }),
    );
  });
}

/** Invokes a command; resolves `{ok: value}` or `{code}` so errors can be asserted. */
const call = (command, payload = {}) =>
  evaluate(
    `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(payload)}).then((v) => ({ ok: v }), (e) => ({ code: e?.code ?? 'error' }))`,
  );

async function must(command, payload) {
  const r = await call(command, payload);
  if (r.code) throw new Error(`${command} failed: ${r.code}`);
  return r.ok;
}

async function startApp() {
  adb("shell", "am", "force-stop", PACKAGE);
  adb("shell", "am", "start", "-n", ACTIVITY);
  await until("the web view", async () => {
    await connect();
    return evaluate("typeof window.__TAURI_INTERNALS__ !== 'undefined'");
  });
}

// --- System picker ------------------------------------------------------

function dumpNodes() {
  adb("shell", "uiautomator", "dump", "/sdcard/mt-picker.xml");
  const xml = adb("shell", "cat", "/sdcard/mt-picker.xml");
  const nodes = [];
  for (const m of xml.matchAll(/<node[^>]*?text="([^"]*)"[^>]*?resource-id="([^"]*)"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/g)) {
    nodes.push({
      text: m[1],
      id: m[2],
      x: Math.round((Number(m[3]) + Number(m[5])) / 2),
      y: Math.round((Number(m[4]) + Number(m[6])) / 2),
    });
  }
  return nodes;
}

function topActivity() {
  return adb("shell", "dumpsys", "activity", "activities").match(/topResumedActivity=.*/)?.[0] ?? "";
}

/** Taps the first node whose text matches, dismissing "isn't responding" dialogs on the way. */
async function tapWhen(label, predicate, ms = 60_000) {
  await until(label, () => {
    let nodes;
    try {
      nodes = dumpNodes();
    } catch {
      return false;
    }
    const wait = nodes.find((n) => n.id === "android:id/aerr_wait");
    if (wait) {
      adb("shell", "input", "tap", String(wait.x), String(wait.y));
      return false;
    }
    const hit = nodes.find(predicate);
    if (!hit) return false;
    adb("shell", "input", "tap", String(hit.x), String(hit.y));
    return true;
  }, ms);
}

const inPicker = () => /documentsui/.test(topActivity());

// --- Snapshot -----------------------------------------------------------

async function snapshot() {
  const accounts = (await must("accounts_list", { includeArchived: true })).map((a) => ({
    name: a.name,
    balance: a.balanceMinor,
    archived: a.archived,
  }));
  const rows = [];
  let cursor = null;
  do {
    const page = await must("transactions_list", { filter: {}, cursor, limit: 100 });
    for (const t of page.items) {
      rows.push([t.accountName, t.categoryName, t.kind, t.amountMinor, t.note, t.occurredOn]);
    }
    cursor = page.next;
  } while (cursor);
  const categories = (await must("categories_list", { includeArchived: true })).length;
  return { accounts, rows, categories };
}

// --- Flow ---------------------------------------------------------------

async function seed() {
  const dompet = await must("account_create", { input: { name: "Dompet", kind: "cash", openingBalanceMinor: "1500000" } });
  const tabungan = await must("account_create", { input: { name: "Tabungan", kind: "bank", openingBalanceMinor: "2500000" } });
  const categories = await must("categories_list", { includeArchived: false });
  const expense = categories.find((c) => c.kind === "expense").id;
  const income = categories.find((c) => c.kind === "income").id;
  for (let i = 0; i < 40; i += 1) {
    await must("transaction_create", {
      input: {
        accountId: i % 2 === 0 ? dompet.id : tabungan.id,
        categoryId: i % 5 === 0 ? income : expense,
        kind: i % 5 === 0 ? "income" : "expense",
        amountMinor: String(1000 + i * 137),
        note: `catatan ${i} "uji" 🍛`,
        occurredOn: `2026-10-${String(1 + (i % 28)).padStart(2, "0")}`,
      },
    });
  }
  await must("transfer_create", {
    input: { fromAccountId: tabungan.id, toAccountId: dompet.id, amountMinor: "250000", note: "tarik tunai", occurredOn: "2026-10-05" },
  });
}

// --- Recovery: damage the installation, check the app's response -------------

/** Runs a shell command inside the app's sandbox (debug builds only). */
const runAs = (command) => adb("shell", "run-as", PACKAGE, "sh", "-c", `"${command}"`);

function dataDir() {
  const found = runAs("find . -name money.db -not -path './cache/*' | head -1");
  return found ? path.posix.dirname(found) : "files";
}

const filesIn = (dir) => runAs(`ls ${dir}`).split("\n").filter(Boolean).sort();

async function storage() {
  return (await must("lock_status")).storage;
}

/** Stops the app, applies `damage`, starts it again. */
async function damageAndStart(damage) {
  adb("shell", "am", "force-stop", PACKAGE);
  damage();
  await startApp();
}

async function restoreThroughPicker(before, label, pin = null) {
  const picking = call("backup_restore_pick");
  await until("open picker", async () => inPicker(), 240_000);
  await tapWhen("the backup file", (n) => n.text === FILE, 240_000);
  const picked = await picking;
  check(`${label}: the backup file was chosen`, picked.ok === true);
  await until("app in front", async () => topActivity().includes(PACKAGE));
  const preview = await must("backup_restore_prepare", { passphrase: PASSPHRASE });
  check(`${label}: the preview shows the backup, with nothing current to compare`, preview.backup.transactions === 42 && preview.current.transactions === 0, JSON.stringify(preview));
  if (pin !== null) {
    // A valid lock guards the destructive step: the PIN first.
    const early = await call("backup_restore_commit");
    check(`${label}: restore is refused before the PIN`, early.code === "pin_required", early.code);
    check(`${label}: the refusal changed nothing`, (await storage()) !== "ok");
    const wrongPin = await call("recovery_authenticate", { pin: "000001" });
    check(`${label}: a wrong PIN is refused`, wrongPin.code === "wrong_pin", wrongPin.code);
    await must("recovery_authenticate", { pin });
    // The refusal left the staged restore waiting: the commit below needs no second prepare.
  }
  await must("backup_restore_commit");
  check(`${label}: storage is usable again`, (await storage()) === "ok");
  check(`${label}: the restored data is identical`, JSON.stringify(await snapshot()) === JSON.stringify(before));
}

async function recoveryScenarios(before) {
  const dir = dataDir();
  const db = `${dir}/money.db`;
  const key = `${dir}/db.key`;

  // 1. The database file is gone but the key is not: never recreate it silently.
  await damageAndStart(() => runAs(`rm -f ${db} ${db}-wal ${db}-shm`));
  check("database missing is reported", (await storage()) === "database_missing");
  check("no database was created behind the user's back", !filesIn(dir).includes("money.db"), filesIn(dir).join(","));
  check("data commands are refused meanwhile", (await call("accounts_list", { includeArchived: false })).code === "unavailable");
  check("starting fresh without the typed word is refused", (await call("recovery_start_fresh", { typed: "" })).code === "invalid_input");
  check("a lower-case word does not confirm", (await call("recovery_start_fresh", { typed: "hapus" })).code === "invalid_input");
  check("the refusal changed nothing", (await storage()) === "database_missing" && !filesIn(dir).includes("money.db"));
  await restoreThroughPicker(before, "database missing");

  // 2. A zero-byte database counts as missing too.
  await damageAndStart(() => runAs(`: > ${db}; rm -f ${db}-wal ${db}-shm`));
  check("an empty database file is reported as missing", (await storage()) === "database_missing");
  check("the empty file was left alone", runAs(`wc -c < ${db}`).trim() === "0");

  // 3. The key blob is damaged: the data cannot be opened and nothing is replaced.
  await restoreThroughPicker(before, "empty file").catch(() => {
    throw new Error("restore after an empty database failed");
  });
  const dbSizeBefore = runAs(`wc -c < ${db}`).trim();
  await damageAndStart(() => runAs(`head -c 60 /dev/urandom > ${key}`));
  check("a damaged key is reported as unusable", (await storage()) === "key_unusable");
  check("the database was not touched", runAs(`wc -c < ${db}`).trim() === dbSizeBefore);
  await restoreThroughPicker(before, "unusable key");

  // 4. The key file is gone while the database exists.
  const keyBlobBefore = runAs(`wc -c < ${key}`).trim();
  await damageAndStart(() => runAs(`rm -f ${key}`));
  check("a missing key file is reported", (await storage()) === "key_missing");
  check("no key was created behind the user's back", !filesIn(dir).includes("db.key"), keyBlobBefore);
  check("starting fresh needs the typed word", (await call("recovery_start_fresh", { typed: "YA" })).code === "invalid_input");
  const fresh = await must("recovery_start_fresh", { typed: "HAPUS" });
  check("starting fresh gives a usable, empty database", fresh.storage === "ok" && (await snapshot()).rows.length === 0);
  check("starting fresh created the key and the database", ["db.key", "money.db"].every((f) => filesIn(dir).includes(f)));

  // 5. A lock with no data behind it: the PIN does not make the data reappear,
  //    and recovery replaces the lock together with the data.
  await must("lock_setup", { pin: PIN });
  await damageAndStart(() => runAs(`rm -f ${db} ${db}-wal ${db}-shm ${key}`));
  const locked = await must("lock_status");
  check("a lock with no data is reported as lost data, not as a locked app", locked.storage === "database_missing" && locked.configured === true);
  check("recovery asks for the PIN before anything destructive", locked.recoveryAuthenticated === false);
  const noWord = await call("recovery_start_fresh", { typed: "HAPUS" });
  check("starting fresh is refused without the PIN even with the word", noWord.code === "pin_required", noWord.code);
  check("the refusal changed nothing", (await storage()) === "database_missing" && !filesIn(dir).includes("money.db"));
  await restoreThroughPicker(before, "lock without data", PIN);
  const afterLock = await must("lock_status");
  check("the old PIN is gone after the data was replaced", afterLock.configured === false && afterLock.locked === false);
  check("no markers of the swap remain", !filesIn(dir).some((f) => /\.(old|new)$/.test(f)), filesIn(dir).join(","));

  // 6. A false alarm: the key file comes back before the destructive step. The
  //    second look finds the data readable and nothing is destroyed.
  await must("lock_setup", { pin: PIN });
  runAs(`cp ${key} ${dir}/key.keep`);
  await damageAndStart(() => runAs(`rm -f ${key}`));
  check("a missing key is reported", (await storage()) === "key_missing");
  await must("recovery_authenticate", { pin: PIN });
  runAs(`cp ${dir}/key.keep ${key}`);
  const alarm = await call("recovery_start_fresh", { typed: "HAPUS" });
  check("the second look refuses to destroy readable data", alarm.code === "invalid_input", alarm.code);
  check("the data and the PIN are intact after the false alarm", (await must("lock_status")).configured === true && filesIn(dir).includes("money.db"));
  runAs(`rm -f ${dir}/key.keep`);

  // 7. With a valid lock, starting fresh needs the PIN as well as the word, and
  //    then leaves a working empty database and no old files.
  const status7 = await must("lock_status");
  check("the lock is still in place", status7.configured === true);
  await damageAndStart(() => runAs(`rm -f ${db} ${db}-wal ${db}-shm`));
  check("data lost behind a valid lock is reported", (await storage()) === "database_missing");
  const wrongPin = await call("recovery_authenticate", { pin: "000001" });
  check("a wrong PIN does not open the destructive steps", wrongPin.code === "wrong_pin");
  await must("recovery_authenticate", { pin: PIN });
  const fresh7 = await must("recovery_start_fresh", { typed: "HAPUS" });
  check("starting fresh behind a lock works with PIN and word", fresh7.storage === "ok" && fresh7.configured === false);
  check("it left a usable empty database", (await snapshot()).rows.length === 0);
  check("no markers or staged files remain", !filesIn(dir).some((f) => /\.(old|new)$|restore-staged/.test(f)), filesIn(dir).join(","));
}

try {
  try {
    adb("uninstall", PACKAGE);
  } catch {
    // not installed
  }
  adb("install", apk);
  await startApp();
  check("fresh install starts", true);

  await seed();
  const before = await snapshot();
  check("seeded 2 accounts and 41 rows", before.accounts.length === 2 && before.rows.length === 42, `${before.rows.length} rows`);

  const status = await must("lock_setup", { pin: PIN });
  check("a PIN is set (auto-lock: immediately)", status.configured && !status.locked);

  // A weak passphrase is refused before the picker opens.
  const weak = await call("backup_export", { passphrase: "pendek", name: FILE });
  check("a short passphrase is refused", weak.code === "weak_passphrase", weak.code);
  check("no picker opened for it", !inPicker());

  // Export through the picker. The app is backgrounded by it and must stay unlocked.
  const exporting = call("backup_export", { passphrase: PASSPHRASE, name: FILE });
  await until("save picker", async () => inPicker(), 240_000);
  check("the system save picker is shown", true);
  // The picker's confirm button is the dialog's positive button, labelled SAVE.
  await tapWhen("Save button", (n) => n.id === "android:id/button1" && /^save$/i.test(n.text));
  const exported = await exporting;
  check("export reports saved", exported.ok?.saved === true, JSON.stringify(exported).slice(0, 120));
  await until("app in front", async () => topActivity().includes(PACKAGE));
  check("the picker did not lock the app", (await must("lock_status")).locked === false);

  // The file on the device.
  const dir = mkdtempSync(path.join(tmpdir(), "mt-e2e-"));
  const local = path.join(dir, FILE);
  adb("pull", `/sdcard/Download/${FILE}`, local);
  const bytes = readFileSync(local);
  check("the file starts with the container magic", bytes.subarray(0, 4).toString() === "MTBK");
  check("the file is not trivially small", bytes.length > 500, `${bytes.length} bytes`);
  const text = bytes.toString("latin1");
  check("the file contains no plaintext", !["Dompet", "Tabungan", "catatan", "money-tracker-logical"].some((s) => text.includes(s)));

  // Cancelling the open picker changes nothing.
  const pickingCancel = call("backup_restore_pick");
  await until("open picker", async () => inPicker(), 240_000);
  adb("shell", "input", "keyevent", "KEYCODE_BACK");
  const cancelled = await pickingCancel;
  check("cancelling the picker resolves false", cancelled.ok === false, JSON.stringify(cancelled));
  await until("app in front", async () => topActivity().includes(PACKAGE));
  check("the app is still unlocked after the cancel", (await must("lock_status")).locked === false);

  // A new installation.
  adb("uninstall", PACKAGE);
  adb("install", apk);
  await startApp();
  const empty = await snapshot();
  check("the reinstalled app has no accounts", empty.accounts.length === 0 && empty.rows.length === 0);

  // Restore: pick the file in the system picker.
  const picking = call("backup_restore_pick");
  await until("open picker", async () => inPicker(), 240_000);
  await tapWhen("the backup file", (n) => n.text === FILE, 90_000);
  const picked = await picking;
  check("the file was chosen", picked.ok === true, JSON.stringify(picked));
  await until("app in front", async () => topActivity().includes(PACKAGE));

  const wrong = await call("backup_restore_prepare", { passphrase: "bukan frasa sandi yang benar" });
  check("a wrong passphrase is reported", wrong.code === "wrong_passphrase", wrong.code);
  check("nothing changed after the wrong passphrase", (await snapshot()).accounts.length === 0);

  const preview = await must("backup_restore_prepare", { passphrase: PASSPHRASE });
  check("the preview shows the backup contents", preview.backup.accounts === 2 && preview.backup.transactions === 42, JSON.stringify(preview.backup));
  check("the preview shows what will be replaced", preview.current.accounts === 0);
  check("live data is untouched before confirmation", (await snapshot()).accounts.length === 0);

  const committed = await must("backup_restore_commit");
  check("restore reports the counts", committed.accounts === 2 && committed.transactions === 42);

  const after = await snapshot();
  check("accounts and balances are identical", JSON.stringify(after.accounts) === JSON.stringify(before.accounts));
  check("every transaction is identical", JSON.stringify(after.rows) === JSON.stringify(before.rows), `${after.rows.length} rows`);
  check("categories are identical", after.categories === before.categories);
  check("the PIN did not travel with the backup", (await must("lock_status")).configured === false);

  // Restore over existing data replaces it.
  const extra = (await must("accounts_list", { includeArchived: false }))[0];
  const cats = await must("categories_list", { includeArchived: false });
  await must("transaction_create", {
    input: { accountId: extra.id, categoryId: cats.find((c) => c.kind === "expense").id, kind: "expense", amountMinor: "999", note: "tambahan", occurredOn: "2026-10-14" },
  });
  check("an extra row exists before the second restore", (await snapshot()).rows.length === 43);
  const picking2 = call("backup_restore_pick");
  await until("open picker", async () => inPicker(), 240_000);
  await tapWhen("the backup file", (n) => n.text === FILE, 90_000);
  await picking2;
  await until("app in front", async () => topActivity().includes(PACKAGE));
  const preview2 = await must("backup_restore_prepare", { passphrase: PASSPHRASE });
  check("the second preview shows the current data", preview2.current.transactions === 43);
  await must("backup_restore_commit");
  const again = await snapshot();
  check("restoring over existing data replaces it", JSON.stringify(again) === JSON.stringify(before));

  // The result survives a restart (the swap persisted and the key matches).
  await startApp();
  const restarted = await snapshot();
  check("data is still there after a restart", JSON.stringify(restarted) === JSON.stringify(before));

  await recoveryScenarios(before);

  console.log(`\nBACKUP E2E OK (${checks} checks)`);
  process.exitCode = 0;
} catch (error) {
  console.error(`\nBACKUP E2E FAILED: ${error.message}`);
  process.exitCode = 1;
} finally {
  try {
    ws?.close();
    adb("forward", "--remove", `tcp:${PORT}`);
  } catch {
    // best effort
  }
  process.exit(process.exitCode ?? 0);
}
