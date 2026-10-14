//! Window behaviour the WebView cannot control. Carries no user data.

use tauri::AppHandle;
use tauri_plugin_mt_platform::MtPlatformExt;

use crate::error::{AppError, ErrorCode};

/// Matches the status/navigation bar icons to the app theme (`dark` = dark theme).
///
/// Must stay `async`: a synchronous command runs on the main thread, and the
/// Kotlin side posts to that same thread, so waiting for it would deadlock
/// (the app then shows an ANR).
#[tauri::command]
pub async fn system_bars_set(app: AppHandle, dark: bool) -> Result<(), AppError> {
    app.mt_platform()
        .set_system_bars_dark(dark)
        .map_err(|_| AppError::new(ErrorCode::Internal, "could not update the system bars"))
}
