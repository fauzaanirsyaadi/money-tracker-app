//! Encrypted backup and restore: the parts that need no Tauri.
//!
//! Export streams the logical dump straight into the encrypting writer, so
//! memory does not grow with the database. Restore builds a *separate*
//! database file from the backup, checks it completely, and only then swaps it
//! in with two renames. The previous database is kept until the new one has
//! opened and verified, and a crash at any point is undone at the next start.
//!
//! Nothing here logs passphrases, keys or contents.

use std::{
    fs,
    io::{self, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

use mt_backup::{BackupError, BackupReader, BackupWriter, KdfParams, PassphraseProblem};
use mt_storage::{Db, DbKey, LogicalSummary, StorageError};

pub const LIVE_DB: &str = "money.db";
const OLD_SUFFIX: &str = ".old";
/// Files that make up one SQLite database.
const COMPANIONS: [&str; 3] = ["", "-wal", "-shm"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupFailure {
    /// The passphrase is not acceptable for a new backup.
    WeakPassphrase(PassphraseProblem),
    /// Wrong passphrase, or the file was altered (indistinguishable).
    WrongPassphrase,
    NotABackup,
    /// Written by a newer app (container format or schema).
    Unsupported,
    /// Incomplete, reordered or otherwise inconsistent content.
    Damaged,
    Io,
    Storage,
}

impl From<BackupError> for BackupFailure {
    fn from(err: BackupError) -> Self {
        match err {
            BackupError::NotABackup => Self::NotABackup,
            BackupError::UnsupportedVersion(_) => Self::Unsupported,
            BackupError::WrongPassphraseOrCorrupt => Self::WrongPassphrase,
            BackupError::BadParameters | BackupError::Truncated | BackupError::TrailingData => {
                Self::Damaged
            }
            BackupError::Io => Self::Io,
        }
    }
}

impl From<StorageError> for BackupFailure {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::SchemaTooNew { .. } => Self::Unsupported,
            StorageError::Io => Self::Io,
            StorageError::BackupInvalid | StorageError::WrongKeyOrCorrupt => Self::Damaged,
            _ => Self::Storage,
        }
    }
}

impl From<io::Error> for BackupFailure {
    fn from(err: io::Error) -> Self {
        // A failed read through the decrypting reader carries its own reason.
        match BackupError::from_io(&err) {
            BackupError::Io => Self::Io,
            other => other.into(),
        }
    }
}

/// Encrypts a logical dump into `dest`.
///
/// The key is derived before `dump` runs, so the (slow) Argon2 work does not
/// hold the database while it happens.
pub fn write_backup(
    dest: fs::File,
    passphrase: &str,
    params: KdfParams,
    dump: impl FnOnce(&mut dyn Write) -> Result<LogicalSummary, BackupFailure>,
) -> Result<LogicalSummary, BackupFailure> {
    mt_backup::validate_passphrase(passphrase).map_err(BackupFailure::WeakPassphrase)?;
    let mut writer = BackupWriter::new(BufWriter::new(dest), passphrase, params)?;
    let summary = dump(&mut writer)?;
    let buffered = writer.finish()?;
    let file = buffered.into_inner().map_err(|_| BackupFailure::Io)?;
    file.sync_all()?;
    Ok(summary)
}

/// Decrypts `source` and builds a verified database at `staged` (which must
/// not exist), keyed with `key`. The returned database is closed again, so the
/// file can be renamed.
pub fn stage_restore(
    source: fs::File,
    passphrase: &str,
    key: &DbKey,
    staged: &Path,
) -> Result<LogicalSummary, BackupFailure> {
    let reader = BackupReader::open(BufReader::new(source), passphrase)?;
    let (db, summary) = Db::restore_logical(staged, key, BufReader::new(reader))?;
    drop(db);
    Ok(summary)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn companions(path: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    COMPANIONS
        .iter()
        .map(move |suffix| with_suffix(path, suffix))
}

pub fn remove_database(path: &Path) {
    for file in companions(path) {
        let _ = fs::remove_file(file);
    }
}

/// The steps of a swap, in order. Tests make any one of them fail, or stop the
/// process there, to prove the old data survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapStep {
    /// Writes the new wrapped key next to the old one (recovery only).
    WriteNewKey,
    MoveDbAside,
    /// Moves the old `db.key` aside (recovery only).
    MoveKeyAside,
    /// Renames the new key into place (recovery only).
    PromoteKey,
    MoveStagedIn,
}

const KEY_FILE: &str = crate::vault::KEY_FILE;

fn old_path(dir: &Path) -> PathBuf {
    with_suffix(&dir.join(LIVE_DB), OLD_SUFFIX)
}

fn old_key_path(dir: &Path) -> PathBuf {
    with_suffix(&dir.join(KEY_FILE), OLD_SUFFIX)
}

fn new_key_path(dir: &Path) -> PathBuf {
    with_suffix(&dir.join(KEY_FILE), ".new")
}

fn rename_set(from: &Path, to: &Path) -> io::Result<()> {
    for suffix in COMPANIONS {
        let (src, dst) = (with_suffix(from, suffix), with_suffix(to, suffix));
        // `fs::exists` fails on anything but "not found" instead of calling it absent.
        if fs::exists(&src)? {
            fs::rename(src, dst)?;
        }
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// What a swap has done so far, so a failure can undo exactly that.
#[derive(Default, Clone)]
struct Progress {
    new_key_written: bool,
    db_moved: bool,
    key_moved: bool,
    key_promoted: bool,
    /// Where the staged database was moved from, once it has been moved in.
    staged_from: Option<PathBuf>,
}

/// Replaces the live database with `staged`, keeping the old one aside as
/// `money.db.old`. The database must be closed. On failure the old database is
/// put back before returning.
pub fn swap_in(dir: &Path, staged: &Path) -> io::Result<()> {
    swap_with(dir, staged, None, |_| Ok(())).map(|_| ())
}

/// What a finished swap moved, kept so that a failed verification can undo
/// exactly that, including removing new files when there was nothing old to
/// bring back.
pub struct SwapRecord(Progress);

/// Replaces `money.db` and, when `new_key` is given, `db.key` **together**. The
/// old files are kept as `*.old` until [`finish_swap`]; a failure at any step
/// puts them back, and a crash is settled at the next start by
/// [`recover_interrupted`]. Nothing old is deleted here.
pub fn swap_with(
    dir: &Path,
    staged: &Path,
    new_key: Option<&[u8]>,
    hook: impl FnMut(SwapStep) -> io::Result<()>,
) -> io::Result<SwapRecord> {
    swap_inner(dir, staged, new_key, hook, true)
}

fn swap_inner(
    dir: &Path,
    staged: &Path,
    new_key: Option<&[u8]>,
    mut hook: impl FnMut(SwapStep) -> io::Result<()>,
    rollback_on_error: bool,
) -> io::Result<SwapRecord> {
    // A leftover from an earlier crash is resolved at startup, never here.
    for leftover in [old_path(dir), old_key_path(dir), new_key_path(dir)] {
        if fs::exists(&leftover)? {
            return Err(io::Error::other("an earlier swap is unresolved"));
        }
    }
    let mut progress = Progress::default();
    let result = run_swap(dir, staged, new_key, &mut hook, &mut progress);
    if let Err(err) = result {
        if rollback_on_error {
            restore_old(dir, &progress)?;
        }
        return Err(err);
    }
    Ok(SwapRecord(progress))
}

fn run_swap(
    dir: &Path,
    staged: &Path,
    new_key: Option<&[u8]>,
    hook: &mut impl FnMut(SwapStep) -> io::Result<()>,
    progress: &mut Progress,
) -> io::Result<()> {
    let live = dir.join(LIVE_DB);
    let key = dir.join(KEY_FILE);
    if let Some(blob) = new_key {
        hook(SwapStep::WriteNewKey)?;
        write_synced(&new_key_path(dir), blob)?;
        progress.new_key_written = true;
    }
    hook(SwapStep::MoveDbAside)?;
    if fs::exists(&live)? {
        rename_set(&live, &old_path(dir))?;
        progress.db_moved = true;
    }
    if new_key.is_some() {
        hook(SwapStep::MoveKeyAside)?;
        if fs::exists(&key)? {
            fs::rename(&key, old_key_path(dir))?;
            progress.key_moved = true;
        }
        hook(SwapStep::PromoteKey)?;
        fs::rename(new_key_path(dir), &key)?;
        progress.key_promoted = true;
    }
    hook(SwapStep::MoveStagedIn)?;
    rename_set(staged, &live)?;
    progress.staged_from = Some(staged.to_path_buf());
    Ok(())
}

/// Undoes what [`run_swap`] did, newest first.
fn restore_old(dir: &Path, progress: &Progress) -> io::Result<()> {
    let live = dir.join(LIVE_DB);
    let key = dir.join(KEY_FILE);
    if let Some(staged) = &progress.staged_from {
        // The staged database goes back where it came from rather than being
        // deleted: a retry does not have to rebuild it.
        if rename_set(&live, staged).is_err() {
            remove_database(&live);
        }
    }
    if progress.key_promoted {
        let _ = fs::remove_file(&key);
    }
    if progress.key_moved {
        fs::rename(old_key_path(dir), &key)?;
    }
    if progress.db_moved {
        remove_database(&live);
        rename_set(&old_path(dir), &live)?;
    }
    if progress.new_key_written {
        let _ = fs::remove_file(new_key_path(dir));
    }
    Ok(())
}

/// Undoes a finished swap whose result failed verification: new files are
/// removed and old ones brought back, whatever was (or was not) there before.
pub fn undo_recorded(dir: &Path, record: &SwapRecord) -> io::Result<()> {
    restore_old(dir, &record.0)
}

/// The new data opened and verified: the old files can go.
pub fn finish_swap(dir: &Path) {
    remove_database(&old_path(dir));
    let _ = fs::remove_file(old_key_path(dir));
    let _ = fs::remove_file(new_key_path(dir));
}

/// The new data was bad: bring the old files back. Needs no memory of what the
/// swap did (it also settles a crashed one), so it only looks at what is on disk.
pub fn undo_swap(dir: &Path) -> io::Result<()> {
    let live = dir.join(LIVE_DB);
    let key = dir.join(KEY_FILE);
    if fs::exists(old_path(dir))? {
        remove_database(&live);
        rename_set(&old_path(dir), &live)?;
    }
    if fs::exists(old_key_path(dir))? {
        let _ = fs::remove_file(&key);
        fs::rename(old_key_path(dir), &key)?;
    }
    let _ = fs::remove_file(new_key_path(dir));
    Ok(())
}

/// What a look at the files in place says about the new data after a swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// The new key and database open together.
    Opens,
    /// A **definite** answer that they do not: missing, not a database, a key
    /// blob that is permanently unusable.
    Corrupt,
    /// Could not tell: a busy or failing Keystore, an I/O error, a locked file.
    /// Nothing may be decided, let alone deleted, on this.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovered {
    /// No swap was interrupted.
    Nothing,
    /// The new files were good: the old ones were deleted.
    Committed,
    /// The new files were definitely incomplete or corrupt: the old ones are back.
    RolledBack,
    /// Could not decide. Both copies are untouched; try again later.
    Undecided,
}

/// Settles a swap that a crash or a power loss interrupted. Call before
/// anything opens the database at startup.
///
/// A swap leaves marker files (`money.db.old`, `db.key.old`, `db.key.new`).
/// The old copies are restored **only on a definite answer**:
/// * `db.key.new` still present: the swap never finished its renames;
/// * the live database is missing;
/// * the probe says the pair in place is corrupt.
///
/// If the probe says the pair opens, the old copies are deleted. If it cannot
/// tell, or the files cannot even be examined, **nothing is changed**: a
/// transient failure must never turn newer data into the stale older copy.
pub fn recover_interrupted(dir: &Path, probe: impl FnOnce() -> Probe) -> Recovered {
    let markers = [old_path(dir), old_key_path(dir), new_key_path(dir)];
    let mut present = [false; 3];
    for (slot, marker) in present.iter_mut().zip(&markers) {
        match fs::exists(marker) {
            Ok(found) => *slot = found,
            Err(_) => return Recovered::Undecided,
        }
    }
    if !present.iter().any(|found| *found) {
        return Recovered::Nothing;
    }
    let unfinished = present[2];
    let live_present = match fs::exists(dir.join(LIVE_DB)) {
        Ok(found) => found,
        Err(_) => return Recovered::Undecided,
    };
    let verdict = if unfinished || !live_present {
        Probe::Corrupt
    } else {
        probe()
    };
    match verdict {
        Probe::Opens => {
            finish_swap(dir);
            Recovered::Committed
        }
        Probe::Corrupt => match undo_swap(dir) {
            Ok(()) => Recovered::RolledBack,
            Err(_) => Recovered::Undecided,
        },
        Probe::Unknown => Recovered::Undecided,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Read,
        sync::atomic::{AtomicU32, Ordering},
        time::Instant,
    };

    use super::*;

    const PASS: &str = "gantungkan cita-cita setinggi langit";

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "mt-backup-app-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn key(byte: u8) -> DbKey {
        DbKey::new([byte; 32])
    }

    /// A deterministic logical dump with `n` transactions, built as text so the
    /// import path (and not only export) is exercised at size.
    fn dump_text(n: u64) -> (String, Vec<(i64, i64)>) {
        let mut out = String::new();
        let push = |out: &mut String, line: String| {
            out.push_str(&line);
            out.push('\n');
        };
        push(
            &mut out,
            r#"{"format":"money-tracker-logical","version":1,"schema":2}"#.into(),
        );
        push(
            &mut out,
            r#"{"table":"settings","columns":["key","value"],"rows":1}"#.into(),
        );
        push(&mut out, r#"["currency","IDR"]"#.into());
        let accounts = 5i64;
        push(
            &mut out,
            format!(
                r#"{{"table":"accounts","columns":["id","name","kind","currency","opening_balance_minor","archived","sort_order","created_at"],"rows":{accounts}}}"#
            ),
        );
        let mut balance = vec![0i64; accounts as usize];
        for a in 1..=accounts {
            let opening = a * 1_000_000;
            balance[(a - 1) as usize] = opening;
            push(
                &mut out,
                format!(r#"[{a},"Rekening {a}","bank","IDR",{opening},0,{a},0]"#),
            );
        }
        push(&mut out, r#"{"table":"categories","columns":["id","name","kind","parent_id","archived"],"rows":2}"#.into());
        push(&mut out, r#"[1,"Makanan","expense",null,0]"#.into());
        push(&mut out, r#"[2,"Gaji","income",null,0]"#.into());
        push(&mut out, r#"{"table":"recurring_rules","columns":["id","account_id","category_id","kind","amount_minor","note","freq","every_n","anchor_day","start_on","end_on","next_due","active"],"rows":0}"#.into());
        push(
            &mut out,
            format!(
                r#"{{"table":"transactions","columns":["id","account_id","category_id","amount_minor","kind","note","occurred_on","transfer_group_id","recurring_rule_id","created_at"],"rows":{n}}}"#
            ),
        );
        for i in 1..=n {
            let account = (i % 5) as i64 + 1;
            let (kind, category, amount) = if i % 10 == 0 {
                ("income", 2, 50_000 + (i % 7_000) as i64)
            } else {
                ("expense", 1, -(100 + (i % 9_000) as i64))
            };
            balance[(account - 1) as usize] += amount;
            let month = 1 + i % 12;
            let day = 1 + i % 28;
            push(
                &mut out,
                format!(
                    r#"[{i},{account},{category},{amount},"{kind}","catatan {i} \"uji\"","2026-{month:02}-{day:02}",null,null,{i}]"#
                ),
            );
        }
        push(
            &mut out,
            r#"{"table":"budgets","columns":["id","category_id","month","limit_minor"],"rows":0}"#
                .into(),
        );
        let balances: Vec<String> = balance
            .iter()
            .enumerate()
            .map(|(i, b)| format!(r#"[{},"{b}"]"#, i + 1))
            .collect();
        push(
            &mut out,
            format!(r#"{{"end":true,"balances":[{}]}}"#, balances.join(",")),
        );
        let pairs = balance
            .iter()
            .enumerate()
            .map(|(i, b)| (i as i64 + 1, *b))
            .collect();
        (out, pairs)
    }

    fn live_db_from(text: &str, dir: &Dir, name: &str, k: &DbKey) -> (Db, LogicalSummary) {
        Db::restore_logical(&dir.file(name), k, text.as_bytes()).unwrap()
    }

    fn export_to(db: &Db, path: &Path, pass: &str) -> Result<LogicalSummary, BackupFailure> {
        let file = fs::File::create(path).unwrap();
        write_backup(file, pass, KdfParams::for_tests(), |w| {
            db.export_logical(w).map_err(BackupFailure::from)
        })
    }

    #[test]
    fn export_then_stage_restore_gives_identical_counts_and_balances() {
        let dir = Dir::new();
        let (text, expected) = dump_text(2_000);
        let (db, original) = live_db_from(&text, &dir, "src.db", &key(1));
        assert_eq!(original.balances, expected);

        let file = dir.file("backup.mtbak");
        let exported = export_to(&db, &file, PASS).unwrap();
        assert_eq!(exported, original);

        let staged = dir.file("staged.db");
        let k = key(1);
        let restored = stage_restore(fs::File::open(&file).unwrap(), PASS, &k, &staged).unwrap();
        assert_eq!(restored, original);
        assert_eq!(restored.count("transactions"), 2_000);

        // Open the staged file like the app would and compare per account.
        let reopened = Db::open(&staged, &k).unwrap();
        let balances: Vec<(i64, i64)> = reopened
            .accounts(true)
            .unwrap()
            .into_iter()
            .map(|a| (a.id, a.balance.minor()))
            .collect();
        assert_eq!(balances, expected);
    }

    #[test]
    fn the_backup_file_holds_no_plaintext() {
        let dir = Dir::new();
        let (text, _) = dump_text(200);
        let (db, _) = live_db_from(&text, &dir, "src.db", &key(1));
        let file = dir.file("backup.mtbak");
        export_to(&db, &file, PASS).unwrap();
        let bytes = fs::read(&file).unwrap();
        for needle in [
            &b"catatan"[..],
            b"Rekening",
            b"money-tracker-logical",
            b"IDR",
        ] {
            assert!(!bytes.windows(needle.len()).any(|w| w == needle));
        }
    }

    #[test]
    fn a_weak_passphrase_is_refused_before_anything_is_written() {
        let dir = Dir::new();
        let (text, _) = dump_text(10);
        let (db, _) = live_db_from(&text, &dir, "src.db", &key(1));
        let file = dir.file("weak.mtbak");
        assert_eq!(
            export_to(&db, &file, "pendek").err(),
            Some(BackupFailure::WeakPassphrase(PassphraseProblem::TooShort))
        );
        assert_eq!(fs::metadata(&file).unwrap().len(), 0, "nothing was written");
    }

    #[test]
    fn restore_failures_are_classified_and_leave_no_staged_file() {
        let dir = Dir::new();
        // Several chunks, so damage can land after the first (which opens fine).
        let (text, _) = dump_text(3_000);
        let (db, _) = live_db_from(&text, &dir, "src.db", &key(1));
        let file = dir.file("backup.mtbak");
        export_to(&db, &file, PASS).unwrap();
        let good = fs::read(&file).unwrap();
        let k = key(1);

        let attempt = |bytes: &[u8], pass: &str, tag: &str| {
            let src = dir.file(&format!("in-{tag}.mtbak"));
            fs::write(&src, bytes).unwrap();
            let staged = dir.file(&format!("staged-{tag}.db"));
            let result = stage_restore(fs::File::open(&src).unwrap(), pass, &k, &staged);
            if result.is_err() {
                for f in companions(&staged) {
                    assert!(!f.exists(), "{tag}: {} left behind", f.display());
                }
            }
            result.err()
        };

        assert_eq!(
            attempt(&good, "salah salah salah", "pass"),
            Some(BackupFailure::WrongPassphrase)
        );
        let mut flipped = good.clone();
        let mid = flipped.len() / 2;
        flipped[mid] ^= 1;
        // The first chunk opened with this passphrase, so damage further in is
        // reported as damage, not as a wrong passphrase.
        assert_eq!(
            attempt(&flipped, PASS, "flip"),
            Some(BackupFailure::Damaged)
        );
        assert_eq!(
            attempt(&good[..good.len() - 40], PASS, "cut"),
            Some(BackupFailure::Damaged)
        );
        assert_eq!(
            attempt(&good[..30], PASS, "short"),
            Some(BackupFailure::NotABackup)
        );
        assert_eq!(attempt(b"", PASS, "empty"), Some(BackupFailure::NotABackup));
        let mut future = good.clone();
        future[4] = 9;
        assert_eq!(
            attempt(&future, PASS, "version"),
            Some(BackupFailure::Unsupported)
        );
        let mut trailing = good.clone();
        trailing.extend_from_slice(b"junk");
        assert!(attempt(&trailing, PASS, "trail").is_some());
        // The good file still restores after all of that.
        assert_eq!(attempt(&good, PASS, "ok"), None);
    }

    #[test]
    fn a_valid_container_with_a_bad_payload_is_damaged_not_applied() {
        // Someone who knows the passphrase can build a well-formed container
        // around nonsense; the import must still refuse it.
        let dir = Dir::new();
        let file = dir.file("odd.mtbak");
        let out = fs::File::create(&file).unwrap();
        let mut w = BackupWriter::new(out, PASS, KdfParams::for_tests()).unwrap();
        w.write_all(
            b"{\"format\":\"money-tracker-logical\",\"version\":1,\"schema\":2}\nnot json\n",
        )
        .unwrap();
        w.finish().unwrap();
        let staged = dir.file("s.db");
        let err = stage_restore(fs::File::open(&file).unwrap(), PASS, &key(1), &staged).err();
        assert_eq!(err, Some(BackupFailure::Damaged));
        assert!(!staged.exists());
    }

    // --- swap: one database ------------------------------------------------

    fn make_db(dir: &Dir, name: &str, text: &str, k: &DbKey) -> LogicalSummary {
        let (db, summary) = live_db_from(text, dir, name, k);
        drop(db);
        summary
    }

    fn open_summary(path: &Path, k: &DbKey) -> u64 {
        let db = Db::open(path, k).unwrap();
        let rows: i64 = db.accounts(true).unwrap().len().try_into().unwrap();
        rows as u64
    }

    #[test]
    fn swap_in_replaces_the_live_database_and_keeps_the_old_until_finished() {
        let dir = Dir::new();
        let k = key(4);
        let (small, _) = dump_text(10);
        let (large, _) = dump_text(50);
        make_db(&dir, LIVE_DB, &small, &k);
        make_db(&dir, "staged.db", &large, &k);

        swap_in(&dir.0, &dir.file("staged.db")).unwrap();
        assert!(!dir.file("staged.db").exists());
        assert!(
            dir.file("money.db.old").exists(),
            "the old database is kept"
        );
        let db = Db::open(&dir.file(LIVE_DB), &k).unwrap();
        assert_eq!(db.accounts(true).unwrap().len(), 5);
        drop(db);

        finish_swap(&dir.0);
        assert!(!dir.file("money.db.old").exists());
        assert_eq!(open_summary(&dir.file(LIVE_DB), &k), 5);
    }

    #[test]
    fn undo_swap_brings_the_old_database_back() {
        let dir = Dir::new();
        let k = key(4);
        let (small, expected_small) = dump_text(10);
        let (large, _) = dump_text(50);
        make_db(&dir, LIVE_DB, &small, &k);
        make_db(&dir, "staged.db", &large, &k);
        swap_in(&dir.0, &dir.file("staged.db")).unwrap();
        undo_swap(&dir.0).unwrap();
        assert!(!dir.file("money.db.old").exists());
        let db = Db::open(&dir.file(LIVE_DB), &k).unwrap();
        assert_eq!(
            db.accounts(true).unwrap()[0].balance.minor(),
            expected_small[0].1
        );
    }

    #[test]
    fn swap_refuses_while_an_earlier_swap_is_unresolved() {
        for marker in ["money.db.old", "db.key.old", "db.key.new"] {
            let dir = Dir::new();
            let k = key(4);
            let (small, _) = dump_text(10);
            make_db(&dir, LIVE_DB, &small, &k);
            make_db(&dir, "staged.db", &small, &k);
            fs::write(dir.file(marker), b"x").unwrap();
            assert!(swap_in(&dir.0, &dir.file("staged.db")).is_err(), "{marker}");
            assert!(dir.file(LIVE_DB).exists(), "{marker}");
        }
    }

    // --- swap: database and key together (recovery) -----------------------------

    const OLD_KEY: u8 = 4;
    const NEW_KEY: u8 = 9;

    /// A directory holding an *old* pair and a staged database under a new key.
    /// Returns (dir, old key blob, new key blob, old expected balance, new expected balance).
    fn pair_fixture() -> (Dir, Vec<u8>, Vec<u8>, i64, i64) {
        let dir = Dir::new();
        let (small, old_expected) = dump_text(10);
        let (large, new_expected) = dump_text(50);
        make_db(&dir, LIVE_DB, &small, &key(OLD_KEY));
        make_db(&dir, "staged.db", &large, &key(NEW_KEY));
        // The Keystore stand-in: wrapping is a reversible byte transform.
        let old_blob: Vec<u8> = [OLD_KEY; 32].iter().map(|b| b ^ 0x5a).collect();
        let new_blob: Vec<u8> = [NEW_KEY; 32].iter().map(|b| b ^ 0x5a).collect();
        fs::write(dir.file("db.key"), &old_blob).unwrap();
        (
            dir,
            old_blob,
            new_blob,
            old_expected[0].1,
            new_expected[0].1,
        )
    }

    /// Opens whatever pair is in place, the way startup does, and says which
    /// data it holds (first account balance) or None if it does not open.
    fn open_pair(dir: &Dir) -> Option<i64> {
        let blob = fs::read(dir.file("db.key")).ok()?;
        let plain: Vec<u8> = blob.iter().map(|b| b ^ 0x5a).collect();
        let bytes: [u8; 32] = plain.try_into().ok()?;
        let db = Db::open(&dir.file(LIVE_DB), &DbKey::new(bytes)).ok()?;
        db.accounts(true).ok()?.first().map(|a| a.balance.minor())
    }

    fn probe_of(dir: &Dir) -> Probe {
        if open_pair(dir).is_some() {
            Probe::Opens
        } else {
            Probe::Corrupt
        }
    }

    fn assert_no_markers(dir: &Dir, context: &str) {
        for marker in ["money.db.old", "db.key.old", "db.key.new"] {
            assert!(
                !dir.file(marker).exists(),
                "{context}: {marker} left behind"
            );
        }
    }

    const STEPS: [SwapStep; 5] = [
        SwapStep::WriteNewKey,
        SwapStep::MoveDbAside,
        SwapStep::MoveKeyAside,
        SwapStep::PromoteKey,
        SwapStep::MoveStagedIn,
    ];

    #[test]
    fn a_complete_pair_swap_puts_the_new_data_in_place_and_keeps_the_old_aside() {
        let (dir, old_blob, new_blob, _, new_expected) = pair_fixture();
        swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |_| Ok(())).unwrap();
        assert_eq!(open_pair(&dir), Some(new_expected));
        assert_eq!(
            fs::read(dir.file("db.key.old")).unwrap(),
            old_blob,
            "the old key is kept"
        );
        assert!(
            dir.file("money.db.old").exists(),
            "the old database is kept"
        );
        finish_swap(&dir.0);
        assert_no_markers(&dir, "after finish");
        assert_eq!(open_pair(&dir), Some(new_expected));
    }

    #[test]
    fn a_failure_at_any_step_leaves_the_old_data_intact_and_readable() {
        for failing in STEPS {
            let (dir, old_blob, new_blob, old_expected, _) = pair_fixture();
            let old_db_bytes = fs::read(dir.file(LIVE_DB)).unwrap();

            let err = swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |step| {
                if step == failing {
                    Err(io::Error::other("injected"))
                } else {
                    Ok(())
                }
            })
            .err();
            assert!(err.is_some(), "{failing:?}");
            assert_no_markers(&dir, &format!("{failing:?}"));
            assert_eq!(
                fs::read(dir.file("db.key")).unwrap(),
                old_blob,
                "{failing:?}: key"
            );
            assert_eq!(
                fs::read(dir.file(LIVE_DB)).unwrap(),
                old_db_bytes,
                "{failing:?}: database"
            );
            assert_eq!(
                open_pair(&dir),
                Some(old_expected),
                "{failing:?}: old data readable"
            );
            assert!(
                dir.file("staged.db").exists(),
                "{failing:?}: the staged database survives"
            );
        }
    }

    #[test]
    fn a_crash_at_any_step_is_settled_at_the_next_start_with_old_or_new_never_a_mix() {
        for crash_before in STEPS {
            let (dir, _, new_blob, old_expected, new_expected) = pair_fixture();
            // The process dies before `crash_before`: no rollback runs.
            let _ = swap_inner(
                &dir.0,
                &dir.file("staged.db"),
                Some(&new_blob),
                |step| {
                    if step == crash_before {
                        Err(io::Error::other("crash"))
                    } else {
                        Ok(())
                    }
                },
                false,
            );
            recover_interrupted(&dir.0, || probe_of(&dir));
            let state = open_pair(&dir);
            assert!(
                state == Some(old_expected) || state == Some(new_expected),
                "{crash_before:?}: the pair must open as old or as new, got {state:?}"
            );
            assert_no_markers(&dir, &format!("{crash_before:?}"));
        }
    }

    #[test]
    fn a_crash_after_the_last_rename_keeps_the_new_data() {
        let (dir, _, new_blob, _, new_expected) = pair_fixture();
        swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |_| Ok(())).unwrap();
        // Power loss before finish_swap: markers remain, the new pair opens.
        recover_interrupted(&dir.0, || probe_of(&dir));
        assert_eq!(open_pair(&dir), Some(new_expected));
        assert_no_markers(&dir, "after recovery");
    }

    #[test]
    fn a_failed_verification_goes_back_to_the_old_data() {
        let (dir, old_blob, new_blob, old_expected, _) = pair_fixture();
        let record =
            swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |_| Ok(())).unwrap();
        undo_recorded(&dir.0, &record).unwrap();
        assert_no_markers(&dir, "after undo");
        assert_eq!(fs::read(dir.file("db.key")).unwrap(), old_blob);
        assert_eq!(open_pair(&dir), Some(old_expected));
    }

    #[test]
    fn a_pair_swap_with_no_old_files_at_all_also_works_and_undoes() {
        // Recovery from "database missing" / "key missing": nothing old to keep.
        let dir = Dir::new();
        let (large, expected) = dump_text(50);
        make_db(&dir, "staged.db", &large, &key(NEW_KEY));
        let new_blob: Vec<u8> = [NEW_KEY; 32].iter().map(|b| b ^ 0x5a).collect();
        let record =
            swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |_| Ok(())).unwrap();
        assert_eq!(open_pair(&dir), Some(expected[0].1));
        undo_recorded(&dir.0, &record).unwrap();
        assert!(!dir.file(LIVE_DB).exists() && !dir.file("db.key").exists());
        assert_no_markers(&dir, "empty start");
    }

    #[test]
    fn startup_recovery_does_nothing_when_no_swap_was_interrupted() {
        let (dir, old_blob, _, old_expected, _) = pair_fixture();
        recover_interrupted(&dir.0, || unreachable!("not consulted without markers"));
        assert_eq!(fs::read(dir.file("db.key")).unwrap(), old_blob);
        assert_eq!(open_pair(&dir), Some(old_expected));
    }

    #[test]
    fn startup_recovery_covers_the_single_database_crash_points_too() {
        let k = key(4);
        let (small, expected_small) = dump_text(10);
        let (large, expected_large) = dump_text(50);
        let blob: Vec<u8> = [4u8; 32].iter().map(|b| b ^ 0x5a).collect();

        // Crash between the two renames: money.db is missing, money.db.old holds the data.
        let dir = Dir::new();
        fs::write(dir.file("db.key"), &blob).unwrap();
        make_db(&dir, "money.db.old", &small, &k);
        recover_interrupted(&dir.0, || {
            unreachable!("not consulted when the live database is gone")
        });
        assert_eq!(open_pair(&dir), Some(expected_small[0].1));

        // Crash after the swap, new database readable: keep it, drop the old.
        let dir = Dir::new();
        fs::write(dir.file("db.key"), &blob).unwrap();
        make_db(&dir, LIVE_DB, &large, &k);
        make_db(&dir, "money.db.old", &small, &k);
        recover_interrupted(&dir.0, || probe_of(&dir));
        assert_eq!(open_pair(&dir), Some(expected_large[0].1));
        assert_no_markers(&dir, "kept new");

        // Crash after the swap, new database unreadable: put the old one back.
        let dir = Dir::new();
        fs::write(dir.file("db.key"), &blob).unwrap();
        fs::write(dir.file(LIVE_DB), b"garbage, not a database").unwrap();
        make_db(&dir, "money.db.old", &small, &k);
        recover_interrupted(&dir.0, || probe_of(&dir));
        assert_eq!(open_pair(&dir), Some(expected_small[0].1));
    }

    // --- F13: only a definite answer rolls back --------------------------------

    #[test]
    fn a_probe_that_cannot_tell_changes_nothing_and_keeps_both_copies() {
        let (dir, old_blob, new_blob, _, new_expected) = pair_fixture();
        swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |_| Ok(())).unwrap();
        let before: Vec<_> = ["money.db", "money.db.old", "db.key", "db.key.old"]
            .iter()
            .map(|name| fs::read(dir.file(name)).unwrap())
            .collect();

        let outcome = recover_interrupted(&dir.0, || Probe::Unknown);
        assert_eq!(outcome, Recovered::Undecided);

        let after: Vec<_> = ["money.db", "money.db.old", "db.key", "db.key.old"]
            .iter()
            .map(|name| fs::read(dir.file(name)).unwrap())
            .collect();
        assert_eq!(
            before, after,
            "an unknown answer must not move or delete anything"
        );
        assert_eq!(fs::read(dir.file("db.key.old")).unwrap(), old_blob);
        assert_eq!(
            open_pair(&dir),
            Some(new_expected),
            "the newer data is still in place"
        );
    }

    #[test]
    fn a_stale_old_copy_never_replaces_newer_data_because_of_a_transient_error() {
        // finish_swap's deletion failed once, so .old survived for a long time.
        // Later a transient error (busy, I/O) hits the probe at startup.
        let (dir, _, new_blob, old_expected, new_expected) = pair_fixture();
        swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |_| Ok(())).unwrap();

        assert_eq!(
            recover_interrupted(&dir.0, || Probe::Unknown),
            Recovered::Undecided
        );
        assert_eq!(
            open_pair(&dir),
            Some(new_expected),
            "newer data kept after the transient error"
        );

        // The error passes: the new pair opens, so the stale copy is simply deleted.
        assert_eq!(
            recover_interrupted(&dir.0, || probe_of(&dir)),
            Recovered::Committed
        );
        assert_eq!(open_pair(&dir), Some(new_expected));
        assert_ne!(new_expected, old_expected);
        assert_no_markers(&dir, "after commit");
    }

    #[test]
    fn a_definite_corruption_rolls_back_and_a_definite_success_commits() {
        let (dir, _, new_blob, old_expected, _) = pair_fixture();
        swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |_| Ok(())).unwrap();
        assert_eq!(
            recover_interrupted(&dir.0, || Probe::Corrupt),
            Recovered::RolledBack
        );
        assert_eq!(open_pair(&dir), Some(old_expected));
        assert_no_markers(&dir, "after rollback");

        let (dir, _, new_blob, _, new_expected) = pair_fixture();
        swap_with(&dir.0, &dir.file("staged.db"), Some(&new_blob), |_| Ok(())).unwrap();
        assert_eq!(
            recover_interrupted(&dir.0, || Probe::Opens),
            Recovered::Committed
        );
        assert_eq!(open_pair(&dir), Some(new_expected));
    }

    #[test]
    fn an_unfinished_key_or_a_missing_database_is_definite_without_asking_the_probe() {
        // db.key.new still present: the renames never finished.
        let (dir, old_blob, new_blob, old_expected, _) = pair_fixture();
        let _ = swap_inner(
            &dir.0,
            &dir.file("staged.db"),
            Some(&new_blob),
            |step| {
                if step == SwapStep::MoveKeyAside {
                    Err(io::Error::other("crash"))
                } else {
                    Ok(())
                }
            },
            false,
        );
        assert!(dir.file("db.key.new").exists());
        let outcome = recover_interrupted(&dir.0, || unreachable!("definite without the probe"));
        assert_eq!(outcome, Recovered::RolledBack);
        assert_eq!(fs::read(dir.file("db.key")).unwrap(), old_blob);
        assert_eq!(open_pair(&dir), Some(old_expected));

        // The live database is gone but money.db.old exists.
        let dir = Dir::new();
        let k = key(4);
        let (small, expected) = dump_text(10);
        make_db(&dir, "money.db.old", &small, &k);
        fs::write(
            dir.file("db.key"),
            [4u8; 32].iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
        )
        .unwrap();
        let outcome = recover_interrupted(&dir.0, || unreachable!("definite without the probe"));
        assert_eq!(outcome, Recovered::RolledBack);
        assert_eq!(open_pair(&dir), Some(expected[0].1));
    }

    #[test]
    fn nothing_is_reported_when_no_swap_was_interrupted() {
        let (dir, _, _, _, _) = pair_fixture();
        assert_eq!(
            recover_interrupted(&dir.0, || unreachable!()),
            Recovered::Nothing
        );
    }

    // --- scale ------------------------------------------------------------

    /// 100 000 transactions end to end, with the default (production) KDF cost.
    /// Run: `cargo test -p money-tracker-app --release -- --ignored --nocapture big_backup`
    /// and add `/usr/bin/time -l` on the test binary for peak memory.
    #[test]
    #[allow(clippy::print_stdout, reason = "this test exists to report timings")]
    #[ignore = "slow: 100k rows with production Argon2 cost; run with --release --ignored"]
    fn big_backup_round_trip_with_timing() {
        let dir = Dir::new();
        let k = key(8);
        let (text, expected) = dump_text(100_000);

        let t = Instant::now();
        let (db, original) = live_db_from(&text, &dir, "src.db", &k);
        println!("import of the plain dump: {:?}", t.elapsed());
        assert_eq!(original.balances, expected);
        drop(text);

        let file = dir.file("big.mtbak");
        let t = Instant::now();
        let exported = {
            let out = fs::File::create(&file).unwrap();
            write_backup(out, PASS, KdfParams::DEFAULT, |w| {
                db.export_logical(w).map_err(BackupFailure::from)
            })
            .unwrap()
        };
        println!("export + encrypt (m=64MiB t=3): {:?}", t.elapsed());
        println!("backup size: {} bytes", fs::metadata(&file).unwrap().len());
        assert_eq!(exported, original);

        let staged = dir.file("staged.db");
        let t = Instant::now();
        let restored = stage_restore(fs::File::open(&file).unwrap(), PASS, &k, &staged).unwrap();
        println!("decrypt + restore + verify: {:?}", t.elapsed());
        assert_eq!(restored, original);
        assert_eq!(restored.count("transactions"), 100_000);

        let mut bytes = Vec::new();
        fs::File::open(&staged)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert!(
            !bytes.windows(7).any(|w| w == b"catatan"),
            "staged file is encrypted"
        );
    }
}
