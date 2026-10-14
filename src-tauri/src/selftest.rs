//! Debug-only self-test used by the Phase 3 Android spike.
//!
//! Compiled out of release builds. Reports booleans and counts only; it never
//! returns key bytes or financial data.

use std::fs;

use mt_storage::{Db, DbKey, StorageError};
use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_mt_keystore::{MtKeystoreExt, SecurityLevel};
use zeroize::Zeroizing;

use crate::vault;

fn fail<E>(stage: &'static str) -> impl FnOnce(E) -> String {
    move |_| format!("selftest failed at {stage}")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    keystore_level: String,
    db_opened: bool,
    schema_version: u32,
    header_is_plaintext: bool,
    wrong_key_rejected: bool,
    wrapped_blob_is_not_the_key: bool,
    wrapped_blob_len: usize,
    reopened_with_same_key: bool,
}

#[tauri::command]
pub fn storage_selftest<R: Runtime>(app: AppHandle<R>) -> Result<Report, String> {
    let keystore_level = match app.mt_keystore().security_level() {
        Ok(SecurityLevel::StrongBox) => "strongbox",
        Ok(SecurityLevel::TrustedEnvironment) => "tee",
        Ok(SecurityLevel::Software) => "software",
        Ok(SecurityLevel::Unknown) => "unknown",
        Err(_) => "error",
    }
    .to_owned();

    let db = vault::open_database(&app).map_err(fail("open"))?;
    let schema_version = db.schema_version().map_err(fail("schema"))?;
    drop(db);

    let path = vault::db_path(&app).map_err(fail("path"))?;
    let header = fs::read(&path).map_err(fail("read"))?;
    let header_is_plaintext = header.starts_with(b"SQLite format 3");

    // A random key that is not the real one must be rejected.
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(fail("rng"))?;
    let wrong: DbKey = Zeroizing::new(random);
    let wrong_key_rejected = matches!(
        Db::open(&path, &wrong),
        Err(StorageError::WrongKeyOrCorrupt)
    );

    // The on-disk blob is ciphertext: using its first 32 bytes as the key must fail.
    let blob = vault::wrapped_key_bytes(&app).map_err(fail("blob"))?;
    let mut prefix = [0u8; 32];
    let head = blob.get(..32).ok_or("selftest failed at blob length")?;
    prefix.copy_from_slice(head);
    let blob_key: DbKey = Zeroizing::new(prefix);
    let wrapped_blob_is_not_the_key = matches!(
        Db::open(&path, &blob_key),
        Err(StorageError::WrongKeyOrCorrupt)
    );

    let reopened_with_same_key = vault::open_database(&app).is_ok();

    Ok(Report {
        keystore_level,
        db_opened: true,
        schema_version,
        header_is_plaintext,
        wrong_key_rejected,
        wrapped_blob_is_not_the_key,
        wrapped_blob_len: blob.len(),
        reopened_with_same_key,
    })
}
