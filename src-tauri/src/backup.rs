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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapStep {
    MoveLiveAside,
    MoveStagedIn,
}

fn old_path(dir: &Path) -> PathBuf {
    with_suffix(&dir.join(LIVE_DB), OLD_SUFFIX)
}

fn rename_set(from: &Path, to: &Path) -> io::Result<()> {
    for suffix in COMPANIONS {
        let (src, dst) = (with_suffix(from, suffix), with_suffix(to, suffix));
        if src.exists() {
            fs::rename(src, dst)?;
        }
    }
    Ok(())
}

/// Replaces the live database with `staged`, keeping the old one aside as
/// `money.db.old`. Both databases must be closed. On failure the old database
/// is put back before returning.
pub fn swap_in(dir: &Path, staged: &Path) -> io::Result<()> {
    swap_in_with(dir, staged, |_| Ok(()))
}

pub fn swap_in_with(
    dir: &Path,
    staged: &Path,
    mut step: impl FnMut(SwapStep) -> io::Result<()>,
) -> io::Result<()> {
    let live = dir.join(LIVE_DB);
    let old = old_path(dir);
    // A leftover from an earlier crash is resolved at startup, never here.
    if old.exists() {
        return Err(io::Error::other("an earlier restore is unresolved"));
    }

    step(SwapStep::MoveLiveAside)?;
    rename_set(&live, &old)?;
    let moved_in = step(SwapStep::MoveStagedIn).and_then(|()| rename_set(staged, &live));
    if let Err(err) = moved_in {
        // Undo: whatever reached `live` is removed and the old set returns.
        remove_database(&live);
        rename_set(&old, &live)?;
        return Err(err);
    }
    Ok(())
}

/// The new database opened and verified: the old one can go.
pub fn finish_swap(dir: &Path) {
    remove_database(&old_path(dir));
}

/// The new database was bad: restore the old one.
pub fn undo_swap(dir: &Path) -> io::Result<()> {
    let live = dir.join(LIVE_DB);
    remove_database(&live);
    rename_set(&old_path(dir), &live)
}

/// Resolves a restore that was interrupted by a crash or a power loss.
/// Call before opening the database at startup.
///
/// * live missing, old present: the swap stopped between its two renames, so
///   the old database comes back.
/// * both present: the swap completed but was not confirmed; the new database
///   stays if `new_opens` says it is readable, otherwise the old one returns.
pub fn recover_interrupted(dir: &Path, new_opens: impl FnOnce(&Path) -> bool) {
    let live = dir.join(LIVE_DB);
    let old = old_path(dir);
    let old_exists = old.exists();
    if !old_exists {
        return;
    }
    if !live.exists() {
        let _ = rename_set(&old, &live);
    } else if new_opens(&live) {
        remove_database(&old);
    } else {
        let _ = undo_swap(dir);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Read,
        sync::atomic::{AtomicU32, Ordering},
        time::Instant,
    };

    use zeroize::Zeroizing;

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
        Zeroizing::new([byte; 32])
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

    // --- swap -------------------------------------------------------------

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
        let tx = db.accounts(true).unwrap();
        assert_eq!(tx.len(), 5);
        drop(db);

        finish_swap(&dir.0);
        assert!(!dir.file("money.db.old").exists());
        assert_eq!(open_summary(&dir.file(LIVE_DB), &k), 5);
    }

    #[test]
    fn undo_swap_brings_the_old_database_back() {
        let dir = Dir::new();
        let k = key(4);
        let (small, _) = dump_text(10);
        let (large, _) = dump_text(50);
        make_db(&dir, LIVE_DB, &small, &k);
        make_db(&dir, "staged.db", &large, &k);
        swap_in(&dir.0, &dir.file("staged.db")).unwrap();
        undo_swap(&dir.0).unwrap();
        assert!(!dir.file("money.db.old").exists());
        let db = Db::open(&dir.file(LIVE_DB), &k).unwrap();
        // The original has only 10 transactions: balance of account 1 shows it.
        let first = db.accounts(true).unwrap()[0].balance.minor();
        let (_, expected_small) = dump_text(10);
        assert_eq!(first, expected_small[0].1);
    }

    #[test]
    fn a_failure_at_either_step_leaves_the_old_database_in_place() {
        for failing in [SwapStep::MoveLiveAside, SwapStep::MoveStagedIn] {
            let dir = Dir::new();
            let k = key(4);
            let (small, expected) = dump_text(10);
            let (large, _) = dump_text(50);
            make_db(&dir, LIVE_DB, &small, &k);
            make_db(&dir, "staged.db", &large, &k);

            let err = swap_in_with(&dir.0, &dir.file("staged.db"), |step| {
                if step == failing {
                    Err(io::Error::other("injected"))
                } else {
                    Ok(())
                }
            })
            .err();
            assert!(err.is_some(), "{failing:?}");
            assert!(
                !dir.file("money.db.old").exists(),
                "{failing:?}: old set not restored"
            );
            assert!(
                dir.file("staged.db").exists(),
                "{failing:?}: staged file must survive"
            );
            let db = Db::open(&dir.file(LIVE_DB), &k).unwrap();
            assert_eq!(
                db.accounts(true).unwrap()[0].balance.minor(),
                expected[0].1,
                "{failing:?}"
            );
        }
    }

    #[test]
    fn startup_recovery_handles_both_crash_points() {
        let k = key(4);
        let (small, expected_small) = dump_text(10);
        let (large, expected_large) = dump_text(50);

        // Crash between the two renames: money.db is missing, money.db.old holds the data.
        let dir = Dir::new();
        make_db(&dir, "money.db.old", &small, &k);
        recover_interrupted(&dir.0, |_| {
            unreachable!("not consulted when live is missing")
        });
        let db = Db::open(&dir.file(LIVE_DB), &k).unwrap();
        assert_eq!(
            db.accounts(true).unwrap()[0].balance.minor(),
            expected_small[0].1
        );
        assert!(!dir.file("money.db.old").exists());
        drop(db);

        // Crash after the swap, new database readable: keep it, drop the old.
        let dir = Dir::new();
        make_db(&dir, LIVE_DB, &large, &k);
        make_db(&dir, "money.db.old", &small, &k);
        recover_interrupted(&dir.0, |path| Db::open(path, &k).is_ok());
        let db = Db::open(&dir.file(LIVE_DB), &k).unwrap();
        assert_eq!(
            db.accounts(true).unwrap()[0].balance.minor(),
            expected_large[0].1
        );
        assert!(!dir.file("money.db.old").exists());
        drop(db);

        // Crash after the swap, new database unreadable: put the old one back.
        let dir = Dir::new();
        fs::write(dir.file(LIVE_DB), b"garbage, not a database").unwrap();
        make_db(&dir, "money.db.old", &small, &k);
        recover_interrupted(&dir.0, |path| Db::open(path, &k).is_ok());
        let db = Db::open(&dir.file(LIVE_DB), &k).unwrap();
        assert_eq!(
            db.accounts(true).unwrap()[0].balance.minor(),
            expected_small[0].1
        );
    }

    #[test]
    fn recovery_does_nothing_when_no_restore_was_interrupted() {
        let dir = Dir::new();
        let k = key(4);
        let (small, _) = dump_text(10);
        make_db(&dir, LIVE_DB, &small, &k);
        recover_interrupted(&dir.0, |_| unreachable!());
        assert!(dir.file(LIVE_DB).exists());
    }

    #[test]
    fn swap_refuses_while_an_earlier_restore_is_unresolved() {
        let dir = Dir::new();
        let k = key(4);
        let (small, _) = dump_text(10);
        make_db(&dir, LIVE_DB, &small, &k);
        make_db(&dir, "staged.db", &small, &k);
        fs::write(dir.file("money.db.old"), b"x").unwrap();
        assert!(swap_in(&dir.0, &dir.file("staged.db")).is_err());
        assert!(dir.file(LIVE_DB).exists());
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
