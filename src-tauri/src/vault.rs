#![allow(
    dead_code,
    reason = "wired into the unlock flow in a later phase; exercised by the debug self-test"
)]

//! Creates, wraps and loads the database key, then opens the database.
//!
//! The 32-byte SQLCipher key is random, never leaves this process in clear
//! text, and is stored only as a Keystore-wrapped blob next to the database.

use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use mt_storage::{Db, DbKey, StorageError};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_mt_keystore::MtKeystoreExt;
use zeroize::Zeroizing;

const KEY_FILE: &str = "db.key";
const DB_FILE: &str = "money.db";

#[derive(Debug)]
pub enum VaultError {
    Io,
    Keystore,
    Storage(StorageError),
    /// The database exists but its wrapped key is missing: refusing to
    /// create a new key, which would make the data unreadable.
    KeyMissing,
    BadKey,
}

impl From<io::Error> for VaultError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

impl From<StorageError> for VaultError {
    fn from(err: StorageError) -> Self {
        Self::Storage(err)
    }
}

impl From<tauri_plugin_mt_keystore::Error> for VaultError {
    fn from(_: tauri_plugin_mt_keystore::Error) -> Self {
        Self::Keystore
    }
}

/// Loads (or on first run creates) the key and opens the database.
pub fn open_database<R: Runtime>(app: &AppHandle<R>) -> Result<Db, VaultError> {
    let dir = app.path().app_data_dir().map_err(|_| VaultError::Io)?;
    fs::create_dir_all(&dir)?;
    let key = load_or_create_key(app, &dir)?;
    Ok(Db::open(&dir.join(DB_FILE), &key)?)
}

fn load_or_create_key<R: Runtime>(app: &AppHandle<R>, dir: &Path) -> Result<DbKey, VaultError> {
    let key_path = dir.join(KEY_FILE);
    let keystore = app.mt_keystore();

    match fs::read(&key_path) {
        Ok(blob) => {
            let plain = Zeroizing::new(keystore.unwrap(&blob)?);
            let bytes: [u8; 32] = plain
                .as_slice()
                .try_into()
                .map_err(|_| VaultError::BadKey)?;
            Ok(Zeroizing::new(bytes))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if dir.join(DB_FILE).exists() {
                return Err(VaultError::KeyMissing);
            }
            let mut bytes = Zeroizing::new([0u8; 32]);
            getrandom::fill(bytes.as_mut_slice()).map_err(|_| VaultError::Io)?;
            let blob = keystore.wrap(bytes.as_slice())?;
            // Write, fsync, then rename: a crash or power loss never leaves a
            // truncated or empty key file in place of the real one.
            let tmp = dir.join(format!("{KEY_FILE}.tmp"));
            let mut file = fs::File::create(&tmp)?;
            file.write_all(&blob)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &key_path)?;
            Ok(bytes)
        }
        Err(_) => Err(VaultError::Io),
    }
}

/// Wrapped key bytes as stored on disk, for the self-test only.
#[cfg(debug_assertions)]
pub fn wrapped_key_bytes<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<u8>, VaultError> {
    let dir = app.path().app_data_dir().map_err(|_| VaultError::Io)?;
    Ok(fs::read(dir.join(KEY_FILE))?)
}

#[cfg(debug_assertions)]
pub fn db_path<R: Runtime>(app: &AppHandle<R>) -> Result<std::path::PathBuf, VaultError> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| VaultError::Io)?
        .join(DB_FILE))
}
