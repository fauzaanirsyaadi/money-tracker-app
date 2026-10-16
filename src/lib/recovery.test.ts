import assert from "node:assert/strict";
import { test } from "node:test";

import type { StorageState } from "./api.ts";
import { confirmationMatches, needsPin, recoveryText, START_FRESH_WORD } from "./recovery.ts";

const LOSS: StorageState[] = ["database_missing", "key_missing", "key_unusable", "database_unreadable"];

test("every kind of loss explains itself and offers recovery", () => {
  const titles = new Set<string>();
  for (const state of LOSS) {
    const text = recoveryText(state);
    assert.ok(text.title.length > 0 && text.explanation.length > 0, state);
    assert.equal(text.canRecover, true, state);
    assert.match(text.explanation, /tidak ada yang dibuat ulang|tidak dapat dibaca|tidak dapat dibuka/i, state);
    titles.add(text.title);
  }
  assert.equal(titles.size, LOSS.length, "each reason has its own title");
});

test("a transient failure only offers trying again and promises nothing was changed", () => {
  const text = recoveryText("retry");
  assert.equal(text.canRecover, false);
  assert.match(text.explanation, /tidak diubah/);
});

test("a healthy state has no recovery text", () => {
  assert.deepEqual(recoveryText("ok"), { title: "", explanation: "", canRecover: false });
});

test("only the exact word confirms losing the data", () => {
  assert.equal(START_FRESH_WORD, "HAPUS");
  assert.equal(confirmationMatches("HAPUS"), true);
  assert.equal(confirmationMatches("  HAPUS  "), true);
  for (const typed of ["", "hapus", "Hapus", "YA", "HAPUS!", "H A P U S"]) {
    assert.equal(confirmationMatches(typed), false, typed);
  }
});

test("the PIN is asked first whenever a valid lock exists and was not just confirmed", () => {
  assert.equal(needsPin({ configured: true, broken: false, recoveryAuthenticated: false }), true);
  assert.equal(needsPin({ configured: true, broken: false, recoveryAuthenticated: true }), false);
  // No lock, or a damaged lock file that cannot be checked against: no PIN to ask for.
  assert.equal(needsPin({ configured: false, broken: false, recoveryAuthenticated: false }), false);
  assert.equal(needsPin({ configured: true, broken: true, recoveryAuthenticated: false }), false);
});
