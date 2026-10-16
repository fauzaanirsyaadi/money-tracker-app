import assert from "node:assert/strict";
import { test } from "node:test";

import { backupErrorMessage, backupFileName, passphraseProblem } from "./backup.ts";

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
