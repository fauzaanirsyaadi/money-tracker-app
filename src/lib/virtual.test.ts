import assert from "node:assert/strict";
import { test } from "node:test";

import { nearEnd, visibleRange } from "./virtual.ts";

test("renders only the rows in view plus overscan", () => {
  // 100 rows of 60 px, a 600 px viewport scrolled to row 20.
  const range = visibleRange(1200, 600, 60, 100, 4);
  assert.deepEqual(range, { start: 16, end: 34 });
  assert.ok(range.end - range.start < 25);
});

test("clamps at both ends", () => {
  assert.deepEqual(visibleRange(0, 600, 60, 100, 4), { start: 0, end: 14 });
  // Scrolled far past the content: clamp to the last screenful, never start > end.
  assert.deepEqual(visibleRange(99_999, 600, 60, 100, 4), { start: 86, end: 100 });
  assert.deepEqual(visibleRange(-50, 600, 60, 100, 0), { start: 0, end: 10 });
});

test("empty lists and bad row heights render nothing", () => {
  assert.deepEqual(visibleRange(0, 600, 60, 0), { start: 0, end: 0 });
  assert.deepEqual(visibleRange(0, 600, 0, 10), { start: 0, end: 0 });
});

test("a list that shrank under the scroll position still renders its rows", () => {
  const range = visibleRange(60 * 500, 600, 60, 30, 4);
  assert.ok(range.start < range.end, JSON.stringify(range));
  assert.equal(range.end, 30);
});

test("a short list renders entirely", () => {
  assert.deepEqual(visibleRange(0, 600, 60, 5, 4), { start: 0, end: 5 });
});

test("100 000 rows still render a small constant window", () => {
  const range = visibleRange(60 * 50_000, 700, 60, 100_000, 6);
  assert.ok(range.end - range.start <= 24, `${range.end - range.start}`);
  assert.equal(range.start > 49_000, true);
});

test("nearEnd fires inside the threshold only", () => {
  assert.equal(nearEnd(0, 600, 60, 100, 10), false);
  assert.equal(nearEnd(60 * 80, 600, 60, 100, 10), true);
  assert.equal(nearEnd(0, 600, 60, 0, 10), false);
  assert.equal(nearEnd(0, 600, 60, 8, 10), true);
});
