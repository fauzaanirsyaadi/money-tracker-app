//! Backup and restore commands.
//!
//! Export: encrypt to a private temporary file, then hand it to the system
//! "save as" picker, which only ever sees ciphertext. Restore is three steps so
//! the user confirms with the facts in front of them: choose a file, enter the
//! passphrase (verified and staged without touching live data), then confirm.

use std::{fs, io, path::PathBuf, time::Instant};

use mt_backup::KdfParams;
use mt_storage::{Db, LogicalSummary};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_mt_platform::{FileOutcome, MtPlatformExt};
use zeroize::Zeroizing;

use crate::{
    backup::{self, BackupFailure},
    error::{AppError, ErrorCode},
    lock::require_unlocked,
    state::{AppState, StagedRestore},
    vault,
};

const PICKED_FILE: &str = "restore-input.mtbak";
const STAGED_FILE: &str = "restore-staged.db";
const EXPORT_PREFIX: &str = "export-";
const DEFAULT_NAME: &str = "money-tracker.mtbak";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
                "wrong passphrase or damaged backup",
            ),
            BackupFailure::NotABackup => {
                Self::new(ErrorCode::NotABackup, "not a Money Tracker backup")
            }
            BackupFailure::Unsupported => Self::new(
                ErrorCode::UnsupportedBackup,
                "the backup was made by a newer version",
            ),
            BackupFailure::Damaged => Self::new(
                ErrorCode::BackupDamaged,
                "the backup is incomplete or damaged",
            ),
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
    if let Ok(cache) = app.path().app_cache_dir() {
        let _ = fs::remove_file(cache.join(PICKED_FILE));
        if let Ok(entries) = fs::read_dir(&cache) {
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
    if let Ok(dir) = vault::data_dir(app) {
        backup::remove_database(&dir.join(STAGED_FILE));
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
    require_unlocked(&state)?;
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
    require_unlocked(&state)?;
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
            let key = vault::load_existing_key(&app).map_err(|_| internal())?;
            let file = fs::File::open(&picked).map_err(|_| internal())?;
            backup::stage_restore(file, &passphrase, &key, &staged_path).map_err(AppError::from)
        })
        .await
    };

    match result {
        Ok(summary) => {
            let current =
                state.with_db(|db| db.export_logical(&mut io::sink()).map_err(AppError::from))?;
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
    require_unlocked(&state)?;
    let staged = state
        .restore_session(|s| s.staged.take())
        .ok_or_else(|| AppError::invalid("no verified backup is waiting"))?;
    // The picked copy (if any) is stale from here on.
    state.restore_session(|s| {
        if let Some(path) = s.picked.take() {
            let _ = fs::remove_file(path);
        }
    });

    blocking(move || {
        let state = app.state::<AppState>();
        commit(&app, &state, &staged)
    })
    .await
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
