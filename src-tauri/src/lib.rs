#![forbid(unsafe_code)]

mod accounts;
mod applock;
mod backup;
mod backup_cmd;
mod budgets;
mod error;
mod lock;
mod platform;
mod recovery;
mod recurring;
#[cfg(test)]
mod redaction_tests;
mod reports;
mod state;
mod transactions;
mod vault;

use std::time::Instant;
#[cfg(mobile)]
use std::time::SystemTime;

use tauri::Manager;

use crate::{applock::LockService, state::AppState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used, reason = "startup failure is unrecoverable")]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_mt_keystore::init())
        .plugin(tauri_plugin_mt_platform::init())
        .setup(|app| {
            // The database is opened here only when no app lock is set. With a
            // lock it stays closed until the PIN or biometrics unlock it. Lost
            // or unreadable data is reported, never silently replaced.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let lock = LockService::load(&dir, Instant::now());
            backup_cmd::clean_leftovers(app.handle());
            app.manage(AppState::new(None, lock));
            let state = app.state::<AppState>();
            recovery::initialize(app.handle(), &state);
            Ok(())
        });

    // Leaving the foreground starts the auto-lock; coming back checks it. The
    // lock is enforced here, not in the WebView, which may be frozen by then.
    #[cfg(mobile)]
    let builder = builder.on_window_event(|window, event| {
        let state = window.app_handle().state::<AppState>();
        match event {
            tauri::WindowEvent::Suspended => {
                state
                    .applock()
                    .on_suspended(Instant::now(), SystemTime::now());
            }
            tauri::WindowEvent::Resumed => {
                state
                    .applock()
                    .on_resumed(Instant::now(), SystemTime::now());
            }
            _ => return,
        }
        if state.applock().is_locked() {
            state.close_db();
        }
    });

    let builder = builder.invoke_handler(tauri::generate_handler![
        accounts::accounts_list,
        accounts::account_create,
        accounts::account_update,
        accounts::account_archive,
        transactions::categories_list,
        transactions::category_create,
        transactions::transactions_list,
        transactions::transaction_create,
        transactions::transaction_update,
        transactions::transaction_delete,
        transactions::transfer_create,
        budgets::budgets_list,
        budgets::budget_set,
        budgets::budget_delete,
        recurring::recurring_list,
        recurring::recurring_create,
        recurring::recurring_update,
        recurring::recurring_set_active,
        recurring::recurring_delete,
        recurring::recurring_catch_up,
        reports::report_spending,
        reports::report_trend,
        reports::report_net_worth,
        platform::system_bars_set,
        lock::lock_status,
        lock::lock_unlock_pin,
        lock::lock_unlock_biometric,
        lock::lock_setup,
        lock::lock_change_pin,
        lock::lock_remove,
        lock::lock_biometric_set,
        lock::lock_auto_set,
        lock::lock_now,
        recovery::recovery_retry,
        recovery::recovery_authenticate,
        recovery::recovery_start_fresh,
        backup_cmd::backup_export,
        backup_cmd::backup_restore_pick,
        backup_cmd::backup_restore_prepare,
        backup_cmd::backup_restore_commit,
        backup_cmd::backup_restore_cancel,
    ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
