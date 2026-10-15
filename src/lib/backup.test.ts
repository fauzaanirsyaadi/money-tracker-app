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

test("file names carry the date", () => {
  assert.equal(backupFileName("2026-10-14"), "money-tracker-20261014.mtbak");
});

test("every backup failure has its own message and never blames the data silently", () => {
  const codes = ["wrong_passphrase", "not_a_backup", "unsupported_backup", "backup_damaged", "weak_passphrase"] as const;
  const messages = new Set(codes.map(backupErrorMessage));
  assert.equal(messages.size, codes.length);
  assert.match(backupErrorMessage("internal"), /tidak diubah/);
});
