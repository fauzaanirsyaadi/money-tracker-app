import assert from "node:assert/strict";
import { test } from "node:test";

import { formatShare, maxOf, percentOf } from "./charts.ts";

test("percentOf scales and clamps", () => {
  assert.equal(percentOf("50", "200"), 25);
  assert.equal(percentOf("1", "3"), 33);
  assert.equal(percentOf("2", "3"), 67);
  assert.equal(percentOf("200", "200"), 100);
  assert.equal(percentOf("300", "200"), 100);
  assert.equal(percentOf("0", "200"), 0);
  assert.equal(percentOf("-5", "200"), 0);
  assert.equal(percentOf("5", "0"), 0);
});

test("percentOf is exact beyond Number precision", () => {
  assert.equal(percentOf("4503599627370495", "9007199254740991"), 50);
  assert.equal(percentOf("9007199254740990", "9007199254740991"), 100);
});

test("maxOf picks the largest and ignores negatives", () => {
  assert.equal(maxOf(["5", "900", "20"]), "900");
  assert.equal(maxOf(["-5", "-9"]), "0");
  assert.equal(maxOf([]), "0");
});

test("formatShare uses a decimal comma and drops a zero decimal", () => {
  assert.equal(formatShare(10_000), "100%");
  assert.equal(formatShare(3_750), "37,5%");
  assert.equal(formatShare(3_333), "33,3%");
  assert.equal(formatShare(5), "0,1%");
  assert.equal(formatShare(0), "0%");
});
