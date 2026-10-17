#![forbid(unsafe_code)]

mod accounts;
mod applock;
mod backup;
mod backup_cmd;
mod backup_meta;
mod budgets;
mod durable;
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

/// Reads the data directory and the lock, cleans leftovers, inspects the storage
/// and declares the state ready. Returns false if the data directory is unusable.
fn start_up(app: &tauri::AppHandle) -> bool {
    let state = app.state::<AppState>();
    let Ok(dir) = app.path().app_data_dir() else {
        return false;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    state.publish_lock(LockService::load(&dir, Instant::now()));
    backup_cmd::clean_leftovers(app);
    recovery::initialize(app, &state);
    state.mark_ready();
    true
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used, reason = "startup failure is unrecoverable")]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_mt_keystore::init())
        .plugin(tauri_plugin_mt_platform::init())
        .setup(|app| {
            // `setup` holds the window back, so it does nothing slow. The state
            // starts out *pending*: until the thread below has read the lock and
            // inspected the storage, every command waits and nothing is open. The
            // database is opened there only when no app lock is set; with a lock it
            // stays closed until the PIN or biometrics unlock it. Lost or unreadable
            // data is reported, never silently replaced.
            app.manage(AppState::pending());
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("mt-startup".into())
                .spawn(move || {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        start_up(&handle)
                    }));
                    if !matches!(outcome, Ok(true)) {
                        // The data directory cannot be resolved (or start-up panicked):
                        // as before, there is nothing useful the app can do.
                        handle.exit(1);
                    }
                })?;
            Ok(())
        });

    // Leaving the foreground starts the auto-lock; coming back checks it. The
    // lock is enforced here, not in the WebView, which may be frozen by then.
    #[cfg(mobile)]
    let builder = builder.on_window_event(|window, event| {
        // A window can exist (and report events) before `setup` has managed the state.
        let Some(state) = window.app_handle().try_state::<AppState>() else {
            return;
        };
        // The main thread must never wait for start-up (start-up waits for the
        // main thread). Before it is done the app starts locked anyway, so an
        // event that arrives earlier loses nothing.
        let Some(lock) = state.applock_if_ready() else {
            return;
        };
        match event {
            tauri::WindowEvent::Suspended => lock.on_suspended(Instant::now(), SystemTime::now()),
            tauri::WindowEvent::Resumed => lock.on_resumed(Instant::now(), SystemTime::now()),
            _ => return,
        }
        if lock.is_locked() {
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
        lock::lock_step_up,
        lock::lock_now,
        recovery::recovery_retry,
        recovery::recovery_start_fresh,
        backup_cmd::backup_export,
        backup_cmd::backup_status,
        backup_cmd::backup_restore_pick,
        backup_cmd::backup_restore_prepare,
        backup_cmd::backup_restore_commit,
        backup_cmd::backup_restore_cancel,
    ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
