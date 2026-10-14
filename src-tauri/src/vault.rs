//! Creates, wraps and loads the database key, then opens the database.
//!
//! The 32-byte SQLCipher key is random, never leaves this process in clear
//! text, and is stored only as a Keystore-wrapped blob next to the database.

use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::{Mutex, PoisonError},
};

use mt_storage::{Db, DbKey, StorageError};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_mt_keystore::MtKeystoreExt;
use zeroize::Zeroizing;

const KEY_FILE: &str = "db.key";
const DB_FILE: &str = "money.db";

/// Serialises key creation and loading. Tauri runs commands on several
/// threads; without this, two first-run callers could each generate a key and
/// the second rename would overwrite the key the database was created with.
static KEY_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug)]
pub enum VaultError {
    Io,
    Keystore,
    /// The database could not be opened or migrated (detail stays out of logs).
    Storage,
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
    fn from(_: StorageError) -> Self {
        Self::Storage
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
    let keystore = app.mt_keystore();
    // Held across key load/creation *and* open, so a second caller sees the
    // finished database rather than racing the first run.
    let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let key = load_or_create_key(
        &dir,
        |plain| Ok(keystore.wrap(plain)?),
        |blob| Ok(keystore.unwrap(blob)?),
    )?;
    Ok(Db::open(&dir.join(DB_FILE), &key)?)
}

/// Platform-independent key handling: `wrap`/`unwrap` come from the Keystore on
/// Android and from a fake in tests. The caller must hold [`KEY_LOCK`].
fn load_or_create_key(
    dir: &Path,
    wrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
    unwrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
) -> Result<DbKey, VaultError> {
    let key_path = dir.join(KEY_FILE);

    match fs::read(&key_path) {
        Ok(blob) => {
            let plain = Zeroizing::new(unwrap(&blob)?);
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
            let blob = wrap(bytes.as_slice())?;
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

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Arc, Barrier},
        thread,
    };

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("mt-vault-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Stand-in for the Keystore: a reversible, key-less transform.
    fn fake_wrap(plain: &[u8]) -> Result<Vec<u8>, VaultError> {
        Ok(plain.iter().map(|b| b ^ 0x5a).collect())
    }

    fn fake_unwrap(blob: &[u8]) -> Result<Vec<u8>, VaultError> {
        fake_wrap(blob)
    }

    fn load(dir: &Path) -> Result<DbKey, VaultError> {
        let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        load_or_create_key(dir, fake_wrap, fake_unwrap)
    }

    #[test]
    fn creates_once_then_reloads_the_same_key() {
        let dir = TempDir::new("reload");
        let first = load(&dir.0).unwrap();
        let second = load(&dir.0).unwrap();
        assert_eq!(*first, *second);
        assert!(!dir.0.join("db.key.tmp").exists());
    }

    #[test]
    fn stored_blob_is_not_the_plain_key() {
        let dir = TempDir::new("blob");
        let key = load(&dir.0).unwrap();
        let blob = fs::read(dir.0.join(KEY_FILE)).unwrap();
        assert_ne!(blob.as_slice(), key.as_slice());
    }

    #[test]
    fn refuses_to_replace_a_key_for_an_existing_database() {
        let dir = TempDir::new("keymissing");
        fs::write(dir.0.join(DB_FILE), b"opaque").unwrap();
        assert!(matches!(load(&dir.0), Err(VaultError::KeyMissing)));
        assert!(!dir.0.join(KEY_FILE).exists());
    }

    #[test]
    fn rejects_a_blob_of_the_wrong_length() {
        let dir = TempDir::new("badkey");
        fs::write(dir.0.join(KEY_FILE), [1u8; 7]).unwrap();
        assert!(matches!(load(&dir.0), Err(VaultError::BadKey)));
    }

    #[test]
    fn concurrent_first_runs_agree_on_one_key() {
        let dir = Arc::new(TempDir::new("race"));
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let dir = Arc::clone(&dir);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    *load(&dir.0).unwrap()
                })
            })
            .collect();
        let keys: Vec<[u8; 32]> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(keys.windows(2).all(|pair| pair[0] == pair[1]));
    }
}
