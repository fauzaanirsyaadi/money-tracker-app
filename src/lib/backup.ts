// Pure helpers for the backup screen. The Rust side enforces the same rules;
// these give instant feedback while typing.

import type { ErrorCode } from "./api";

export const PASSPHRASE_MIN = 12;
export const PASSPHRASE_MAX = 128;

/** What the backend would say; the strings are `PassphraseProblem` in `crates/backup`. */
export type PassphraseVerdict = "ok" | "too_short" | "too_long" | "too_repetitive";

function judgeOne(text: string): PassphraseVerdict {
  const chars = [...text];
  if (chars.length < PASSPHRASE_MIN) return "too_short";
  if (chars.length > PASSPHRASE_MAX) return "too_long";
  if (chars.every((c) => c === chars[0])) return "too_repetitive";
  return "ok";
}

/**
 * The same verdict as `validate_passphrase` in Rust, which is the authority:
 * the text as typed **and** its NFKC form (what the backup hashes) are both
 * judged and the stricter wins, "too short" before anything else, otherwise the
 * verdict on the text as typed. NFKC can expand one character into many (U+FDFA
 * gives 18) or shrink decomposed text, so neither form alone is enough. This is
 * advice while typing; the backend decides.
 * `src/lib/backup.test.ts` runs the shared vectors in `testdata/`.
 */
export function passphraseVerdict(passphrase: string): PassphraseVerdict {
  const raw = judgeOne(passphrase);
  const folded = judgeOne(passphrase.normalize("NFKC"));
  if (raw === "ok") return folded;
  if (folded === "ok") return raw;
  if (raw === "too_short" || folded === "too_short") return "too_short";
  return raw;
}

/** Why `passphrase` cannot protect a new backup, or null when it can. */
export function passphraseProblem(passphrase: string): string | null {
  switch (passphraseVerdict(passphrase)) {
    case "ok":
      return null;
    case "too_short":
      return `Frasa sandi minimal ${PASSPHRASE_MIN} karakter (sekarang ${[...passphrase].length}).`;
    case "too_long":
      return `Frasa sandi maksimal ${PASSPHRASE_MAX} karakter.`;
    case "too_repetitive":
      return "Frasa sandi terlalu mudah ditebak.";
  }
}

/** `money-tracker-YYYYMMDD.mtbak` from an ISO date (`YYYY-MM-DD`). */
export function backupFileName(isoDate: string): string {
  return `money-tracker-${isoDate.replaceAll("-", "")}.mtbak`;
}

export function backupErrorMessage(code: ErrorCode): string {
  switch (code) {
    case "weak_passphrase":
      return "Frasa sandi terlalu lemah. Pakai minimal 12 karakter.";
    case "wrong_passphrase":
      return "Frasa sandi salah atau cadangan rusak.";
    case "not_a_backup":
      return "Berkas ini bukan cadangan Money Tracker.";
    case "unsupported_backup":
      return "Cadangan dibuat oleh versi aplikasi yang lebih baru. Perbarui aplikasi dulu.";
    case "backup_damaged":
      return "Cadangan rusak (tidak lengkap atau telah diubah). Data Anda tidak diubah.";
    case "locked":
      return "Aplikasi terkunci. Buka kunci dulu.";
    case "pin_required":
      return "Masukkan PIN lagi untuk melanjutkan.";
    case "cancelled":
      return "Dibatalkan. Tidak ada yang diubah.";
    default:
      return "Tidak berhasil. Data Anda tidak diubah.";
  }
}

// --- Reminder: how long since the last backup (all local, no network) -------------

/** After this many days without a backup the app says so at the top of every screen. */
export const BACKUP_STALE_DAYS = 7;

const DAY_MS = 86_400_000;

export interface BackupStatus {
  /** Epoch milliseconds of the last saved export, or null when there is none. */
  lastBackupAtMs: number | null;
  /** Epoch milliseconds of the first time the app looked, so "never" has an age. */
  sinceMs: number;
}

/**
 * Whole days since the last backup, or since the app first looked when there is
 * none. A stamp in the future (the clock was set back) counts as today: the
 * reminder is advice, so it never errs towards alarming.
 */
export function backupAgeDays(status: BackupStatus, nowMs: number): number {
  const reference = status.lastBackupAtMs ?? status.sinceMs;
  return Math.max(0, Math.floor((nowMs - reference) / DAY_MS));
}

/** One line for the backup card, always shown. */
export function backupAgeText(status: BackupStatus, nowMs: number): string {
  const days = backupAgeDays(status, nowMs);
  if (status.lastBackupAtMs === null) return "Belum pernah membuat cadangan.";
  if (days === 0) return "Terakhir backup hari ini.";
  return `Terakhir backup ${days} hari lalu.`;
}

/** The banner text, or null while the last backup is recent enough. */
export function backupBanner(status: BackupStatus, nowMs: number): string | null {
  const days = backupAgeDays(status, nowMs);
  if (days < BACKUP_STALE_DAYS) return null;
  return status.lastBackupAtMs === null
    ? `Data belum pernah dicadangkan (${days} hari sejak dipakai). Buat cadangan.`
    : `Terakhir backup ${days} hari lalu. Buat cadangan baru.`;
}
