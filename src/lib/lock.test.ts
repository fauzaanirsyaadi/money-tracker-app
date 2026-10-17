import assert from "node:assert/strict";
import { test } from "node:test";

import {
  autoLockLabel,
  biometricFailureMessage,
  formatCountdown,
  newPinProblem,
  STARTUP_STALL_MS,
  startupStalled,
} from "./lock.ts";

test("acceptable PINs pass", () => {
  for (const pin of ["493017", "7391028456", "123457", "1234560"]) {
    assert.equal(newPinProblem(pin), null, pin);
  }
});

test("short, long and non-digit PINs are explained", () => {
  assert.match(newPinProblem("49301")!, /minimal 6/);
  assert.match(newPinProblem("4".repeat(17) + "1")!, /maksimal 16/);
  assert.match(newPinProblem("49a017")!, /angka/);
  assert.match(newPinProblem("")!, /minimal 6/);
});

test("repeated and straight-run PINs are refused like the backend does", () => {
  for (const pin of ["000000", "111111", "123456", "654321", "9876543210"]) {
    assert.match(newPinProblem(pin)!, /terlalu mudah/, pin);
  }
});

test("countdown rounds up and never goes negative", () => {
  assert.equal(formatCountdown(30_000), "0:30");
  assert.equal(formatCountdown(29_001), "0:30");
  assert.equal(formatCountdown(61_000), "1:01");
  assert.equal(formatCountdown(3_600_000), "60:00");
  assert.equal(formatCountdown(-5), "0:00");
});

test("auto-lock labels", () => {
  assert.equal(autoLockLabel(0), "Segera");
  assert.equal(autoLockLabel(30), "30 dtk");
  assert.equal(autoLockLabel(60), "1 mnt");
  assert.equal(autoLockLabel(300), "5 mnt");
});

test("a dismissed biometric prompt says nothing; a changed enrolment explains the PIN", () => {
  assert.equal(biometricFailureMessage("cancelled"), null);
  assert.match(biometricFailureMessage("biometric_invalidated")!, /dimatikan.*PIN.*Pengaturan/);
  assert.match(biometricFailureMessage("unavailable")!, /Gunakan PIN/);
  // The enrolment message never talks about data or recovery.
  assert.doesNotMatch(biometricFailureMessage("biometric_invalidated")!, /data|pulih|cadangan/i);
});

test("a start-up that has not answered for too long is called stalled, once it has answered never", () => {
  assert.equal(startupStalled(STARTUP_STALL_MS - 1, false), false);
  assert.equal(startupStalled(STARTUP_STALL_MS, false), true);
  assert.equal(startupStalled(STARTUP_STALL_MS * 10, true), false);
  assert.equal(startupStalled(0, false), false);
});
