// Calendar dates travel as "YYYY-MM-DD" in the device's local calendar.

function pad(value: number, width = 2): string {
  return String(value).padStart(width, "0");
}

/** The local calendar date of `now` as "YYYY-MM-DD". */
export function isoDate(now: Date = new Date()): string {
  return `${pad(now.getFullYear(), 4)}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** "2026-10-14" -> "14 Okt 2026". Returns the input unchanged if it is malformed. */
export function formatDate(iso: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!match) return iso;
  const months = ["Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des"];
  const month = months[Number(match[2]) - 1];
  return month ? `${Number(match[3])} ${month} ${match[1]}` : iso;
}

/** The local month of `now` as "YYYY-MM". */
export function isoMonth(now: Date = new Date()): string {
  return isoDate(now).slice(0, 7);
}

/** "2026-12" shifted by +1 -> "2027-01"; by -1 from "2026-01" -> "2025-12". */
export function shiftMonth(month: string, delta: number): string {
  const [year, mon] = month.split("-").map(Number);
  const index = (year ?? 0) * 12 + ((mon ?? 1) - 1) + delta;
  return `${pad(Math.floor(index / 12), 4)}-${pad((index % 12) + 1)}`;
}

/** "2026-10" -> "Oktober 2026". */
export function formatMonth(month: string): string {
  const names = [
    "Januari", "Februari", "Maret", "April", "Mei", "Juni",
    "Juli", "Agustus", "September", "Oktober", "November", "Desember",
  ];
  const match = /^(\d{4})-(\d{2})$/.exec(month);
  const name = match ? names[Number(match[2]) - 1] : undefined;
  return match && name ? `${name} ${match[1]}` : month;
}
