// Run with: node --test src/lib/money.test.ts   (Node strips the types itself)
import assert from "node:assert/strict";
import { test } from "node:test";

import { formatMinor, parseAmount } from "./money.ts";

test("formats whole-unit currencies with grouping", () => {
  assert.equal(formatMinor("0", 0), "0");
  assert.equal(formatMinor("999", 0), "999");
  assert.equal(formatMinor("1000", 0), "1.000");
  assert.equal(formatMinor("1250000", 0), "1.250.000");
  assert.equal(formatMinor("-1250000", 0), "-1.250.000");
});

test("formats decimals without floating point", () => {
  assert.equal(formatMinor("5", 2), "0,05");
  assert.equal(formatMinor("-5", 2), "-0,05");
  assert.equal(formatMinor("123456", 2), "1.234,56");
  assert.equal(formatMinor("-1", 3), "-0,001");
});

test("handles values beyond Number precision", () => {
  assert.equal(formatMinor("9007199254740991", 0), "9.007.199.254.740.991");
  assert.equal(formatMinor("9007199254740991", 2), "90.071.992.547.409,91");
});

test("parses plain and grouped whole amounts", () => {
  assert.equal(parseAmount("1500000", 0), "1500000");
  assert.equal(parseAmount("1.500.000", 0), "1500000");
  assert.equal(parseAmount("1,500,000", 0), "1500000");
  assert.equal(parseAmount(" 0 ", 0), "0");
  assert.equal(parseAmount("-250.000", 0), "-250000");
});

test("parses decimals when the currency has them", () => {
  assert.equal(parseAmount("12,5", 2), "1250");
  assert.equal(parseAmount("12.05", 2), "1205");
  assert.equal(parseAmount("1.234,56", 2), "123456");
  assert.equal(parseAmount("1,234.56", 2), "123456");
  assert.equal(parseAmount("1.500", 2), "150000"); // three digits: grouping
  assert.equal(parseAmount("0,001", 3), "1");
});

test("rejects unclear or invalid input", () => {
  for (const bad of ["", " ", "-", "abc", "1e3", "1.2.3", "12,3456", "1,5", "--1", "1.50", ",", "."]) {
    assert.equal(parseAmount(bad, 0), null, JSON.stringify(bad));
  }
  assert.equal(parseAmount("12,345", 2), "1234500"); // 3 digits -> grouping
  assert.equal(parseAmount("12,3456", 2), null);
});

test("enforces the shared range and never returns negative zero", () => {
  assert.equal(parseAmount("9007199254740991", 0), "9007199254740991");
  assert.equal(parseAmount("9007199254740992", 0), null);
  assert.equal(parseAmount("-0", 0), "0");
});

import { formatAmountInput } from "./money.ts";

test("amount fields group digits as you type", () => {
  assert.equal(formatAmountInput("", 0), "");
  assert.equal(formatAmountInput("5", 0), "5");
  assert.equal(formatAmountInput("1500000", 0), "1.500.000");
  assert.equal(formatAmountInput("1.500.0000", 0), "15.000.000");
  assert.equal(formatAmountInput("007", 0), "7");
  assert.equal(formatAmountInput("12abc3", 0), "123");
});

test("amount fields keep a decimal part only when the currency has one", () => {
  assert.equal(formatAmountInput("12,5", 2), "12,5");
  assert.equal(formatAmountInput("12,567", 2), "12,56");
  assert.equal(formatAmountInput(",5", 2), "0,5");
  assert.equal(formatAmountInput("1234,5", 2), "1.234,5");
  assert.equal(formatAmountInput("12,5", 0), "125");
  // Feeding the field its own output while typing must be stable.
  assert.equal(formatAmountInput("1.5000", 0), "15.000");
  assert.equal(formatAmountInput(formatAmountInput("1234", 2) + "5", 2), "12.345");
});
