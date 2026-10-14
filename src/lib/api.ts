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

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    throw toApiError(raw);
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
