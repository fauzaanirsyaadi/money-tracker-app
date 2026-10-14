# Money Tracker App
Personal finance app. Android first, local-only mode. Stack: Tauri 2, Rust, Svelte 5.

## Rules
- Money is always i64 minor units inside a Money type. Never f32/f64.
- Fully offline: no network code, no analytics, no INTERNET permission in release builds.
- #![forbid(unsafe_code)] in the app crate.
- Minimal, targeted changes only. Do not refactor outside the requested scope.
- After each step run the validation gate and report the result.
- No new dependency without stating why, its license and its maintenance status.
- Never log financial data.
- Minimal Tauri capabilities and a strict CSP.
