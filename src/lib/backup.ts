// Pure helpers for the backup screen. The Rust side enforces the same rules;
// these give instant feedback while typing.

import type { ErrorCode } from "./api";

export const PASSPHRASE_MIN = 12;
export const PASSPHRASE_MAX = 128;

/** Why `passphrase` cannot protect a new backup, or null when it can. */
export function passphraseProblem(passphrase: string): string | null {
  // Judged on the NFKC form, which is what the backup hashes.
  const chars = [...passphrase.normalize("NFKC")];
  if (chars.length < PASSPHRASE_MIN) {
    return `Frasa sandi minimal ${PASSPHRASE_MIN} karakter (sekarang ${chars.length}).`;
  }
  if (chars.length > PASSPHRASE_MAX) return `Frasa sandi maksimal ${PASSPHRASE_MAX} karakter.`;
  if (chars.every((c) => c === chars[0])) return "Frasa sandi terlalu mudah ditebak.";
  return null;
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
      return "Masukkan PIN lagi untuk melanjutkan pemulihan.";
    default:
      return "Tidak berhasil. Data Anda tidak diubah.";
  }
}
