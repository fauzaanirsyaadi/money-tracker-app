import assert from "node:assert/strict";
import { test } from "node:test";

import { formatDate, isoDate } from "./dates.ts";

test("isoDate uses the local calendar and zero-pads", () => {
  assert.equal(isoDate(new Date(2026, 0, 5)), "2026-01-05");
  assert.equal(isoDate(new Date(2026, 11, 31)), "2026-12-31");
  assert.equal(isoDate(new Date(999, 5, 9)), "0999-06-09");
});

test("formatDate writes Indonesian month abbreviations", () => {
  assert.equal(formatDate("2026-10-14"), "14 Okt 2026");
  assert.equal(formatDate("2026-05-01"), "1 Mei 2026");
});

test("formatDate leaves malformed text alone", () => {
  assert.equal(formatDate("kemarin"), "kemarin");
  assert.equal(formatDate("2026-13-01"), "2026-13-01");
});
