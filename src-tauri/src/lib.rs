#![forbid(unsafe_code)]

mod accounts;
mod error;
mod state;
mod vault;

#[cfg(debug_assertions)]
mod selftest;

use tauri::Manager;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used, reason = "startup failure is unrecoverable")]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_mt_keystore::init())
        .setup(|app| {
            // Without the app lock (phase 7) the database opens at startup. A
            // failure leaves storage unavailable; commands then report that.
            let db = vault::open_database(app.handle()).ok();
            app.manage(AppState::new(db));
            Ok(())
        });

    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        accounts::accounts_list,
        accounts::account_create,
        accounts::account_update,
        accounts::account_archive,
        selftest::storage_selftest,
    ]);
    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        accounts::accounts_list,
        accounts::account_create,
        accounts::account_update,
        accounts::account_archive,
    ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
