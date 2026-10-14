// Small per-device conveniences kept in localStorage: theme and the last
// account/category used by quick-add. Never anything financial, and every
// access tolerates storage being unavailable.

export type ThemePref = "auto" | "light" | "dark";

export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const THEME_KEY = "mt.theme";
const LAST_KEY = "mt.quickadd.last";

function defaultStore(): KeyValueStore | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

function read(store: KeyValueStore | null, key: string): string | null {
  try {
    return store?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function write(store: KeyValueStore | null, key: string, value: string): void {
  try {
    store?.setItem(key, value);
  } catch {
    // Private mode or a full quota: the preference is simply not remembered.
  }
}

export function parseTheme(value: string | null): ThemePref {
  return value === "light" || value === "dark" ? value : "auto";
}

/** auto -> light -> dark -> auto. */
export function nextTheme(current: ThemePref): ThemePref {
  return current === "auto" ? "light" : current === "light" ? "dark" : "auto";
}

/** Whether the theme actually shown is dark, given the preference and the OS setting. */
export function isDarkTheme(pref: ThemePref, systemDark: boolean): boolean {
  return pref === "auto" ? systemDark : pref === "dark";
}

export function loadTheme(store: KeyValueStore | null = defaultStore()): ThemePref {
  return parseTheme(read(store, THEME_KEY));
}

export function saveTheme(theme: ThemePref, store: KeyValueStore | null = defaultStore()): void {
  write(store, THEME_KEY, theme);
}

export interface LastUsed {
  accountId?: number;
  expenseCategoryId?: number;
  incomeCategoryId?: number;
}

function isId(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

export function loadLastUsed(store: KeyValueStore | null = defaultStore()): LastUsed {
  const raw = read(store, LAST_KEY);
  if (!raw) return {};
  try {
    const data = JSON.parse(raw) as Record<string, unknown>;
    const out: LastUsed = {};
    if (isId(data.accountId)) out.accountId = data.accountId;
    if (isId(data.expenseCategoryId)) out.expenseCategoryId = data.expenseCategoryId;
    if (isId(data.incomeCategoryId)) out.incomeCategoryId = data.incomeCategoryId;
    return out;
  } catch {
    return {};
  }
}

export function saveLastUsed(
  update: LastUsed,
  store: KeyValueStore | null = defaultStore(),
): void {
  write(store, LAST_KEY, JSON.stringify({ ...loadLastUsed(store), ...update }));
}
