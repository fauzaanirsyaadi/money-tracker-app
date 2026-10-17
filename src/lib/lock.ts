// Small pure helpers for the lock screen and settings. The Rust side is the
// authority on PIN rules; these only give instant feedback while typing.

export const PIN_MIN = 6;
export const PIN_MAX = 16;

/** Why `pin` would be refused as a new PIN, or null when it is acceptable. */
export function newPinProblem(pin: string): string | null {
  if (!/^[0-9]*$/.test(pin)) return "PIN hanya boleh berisi angka.";
  if (pin.length < PIN_MIN) return `PIN minimal ${PIN_MIN} angka.`;
  if (pin.length > PIN_MAX) return `PIN maksimal ${PIN_MAX} angka.`;
  const digits = [...pin].map(Number);
  const step = digits[1]! - digits[0]!;
  const run = Math.abs(step) <= 1 && digits.every((d, i) => i === 0 || d - digits[i - 1]! === step);
  if (run) return "PIN terlalu mudah ditebak (angka berulang atau berurutan).";
  return null;
}

/** `mm:ss` for a countdown, rounding up so it never shows 0:00 while still locked. */
export function formatCountdown(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

export function autoLockLabel(seconds: number): string {
  if (seconds === 0) return "Segera";
  if (seconds < 60) return `${seconds} dtk`;
  return `${seconds / 60} mnt`;
}

/**
 * What to tell the user when the fingerprint/face prompt did not unlock the app,
 * or null when nothing needs saying (they just dismissed it).
 */
export function biometricFailureMessage(code: string): string | null {
  switch (code) {
    case "cancelled":
      return null;
    case "biometric_invalidated":
      return "Sidik jari atau wajah di ponsel berubah, jadi buka dengan biometrik dimatikan. Buka dengan PIN, lalu aktifkan lagi di Pengaturan.";
    default:
      return "Biometrik tidak berhasil. Gunakan PIN.";
  }
}

/** How long the app may take to start before the screen says so (nothing is open meanwhile). */
export const STARTUP_STALL_MS = 25_000;

/**
 * Whether start-up has taken suspiciously long: the first answer from the backend
 * is still missing. Start-up runs in the background and every command waits for
 * it, so a hung Keystore would otherwise look like a blank screen forever.
 */
export function startupStalled(elapsedMs: number, statusKnown: boolean): boolean {
  return !statusKnown && elapsedMs >= STARTUP_STALL_MS;
}
