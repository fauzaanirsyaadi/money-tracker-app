// Wording for the recovery screen. Pure, so each state's text is tested.

import type { StorageState } from "./api";

/** The word typed to confirm that all existing data will be lost (matches the backend). */
export const START_FRESH_WORD = "HAPUS";

export function confirmationMatches(typed: string): boolean {
  return typed.trim() === START_FRESH_WORD;
}

/** Whether the screen must ask for the PIN before offering anything destructive. */
export function needsPin(status: {
  configured: boolean;
  broken: boolean;
  recoveryAuthenticated: boolean;
}): boolean {
  return status.configured && !status.broken && !status.recoveryAuthenticated;
}

export interface RecoveryText {
  title: string;
  explanation: string;
  /** Whether restoring a backup or starting fresh is on offer (real loss), or only retrying. */
  canRecover: boolean;
}

export function recoveryText(state: StorageState): RecoveryText {
  switch (state) {
    case "database_missing":
      return {
        title: "Data tidak ditemukan",
        explanation:
          "Berkas data tidak ada, padahal kunci atau pengaturan kunci aplikasi masih tersimpan. Data kemungkinan terhapus. Tidak ada yang dibuat ulang secara otomatis.",
        canRecover: true,
      };
    case "key_missing":
      return {
        title: "Kunci data hilang",
        explanation:
          "Data masih ada, tetapi kunci enkripsinya hilang sehingga tidak dapat dibaca. Tidak ada yang dibuat ulang secara otomatis.",
        canRecover: true,
      };
    case "key_unusable":
      return {
        title: "Kunci keamanan perangkat tidak dapat dipakai",
        explanation:
          "Kunci di Android Keystore yang melindungi data sudah dihapus atau tidak berlaku lagi (misalnya setelah pengaturan ulang keamanan perangkat). Data lama tidak dapat dibaca.",
        canRecover: true,
      };
    case "database_unreadable":
      return {
        title: "Data tidak dapat dibuka",
        explanation:
          "Data ada, tetapi tidak dapat dibuka dengan kunci yang tersimpan. Berkasnya mungkin rusak.",
        canRecover: true,
      };
    case "retry":
      return {
        title: "Penyimpanan belum dapat dibuka",
        explanation:
          "Ini mungkin sementara (misalnya Keystore sedang sibuk). Data Anda tidak diubah. Coba lagi sebentar lagi.",
        canRecover: false,
      };
    case "ok":
      return { title: "", explanation: "", canRecover: false };
  }
}
