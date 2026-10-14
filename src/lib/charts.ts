// Tiny helpers for the report charts. Values are minor-unit strings and can
// exceed Number precision, so scaling is done with BigInt.

/**
 * `value / max` as a whole percentage 0..100 (rounded half up), for bar widths.
 * Negative or zero values and a zero `max` give 0; values above `max` clamp to 100.
 */
export function percentOf(value: string, max: string): number {
  const v = BigInt(value);
  const m = BigInt(max);
  if (v <= 0n || m <= 0n) return 0;
  if (v >= m) return 100;
  return Number((v * 100n * 2n + m) / (m * 2n));
}

/** The largest of several minor-unit strings (as a string); "0" for none. */
export function maxOf(values: string[]): string {
  let best = 0n;
  for (const value of values) {
    const v = BigInt(value);
    if (v > best) best = v;
  }
  return best.toString();
}

/** Basis points -> "37,5%" (Indonesian decimal comma, one decimal). */
export function formatShare(bp: number): string {
  const tenths = Math.round(bp / 10);
  const whole = Math.floor(tenths / 10);
  const decimal = tenths % 10;
  return decimal === 0 ? `${whole}%` : `${whole},${decimal}%`;
}
