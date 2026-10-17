// The only module that talks to Rust. Each function is one Tauri command; keep
// this file in step with src-tauri/src/*.rs (docs/ARCHITECTURE.md section 6).
//
// Money is a string of minor units on the wire ("1250000"); see money.ts.

import { invoke } from "@tauri-apps/api/core";

export type AccountKind = "cash" | "bank" | "ewallet";

export interface Account {
  id: number;
  name: string;
  kind: AccountKind;
  currency: string;
  exponent: number;
  openingBalanceMinor: string;
  balanceMinor: string;
  archived: boolean;
}

export interface AccountInput {
  name: string;
  kind: AccountKind;
  openingBalanceMinor: string;
}

export type FlowKind = "income" | "expense";
export type RecordKind = FlowKind | "transfer_out" | "transfer_in";

export interface Category {
  id: number;
  name: string;
  kind: FlowKind;
  parentId: number | null;
  archived: boolean;
}

export interface Transaction {
  id: number;
  accountId: number;
  accountName: string;
  categoryId: number | null;
  categoryName: string | null;
  kind: RecordKind;
  /** Signed minor units. */
  amountMinor: string;
  currency: string;
  exponent: number;
  note: string;
  occurredOn: string;
  transferGroupId: number | null;
  recurring: boolean;
}

export interface TransactionInput {
  accountId: number;
  categoryId: number;
  kind: FlowKind;
  /** Positive magnitude in minor units. */
  amountMinor: string;
  note: string;
  occurredOn: string;
}

export interface TransferInput {
  fromAccountId: number;
  toAccountId: number;
  amountMinor: string;
  note: string;
  occurredOn: string;
}

export interface TransactionFilter {
  accountId?: number;
  categoryId?: number;
  kind?: "income" | "expense" | "transfer";
  from?: string;
  to?: string;
  search?: string;
}

export interface Cursor {
  occurredOn: string;
  id: number;
}

export interface TransactionPage {
  items: Transaction[];
  next: Cursor | null;
}

export type ErrorCode =
  | "unavailable"
  | "invalid_input"
  | "not_found"
  | "name_in_use"
  | "archived"
  | "currency_mismatch"
  | "amount_out_of_range"
  | "locked"
  | "wrong_pin"
  | "locked_out"
  | "cancelled"
  | "biometric_invalidated"
  | "pin_required"
  | "weak_passphrase"
  | "wrong_passphrase"
  | "not_a_backup"
  | "unsupported_backup"
  | "backup_damaged"
  | "internal";

export class ApiError extends Error {
  readonly code: ErrorCode;

  constructor(code: ErrorCode, message: string) {
    super(message);
    this.code = code;
  }
}

function toApiError(raw: unknown): ApiError {
  if (typeof raw === "object" && raw !== null && "code" in raw) {
    const { code, message } = raw as { code: ErrorCode; message?: string };
    return new ApiError(code, message ?? code);
  }
  return new ApiError("internal", "unexpected error");
}

let onLocked: (() => void) | null = null;

/** Called when any command reports the app is locked, so the UI shows the lock screen. */
export function setLockedHandler(handler: (() => void) | null): void {
  onLocked = handler;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    const error = toApiError(raw);
    if (error.code === "locked") onLocked?.();
    throw error;
  }
}

export const accounts = {
  list: (includeArchived: boolean) =>
    call<Account[]>("accounts_list", { includeArchived }),
  create: (input: AccountInput) => call<Account>("account_create", { input }),
  update: (id: number, input: AccountInput) =>
    call<Account>("account_update", { id, input }),
  archive: (id: number, archived: boolean) =>
    call<Account>("account_archive", { id, archived }),
};

export const categories = {
  list: (includeArchived = false) =>
    call<Category[]>("categories_list", { includeArchived }),
  create: (input: { name: string; kind: FlowKind; parentId?: number }) =>
    call<Category>("category_create", {
      input: { parentId: null, ...input },
    }),
};

export const transactions = {
  list: (filter: TransactionFilter = {}, cursor?: Cursor | null, limit = 50) =>
    call<TransactionPage>("transactions_list", {
      filter,
      cursor: cursor ?? null,
      limit,
    }),
  create: (input: TransactionInput) =>
    call<Transaction>("transaction_create", { input }),
  update: (id: number, input: TransactionInput) =>
    call<Transaction>("transaction_update", { id, input }),
  /** Resolves to the number of rows removed (2 for a transfer). */
  remove: (id: number) => call<number>("transaction_delete", { id }),
  transfer: (input: TransferInput) =>
    call<Transaction>("transfer_create", { input }),
};

export type BudgetStatus = "ok" | "warning" | "over";

export interface Budget {
  id: number;
  categoryId: number;
  categoryName: string;
  month: string;
  currency: string;
  exponent: number;
  limitMinor: string;
  spentMinor: string;
  /** limit - spent; negative once exceeded. */
  remainingMinor: string;
  /** Share of the limit used in basis points (10 000 = 100 %). */
  usedBp: number;
  status: BudgetStatus;
}

export const budgets = {
  list: (month: string) => call<Budget[]>("budgets_list", { month }),
  set: (input: { categoryId: number; month: string; limitMinor: string }) =>
    call<Budget>("budget_set", { input }),
  remove: (id: number) => call<void>("budget_delete", { id }),
};

export type Frequency = "daily" | "weekly" | "monthly" | "yearly";

export interface RecurringRule {
  id: number;
  accountId: number;
  accountName: string;
  categoryId: number;
  categoryName: string;
  kind: FlowKind;
  /** Positive magnitude of each occurrence. */
  amountMinor: string;
  currency: string;
  exponent: number;
  note: string;
  frequency: Frequency;
  every: number;
  startOn: string;
  endOn: string | null;
  nextDue: string;
  active: boolean;
}

export interface RecurringInput {
  accountId: number;
  categoryId: number;
  kind: FlowKind;
  amountMinor: string;
  note: string;
  frequency: Frequency;
  every: number;
  startOn: string;
  endOn: string | null;
}

export interface RecurringEditInput {
  accountId: number;
  categoryId: number;
  amountMinor: string;
  note: string;
  endOn: string | null;
}

export interface CatchUpReport {
  rulesAdvanced: number;
  transactionsCreated: number;
  rulesFinished: number;
  rulesInvalid: number;
}

export const recurring = {
  list: () => call<RecurringRule[]>("recurring_list"),
  create: (input: RecurringInput) => call<RecurringRule>("recurring_create", { input }),
  update: (id: number, input: RecurringEditInput) =>
    call<RecurringRule>("recurring_update", { id, input }),
  setActive: (id: number, active: boolean, today: string) =>
    call<RecurringRule>("recurring_set_active", { id, active, today }),
  remove: (id: number) => call<void>("recurring_delete", { id }),
  /** Run once when the app opens; records every occurrence now due. */
  catchUp: (today: string) => call<CatchUpReport>("recurring_catch_up", { today }),
};

export interface ChildSpending {
  categoryId: number;
  name: string;
  spentMinor: string;
  count: number;
}

export interface CategorySpending {
  categoryId: number;
  name: string;
  spentMinor: string;
  count: number;
  /** Basis points of all spending in the period (10 000 = 100 %). */
  shareBp: number;
  children: ChildSpending[];
}

export interface SpendingReport {
  from: string;
  to: string;
  currency: string;
  exponent: number;
  totalMinor: string;
  categories: CategorySpending[];
}

export interface MonthSummary {
  month: string;
  incomeMinor: string;
  expenseMinor: string;
  /** income - expense, signed. */
  netMinor: string;
  /** Net worth at the end of the month, signed. */
  netWorthMinor: string;
}

export interface Trend {
  currency: string;
  exponent: number;
  months: MonthSummary[];
}

export interface NetWorth {
  currency: string;
  exponent: number;
  totalMinor: string;
  accounts: Account[];
}

export const reports = {
  spending: (from: string, to: string) =>
    call<SpendingReport>("report_spending", { from, to }),
  trend: (from: string, to: string) => call<Trend>("report_trend", { from, to }),
  netWorth: () => call<NetWorth>("report_net_worth"),
};

export const platform = {
  /** Matches the status/navigation bar icons to the app theme. */
  systemBars: (dark: boolean) => call<void>("system_bars_set", { dark }),
};

export type BiometricAvailability = "available" | "none_enrolled" | "unavailable";

export interface LockStatus {
  /** A PIN is set (or the lock data is damaged). */
  configured: boolean;
  locked: boolean;
  /** The lock data is damaged; the app stays locked. */
  broken: boolean;
  biometricEnabled: boolean;
  biometric: BiometricAvailability;
  autoLockSeconds: number;
  failedAttempts: number;
  /** Milliseconds until another PIN guess is allowed, or null. */
  retryAfterMs: number | null;
  /** Whether the stored data can be used; see `StorageState`. */
  storage: StorageState;
  /** The PIN was confirmed within the last minute for guarded actions. */
  stepUpActive: boolean;
}

/**
 * `ok`; `retry` for a failure that may pass (nothing was changed); or, when
 * the data is lost or unreadable, the reason recovery is on offer.
 */
export type StorageState =
  | "ok"
  | "retry"
  | "database_missing"
  | "key_missing"
  | "key_unusable"
  | "database_unreadable";

export const AUTO_LOCK_CHOICES = [0, 30, 60, 300] as const;

export const lock = {
  status: () => call<LockStatus>("lock_status"),
  unlockPin: (pin: string) => call<LockStatus>("lock_unlock_pin", { pin }),
  unlockBiometric: () => call<LockStatus>("lock_unlock_biometric"),
  setup: (pin: string) => call<LockStatus>("lock_setup", { pin }),
  changePin: (current: string, newPin: string) =>
    call<LockStatus>("lock_change_pin", { current, new: newPin }),
  remove: (current: string) => call<LockStatus>("lock_remove", { current }),
  setBiometric: (enabled: boolean) => call<LockStatus>("lock_biometric_set", { enabled }),
  setAutoLock: (seconds: number) => call<LockStatus>("lock_auto_set", { seconds }),
  lockNow: () => call<LockStatus>("lock_now"),
  /**
   * Confirms the PIN for guarded actions (export, restore, biometric and
   * auto-lock changes, destructive recovery) for the next minute. Counts
   * against the same attempt limit as unlocking.
   */
  stepUp: (pin: string) => call<LockStatus>("lock_step_up", { pin }),
};

export interface BackupCounts {
  accounts: number;
  categories: number;
  transactions: number;
  budgets: number;
  recurringRules: number;
}

export interface RestorePreview {
  backup: BackupCounts;
  current: BackupCounts;
}

export const backup = {
  /** Encrypts and offers the file to the system "save as" picker. */
  export: (passphrase: string, name: string) =>
    call<{ saved: boolean; counts: BackupCounts }>("backup_export", { passphrase, name }),
  /** Step 1 of a restore: choose the file. Resolves false when cancelled. */
  restorePick: () => call<boolean>("backup_restore_pick"),
  /** Step 2: decrypt and verify without touching current data. */
  restorePrepare: (passphrase: string) =>
    call<RestorePreview>("backup_restore_prepare", { passphrase }),
  /** Step 3: replace current data with the verified backup. */
  restoreCommit: () => call<BackupCounts>("backup_restore_commit"),
  restoreCancel: () => call<void>("backup_restore_cancel"),
};

export const recovery = {
  /** Tries again after a failure that may have passed. */
  retry: () => call<LockStatus>("recovery_retry"),
  /**
   * Discards unreadable data and starts with an empty database. Needs the PIN
   * (when a lock exists) and the word typed to confirm that all existing data
   * will be lost.
   */
  startFresh: (typed: string) => call<LockStatus>("recovery_start_fresh", { typed }),
};
