//! Creates, wraps and loads the database key, then opens the database.
//!
//! The 32-byte SQLCipher key is random, never leaves this process in clear
//! text, and is stored only as a Keystore-wrapped blob next to the database.
//!
//! **Nothing is ever created silently, except on a true first run.** Whether
//! this start may create a database and a key is decided by [`classify`] from
//! what exists on disk. Anything that looks like lost data (a key without a
//! database, a database without a key, a lock without either) is reported as a
//! recovery state and left untouched; so is a key the Keystore can no longer
//! use. Transient Keystore failures are reported as "try again" and never
//! lead to a recovery or a new key.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use mt_storage::{Db, DbKey, StorageError};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_mt_keystore::MtKeystoreExt;
use zeroize::Zeroizing;

use crate::{applock::LOCK_FILE, backup::LIVE_DB};

pub const KEY_FILE: &str = "db.key";
const DB_FILE: &str = LIVE_DB;
const OLD_DB_FILE: &str = "money.db.old";

/// Serialises key creation and loading. Tauri runs commands on several
/// threads; without this, two first-run callers could each generate a key and
/// the second rename would overwrite the key the database was created with.
static KEY_LOCK: Mutex<()> = Mutex::new(());

/// Why the data cannot be opened and, with the user's explicit consent, has to
/// be restored from a backup or replaced by an empty database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReason {
    /// A key (or lock) exists but the database file is gone or empty.
    DatabaseMissing,
    /// A database exists but its wrapped key is gone.
    KeyMissing,
    /// The key blob cannot be opened any more (Keystore key deleted or
    /// invalidated, blob altered).
    KeyUnusable,
    /// The key opens but the database does not (wrong key or damaged file).
    DatabaseUnreadable,
}

#[derive(Debug, PartialEq, Eq)]
pub enum VaultError {
    Io,
    /// Data is lost or unreadable: needs the recovery screen.
    Recovery(RecoveryReason),
    /// The Keystore or storage failed in a way that may pass. No recovery is
    /// offered and nothing is created or deleted.
    Unavailable,
}

impl From<io::Error> for VaultError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

impl From<StorageError> for VaultError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::WrongKeyOrCorrupt => Self::Recovery(RecoveryReason::DatabaseUnreadable),
            // A database from a newer app is not damaged: never offer to replace it.
            _ => Self::Unavailable,
        }
    }
}

impl From<tauri_plugin_mt_keystore::Error> for VaultError {
    fn from(err: tauri_plugin_mt_keystore::Error) -> Self {
        match err {
            tauri_plugin_mt_keystore::Error::KeyGone => Self::Recovery(RecoveryReason::KeyUnusable),
            _ => Self::Unavailable,
        }
    }
}

/// What exists in the data directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirState {
    /// `money.db` exists and is not empty (or an interrupted restore left
    /// `money.db.old`, which startup recovery brings back).
    pub db: bool,
    /// The wrapped key file `db.key` exists.
    pub key: bool,
    /// `lock.json` exists.
    pub lock: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Startup {
    /// No database, no key, no lock: a true first run. The only state in
    /// which a key and a database may be created.
    FirstRun,
    Ready,
    NeedsRecovery(RecoveryReason),
}

/// Decides what this start may do. Pure, so the whole matrix is tested.
pub fn classify(state: DirState) -> Startup {
    match (state.db, state.key, state.lock) {
        (false, false, false) => Startup::FirstRun,
        (true, true, _) => Startup::Ready,
        (true, false, _) => Startup::NeedsRecovery(RecoveryReason::KeyMissing),
        // A key or a lock without data: the data was lost, not never made.
        (false, true, _) | (false, false, true) => {
            Startup::NeedsRecovery(RecoveryReason::DatabaseMissing)
        }
    }
}

fn non_empty(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.len() > 0)
}

pub fn inspect(dir: &Path) -> DirState {
    DirState {
        db: non_empty(&dir.join(DB_FILE)) || dir.join(OLD_DB_FILE).exists(),
        key: dir.join(KEY_FILE).exists(),
        lock: dir.join(LOCK_FILE).exists(),
    }
}

/// The app's private data directory (created if needed).
pub fn data_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, VaultError> {
    let dir = app.path().app_data_dir().map_err(|_| VaultError::Io)?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Whether this installation's files call for recovery, without opening anything.
pub fn preflight(dir: &Path) -> Option<RecoveryReason> {
    match classify(inspect(dir)) {
        Startup::NeedsRecovery(reason) => Some(reason),
        Startup::FirstRun | Startup::Ready => None,
    }
}

/// Loads the key of an existing installation. Never creates one: a restore
/// must reuse the key the live database already has.
pub fn load_existing_key<R: Runtime>(app: &AppHandle<R>) -> Result<DbKey, VaultError> {
    let dir = data_dir(app)?;
    let keystore = app.mt_keystore();
    let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    load_key(
        &dir,
        &|plain| Ok(keystore.wrap(plain)?),
        &|blob| Ok(keystore.unwrap(blob)?),
        false,
    )
}

/// Opens the database, creating the key and the file only on a true first run.
pub fn open_database<R: Runtime>(app: &AppHandle<R>) -> Result<Db, VaultError> {
    let dir = data_dir(app)?;
    let keystore = app.mt_keystore();
    // Held across key load/creation *and* open, so a second caller sees the
    // finished database rather than racing the first run.
    let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    open_in_dir(
        &dir,
        |plain| Ok(keystore.wrap(plain)?),
        |blob| Ok(keystore.unwrap(blob)?),
    )
}

type Wrap<'a> = dyn Fn(&[u8]) -> Result<Vec<u8>, VaultError> + 'a;

/// The testable core of [`open_database`]. The caller must hold [`KEY_LOCK`].
fn open_in_dir(
    dir: &Path,
    wrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
    unwrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
) -> Result<Db, VaultError> {
    let first_run = match classify(inspect(dir)) {
        Startup::NeedsRecovery(reason) => return Err(VaultError::Recovery(reason)),
        Startup::FirstRun => true,
        Startup::Ready => false,
    };
    let key = load_key(dir, &wrap, &unwrap, first_run)?;
    // A restore interrupted by a crash is resolved before anything opens the file.
    crate::backup::recover_interrupted(dir, |path| Db::open(path, &key).is_ok());
    Ok(Db::open(&dir.join(DB_FILE), &key)?)
}

/// Key handling: `wrap`/`unwrap` come from the Keystore on Android and from a
/// fake in tests. A key is created only when `allow_create` is set, which the
/// caller grants for a true first run alone. The caller must hold [`KEY_LOCK`].
fn load_key(
    dir: &Path,
    wrap: &Wrap<'_>,
    unwrap: &Wrap<'_>,
    allow_create: bool,
) -> Result<DbKey, VaultError> {
    let key_path = dir.join(KEY_FILE);

    match fs::read(&key_path) {
        Ok(blob) => {
            let plain = Zeroizing::new(unwrap(&blob)?);
            let bytes: [u8; 32] = plain
                .as_slice()
                .try_into()
                .map_err(|_| VaultError::Recovery(RecoveryReason::KeyUnusable))?;
            Ok(Zeroizing::new(bytes))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if !allow_create {
                return Err(VaultError::Recovery(RecoveryReason::KeyMissing));
            }
            let mut bytes = Zeroizing::new([0u8; 32]);
            getrandom::fill(bytes.as_mut_slice()).map_err(|_| VaultError::Io)?;
            let blob = wrap(bytes.as_slice())?;
            write_key_file(dir, &blob)?;
            Ok(bytes)
        }
        Err(_) => Err(VaultError::Io),
    }
}

/// Write, fsync, then rename: a crash or power loss never leaves a truncated
/// or empty key file in place of the real one.
fn write_key_file(dir: &Path, blob: &[u8]) -> io::Result<()> {
    let tmp = dir.join(format!("{KEY_FILE}.tmp"));
    let mut file = fs::File::create(&tmp)?;
    file.write_all(blob)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, dir.join(KEY_FILE))
}

/// Puts a restored database in place of whatever is unreadable: stores the new
/// wrapped key, removes the old database files, moves `staged` into place.
///
/// The order matters. The key goes first, then the old files, then the rename:
/// a crash at any point leaves a state `classify` reports as recovery, never
/// one that looks like a first run. `lock.json` is removed by the caller last.
pub fn install_recovered_database(dir: &Path, wrapped_key: &[u8], staged: &Path) -> io::Result<()> {
    let live = dir.join(DB_FILE);
    write_key_file(dir, wrapped_key)?;
    crate::backup::remove_database(&live);
    crate::backup::remove_database(&dir.join(OLD_DB_FILE));
    // The staged file's WAL and SHM move with it (rename_set semantics).
    for suffix in ["", "-wal", "-shm"] {
        let mut from = staged.as_os_str().to_owned();
        from.push(suffix);
        let from = PathBuf::from(from);
        if from.exists() {
            let mut to = live.as_os_str().to_owned();
            to.push(suffix);
            fs::rename(from, PathBuf::from(to))?;
        }
    }
    Ok(())
}

/// A fresh random database key, for a restore that replaces unreadable data.
pub fn fresh_key() -> Result<DbKey, VaultError> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut_slice()).map_err(|_| VaultError::Io)?;
    Ok(bytes)
}

/// Deletes the database, its key and every companion, for "start fresh". Only
/// called after the user confirmed that the existing data will be lost.
pub fn erase_all(dir: &Path) {
    for name in [DB_FILE, OLD_DB_FILE] {
        crate::backup::remove_database(&dir.join(name));
    }
    let _ = fs::remove_file(dir.join(KEY_FILE));
    let _ = fs::remove_file(dir.join(format!("{KEY_FILE}.tmp")));
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::{Arc, Barrier},
        thread,
    };

    use mt_core::{AccountDraft, AccountKind, Currency, Money};

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

    const KEY: [u8; 32] = [7; 32];

    fn key_of(bytes: [u8; 32]) -> DbKey {
        Zeroizing::new(bytes)
    }

    fn load(dir: &Path, allow_create: bool) -> Result<DbKey, VaultError> {
        let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        load_key(dir, &fake_wrap, &fake_unwrap, allow_create)
    }

    fn open(dir: &Path) -> Result<Db, VaultError> {
        open_with(dir, fake_wrap, fake_unwrap)
    }

    fn open_with(
        dir: &Path,
        wrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
        unwrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
    ) -> Result<Db, VaultError> {
        let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        open_in_dir(dir, wrap, unwrap)
    }

    /// Creates an encrypted database file (with one account) under `key`.
    fn make_db(path: &Path, key: [u8; 32], account: &str) {
        let mut db = Db::open(path, &key_of(key)).unwrap();
        let draft = AccountDraft::new(
            account,
            AccountKind::Cash,
            Money::from_minor(0, Currency::IDR).unwrap(),
        )
        .unwrap();
        db.create_account(&draft, 0).unwrap();
    }

    /// Lays out a directory with the given pieces present.
    fn arrange(dir: &Path, db: bool, key: bool, lock: bool) {
        if db {
            make_db(&dir.join(DB_FILE), KEY, "Dompet");
        }
        if key {
            fs::write(dir.join(KEY_FILE), fake_wrap(&KEY).unwrap()).unwrap();
        }
        if lock {
            fs::write(dir.join(LOCK_FILE), b"{}").unwrap();
        }
    }

    /// Every file with its bytes, to prove a refusal changed nothing.
    fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    fn state(db: bool, key: bool, lock: bool) -> DirState {
        DirState { db, key, lock }
    }

    // --- the state matrix -----------------------------------------------------

    /// (db, key, lock) -> what classify says. This is the specification.
    const MATRIX: [((bool, bool, bool), Startup); 8] = [
        ((false, false, false), Startup::FirstRun),
        (
            (false, false, true),
            Startup::NeedsRecovery(RecoveryReason::DatabaseMissing),
        ),
        (
            (false, true, false),
            Startup::NeedsRecovery(RecoveryReason::DatabaseMissing),
        ),
        (
            (false, true, true),
            Startup::NeedsRecovery(RecoveryReason::DatabaseMissing),
        ),
        (
            (true, false, false),
            Startup::NeedsRecovery(RecoveryReason::KeyMissing),
        ),
        (
            (true, false, true),
            Startup::NeedsRecovery(RecoveryReason::KeyMissing),
        ),
        ((true, true, false), Startup::Ready),
        ((true, true, true), Startup::Ready),
    ];

    #[test]
    fn classify_matches_the_specification_for_every_combination() {
        for ((db, key, lock), expected) in MATRIX {
            assert_eq!(
                classify(state(db, key, lock)),
                expected,
                "db={db} key={key} lock={lock}"
            );
        }
    }

    #[test]
    fn only_a_true_first_run_may_create_anything() {
        let creating: Vec<_> = MATRIX
            .iter()
            .filter(|(_, outcome)| *outcome == Startup::FirstRun)
            .collect();
        assert_eq!(creating.len(), 1);
        assert_eq!(creating[0].0, (false, false, false));
    }

    #[test]
    fn opening_follows_the_matrix_and_a_refusal_changes_nothing() {
        for (index, ((db, key, lock), expected)) in MATRIX.into_iter().enumerate() {
            let dir = TempDir::new(&format!("matrix{index}"));
            arrange(&dir.0, db, key, lock);
            let before = snapshot(&dir.0);

            let result = open(&dir.0);
            let label = format!("db={db} key={key} lock={lock}");
            match expected {
                Startup::FirstRun => {
                    let opened = result.unwrap_or_else(|e| panic!("{label}: {e:?}"));
                    assert_eq!(opened.accounts(false).unwrap().len(), 0, "{label}");
                    let after = snapshot(&dir.0);
                    assert!(
                        after.contains_key(DB_FILE) && after.contains_key(KEY_FILE),
                        "{label}"
                    );
                }
                Startup::Ready => {
                    let opened = result.unwrap_or_else(|e| panic!("{label}: {e:?}"));
                    assert_eq!(opened.accounts(false).unwrap()[0].name, "Dompet", "{label}");
                }
                Startup::NeedsRecovery(reason) => {
                    assert_eq!(result.err(), Some(VaultError::Recovery(reason)), "{label}");
                    assert_eq!(
                        snapshot(&dir.0),
                        before,
                        "{label}: a refusal must not touch the files"
                    );
                }
            }
        }
    }

    #[test]
    fn an_empty_database_file_counts_as_missing() {
        // SQLite would happily turn a zero-byte file into a new empty database.
        let dir = TempDir::new("zerodb");
        fs::write(dir.0.join(DB_FILE), b"").unwrap();
        fs::write(dir.0.join(KEY_FILE), fake_wrap(&KEY).unwrap()).unwrap();
        let before = snapshot(&dir.0);
        assert_eq!(
            open(&dir.0).err(),
            Some(VaultError::Recovery(RecoveryReason::DatabaseMissing))
        );
        assert_eq!(snapshot(&dir.0), before);
    }

    #[test]
    fn a_zero_byte_file_alone_is_still_a_first_run() {
        // Nothing but an empty file: there is no data to lose.
        let dir = TempDir::new("zeroonly");
        fs::write(dir.0.join(DB_FILE), b"").unwrap();
        assert!(open(&dir.0).is_ok());
    }

    #[test]
    fn an_interrupted_restore_is_finished_before_the_state_is_judged() {
        // money.db is gone but money.db.old holds the data (crash between the renames).
        let dir = TempDir::new("oldonly");
        make_db(&dir.0.join(OLD_DB_FILE), KEY, "Dompet");
        fs::write(dir.0.join(KEY_FILE), fake_wrap(&KEY).unwrap()).unwrap();
        let db = open(&dir.0).unwrap();
        assert_eq!(db.accounts(false).unwrap()[0].name, "Dompet");
        assert!(!dir.0.join(OLD_DB_FILE).exists());
    }

    // --- Keystore failures ----------------------------------------------------

    #[test]
    fn a_permanently_lost_keystore_key_asks_for_recovery_and_creates_nothing() {
        let dir = TempDir::new("keygone");
        arrange(&dir.0, true, true, true);
        let before = snapshot(&dir.0);
        let gone = |_: &[u8]| Err(VaultError::Recovery(RecoveryReason::KeyUnusable));
        assert_eq!(
            open_with(&dir.0, fake_wrap, gone).err(),
            Some(VaultError::Recovery(RecoveryReason::KeyUnusable))
        );
        assert_eq!(snapshot(&dir.0), before, "no new key, no new database");
    }

    #[test]
    fn a_temporary_keystore_failure_never_offers_recovery_or_creates_anything() {
        let dir = TempDir::new("keybusy");
        arrange(&dir.0, true, true, false);
        let before = snapshot(&dir.0);
        let busy = |_: &[u8]| Err(VaultError::Unavailable);
        assert_eq!(
            open_with(&dir.0, fake_wrap, busy).err(),
            Some(VaultError::Unavailable)
        );
        assert_eq!(snapshot(&dir.0), before);
    }

    #[test]
    fn a_keystore_failure_on_first_run_leaves_no_half_made_state() {
        let dir = TempDir::new("firstfail");
        let busy = |_: &[u8]| Err(VaultError::Unavailable);
        assert_eq!(
            open_with(&dir.0, busy, fake_unwrap).err(),
            Some(VaultError::Unavailable)
        );
        assert!(
            snapshot(&dir.0).is_empty(),
            "no key file and no database after a failed first run"
        );
        // The next attempt is still a clean first run.
        assert!(open(&dir.0).is_ok());
    }

    #[test]
    fn a_blob_of_the_wrong_length_is_an_unusable_key() {
        let dir = TempDir::new("badkey");
        make_db(&dir.0.join(DB_FILE), KEY, "Dompet");
        fs::write(dir.0.join(KEY_FILE), [1u8; 7]).unwrap();
        // The fake unwrap returns the 7 bytes back; 7 is not a key.
        assert_eq!(
            open(&dir.0).err(),
            Some(VaultError::Recovery(RecoveryReason::KeyUnusable))
        );
    }

    #[test]
    fn a_key_that_does_not_open_the_database_is_reported_as_an_unreadable_database() {
        let dir = TempDir::new("wrongkey");
        make_db(&dir.0.join(DB_FILE), [9; 32], "Dompet");
        fs::write(dir.0.join(KEY_FILE), fake_wrap(&KEY).unwrap()).unwrap();
        let before = snapshot(&dir.0);
        assert_eq!(
            open(&dir.0).err(),
            Some(VaultError::Recovery(RecoveryReason::DatabaseUnreadable))
        );
        assert_eq!(snapshot(&dir.0).get(KEY_FILE), before.get(KEY_FILE));
    }

    #[test]
    fn a_database_from_a_newer_app_is_not_offered_for_replacement() {
        // Only a wrong key or a damaged file leads to recovery; a newer schema
        // is a healthy database this build cannot read yet.
        assert_eq!(
            VaultError::from(StorageError::SchemaTooNew {
                found: 99,
                supported: 2
            }),
            VaultError::Unavailable
        );
        assert_eq!(
            VaultError::from(StorageError::WrongKeyOrCorrupt),
            VaultError::Recovery(RecoveryReason::DatabaseUnreadable)
        );
        assert_eq!(VaultError::from(StorageError::Io), VaultError::Unavailable);
    }

    // --- key file handling (unchanged behaviour) --------------------------------

    #[test]
    fn creates_once_then_reloads_the_same_key() {
        let dir = TempDir::new("reload");
        let first = load(&dir.0, true).unwrap();
        let second = load(&dir.0, false).unwrap();
        assert_eq!(*first, *second);
        assert!(!dir.0.join("db.key.tmp").exists());
    }

    #[test]
    fn stored_blob_is_not_the_plain_key() {
        let dir = TempDir::new("blob");
        let key = load(&dir.0, true).unwrap();
        let blob = fs::read(dir.0.join(KEY_FILE)).unwrap();
        assert_ne!(blob.as_slice(), key.as_slice());
    }

    #[test]
    fn a_missing_key_is_not_replaced_unless_creation_is_allowed() {
        let dir = TempDir::new("nocreate");
        assert_eq!(
            load(&dir.0, false).err(),
            Some(VaultError::Recovery(RecoveryReason::KeyMissing))
        );
        assert!(!dir.0.join(KEY_FILE).exists());
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
                    *load(&dir.0, true).unwrap()
                })
            })
            .collect();
        let keys: Vec<[u8; 32]> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(keys.windows(2).all(|pair| pair[0] == pair[1]));
    }

    // --- replacing unreadable data ------------------------------------------------

    #[test]
    fn a_recovered_database_replaces_unreadable_data_and_opens_with_its_new_key() {
        let dir = TempDir::new("install");
        // Stale, unreadable leftovers: a key blob that opens nothing, garbage, an old copy.
        fs::write(dir.0.join(KEY_FILE), b"stale wrapped key").unwrap();
        fs::write(dir.0.join(DB_FILE), b"garbage").unwrap();
        fs::write(dir.0.join(OLD_DB_FILE), b"older garbage").unwrap();
        // The staged database, keyed with a fresh key.
        let fresh = [42u8; 32];
        let staged = dir.0.join("restore-staged.db");
        make_db(&staged, fresh, "Dari cadangan");

        install_recovered_database(&dir.0, &fake_wrap(&fresh).unwrap(), &staged).unwrap();

        assert!(!staged.exists());
        assert!(!dir.0.join(OLD_DB_FILE).exists());
        let db = open(&dir.0).unwrap();
        assert_eq!(db.accounts(false).unwrap()[0].name, "Dari cadangan");
    }

    #[test]
    fn erase_all_removes_the_database_and_the_key_but_nothing_else() {
        let dir = TempDir::new("erase");
        arrange(&dir.0, true, true, true);
        fs::write(dir.0.join(OLD_DB_FILE), b"x").unwrap();
        fs::write(dir.0.join("unrelated.txt"), b"keep").unwrap();
        erase_all(&dir.0);
        let left: Vec<String> = snapshot(&dir.0).into_keys().collect();
        assert_eq!(left, [LOCK_FILE.to_owned(), "unrelated.txt".to_owned()]);
    }

    #[test]
    fn preflight_reports_the_reason_without_opening_anything() {
        let dir = TempDir::new("preflight");
        assert_eq!(preflight(&dir.0), None, "first run");
        arrange(&dir.0, false, true, false);
        assert_eq!(preflight(&dir.0), Some(RecoveryReason::DatabaseMissing));
    }
}
