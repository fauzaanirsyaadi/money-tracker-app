//! Storage start-up and recovery.
//!
//! The app never makes a new database or key on its own once data has existed
//! (see `vault`). When data is lost or unreadable the UI shows a recovery
//! screen with exactly two ways out, both explicit: restore from a backup
//! (the normal restore flow, building a *new* key), or start fresh after a
//! confirmation that says the old data will be lost. A failure that may pass,
//! such as a busy Keystore, only offers "try again".

use tauri::{AppHandle, Manager, State};
use tauri_plugin_mt_keystore::MtKeystoreExt;

use crate::{
    error::{AppError, ErrorCode},
    lock::{LockStatusDto, status_of},
    state::{AppState, StorageProblem},
    vault::{self, VaultError},
};

pub fn problem_of(err: &VaultError) -> StorageProblem {
    match err {
        VaultError::Recovery(reason) => StorageProblem::Recovery(*reason),
        VaultError::Io | VaultError::Unavailable => StorageProblem::Retry,
    }
}

/// Inspects the files, and opens the database now unless an app lock defers it
/// to the unlock. Records any problem in `state`. Never creates anything except
/// on a true first run.
pub fn initialize(app: &AppHandle, state: &AppState) {
    state.set_problem(None);
    let dir = match vault::data_dir(app) {
        Ok(dir) => dir,
        Err(err) => {
            state.set_problem(Some(problem_of(&err)));
            return;
        }
    };
    match vault::preflight(&dir) {
        // Storage cannot be read right now: "try again", never recovery.
        Err(err) => {
            state.set_problem(Some(problem_of(&err)));
            return;
        }
        Ok(Some(reason)) => {
            state.set_problem(Some(StorageProblem::Recovery(reason)));
            return;
        }
        Ok(None) => {}
    }
    if state.applock().is_locked() {
        // Opened by the unlock, which reports its own problems.
        return;
    }
    match vault::open_database(app) {
        Ok(db) => state.install_db(db),
        Err(err) => state.set_problem(Some(problem_of(&err))),
    }
}

/// Tries again after a failure that may have passed.
#[tauri::command]
pub async fn recovery_retry(app: AppHandle) -> Result<LockStatusDto, AppError> {
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        // Only a transient problem is retried; real loss needs a decision.
        if matches!(state.problem(), Some(StorageProblem::Retry) | None) {
            initialize(&worker, &state);
        }
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal, "internal error"))?;
    Ok(status_of(&app).await)
}

/// Discards the unreadable data and begins with an empty database. The caller
/// must have shown the user that existing data will be lost; `confirm` is that
/// answer and nothing happens without it.
#[tauri::command]
pub async fn recovery_start_fresh(
    app: AppHandle,
    state: State<'_, AppState>,
    confirm: bool,
) -> Result<LockStatusDto, AppError> {
    if !state.recovering() {
        return Err(AppError::invalid("there is nothing to recover"));
    }
    if !confirm {
        return Err(AppError::invalid(
            "the loss of existing data was not confirmed",
        ));
    }
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let state = worker.state::<AppState>();
        let dir = vault::data_dir(&worker).map_err(|_| AppError::unavailable())?;
        state.close_db();
        // A new wrapping key: the old one is unusable or no longer matches.
        worker
            .mt_keystore()
            .reset()
            .map_err(|_| AppError::unavailable())?;
        vault::erase_all(&dir);
        state
            .applock()
            .erase()
            .map_err(|_| AppError::unavailable())?;
        initialize(&worker, &state);
        match state.problem() {
            None => Ok(()),
            Some(_) => Err(AppError::unavailable()),
        }
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal, "internal error"))??;
    Ok(status_of(&app).await)
}
