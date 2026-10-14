#![forbid(unsafe_code)]

mod vault;

#[cfg(debug_assertions)]
mod selftest;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used, reason = "startup failure is unrecoverable")]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_mt_keystore::init());

    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![selftest::storage_selftest]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
