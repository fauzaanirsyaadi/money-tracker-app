// UI state shared between the shell, the quick-add sheet and the screens.

import { ApiError, lock, setLockedHandler, type LockStatus, type Transaction } from "$lib/api";

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

/** What the app lock currently says. `status` is null until the first answer. */
export const lockState = $state({
  status: null as LockStatus | null,
  /** Counts locked -> unlocked transitions, so work that needs data can re-run. */
  unlocks: 0,
  /** True while the app is hidden: content is covered before the OS snapshots it. */
  covered: false,
});

export function applyLockStatus(next: LockStatus): void {
  const wasLocked = lockState.status?.locked !== false;
  lockState.status = next;
  if (next.locked) {
    // Nothing from before the lock may survive on screen.
    settleStepUp(false);
    ui.sheetOpen = false;
    ui.editing = null;
    ui.notice = "";
  } else if (wasLocked) {
    lockState.unlocks += 1;
    dataChanged();
  }
}

export async function refreshLock(): Promise<void> {
  try {
    applyLockStatus(await lock.status());
  } catch {
    // No backend (plain browser): leave the status unknown.
  }
}

// Any command answering "locked" means the lock engaged behind our back.
setLockedHandler(() => {
  void refreshLock();
});

/** The PIN prompt for guarded actions (see `guarded`). */
export const stepUpUi = $state({ open: false });

let stepUpWaiter: ((confirmed: boolean) => void) | null = null;

function askStepUp(): Promise<boolean> {
  stepUpWaiter?.(false);
  stepUpUi.open = true;
  return new Promise((resolve) => {
    stepUpWaiter = resolve;
  });
}

/** Ends the PIN prompt: `true` after a correct PIN, `false` when dismissed. */
export function settleStepUp(confirmed: boolean): void {
  const waiter = stepUpWaiter;
  stepUpWaiter = null;
  stepUpUi.open = false;
  waiter?.(confirmed);
}

/**
 * Runs an action the backend guards with a fresh PIN. The backend decides: the
 * action is tried, and only when it answers `pin_required` is the PIN asked for,
 * after which the action runs once more. Dismissing the prompt fails the action
 * with `cancelled`; any other error is passed on unchanged.
 */
export async function guarded<T>(action: () => Promise<T>): Promise<T> {
  try {
    return await action();
  } catch (err) {
    if (!(err instanceof ApiError) || err.code !== "pin_required") throw err;
  }
  if (!(await askStepUp())) throw new ApiError("cancelled", "cancelled");
  return action();
}
