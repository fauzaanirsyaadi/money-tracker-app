# Money Tracker App
Personal finance app. Android first, local-only mode. Stack: Tauri 2, Rust, Svelte 5.

## Rules
- Money is always i64 minor units inside a Money type. Never f32/f64.
- Fully offline: no network code, no analytics, no INTERNET permission in release builds.
- #![forbid(unsafe_code)] in the app crate (and `unsafe_code = "forbid"` workspace-wide).
- Every Tauri command that calls into Kotlin (plugin `run_mobile_plugin*`) must be `async`: a sync command runs on the main thread and deadlocks with Kotlin's `runOnUiThread` (ANR).
- Minimal, targeted changes only. Do not refactor outside the requested scope.
- After each step run the validation gate and report the result.
- No new dependency without stating why, its license and its maintenance status.
- Never log financial data.
- Minimal Tauri capabilities and a strict CSP.
- Unknown or unclassified errors are treated as transient. Only explicit permanent
  signals may lead to the recovery screen.
- Destroy last: never delete or reset keys or data before the replacement is created and
  verified.
- Destructive actions require the PIN when lock.json is valid and readable, plus a typed
  confirmation.

## CI guards: exceptions live in one place

- A guard (`scripts/check_*.py`) is only ever told to look away through `scripts/guard_exceptions.json`, with a reason. Inline marker comments (`debug-safe:`, `offline-check: allow`) are refused and reported.
- Tauri capabilities are an allow-list (`ALLOWED_CAPABILITY_PERMISSIONS` in `scripts/check_offline.py`), read from `.json`, `.json5`, `.toml` files and inline in `tauri.conf.json`. A permission that is not listed fails CI; a file the guard cannot parse fails CI.
- Every GitHub Action is pinned to a full commit SHA (`scripts/check_workflows.py`). Bump a pin on purpose, with the tag in the trailing comment.
- `.github/CODEOWNERS` has a rule of its own for the workflows, composite actions, `scripts/`, the exceptions file and `deny.toml` (a catch-all `*` does not count). The placeholder owner is a warning locally and **fails CI**: put the real GitHub username in before the first push.
