// UI state shared between the shell, the quick-add sheet and the screens.

import type { Transaction } from "$lib/api";

export const ui = $state({
  /** Whether the quick-add / edit sheet is open. */
  sheetOpen: false,
  /** The transaction being edited, or null when adding. */
  editing: null as Transaction | null,
  /** Bumped after any write so screens reload their data. */
  version: 0,
  /** A short message shown above the content, or "". */
  notice: "",
});

let noticeTimer: ReturnType<typeof setTimeout> | undefined;

export function openQuickAdd(editing: Transaction | null = null): void {
  ui.editing = editing;
  ui.sheetOpen = true;
}

export function dataChanged(): void {
  ui.version += 1;
}

export function notify(message: string, ms = 3000): void {
  ui.notice = message;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => {
    ui.notice = "";
  }, ms);
}
