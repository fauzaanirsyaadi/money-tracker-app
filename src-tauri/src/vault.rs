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
use zeroize::{Zeroize, Zeroizing};

use crate::{
    applock::LOCK_FILE,
    backup::{LIVE_DB, Probe},
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// `Some(metadata)` when the file exists, `None` only when it is definitely
/// absent. Any other failure (permission, I/O, a path component that is not a
/// directory) is "cannot read storage right now": `Path::exists()` would call
/// all of those "absent" and let a hiccup masquerade as lost data.
fn metadata_of(path: &Path) -> Result<Option<fs::Metadata>, VaultError> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(VaultError::Unavailable),
    }
}

fn present(path: &Path) -> Result<bool, VaultError> {
    Ok(metadata_of(path)?.is_some())
}

/// What exists in the data directory. Fails (as "unavailable", never as
/// recovery) when any of the files cannot be examined.
pub fn inspect(dir: &Path) -> Result<DirState, VaultError> {
    let live_has_data = metadata_of(&dir.join(DB_FILE))?.is_some_and(|meta| meta.len() > 0);
    Ok(DirState {
        db: live_has_data || present(&dir.join(OLD_DB_FILE))?,
        key: present(&dir.join(KEY_FILE))?,
        lock: present(&dir.join(LOCK_FILE))?,
    })
}

/// The app's private data directory (created if needed).
pub fn data_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, VaultError> {
    let dir = app.path().app_data_dir().map_err(|_| VaultError::Io)?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Whether this installation's files call for recovery, without opening
/// anything. `Err` means the files could not be examined: a transient state.
pub fn preflight(dir: &Path) -> Result<Option<RecoveryReason>, VaultError> {
    Ok(match classify(inspect(dir)?) {
        Startup::NeedsRecovery(reason) => Some(reason),
        Startup::FirstRun | Startup::Ready => None,
    })
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
    let db = open_in_dir(
        &dir,
        |plain| Ok(keystore.wrap(plain)?),
        |blob| Ok(keystore.unwrap(blob)?),
    )?;
    // The pair is settled: an alias left over from an earlier replacement can go.
    prune_stale_aliases(&dir, &PluginKeys(app));
    Ok(db)
}

type Wrap<'a> = dyn Fn(&[u8]) -> Result<Vec<u8>, VaultError> + 'a;

/// The testable core of [`open_database`]. The caller must hold [`KEY_LOCK`].
fn open_in_dir(
    dir: &Path,
    wrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
    unwrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
) -> Result<Db, VaultError> {
    // A swap interrupted by a crash is settled before the files are judged: the
    // state it left may otherwise look like lost data.
    if crate::backup::recover_interrupted(dir, || pair_probe(dir, &unwrap))
        == crate::backup::Recovered::Undecided
    {
        // Could not tell what state the swap left. Both copies stay as they are.
        return Err(VaultError::Unavailable);
    }
    let first_run = match classify(inspect(dir)?) {
        Startup::NeedsRecovery(reason) => return Err(VaultError::Recovery(reason)),
        Startup::FirstRun => true,
        Startup::Ready => false,
    };
    let key = load_key(dir, &wrap, &unwrap, first_run)?;
    Ok(Db::open(&dir.join(DB_FILE), &key)?)
}

/// What the `db.key` and `money.db` now in place say about themselves.
/// Only a definite failure is `Corrupt`; anything that may pass is `Unknown`.
fn pair_probe(dir: &Path, unwrap: &impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>) -> Probe {
    let blob = match fs::read(dir.join(KEY_FILE)) {
        Ok(blob) => blob,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Probe::Corrupt,
        Err(_) => return Probe::Unknown,
    };
    let plain = match unwrap(&blob) {
        Ok(plain) => plain,
        Err(VaultError::Recovery(_)) => return Probe::Corrupt,
        Err(VaultError::Io | VaultError::Unavailable) => return Probe::Unknown,
    };
    let Ok(bytes) = <[u8; 32]>::try_from(plain.as_slice()) else {
        return Probe::Corrupt;
    };
    probe_of_open(Db::open(&dir.join(DB_FILE), &DbKey::new(bytes)).map(drop))
}

fn probe_of_open(result: Result<(), StorageError>) -> Probe {
    match result {
        Ok(()) => Probe::Opens,
        // "Not a database" under the right key: the file itself is definitely bad.
        Err(StorageError::WrongKeyOrCorrupt) => Probe::Corrupt,
        // Busy, I/O, a newer schema, anything else: do not decide on it.
        Err(_) => Probe::Unknown,
    }
}

/// The answer of a second look right before something destructive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recheck {
    /// Still lost or unreadable, or nothing exists any more: nothing to protect.
    StillUnreadable,
    /// The data opens after all: the problem was a false alarm. Do nothing.
    Readable,
    /// Could not tell (a failure that may pass). Do nothing.
    Transient,
}

/// Looks again at the storage, one more time, before any destructive recovery
/// step. Opens and closes the database; never creates anything.
pub fn recheck<R: Runtime>(app: &AppHandle<R>) -> Recheck {
    let Ok(dir) = data_dir(app) else {
        return Recheck::Transient;
    };
    let keystore = app.mt_keystore();
    let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    recheck_in_dir(
        &dir,
        |plain| Ok(keystore.wrap(plain)?),
        |blob| Ok(keystore.unwrap(blob)?),
    )
}

fn recheck_in_dir(
    dir: &Path,
    wrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
    unwrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>,
) -> Recheck {
    match inspect(dir) {
        Err(_) => return Recheck::Transient,
        // Missing pieces stay missing. A first-run look must not create anything.
        Ok(state) if classify(state) != Startup::Ready => return Recheck::StillUnreadable,
        Ok(_) => {}
    }
    match open_in_dir(dir, wrap, unwrap) {
        Ok(db) => {
            drop(db);
            Recheck::Readable
        }
        Err(VaultError::Recovery(_)) => Recheck::StillUnreadable,
        Err(VaultError::Io | VaultError::Unavailable) => Recheck::Transient,
    }
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
            Ok(DbKey::new(bytes))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if !allow_create {
                return Err(VaultError::Recovery(RecoveryReason::KeyMissing));
            }
            let key = random_key()?;
            let blob = wrap(key.as_slice())?;
            write_key_file(dir, &blob)?;
            Ok(key)
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

/// Opens whatever `db.key` and `money.db` are in place right now, without
/// settling any interrupted swap and without any classification. Used to verify
/// a swap *before* it is declared complete.
pub fn open_current_pair<R: Runtime>(app: &AppHandle<R>) -> Result<Db, VaultError> {
    let dir = data_dir(app)?;
    let keystore = app.mt_keystore();
    let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let blob = fs::read(dir.join(KEY_FILE))?;
    let plain = Zeroizing::new(keystore.unwrap(&blob)?);
    let bytes: [u8; 32] = plain
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::Recovery(RecoveryReason::KeyUnusable))?;
    Ok(Db::open(&dir.join(DB_FILE), &DbKey::new(bytes))?)
}

/// The Keystore operations that replacing the data needs. Behind a trait so the
/// whole sequence, including failures at every step, is tested with a fake.
pub trait KeyBackend {
    /// The version a replacement key gets: above every existing alias.
    fn next_version(&self) -> Result<u8, VaultError>;
    /// Creates the alias `version` and wraps `plain` with it; older aliases stay.
    fn wrap_new(&self, version: u8, plain: &[u8]) -> Result<Vec<u8>, VaultError>;
    fn unwrap(&self, blob: &[u8]) -> Result<Vec<u8>, VaultError>;
    /// Best effort: a failure leaves an unused alias, which is harmless.
    fn delete(&self, version: u8);
    /// Best effort: deletes every alias older than `keep`.
    fn prune_below(&self, keep: u8);
}

/// The real Keystore.
pub struct PluginKeys<'a, R: Runtime>(pub &'a AppHandle<R>);

impl<R: Runtime> KeyBackend for PluginKeys<'_, R> {
    fn next_version(&self) -> Result<u8, VaultError> {
        Ok(self.0.mt_keystore().next_alias_version()?)
    }

    fn wrap_new(&self, version: u8, plain: &[u8]) -> Result<Vec<u8>, VaultError> {
        Ok(self.0.mt_keystore().wrap_new_alias(version, plain)?)
    }

    fn unwrap(&self, blob: &[u8]) -> Result<Vec<u8>, VaultError> {
        Ok(self.0.mt_keystore().unwrap(blob)?)
    }

    fn delete(&self, version: u8) {
        let _ = self.0.mt_keystore().delete_alias(version);
    }

    fn prune_below(&self, keep: u8) {
        let _ = self.0.mt_keystore().prune_aliases_below(keep);
    }
}

/// The steps of [`replace_data`]. Tests fail each one to prove the old data
/// stays readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceStep {
    CreateAlias,
    RoundTrip,
    Swap(crate::backup::SwapStep),
    Verify,
    /// After success: deleting the old aliases. Cannot undo anything.
    PruneOldAliases,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceError {
    /// Something failed; everything is as it was before.
    Failed,
    /// The new data did not match what was staged; undone, as it was before.
    Unverified,
}

/// Replaces unreadable (or missing) data with `staged`, keyed by `new_key`.
///
/// Destroy last, with no exception. Nothing old is deleted until the new data
/// is verified:
/// 1. a **new** Keystore alias is created and wraps the new key (the old alias
///    is untouched, so the old data stays readable if it ever was);
/// 2. the wrapped key is unwrapped again and compared;
/// 3. `money.db` and `db.key` are swapped together, old ones kept as `*.old`;
/// 4. `verify` opens the new files and compares them with what was staged;
/// 5. only then the old files and the old aliases are deleted.
///
/// Any failure up to step 4 removes the new alias and puts the old files back.
pub fn replace_data(
    dir: &Path,
    keys: &dyn KeyBackend,
    staged: &Path,
    new_key: &[u8],
    mut hook: impl FnMut(ReplaceStep) -> io::Result<()>,
    verify: impl FnOnce() -> bool,
) -> Result<(), ReplaceError> {
    hook(ReplaceStep::CreateAlias).map_err(|_| ReplaceError::Failed)?;
    let version = keys.next_version().map_err(|_| ReplaceError::Failed)?;
    let blob = keys
        .wrap_new(version, new_key)
        .map_err(|_| ReplaceError::Failed)?;
    // From here on a failure also removes the alias just made.
    let drop_new_alias = || keys.delete(version);

    if hook(ReplaceStep::RoundTrip).is_err() {
        drop_new_alias();
        return Err(ReplaceError::Failed);
    }
    match keys.unwrap(&blob) {
        Ok(round_trip) if round_trip.as_slice() == new_key => {}
        _ => {
            drop_new_alias();
            return Err(ReplaceError::Failed);
        }
    }

    let record = match crate::backup::swap_with(dir, staged, Some(&blob), |step| {
        hook(ReplaceStep::Swap(step))
    }) {
        Ok(record) => record,
        Err(_) => {
            drop_new_alias();
            return Err(ReplaceError::Failed);
        }
    };

    if hook(ReplaceStep::Verify).is_err() || !verify() {
        // Back to the old files; the old alias was never touched.
        let _ = crate::backup::undo_recorded(dir, &record);
        drop_new_alias();
        return Err(ReplaceError::Unverified);
    }

    crate::backup::finish_swap(dir);
    // Only now does the old alias go. A failure here leaves a stale alias that
    // the next start removes; it cannot lose data.
    if hook(ReplaceStep::PruneOldAliases).is_ok() {
        keys.prune_below(version);
    }
    Ok(())
}

/// The alias version a wrapped key names (its first byte).
pub fn blob_alias_version(blob: &[u8]) -> Option<u8> {
    blob.first().copied().filter(|version| *version != 0)
}

/// Removes aliases older than the one `db.key` uses, after a successful open.
///
/// Deleting a Keystore key cannot be undone, so this is deliberately timid. It
/// does nothing unless **all** of these hold:
/// * `db.key` is readable and its first byte names an alias version;
/// * the blob actually unwraps to a 32-byte key (which proves that alias exists
///   and works: a garbage first byte such as 200 would otherwise make "older
///   than 200" mean every alias);
/// * no swap marker (`*.old`, `db.key.new`) is present, i.e. the pair is settled.
pub fn prune_stale_aliases(dir: &Path, keys: &dyn KeyBackend) {
    let Ok(blob) = fs::read(dir.join(KEY_FILE)) else {
        return;
    };
    let Some(version) = blob_alias_version(&blob) else {
        return;
    };
    let proven = keys.unwrap(&blob).is_ok_and(|plain| plain.len() == 32);
    let settled = ["money.db.old", "db.key.old", "db.key.new"]
        .iter()
        .all(|name| matches!(fs::exists(dir.join(name)), Ok(false)));
    if proven && settled {
        keys.prune_below(version);
    }
}

/// A fresh random database key, for a restore that replaces unreadable data.
pub fn fresh_key() -> Result<DbKey, VaultError> {
    random_key()
}

/// 32 random bytes from the OS, wiped from the stack buffer once moved into the key.
fn random_key() -> Result<DbKey, VaultError> {
    let mut bytes = [0u8; 32];
    let filled = getrandom::fill(&mut bytes);
    let key = DbKey::new(bytes);
    bytes.zeroize();
    filled.map_err(|_| VaultError::Io)?;
    Ok(key)
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
        DbKey::new(bytes)
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
        // Stale, unreadable leftovers: a key blob that opens nothing, garbage.
        fs::write(dir.0.join(KEY_FILE), b"stale wrapped key").unwrap();
        fs::write(dir.0.join(DB_FILE), b"garbage").unwrap();
        // The staged database, keyed with a fresh key.
        let fresh = [42u8; 32];
        let staged = dir.0.join("restore-staged.db");
        make_db(&staged, fresh, "Dari cadangan");

        crate::backup::swap_with(&dir.0, &staged, Some(&fake_wrap(&fresh).unwrap()), |_| {
            Ok(())
        })
        .unwrap();
        crate::backup::finish_swap(&dir.0);

        assert!(!staged.exists());
        assert!(!dir.0.join(OLD_DB_FILE).exists());
        let db = open(&dir.0).unwrap();
        assert_eq!(db.accounts(false).unwrap()[0].name, "Dari cadangan");
    }

    #[test]
    fn preflight_reports_the_reason_without_opening_anything() {
        let dir = TempDir::new("preflight");
        assert_eq!(preflight(&dir.0), Ok(None), "first run");
        arrange(&dir.0, false, true, false);
        assert_eq!(preflight(&dir.0), Ok(Some(RecoveryReason::DatabaseMissing)));
    }

    // --- unreadable storage is not lost data (review item F4) --------------------

    #[test]
    fn a_path_that_cannot_be_examined_is_unavailable_not_absent() {
        // The data "directory" is a file: statting a child fails with "not a
        // directory", which is not "not found" and must not read as lost data.
        let dir = TempDir::new("notadir");
        let file = dir.0.join("not-a-directory");
        fs::write(&file, b"x").unwrap();
        assert_eq!(inspect(&file).err(), Some(VaultError::Unavailable));
        assert_eq!(preflight(&file).err(), Some(VaultError::Unavailable));
        assert_eq!(open(&file).err(), Some(VaultError::Unavailable));
        assert_eq!(snapshot(&dir.0).len(), 1, "nothing was created");
    }

    #[cfg(unix)]
    #[test]
    fn a_permission_error_is_unavailable_and_changes_nothing() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("denied");
        arrange(&dir.0, true, true, true);
        let before = snapshot(&dir.0);
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o000)).unwrap();
        // A privileged user ignores permissions: then there is nothing to prove.
        let privileged = fs::read_dir(&dir.0).is_ok();
        let outcome = (
            inspect(&dir.0).err(),
            preflight(&dir.0).err(),
            open(&dir.0).err(),
        );
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700)).unwrap();
        if privileged {
            return;
        }
        let unavailable = Some(VaultError::Unavailable);
        assert_eq!(outcome, (unavailable, unavailable, unavailable));
        assert_eq!(snapshot(&dir.0), before, "nothing may change");
    }

    #[test]
    fn present_files_are_still_found_and_missing_ones_are_still_absent() {
        let dir = TempDir::new("metadata");
        assert_eq!(inspect(&dir.0), Ok(state(false, false, false)));
        arrange(&dir.0, true, true, true);
        assert_eq!(inspect(&dir.0), Ok(state(true, true, true)));
    }

    // --- the second look before destroying anything (review item F2) -------------

    fn recheck_with(dir: &Path, unwrap: impl Fn(&[u8]) -> Result<Vec<u8>, VaultError>) -> Recheck {
        let _guard = KEY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        recheck_in_dir(dir, fake_wrap, unwrap)
    }

    #[test]
    fn a_false_alarm_is_detected_and_nothing_is_touched() {
        let dir = TempDir::new("recheck-ok");
        arrange(&dir.0, true, true, true);
        let before = snapshot(&dir.0);
        assert_eq!(recheck_with(&dir.0, fake_unwrap), Recheck::Readable);
        assert_eq!(snapshot(&dir.0), before);
    }

    #[test]
    fn a_transient_keystore_failure_is_not_grounds_to_destroy() {
        let dir = TempDir::new("recheck-busy");
        arrange(&dir.0, true, true, false);
        let before = snapshot(&dir.0);
        let busy = |_: &[u8]| Err(VaultError::Unavailable);
        assert_eq!(recheck_with(&dir.0, busy), Recheck::Transient);
        assert_eq!(snapshot(&dir.0), before);
    }

    #[test]
    fn real_loss_is_confirmed_for_every_unreadable_state() {
        // missing database, missing key, unusable key, wrong key.
        let cases: [(&str, bool, bool, bool); 3] = [
            ("recheck-nodb", false, true, true),
            ("recheck-nokey", true, false, false),
            ("recheck-lockonly", false, false, true),
        ];
        for (tag, db, key, lock) in cases {
            let dir = TempDir::new(tag);
            arrange(&dir.0, db, key, lock);
            let before = snapshot(&dir.0);
            assert_eq!(
                recheck_with(&dir.0, fake_unwrap),
                Recheck::StillUnreadable,
                "{tag}"
            );
            assert_eq!(
                snapshot(&dir.0),
                before,
                "{tag}: a recheck must not change anything"
            );
        }
        let dir = TempDir::new("recheck-gone");
        arrange(&dir.0, true, true, true);
        let gone = |_: &[u8]| Err(VaultError::Recovery(RecoveryReason::KeyUnusable));
        assert_eq!(recheck_with(&dir.0, gone), Recheck::StillUnreadable);
        let dir = TempDir::new("recheck-wrongkey");
        make_db(&dir.0.join(DB_FILE), [9; 32], "Dompet");
        fs::write(dir.0.join(KEY_FILE), fake_wrap(&KEY).unwrap()).unwrap();
        assert_eq!(recheck_with(&dir.0, fake_unwrap), Recheck::StillUnreadable);
    }

    #[test]
    fn a_recheck_never_creates_a_first_run_database() {
        let dir = TempDir::new("recheck-empty");
        assert_eq!(recheck_with(&dir.0, fake_unwrap), Recheck::StillUnreadable);
        assert!(
            snapshot(&dir.0).is_empty(),
            "nothing may be created by looking"
        );
    }

    #[test]
    fn an_unexaminable_directory_is_transient() {
        let dir = TempDir::new("recheck-notadir");
        let file = dir.0.join("f");
        fs::write(&file, b"x").unwrap();
        assert_eq!(recheck_with(&file, fake_unwrap), Recheck::Transient);
    }

    // --- F13: a swap that cannot be judged is not rolled back ---------------------

    #[test]
    fn the_probe_distinguishes_definite_corruption_from_anything_that_may_pass() {
        assert_eq!(probe_of_open(Ok(())), Probe::Opens);
        assert_eq!(
            probe_of_open(Err(StorageError::WrongKeyOrCorrupt)),
            Probe::Corrupt
        );
        for transient in [
            StorageError::Io,
            StorageError::Sqlite { code: 5 }, // SQLITE_BUSY
            StorageError::SchemaTooNew {
                found: 99,
                supported: 2,
            },
            StorageError::EncryptionUnavailable,
        ] {
            assert_eq!(
                probe_of_open(Err(transient)),
                Probe::Unknown,
                "{transient:?}"
            );
        }
    }

    #[test]
    fn a_keystore_that_cannot_answer_leaves_an_interrupted_swap_alone() {
        let dir = TempDir::new("swap-unknown");
        // A finished-but-unconfirmed swap: the new pair in place, the old one aside.
        let fresh = [42u8; 32];
        make_db(&dir.0.join(DB_FILE), fresh, "Nilai baru");
        fs::write(dir.0.join(KEY_FILE), fake_wrap(&fresh).unwrap()).unwrap();
        make_db(&dir.0.join(OLD_DB_FILE), KEY, "Nilai lama");
        fs::write(dir.0.join("db.key.old"), fake_wrap(&KEY).unwrap()).unwrap();
        let before = snapshot(&dir.0);

        let busy = |_: &[u8]| Err(VaultError::Unavailable);
        assert_eq!(
            open_with(&dir.0, fake_wrap, busy).err(),
            Some(VaultError::Unavailable)
        );
        assert_eq!(snapshot(&dir.0), before, "no copy may be moved or deleted");

        // Once the Keystore answers, the new data is kept and the stale copy goes.
        let db = open(&dir.0).unwrap();
        assert_eq!(db.accounts(false).unwrap()[0].name, "Nilai baru");
        assert!(!dir.0.join(OLD_DB_FILE).exists() && !dir.0.join("db.key.old").exists());
    }

    #[test]
    fn a_key_blob_that_is_permanently_unusable_after_a_swap_is_a_definite_rollback() {
        let dir = TempDir::new("swap-gone");
        let fresh = [42u8; 32];
        make_db(&dir.0.join(DB_FILE), fresh, "Nilai baru");
        fs::write(dir.0.join(KEY_FILE), b"new blob that does not open").unwrap();
        make_db(&dir.0.join(OLD_DB_FILE), KEY, "Nilai lama");
        fs::write(dir.0.join("db.key.old"), fake_wrap(&KEY).unwrap()).unwrap();
        // The fake Keystore reports a permanent loss for the new blob only.
        let selective = |blob: &[u8]| {
            if blob == b"new blob that does not open" {
                Err(VaultError::Recovery(RecoveryReason::KeyUnusable))
            } else {
                fake_unwrap(blob)
            }
        };
        let db = open_with(&dir.0, fake_wrap, selective).unwrap();
        assert_eq!(
            db.accounts(false).unwrap()[0].name,
            "Nilai lama",
            "rolled back to the old pair"
        );
    }

    // --- versioned aliases: replacing the data without an exception to "destroy last" ---

    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap as Map,
    };

    use crate::backup::SwapStep;

    /// A Keystore stand-in with versioned aliases. A blob is `version || bytes
    /// xor key(version)`, and it only opens while its alias exists.
    #[derive(Default)]
    struct FakeKeys {
        aliases: RefCell<Map<u8, u8>>, // version -> key byte
        fail_create: Cell<bool>,
        wrong_round_trip: Cell<bool>,
    }

    impl FakeKeys {
        fn with_alias(version: u8) -> Self {
            let keys = Self::default();
            keys.aliases
                .borrow_mut()
                .insert(version, version.wrapping_mul(31).wrapping_add(7));
            keys
        }

        fn versions(&self) -> Vec<u8> {
            self.aliases.borrow().keys().copied().collect()
        }

        fn blob(&self, version: u8, plain: &[u8]) -> Vec<u8> {
            let byte = self.aliases.borrow()[&version];
            let mut blob = vec![version];
            blob.extend(plain.iter().map(|b| b ^ byte));
            blob
        }
    }

    impl KeyBackend for FakeKeys {
        fn next_version(&self) -> Result<u8, VaultError> {
            Ok(self
                .aliases
                .borrow()
                .keys()
                .next_back()
                .map_or(1, |v| v + 1))
        }

        fn wrap_new(&self, version: u8, plain: &[u8]) -> Result<Vec<u8>, VaultError> {
            if self.fail_create.get() {
                return Err(VaultError::Unavailable);
            }
            assert!(
                !self.aliases.borrow().contains_key(&version),
                "a version must never be reused"
            );
            self.aliases
                .borrow_mut()
                .insert(version, version.wrapping_mul(31).wrapping_add(7));
            Ok(self.blob(version, plain))
        }

        fn unwrap(&self, blob: &[u8]) -> Result<Vec<u8>, VaultError> {
            let (&version, body) = blob
                .split_first()
                .ok_or(VaultError::Recovery(RecoveryReason::KeyUnusable))?;
            let byte = *self
                .aliases
                .borrow()
                .get(&version)
                .ok_or(VaultError::Recovery(RecoveryReason::KeyUnusable))?;
            let mut plain: Vec<u8> = body.iter().map(|b| b ^ byte).collect();
            if self.wrong_round_trip.get() {
                plain[0] ^= 1;
            }
            Ok(plain)
        }

        fn delete(&self, version: u8) {
            self.aliases.borrow_mut().remove(&version);
        }

        fn prune_below(&self, keep: u8) {
            self.aliases
                .borrow_mut()
                .retain(|version, _| *version >= keep);
        }
    }

    const OLD_KEY: [u8; 32] = [4; 32];
    const NEW_KEY: [u8; 32] = [9; 32];

    /// An old pair (alias v1) in `dir`, and a staged database under NEW_KEY.
    fn old_pair(tag: &str) -> (TempDir, FakeKeys, PathBuf) {
        let dir = TempDir::new(tag);
        let keys = FakeKeys::with_alias(1);
        make_db(&dir.0.join(DB_FILE), OLD_KEY, "Lama");
        fs::write(dir.0.join(KEY_FILE), keys.blob(1, &OLD_KEY)).unwrap();
        let staged = dir.0.join("restore-staged.db");
        make_db(&staged, NEW_KEY, "Baru");
        (dir, keys, staged)
    }

    fn account_name(dir: &Path, keys: &FakeKeys) -> Option<String> {
        let db = open_with(dir, fake_wrap, |blob| keys.unwrap(blob)).ok()?;
        db.accounts(false).ok()?.first().map(|a| a.name.clone())
    }

    fn no_markers(dir: &Path) -> bool {
        ["money.db.old", "db.key.old", "db.key.new"]
            .iter()
            .all(|name| !dir.join(name).exists())
    }

    fn verify_opens(dir: &Path, keys: &FakeKeys) -> impl FnOnce() -> bool {
        let dir = dir.to_path_buf();
        // The fake backend is not Send; capture only what is needed.
        let blob_path = dir.join(KEY_FILE);
        let versions = keys.aliases.borrow().clone();
        move || {
            let Ok(blob) = fs::read(&blob_path) else {
                return false;
            };
            let Some((&version, body)) = blob.split_first() else {
                return false;
            };
            let Some(&byte) = versions
                .get(&version)
                .or(Some(&(version.wrapping_mul(31).wrapping_add(7))))
            else {
                return false;
            };
            let plain: Vec<u8> = body.iter().map(|b| b ^ byte).collect();
            let Ok(bytes) = <[u8; 32]>::try_from(plain.as_slice()) else {
                return false;
            };
            Db::open(&dir.join(DB_FILE), &DbKey::new(bytes)).is_ok()
        }
    }

    const ALL_STEPS: [ReplaceStep; 9] = [
        ReplaceStep::CreateAlias,
        ReplaceStep::RoundTrip,
        ReplaceStep::Swap(SwapStep::WriteNewKey),
        ReplaceStep::Swap(SwapStep::MoveDbAside),
        ReplaceStep::Swap(SwapStep::MoveKeyAside),
        ReplaceStep::Swap(SwapStep::PromoteKey),
        ReplaceStep::Swap(SwapStep::MoveStagedIn),
        ReplaceStep::Verify,
        ReplaceStep::PruneOldAliases,
    ];

    #[test]
    fn a_replacement_uses_a_new_alias_and_deletes_the_old_one_only_at_the_end() {
        let (dir, keys, staged) = old_pair("alias-ok");
        let mut seen_old_alias_at_verify = false;
        let versions_at_verify = RefCell::new(Vec::new());
        let result = replace_data(
            &dir.0,
            &keys,
            &staged,
            &NEW_KEY,
            |step| {
                if step == ReplaceStep::Verify {
                    seen_old_alias_at_verify = true;
                    *versions_at_verify.borrow_mut() = keys.versions();
                }
                Ok(())
            },
            verify_opens(&dir.0, &keys),
        );
        assert_eq!(result, Ok(()));
        assert!(seen_old_alias_at_verify);
        assert_eq!(
            *versions_at_verify.borrow(),
            vec![1, 2],
            "both aliases exist until the new data is verified"
        );
        assert_eq!(keys.versions(), vec![2], "the old alias is deleted last");
        assert_eq!(account_name(&dir.0, &keys).as_deref(), Some("Baru"));
        assert!(no_markers(&dir.0));
    }

    #[test]
    fn a_failure_at_any_step_leaves_the_old_alias_and_the_old_data_readable() {
        for failing in ALL_STEPS {
            if failing == ReplaceStep::PruneOldAliases {
                continue; // after success: covered by its own test
            }
            let (dir, keys, staged) = old_pair("alias-fail");
            let old_db = fs::read(dir.0.join(DB_FILE)).unwrap();
            let old_key = fs::read(dir.0.join(KEY_FILE)).unwrap();

            let result = replace_data(
                &dir.0,
                &keys,
                &staged,
                &NEW_KEY,
                |step| {
                    if step == failing {
                        Err(io::Error::other("injected"))
                    } else {
                        Ok(())
                    }
                },
                verify_opens(&dir.0, &keys),
            );

            assert!(result.is_err(), "{failing:?}");
            assert_eq!(
                keys.versions(),
                vec![1],
                "{failing:?}: only the old alias may remain"
            );
            assert_eq!(
                fs::read(dir.0.join(DB_FILE)).unwrap(),
                old_db,
                "{failing:?}: database"
            );
            assert_eq!(
                fs::read(dir.0.join(KEY_FILE)).unwrap(),
                old_key,
                "{failing:?}: key blob"
            );
            assert_eq!(
                account_name(&dir.0, &keys).as_deref(),
                Some("Lama"),
                "{failing:?}: old data readable"
            );
            assert!(no_markers(&dir.0), "{failing:?}");
            assert!(staged.exists(), "{failing:?}: the staged database survives");
        }
    }

    #[test]
    fn creating_the_alias_failing_changes_nothing_at_all() {
        let (dir, keys, staged) = old_pair("alias-create");
        keys.fail_create.set(true);
        let result = replace_data(&dir.0, &keys, &staged, &NEW_KEY, |_| Ok(()), || true);
        assert_eq!(result, Err(ReplaceError::Failed));
        assert_eq!(keys.versions(), vec![1]);
        assert_eq!(account_name(&dir.0, &keys).as_deref(), Some("Lama"));
    }

    #[test]
    fn a_wrapped_key_that_does_not_open_to_the_same_key_stops_before_any_file_moves() {
        let (dir, keys, staged) = old_pair("alias-roundtrip");
        keys.wrong_round_trip.set(true);
        let before = snapshot(&dir.0);
        let result = replace_data(&dir.0, &keys, &staged, &NEW_KEY, |_| Ok(()), || true);
        assert_eq!(result, Err(ReplaceError::Failed));
        assert_eq!(keys.versions(), vec![1], "the new alias is removed again");
        assert_eq!(snapshot(&dir.0), before, "no file moved");
    }

    #[test]
    fn a_failed_verification_undoes_everything_including_the_new_alias() {
        let (dir, keys, staged) = old_pair("alias-verify");
        let result = replace_data(&dir.0, &keys, &staged, &NEW_KEY, |_| Ok(()), || false);
        assert_eq!(result, Err(ReplaceError::Unverified));
        assert_eq!(keys.versions(), vec![1]);
        assert_eq!(account_name(&dir.0, &keys).as_deref(), Some("Lama"));
        assert!(no_markers(&dir.0));
    }

    #[test]
    fn a_failure_while_deleting_the_old_alias_cannot_lose_data_and_is_cleaned_up_later() {
        let (dir, keys, staged) = old_pair("alias-prune");
        let result = replace_data(
            &dir.0,
            &keys,
            &staged,
            &NEW_KEY,
            |step| {
                if step == ReplaceStep::PruneOldAliases {
                    Err(io::Error::other("injected"))
                } else {
                    Ok(())
                }
            },
            verify_opens(&dir.0, &keys),
        );
        assert_eq!(result, Ok(()), "the data was already replaced and verified");
        assert_eq!(keys.versions(), vec![1, 2], "the stale alias lingers");
        assert_eq!(account_name(&dir.0, &keys).as_deref(), Some("Baru"));

        // The next successful start removes it.
        prune_stale_aliases(&dir.0, &keys);
        assert_eq!(keys.versions(), vec![2]);
        assert_eq!(account_name(&dir.0, &keys).as_deref(), Some("Baru"));
    }

    #[test]
    fn startup_never_deletes_the_alias_the_data_still_needs() {
        let (dir, keys, _staged) = old_pair("alias-prune-keep");
        prune_stale_aliases(&dir.0, &keys);
        assert_eq!(keys.versions(), vec![1], "v1 wraps the current db.key");
        // A newer alias that nothing uses yet (an unfinished replacement) is not touched either.
        keys.aliases.borrow_mut().insert(2, 99);
        prune_stale_aliases(&dir.0, &keys);
        assert_eq!(keys.versions(), vec![1, 2]);
    }

    #[test]
    fn replacing_lost_data_with_nothing_old_to_keep_also_works_and_undoes_cleanly() {
        // "database missing" or "key missing": there may be no old pair at all.
        let dir = TempDir::new("alias-nothing");
        let keys = FakeKeys::default();
        let staged = dir.0.join("restore-staged.db");
        make_db(&staged, NEW_KEY, "Baru");
        let result = replace_data(
            &dir.0,
            &keys,
            &staged,
            &NEW_KEY,
            |_| Ok(()),
            verify_opens(&dir.0, &keys),
        );
        assert_eq!(result, Ok(()));
        assert_eq!(keys.versions(), vec![1]);
        assert_eq!(account_name(&dir.0, &keys).as_deref(), Some("Baru"));

        let dir = TempDir::new("alias-nothing-undo");
        let keys = FakeKeys::default();
        let staged = dir.0.join("restore-staged.db");
        make_db(&staged, NEW_KEY, "Baru");
        let result = replace_data(&dir.0, &keys, &staged, &NEW_KEY, |_| Ok(()), || false);
        assert_eq!(result, Err(ReplaceError::Unverified));
        assert!(keys.versions().is_empty(), "the unused alias is removed");
        assert!(!dir.0.join(DB_FILE).exists() && !dir.0.join(KEY_FILE).exists());
    }

    #[test]
    fn a_blob_names_the_alias_that_wrapped_it() {
        assert_eq!(blob_alias_version(&[3, 1, 2]), Some(3));
        assert_eq!(blob_alias_version(&[0, 1, 2]), None);
        assert_eq!(blob_alias_version(&[]), None);
    }

    // --- pruning never deletes a key that might still be needed (review item 1) ----

    /// `FakeKeys` blobs wrap a 32-byte key, as the real ones do.
    fn settled_pair(tag: &str, aliases: &[u8]) -> (TempDir, FakeKeys) {
        let dir = TempDir::new(tag);
        let keys = FakeKeys::default();
        for version in aliases {
            keys.aliases
                .borrow_mut()
                .insert(*version, version.wrapping_mul(31).wrapping_add(7));
        }
        let newest = *aliases.last().unwrap();
        fs::write(dir.0.join(KEY_FILE), keys.blob(newest, &[7u8; 32])).unwrap();
        (dir, keys)
    }

    #[test]
    fn the_alias_the_blob_names_is_never_deleted_and_older_ones_go_only_when_proven() {
        let (dir, keys) = settled_pair("prune-proven", &[1, 2, 3]);
        prune_stale_aliases(&dir.0, &keys);
        assert_eq!(
            keys.versions(),
            vec![3],
            "only the proven newest alias is kept"
        );
    }

    #[test]
    fn an_unknown_or_unreadable_prefix_prunes_nothing_at_all() {
        // The blob names alias 200, which does not exist: "older than 200" must not mean all.
        let (dir, keys) = settled_pair("prune-unknown", &[1, 2]);
        fs::write(dir.0.join(KEY_FILE), {
            let mut blob = keys.blob(2, &[7u8; 32]);
            blob[0] = 200;
            blob
        })
        .unwrap();
        prune_stale_aliases(&dir.0, &keys);
        assert_eq!(
            keys.versions(),
            vec![1, 2],
            "nothing may be deleted on an unproven prefix"
        );

        // Version zero, an empty file and a missing file.
        for content in [Some(vec![0u8, 1, 2, 3]), Some(vec![]), None] {
            let (dir, keys) = settled_pair("prune-unreadable", &[1, 2]);
            match content {
                Some(bytes) => fs::write(dir.0.join(KEY_FILE), bytes).unwrap(),
                None => fs::remove_file(dir.0.join(KEY_FILE)).unwrap(),
            }
            prune_stale_aliases(&dir.0, &keys);
            assert_eq!(keys.versions(), vec![1, 2]);
        }
    }

    #[test]
    fn a_blob_that_does_not_unwrap_prunes_nothing() {
        let (dir, keys) = settled_pair("prune-garbled", &[1, 2]);
        // The prefix is right (2) but the body is too short to be a 32-byte key.
        fs::write(dir.0.join(KEY_FILE), [2u8, 9, 9]).unwrap();
        prune_stale_aliases(&dir.0, &keys);
        assert_eq!(keys.versions(), vec![1, 2]);
    }

    #[test]
    fn an_unsettled_swap_prunes_nothing() {
        for marker in ["money.db.old", "db.key.old", "db.key.new"] {
            let (dir, keys) = settled_pair("prune-unsettled", &[1, 2]);
            fs::write(dir.0.join(marker), b"x").unwrap();
            prune_stale_aliases(&dir.0, &keys);
            assert_eq!(keys.versions(), vec![1, 2], "{marker}");
        }
    }
}
