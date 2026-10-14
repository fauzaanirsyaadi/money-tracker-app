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
