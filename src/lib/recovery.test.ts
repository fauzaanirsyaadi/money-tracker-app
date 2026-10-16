import assert from "node:assert/strict";
import { test } from "node:test";

import type { StorageState } from "./api.ts";
import { recoveryText } from "./recovery.ts";

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
