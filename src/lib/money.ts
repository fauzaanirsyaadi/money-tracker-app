// Money formatting and parsing for the UI.
//
// Amounts arrive from Rust as decimal strings of minor units ("1250000") and
// are handled as BigInt, never as Number. Locale: Indonesian (. grouping, , decimal).

const GROUP = ".";
const DECIMAL = ",";

function scale(exponent: number): bigint {
  return 10n ** BigInt(exponent);
}

function group(digits: string): string {
  return digits.replace(/\B(?=(\d{3})+(?!\d))/g, GROUP);
}

/** "1250000", 0 -> "1.250.000";  "-12345", 2 -> "-123,45". */
export function formatMinor(minor: string, exponent: number): string {
  const value = BigInt(minor);
  const negative = value < 0n;
  const abs = negative ? -value : value;
  const unit = scale(exponent);
  const whole = group((abs / unit).toString());
  const text =
    exponent === 0
      ? whole
      : `${whole}${DECIMAL}${(abs % unit).toString().padStart(exponent, "0")}`;
  return negative ? `-${text}` : text;
}

/**
 * Turns what a person typed into a string of minor units, or `null` when it is
 * not a clear amount. Accepts an optional leading "-", digits, and grouping or
 * decimal separators ("." or ","): a separator followed by 1..exponent digits at
 * the end is the decimal point, anything else is grouping.
 */
export function parseAmount(input: string, exponent: number): string | null {
  let text = input.trim().replace(/\s+/g, "");
  let negative = false;
  if (text.startsWith("-")) {
    negative = true;
    text = text.slice(1);
  }
  if (!/^[0-9.,]+$/.test(text) || !/[0-9]/.test(text)) return null;
  if (/^[.,]|[.,]$/.test(text) && !/[.,][0-9]{1,}$/.test(text)) return null;

  let whole = text;
  let fraction = "";
  const last = Math.max(text.lastIndexOf("."), text.lastIndexOf(","));
  if (last >= 0) {
    const after = text.slice(last + 1);
    const asDecimal = exponent > 0 && after.length >= 1 && after.length <= exponent;
    if (asDecimal) {
      whole = text.slice(0, last);
      fraction = after;
    } else if (after.length !== 3) {
      return null; // neither a decimal part nor a clean group of three
    }
  }
  whole = whole.replace(/[.,]/g, "");
  if (whole === "") whole = "0";
  if (!/^[0-9]+$/.test(whole)) return null;

  const minor = BigInt(whole) * scale(exponent) + BigInt(fraction.padEnd(exponent, "0") || "0");
  if (minor > 9007199254740991n) return null;
  return `${negative && minor !== 0n ? "-" : ""}${minor}`;
}

/**
 * Live formatting for an amount field: keeps digits (and, when the currency has
 * decimals, one decimal comma), groups the whole part with ".", and drops
 * anything else. "1500000" -> "1.500.000"; "12,5" -> "12,5"; a typed "." is
 * treated as grouping and ignored.
 */
export function formatAmountInput(raw: string, exponent: number): string {
  // "," is the decimal separator; "." is the grouping this function itself
  // emits, so it is ignored when the field is fed back its own output.
  const text = raw;
  const comma = text.indexOf(",");
  const digits = (value: string) => value.replace(/\D/g, "");
  // Whole-unit currencies have no decimal part: every separator is grouping.
  const wholeSource = comma === -1 || exponent === 0 ? text : text.slice(0, comma);
  let whole = digits(wholeSource).replace(/^0+(?=\d)/, "");
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, GROUP);
  if (comma === -1 || exponent === 0) return grouped;
  const fraction = digits(text.slice(comma + 1)).slice(0, exponent);
  whole = grouped === "" ? "0" : grouped;
  return `${whole}${DECIMAL}${fraction}`;
}
