import assert from "node:assert/strict";
import { test } from "node:test";

import {
  type KeyValueStore,
  loadLastUsed,
  loadTheme,
  nextTheme,
  parseTheme,
  saveLastUsed,
  saveTheme,
} from "./prefs.ts";

function memory(): KeyValueStore & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => void data.set(key, value),
  };
}

const broken: KeyValueStore = {
  getItem() {
    throw new Error("denied");
  },
  setItem() {
    throw new Error("denied");
  },
};

test("theme cycles auto -> light -> dark -> auto", () => {
  assert.equal(nextTheme("auto"), "light");
  assert.equal(nextTheme("light"), "dark");
  assert.equal(nextTheme("dark"), "auto");
});

test("unknown stored themes fall back to auto", () => {
  assert.equal(parseTheme("dark"), "dark");
  assert.equal(parseTheme("sepia"), "auto");
  assert.equal(parseTheme(null), "auto");
});

test("theme round-trips through a store", () => {
  const store = memory();
  assert.equal(loadTheme(store), "auto");
  saveTheme("dark", store);
  assert.equal(loadTheme(store), "dark");
});

test("last-used merges updates and ignores junk", () => {
  const store = memory();
  saveLastUsed({ accountId: 3 }, store);
  saveLastUsed({ expenseCategoryId: 7 }, store);
  assert.deepEqual(loadLastUsed(store), { accountId: 3, expenseCategoryId: 7 });

  store.data.set("mt.quickadd.last", JSON.stringify({ accountId: "x", incomeCategoryId: -2, expenseCategoryId: 4.5 }));
  assert.deepEqual(loadLastUsed(store), {});
  store.data.set("mt.quickadd.last", "{not json");
  assert.deepEqual(loadLastUsed(store), {});
});

test("an unavailable store never throws", () => {
  assert.equal(loadTheme(broken), "auto");
  assert.deepEqual(loadLastUsed(broken), {});
  saveTheme("light", broken);
  saveLastUsed({ accountId: 1 }, broken);
  assert.equal(loadTheme(null), "auto");
});
