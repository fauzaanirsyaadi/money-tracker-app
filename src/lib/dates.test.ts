import assert from "node:assert/strict";
import { test } from "node:test";

import { formatDate, formatMonth, isoDate, isoMonth, shiftMonth } from "./dates.ts";

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

test("months shift across year boundaries", () => {
  assert.equal(shiftMonth("2026-12", 1), "2027-01");
  assert.equal(shiftMonth("2026-01", -1), "2025-12");
  assert.equal(shiftMonth("2026-10", 0), "2026-10");
  assert.equal(shiftMonth("2026-10", 15), "2028-01");
  assert.equal(shiftMonth("2026-03", -14), "2025-01");
});

test("isoMonth and formatMonth", () => {
  assert.equal(isoMonth(new Date(2026, 9, 14)), "2026-10");
  assert.equal(formatMonth("2026-10"), "Oktober 2026");
  assert.equal(formatMonth("2026-13"), "2026-13");
  assert.equal(formatMonth("bulan"), "bulan");
});
