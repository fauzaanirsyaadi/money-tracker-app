//! Storage start-up and recovery.
//!
//! The app never makes a new database or key on its own once data has existed
//! (see `vault`). When data is lost or unreadable the UI shows a recovery
//! screen with exactly two ways out, both explicit: restore from a backup
//! (the normal restore flow, building a *new* key), or start fresh after a
//! confirmation that says the old data will be lost. A failure that may pass,
//! such as a busy Keystore, only offers "try again".

use std::time::Instant;

use crate::{
    error::{AppError, ErrorCode},
    lock::{LockStatusDto, status_of},
    state::{AppState, StorageProblem},
    vault::{self, Recheck, VaultError},
};
use tauri::{AppHandle, Manager, State};

/// The word the user types to confirm that all existing data will be lost.
pub const START_FRESH_WORD: &str = "HAPUS";

pub fn confirmation_matches(typed: &str) -> bool {
    typed.trim() == START_FRESH_WORD
}

/// A second look, right before destroying anything: if the data opens after
/// all, or the state cannot be told, nothing is destroyed.
pub fn recheck_before_destroying(app: &AppHandle, state: &AppState) -> Result<(), AppError> {
    match vault::recheck(app) {
        Recheck::StillUnreadable => Ok(()),
        Recheck::Readable => {
            // A false alarm: resynchronise and leave everything as it is.
            initialize(app, state);
            Err(AppError::invalid(
                "the data can be opened again; nothing was changed",
            ))
        }
        Recheck::Transient => {
            state.set_problem(Some(StorageProblem::Retry));
            Err(AppError::unavailable())
        }
    }
}

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
    let gate = app.state::<AppState>();
    let _op = gate.storage_mutex().lock().await;
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

/// Discards the unreadable data and begins with an empty database.
///
/// Requires, in this order: something to recover, the typed confirmation, the
/// PIN when a valid lock exists, and a second look that still finds the data
/// unreadable. Nothing happens unless all four hold.
#[tauri::command]
pub async fn recovery_start_fresh(
    app: AppHandle,
    state: State<'_, AppState>,
    typed: String,
) -> Result<LockStatusDto, AppError> {
    // One storage operation at a time. Everything below is checked after this,
    // because a restore that finished meanwhile may have resolved the problem.
    let _op = state.storage_mutex().lock().await;
    if !state.recovering() {
        return Err(AppError::invalid("there is nothing to recover"));
    }
    if !confirmation_matches(&typed) {
        return Err(AppError::invalid(
            "the loss of existing data was not confirmed",
        ));
    }
    crate::lock::require_step_up(&state, Instant::now())?;
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let state = worker.state::<AppState>();
        recheck_before_destroying(&worker, &state)?;
        // The same destroy-last path as a restore: an empty database under a new
        // key replaces the unreadable data, and the old files go only once the new
        // ones are in place and verified.
        crate::backup_cmd::start_fresh_commit(&worker, &state)?;
        match state.problem() {
            None => Ok(()),
            Some(_) => Err(AppError::unavailable()),
        }
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal, "internal error"))??;
    Ok(status_of(&app).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_word_confirms() {
        assert!(confirmation_matches("HAPUS"));
        assert!(confirmation_matches(
            "  HAPUS 
"
        ));
        for typed in ["", "hapus", "Hapus", "HAPUS!", "YA", "HAPUSS", "H A P U S"] {
            assert!(!confirmation_matches(typed), "{typed:?}");
        }
    }
}
