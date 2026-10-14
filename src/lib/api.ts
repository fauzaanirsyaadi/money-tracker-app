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

export type ErrorCode =
  | "unavailable"
  | "invalid_input"
  | "not_found"
  | "name_in_use"
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
