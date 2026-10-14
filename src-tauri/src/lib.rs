#![forbid(unsafe_code)]

mod accounts;
mod budgets;
mod error;
mod recurring;
mod state;
mod transactions;
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
        selftest::storage_selftest,
    ]);
    #[cfg(not(debug_assertions))]
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
    ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
