//! Backup and restore commands.
//!
//! Export: encrypt to a private temporary file, then hand it to the system
//! "save as" picker, which only ever sees ciphertext. Restore is three steps so
//! the user confirms with the facts in front of them: choose a file, enter the
//! passphrase (verified and staged without touching live data), then confirm.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Instant,
};

use mt_backup::KdfParams;
use mt_storage::{Db, LogicalSummary};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_mt_keystore::MtKeystoreExt;
use tauri_plugin_mt_platform::{FileOutcome, MtPlatformExt};
use zeroize::Zeroizing;

use crate::{
    backup::{self, BackupFailure},
    error::{AppError, ErrorCode},
    lock::require_unlocked,
    state::{AppState, StagedRestore, StorageProblem},
    vault::{self, RecoveryReason},
};

const PICKED_FILE: &str = "restore-input.mtbak";
pub(crate) const STAGED_FILE: &str = "restore-staged.db";
const EXPORT_PREFIX: &str = "export-";
const DEFAULT_NAME: &str = "money-tracker.mtbak";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountsDto {
    pub accounts: u64,
    pub categories: u64,
    pub transactions: u64,
    pub budgets: u64,
    pub recurring_rules: u64,
}

impl From<&LogicalSummary> for CountsDto {
    fn from(summary: &LogicalSummary) -> Self {
        Self {
            accounts: summary.count("accounts"),
            categories: summary.count("categories"),
            transactions: summary.count("transactions"),
            budgets: summary.count("budgets"),
            recurring_rules: summary.count("recurring_rules"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportDto {
    /// False when the user dismissed the "save as" picker.
    pub saved: bool,
    pub counts: CountsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreviewDto {
    pub backup: CountsDto,
    pub current: CountsDto,
}

impl From<BackupFailure> for AppError {
    fn from(failure: BackupFailure) -> Self {
        match failure {
            BackupFailure::WeakPassphrase(_) => {
                Self::new(ErrorCode::WeakPassphrase, "the passphrase is too weak")
            }
            BackupFailure::WrongPassphrase => Self::new(
                ErrorCode::WrongPassphrase,
                "wrong passphrase or corrupted backup",
            ),
            BackupFailure::NotABackup => {
                Self::new(ErrorCode::NotABackup, "not a Money Tracker backup")
            }
            BackupFailure::Unsupported => Self::new(
                ErrorCode::UnsupportedBackup,
                "the backup was made by a newer version",
            ),
            BackupFailure::Damaged => {
                Self::new(ErrorCode::BackupDamaged, "the backup is corrupted")
            }
            BackupFailure::Io | BackupFailure::Storage => Self::new(
                ErrorCode::Internal,
                "could not complete the backup operation",
            ),
        }
    }
}

fn internal() -> AppError {
    AppError::new(ErrorCode::Internal, "internal error")
}

/// Restoring is allowed when unlocked, and also from the recovery screen,
/// where there is no readable data and so nothing for a PIN to protect.
fn require_unlocked_or_recovering(state: &AppState) -> Result<(), AppError> {
    if state.recovering() {
        Ok(())
    } else {
        require_unlocked(state)
    }
}

/// Keeps only characters that are safe in a file name suggested to the picker.
pub fn safe_file_name(requested: &str) -> String {
    let stem: String = requested
        .trim_end_matches(".mtbak")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(60)
        .collect();
    let stem = stem.trim_matches('.');
    if stem.is_empty() {
        DEFAULT_NAME.to_owned()
    } else {
        format!("{stem}.mtbak")
    }
}

fn cache_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app.path().app_cache_dir().map_err(|_| internal())?;
    fs::create_dir_all(&dir).map_err(|_| internal())?;
    Ok(dir)
}

/// Deletes files a previous run may have left behind (crash, process death
/// while a picker was open). Safe to call at startup.
pub fn clean_leftovers(app: &AppHandle) {
    let cache = app.path().app_cache_dir().ok();
    let data = vault::data_dir(app).ok();
    remove_leftovers(cache.as_deref(), data.as_deref());
}

/// The part of [`clean_leftovers`] that needs no Tauri: temporary ciphertext in
/// the cache (exports in flight, a chosen restore file) and a staged database.
fn remove_leftovers(cache: Option<&Path>, data: Option<&Path>) {
    if let Some(cache) = cache {
        let _ = fs::remove_file(cache.join(PICKED_FILE));
        if let Ok(entries) = fs::read_dir(cache) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(EXPORT_PREFIX)
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
    if let Some(data) = data {
        backup::remove_database(&data.join(STAGED_FILE));
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| internal())?
}

fn random_suffix() -> Result<String, AppError> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| internal())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Exports an encrypted backup through the system "save as" picker.
#[tauri::command]
pub async fn backup_export(
    app: AppHandle,
    state: State<'_, AppState>,
    passphrase: String,
    name: String,
) -> Result<ExportDto, AppError> {
    require_unlocked(&state)?;
    let passphrase = Zeroizing::new(passphrase);
    mt_backup::validate_passphrase(&passphrase)
        .map_err(|problem| AppError::from(BackupFailure::WeakPassphrase(problem)))?;

    let temp = cache_dir(&app)?.join(format!("{EXPORT_PREFIX}{}.mtbak", random_suffix()?));

    let summary = {
        let (app, temp) = (app.clone(), temp.clone());
        blocking(move || {
            let file = fs::File::create(&temp).map_err(|_| internal())?;
            let state = app.state::<AppState>();
            backup::write_backup(file, &passphrase, KdfParams::DEFAULT, |out| {
                state
                    .with_db(|db| db.export_logical(out).map_err(AppError::from))
                    .map_err(|_| BackupFailure::Storage)
            })
            .map_err(AppError::from)
        })
        .await
    };
    let summary = match summary {
        Ok(summary) => summary,
        Err(err) => {
            let _ = fs::remove_file(&temp);
            return Err(err);
        }
    };

    // The picker pauses the app; the lock must not fire while it is open.
    state.applock().begin_external_ui(Instant::now());
    let outcome = app
        .mt_platform()
        .save_file(&temp, &safe_file_name(&name))
        .await;
    state.applock().end_external_ui();
    let _ = fs::remove_file(&temp);

    match outcome {
        Ok(FileOutcome::Done) => Ok(ExportDto {
            saved: true,
            counts: CountsDto::from(&summary),
        }),
        Ok(FileOutcome::Cancelled) => Ok(ExportDto {
            saved: false,
            counts: CountsDto::from(&summary),
        }),
        Err(_) => Err(internal()),
    }
}

/// Step 1 of a restore: let the user choose the backup file.
#[tauri::command]
pub async fn backup_restore_pick(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    require_unlocked_or_recovering(&state)?;
    state.clear_restore();
    let destination = cache_dir(&app)?.join(PICKED_FILE);

    state.applock().begin_external_ui(Instant::now());
    let outcome = app.mt_platform().pick_file(&destination).await;
    state.applock().end_external_ui();

    match outcome {
        Ok(FileOutcome::Done) => {
            state.restore_session(|s| s.picked = Some(destination));
            Ok(true)
        }
        Ok(FileOutcome::Cancelled) => Ok(false),
        Err(_) => {
            let _ = fs::remove_file(&destination);
            Err(internal())
        }
    }
}

/// Step 2: decrypt and verify the chosen backup into a separate database.
/// Live data is not touched. A wrong passphrase may be retried.
#[tauri::command]
pub async fn backup_restore_prepare(
    app: AppHandle,
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<RestorePreviewDto, AppError> {
    require_unlocked_or_recovering(&state)?;
    let recovering = state.recovering();
    let passphrase = Zeroizing::new(passphrase);
    let picked = state
        .restore_session(|s| s.picked.clone())
        .ok_or_else(|| AppError::invalid("no backup file has been chosen"))?;
    // Any earlier staged result is replaced.
    state.restore_session(|s| {
        if let Some(old) = s.staged.take() {
            backup::remove_database(&old.path);
        }
    });

    let staged_path = vault::data_dir(&app)
        .map_err(|_| internal())?
        .join(STAGED_FILE);
    backup::remove_database(&staged_path);

    let result = {
        let (app, picked, staged_path) = (app.clone(), picked.clone(), staged_path.clone());
        blocking(move || {
            // Replacing unreadable data needs a new key; otherwise the live one is reused.
            let key = if recovering {
                vault::fresh_key().map_err(|_| internal())?
            } else {
                vault::load_existing_key(&app).map_err(|_| internal())?
            };
            let file = fs::File::open(&picked).map_err(|_| internal())?;
            let summary = backup::stage_restore(file, &passphrase, &key, &staged_path)
                .map_err(AppError::from)?;
            Ok((summary, key))
        })
        .await
    };

    match result {
        Ok((summary, key)) => {
            let current = if recovering {
                // Nothing readable is being replaced.
                LogicalSummary::default()
            } else {
                state.with_db(|db| db.export_logical(&mut io::sink()).map_err(AppError::from))?
            };
            let preview = RestorePreviewDto {
                backup: CountsDto::from(&summary),
                current: CountsDto::from(&current),
            };
            state.restore_session(|s| {
                // The encrypted copy is no longer needed.
                if let Some(path) = s.picked.take() {
                    let _ = fs::remove_file(path);
                }
                s.staged = Some(StagedRestore {
                    path: staged_path,
                    summary,
                });
                s.fresh_key = recovering.then_some(key);
            });
            Ok(preview)
        }
        Err(err) => {
            // A wrong passphrase keeps the file so the user can try again;
            // anything else means this file cannot work, so discard it.
            if err.code != ErrorCode::WrongPassphrase {
                state.clear_restore();
            }
            Err(err)
        }
    }
}

/// Step 3: replace the live database with the staged one. The caller must have
/// shown the user the preview and obtained confirmation.
#[tauri::command]
pub async fn backup_restore_commit(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CountsDto, AppError> {
    require_unlocked_or_recovering(&state)?;
    let recovering = state.recovering();
    if recovering {
        // Before the staged restore is touched: a refusal must leave it waiting,
        // so that entering the PIN and trying again is all that is needed.
        crate::recovery::require_recovery_authority(&state, Instant::now())?;
    }
    let staged = state
        .restore_session(|s| s.staged.take())
        .ok_or_else(|| AppError::invalid("no verified backup is waiting"))?;
    let fresh_key = state.restore_session(|s| s.fresh_key.take());
    // The picked copy (if any) is stale from here on.
    state.restore_session(|s| {
        if let Some(path) = s.picked.take() {
            let _ = fs::remove_file(path);
        }
    });

    blocking(move || {
        let state = app.state::<AppState>();
        match (recovering, fresh_key) {
            (true, Some(key)) => {
                // Replacing unreadable data is destructive: the PIN first when a
                // valid lock exists, and a second look that still finds the data
                // unreadable.
                crate::recovery::require_recovery_authority(&state, Instant::now())?;
                crate::recovery::recheck_before_destroying(&app, &state)?;
                commit_recovery(&app, &state, &staged, &key)
            }
            (true, None) => Err(internal()),
            (false, _) => commit(&app, &state, &staged),
        }
    })
    .await
}

/// Replaces data that cannot be read with the staged one, under a new key.
/// The user confirmed this on the recovery screen.
///
/// Destroy last. Everything that can fail is done **before** any old file or
/// key is touched:
/// 1. the staged database is already built and verified (restore) or is a fresh
///    empty one (start fresh);
/// 2. the database key is wrapped, and the wrapped key is unwrapped again to
///    prove it works;
/// 3. the old and the new files are exchanged together, keeping the old ones
///    as `*.old`; any failure puts them back;
/// 4. the new files are opened and compared with what was staged. Only then are
///    the old files deleted and the PIN, which belonged to the old data, erased.
///
/// The one unavoidable exception: when the old wrapping key itself is
/// permanently unusable, it is replaced first, since the Keystore holds a single
/// key under one name and a new key cannot be wrapped with an unusable one.
/// Nothing that key protected can be read anyway; the second look made just
/// before still found it unusable.
pub(crate) fn commit_recovery(
    app: &AppHandle,
    state: &AppState,
    staged: &StagedRestore,
    key: &mt_storage::DbKey,
) -> Result<CountsDto, AppError> {
    let Some(StorageProblem::Recovery(reason)) = state.problem() else {
        return Err(internal());
    };
    let dir = vault::data_dir(app).map_err(|_| internal())?;
    let keystore = app.mt_keystore();
    state.close_db();

    if reason == RecoveryReason::KeyUnusable {
        keystore.reset().map_err(|_| internal())?;
    }
    let blob = keystore.wrap(key.as_slice()).map_err(|_| internal())?;
    // The new wrapped key must open to exactly the key it wraps, before any file moves.
    let round_trip = keystore.unwrap(&blob).map_err(|_| internal())?;
    if round_trip.as_slice() != key.as_slice() {
        return Err(internal());
    }

    let record =
        backup::swap_with(&dir, &staged.path, Some(&blob), |_| Ok(())).map_err(|_| internal())?;

    // Verify the files now in place, without settling anything: the swap is not
    // complete until this succeeds.
    let verified = vault::open_current_pair(app).ok().and_then(|db| {
        let found = db.export_logical(&mut io::sink()).ok()?;
        (found == staged.summary).then_some(db)
    });
    match verified {
        Some(db) => {
            backup::finish_swap(&dir);
            // The PIN belonged to the old data. It is set again afterwards.
            let _ = state.applock().erase();
            state.install_db(db);
            state.set_problem(None);
            Ok(CountsDto::from(&staged.summary))
        }
        None => {
            // The old files come back. If even that fails, the markers remain and
            // the next start settles it.
            let _ = backup::undo_recorded(&dir, &record);
            crate::recovery::initialize(app, state);
            Err(AppError::from(BackupFailure::Damaged))
        }
    }
}

/// "Start fresh" as a recovery commit: an empty database under a new key takes
/// the place of the unreadable data through the same destroy-last path.
pub(crate) fn start_fresh_commit(app: &AppHandle, state: &AppState) -> Result<(), AppError> {
    let dir = vault::data_dir(app).map_err(|_| internal())?;
    let key = vault::fresh_key().map_err(|_| internal())?;
    let staged_path = dir.join(STAGED_FILE);
    backup::remove_database(&staged_path);
    let summary = {
        let db = Db::open(&staged_path, &key).map_err(|_| internal())?;
        db.export_logical(&mut io::sink()).map_err(AppError::from)?
    };
    let staged = StagedRestore {
        path: staged_path.clone(),
        summary,
    };
    let result = commit_recovery(app, state, &staged, &key).map(|_| ());
    if result.is_err() {
        backup::remove_database(&staged_path);
    }
    result
}

fn commit(
    app: &AppHandle,
    state: &AppState,
    staged: &StagedRestore,
) -> Result<CountsDto, AppError> {
    let dir = vault::data_dir(app).map_err(|_| internal())?;
    let key = vault::load_existing_key(app).map_err(|_| internal())?;
    let live = dir.join(backup::LIVE_DB);

    // Close the live connection so its files can be renamed, then try the swap.
    drop(state.take_db());
    let reopen_old = |state: &AppState| {
        if let Ok(db) = Db::open(&live, &key) {
            state.install_db(db);
        }
    };

    if backup::swap_in(&dir, &staged.path).is_err() {
        backup::remove_database(&staged.path);
        reopen_old(state);
        return Err(internal());
    }

    // The new database must open and report exactly what was staged.
    let verified = Db::open(&live, &key).ok().and_then(|db| {
        let found = db.export_logical(&mut io::sink()).ok()?;
        (found == staged.summary).then_some(db)
    });
    match verified {
        Some(db) => {
            backup::finish_swap(&dir);
            state.install_db(db);
            Ok(CountsDto::from(&staged.summary))
        }
        None => {
            let _ = backup::undo_swap(&dir);
            reopen_old(state);
            Err(AppError::from(BackupFailure::Damaged))
        }
    }
}

/// Abandons a restore and deletes what it created.
#[tauri::command]
pub async fn backup_restore_cancel(state: State<'_, AppState>) -> Result<(), AppError> {
    state.clear_restore();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_reduced_to_safe_characters() {
        assert_eq!(
            safe_file_name("money-tracker-20261014.mtbak"),
            "money-tracker-20261014.mtbak"
        );
        assert_eq!(safe_file_name("../../etc/passwd"), "etcpasswd.mtbak");
        assert_eq!(safe_file_name("cadangan saya!.mtbak"), "cadangansaya.mtbak");
        assert_eq!(safe_file_name(""), DEFAULT_NAME);
        assert_eq!(safe_file_name("...."), DEFAULT_NAME);
        assert_eq!(safe_file_name(&"a".repeat(200)).len(), 60 + ".mtbak".len());
    }

    #[test]
    fn leftovers_from_a_crash_are_removed_at_startup() {
        let root = std::env::temp_dir().join(format!("mt-leftovers-{}", std::process::id()));
        let (cache, data) = (root.join("cache"), root.join("data"));
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&data).unwrap();
        // What a crash mid-export, mid-pick or mid-restore leaves behind...
        let leftovers = [
            cache.join("export-0011223344556677.mtbak"),
            cache.join("export-aabbccddeeff0011.mtbak"),
            cache.join(PICKED_FILE),
            data.join(STAGED_FILE),
            data.join(format!("{STAGED_FILE}-wal")),
            data.join(format!("{STAGED_FILE}-shm")),
        ];
        for file in &leftovers {
            fs::write(file, b"ciphertext or staged data").unwrap();
        }
        // ...and files that must survive.
        let keep = [
            cache.join("something-else.tmp"),
            data.join(crate::backup::LIVE_DB),
            data.join("db.key"),
            data.join("lock.json"),
        ];
        for file in &keep {
            fs::write(file, b"keep").unwrap();
        }

        remove_leftovers(Some(&cache), Some(&data));

        for file in &leftovers {
            assert!(!file.exists(), "{} was not removed", file.display());
        }
        for file in &keep {
            assert!(file.exists(), "{} must not be removed", file.display());
        }
        // Running it again on a clean directory, or with no directories, is harmless.
        remove_leftovers(Some(&cache), Some(&data));
        remove_leftovers(None, None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn counts_come_from_the_summary() {
        let mut summary = LogicalSummary::default();
        summary.rows.insert("accounts".into(), 3);
        summary.rows.insert("transactions".into(), 120);
        let counts = CountsDto::from(&summary);
        assert_eq!(
            (counts.accounts, counts.transactions, counts.budgets),
            (3, 120, 0)
        );
    }

    #[test]
    fn failures_map_to_distinct_codes() {
        let code = |f: BackupFailure| AppError::from(f).code;
        assert_eq!(
            code(BackupFailure::WrongPassphrase),
            ErrorCode::WrongPassphrase
        );
        assert_eq!(code(BackupFailure::NotABackup), ErrorCode::NotABackup);
        assert_eq!(
            code(BackupFailure::Unsupported),
            ErrorCode::UnsupportedBackup
        );
        assert_eq!(code(BackupFailure::Damaged), ErrorCode::BackupDamaged);
        assert_eq!(
            code(BackupFailure::WeakPassphrase(
                mt_backup::PassphraseProblem::TooShort
            )),
            ErrorCode::WeakPassphrase
        );
    }
}
