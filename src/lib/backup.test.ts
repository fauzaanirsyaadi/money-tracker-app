import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import {
  BACKUP_STALE_DAYS,
  backupAgeDays,
  backupAgeText,
  backupBanner,
  backupErrorMessage,
  backupFileName,
  passphraseProblem,
  passphraseVerdict,
} from "./backup.ts";

test("short, long and repetitive passphrases are explained", () => {
  assert.match(passphraseProblem("pendek")!, /minimal 12/);
  assert.match(passphraseProblem("elevenchars")!, /sekarang 11/);
  assert.match(passphraseProblem("a".repeat(129))!, /maksimal 128/);
  assert.match(passphraseProblem("aaaaaaaaaaaaaaa")!, /terlalu mudah/);
});

test("a twelve character passphrase is accepted, counting characters not bytes", () => {
  assert.equal(passphraseProblem("twelve chars"), null);
  assert.equal(passphraseProblem("あいうえおかきくけこさし"), null);
  assert.equal(passphraseProblem("kopi susu gula aren enak"), null);
});

test("a compatibility character cannot stand in for a long passphrase", () => {
  assert.match(passphraseProblem("\ufdfa")!, /minimal 12/); // U+FDFA is 18 characters once normalised
  assert.match(passphraseProblem("\ufdfd")!, /minimal 12/); // U+FDFD stays one character
  assert.match(passphraseProblem("\ufb03".repeat(4))!, /minimal 12/); // ffi ligature x4
  assert.match(passphraseProblem("\u337f".repeat(3))!, /minimal 12/); // 12 once normalised
  assert.match(passphraseProblem("\u337f".repeat(12))!, /terlalu mudah/);
  assert.match(passphraseProblem("\uff21".repeat(12))!, /terlalu mudah/); // full-width A
});

test("the policy counts the normalised form, like the backup does", () => {
  // Twelve decomposed letters (24 code points) are twelve characters.
  assert.equal(passphraseProblem("e\u0301".repeat(6) + "a\u0301".repeat(6)), null);
  // Full-width letters fold to ASCII: this is still too short after folding.
  assert.match(passphraseProblem("ＡＢＣ")!, /minimal 12/);
});

test("the first-chunk failure says it may be the passphrase or the file", () => {
  assert.equal(backupErrorMessage("wrong_passphrase"), "Frasa sandi salah atau cadangan rusak.");
  assert.match(backupErrorMessage("backup_damaged"), /^Cadangan rusak/);
});

test("file names carry the date", () => {
  assert.equal(backupFileName("2026-10-14"), "money-tracker-20261014.mtbak");
});

test("every backup failure has its own message and never blames the data silently", () => {
  const codes = ["wrong_passphrase", "not_a_backup", "unsupported_backup", "backup_damaged", "weak_passphrase"] as const;
  const messages = new Set(codes.map(backupErrorMessage));
  assert.equal(messages.size, codes.length);
  assert.match(backupErrorMessage("internal"), /tidak diubah/);
});

const DAY = 86_400_000;
const NOW = Date.UTC(2026, 9, 17, 12, 0, 0);

test("the backup age counts whole days from the last backup, or from first use", () => {
  assert.equal(backupAgeDays({ lastBackupAtMs: NOW - 3 * DAY - 1000, sinceMs: NOW - 90 * DAY }, NOW), 3);
  assert.equal(backupAgeDays({ lastBackupAtMs: null, sinceMs: NOW - 10 * DAY }, NOW), 10);
  assert.equal(backupAgeDays({ lastBackupAtMs: NOW - DAY + 1, sinceMs: 0 }, NOW), 0);
});

test("a stamp in the future counts as today and never alarms", () => {
  const status = { lastBackupAtMs: NOW + 5 * DAY, sinceMs: NOW - 30 * DAY };
  assert.equal(backupAgeDays(status, NOW), 0);
  assert.equal(backupBanner(status, NOW), null);
});

test("the card always says how old the last backup is", () => {
  assert.equal(backupAgeText({ lastBackupAtMs: NOW - 2 * DAY, sinceMs: 0 }, NOW), "Terakhir backup 2 hari lalu.");
  assert.equal(backupAgeText({ lastBackupAtMs: NOW - 1000, sinceMs: 0 }, NOW), "Terakhir backup hari ini.");
  assert.equal(backupAgeText({ lastBackupAtMs: null, sinceMs: NOW }, NOW), "Belum pernah membuat cadangan.");
});

test("the banner appears only once the backup is stale", () => {
  const at = (days: number) => ({ lastBackupAtMs: NOW - days * DAY, sinceMs: 0 });
  assert.equal(backupBanner(at(BACKUP_STALE_DAYS - 1), NOW), null);
  assert.equal(backupBanner(at(BACKUP_STALE_DAYS), NOW), "Terakhir backup 7 hari lalu. Buat cadangan baru.");
  assert.match(backupBanner(at(30), NOW)!, /30 hari lalu/);
});

test("never backed up: quiet at first, then a banner that says for how long", () => {
  const fresh = { lastBackupAtMs: null, sinceMs: NOW - 2 * DAY };
  assert.equal(backupBanner(fresh, NOW), null);
  const old = { lastBackupAtMs: null, sinceMs: NOW - 9 * DAY };
  assert.match(backupBanner(old, NOW)!, /belum pernah dicadangkan \(9 hari sejak dipakai\)/);
});

// The same vectors run in Rust (crates/backup), which is the authority. Here the
// UI's advice must never disagree with it.
test("the shared passphrase vectors give the verdicts the backend gives", () => {
  const file = new URL("../../testdata/passphrase_vectors.json", import.meta.url);
  const { vectors } = JSON.parse(readFileSync(file, "utf8")) as {
    vectors: { name: string; passphrase: string; verdict: string }[];
  };
  assert.ok(vectors.length >= 20);
  for (const vector of vectors) {
    assert.equal(passphraseVerdict(vector.passphrase), vector.verdict, vector.name);
    // The message follows the verdict: advice exists exactly when the backend refuses.
    assert.equal(passphraseProblem(vector.passphrase) === null, vector.verdict === "ok", vector.name);
  }
});
