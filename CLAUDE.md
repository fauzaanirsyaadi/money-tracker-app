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
