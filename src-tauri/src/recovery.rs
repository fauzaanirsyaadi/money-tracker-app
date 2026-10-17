//! Storage start-up and recovery.
//!
//! The app never makes a new database or key on its own once data has existed
//! (see `vault`). When data is lost or unreadable the UI shows a recovery
//! screen with exactly two ways out, both explicit: restore from a backup
//! (the normal restore flow, building a *new* key), or start fresh after a
//! confirmation that says the old data will be lost. A failure that may pass,
//! such as a busy Keystore, only offers "try again".

use std::time::Instant;

use tauri::{AppHandle, Manager, State};
use zeroize::Zeroizing;

use crate::{
    error::{AppError, ErrorCode},
    lock::{LockStatusDto, status_of},
    state::{AppState, StorageProblem},
    vault::{self, Recheck, VaultError},
};

/// The word the user types to confirm that all existing data will be lost.
pub const START_FRESH_WORD: &str = "HAPUS";

pub fn confirmation_matches(typed: &str) -> bool {
    typed.trim() == START_FRESH_WORD
}

/// Whether a valid, readable app lock guards the destructive recovery steps.
/// A damaged `lock.json` cannot be checked against, so it does not block.
pub fn pin_required(state: &AppState) -> bool {
    let lock = state.applock().snapshot(Instant::now());
    lock.configured && !lock.broken
}

/// Destructive recovery steps need the PIN while a valid lock exists: anyone
/// holding the phone could otherwise wipe it from the recovery screen.
pub fn require_recovery_authority(state: &AppState, now: Instant) -> Result<(), AppError> {
    if !pin_required(state) || state.recovery_auth_valid(now) {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::PinRequired,
            "enter the PIN to continue the recovery",
        ))
    }
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

/// Confirms the PIN for the destructive recovery steps (restore over the
/// unreadable data, start fresh). Counts against the same rate limit as
/// unlocking. Without a valid lock no PIN exists to ask for.
#[tauri::command]
pub async fn recovery_authenticate(
    app: AppHandle,
    state: State<'_, AppState>,
    pin: Zeroizing<String>,
) -> Result<LockStatusDto, AppError> {
    if !state.recovering() {
        return Err(AppError::invalid("there is nothing to recover"));
    }
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let state = worker.state::<AppState>();
        let now = Instant::now();
        if pin_required(&state) {
            state.applock().verify_pin(&pin, now)?;
        }
        state.grant_recovery_auth(now);
        Ok(())
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Internal, "internal error"))??;
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
    if !state.recovering() {
        return Err(AppError::invalid("there is nothing to recover"));
    }
    if !confirmation_matches(&typed) {
        return Err(AppError::invalid(
            "the loss of existing data was not confirmed",
        ));
    }
    require_recovery_authority(&state, Instant::now())?;
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
    use std::{path::PathBuf, time::Duration};

    use zeroize::Zeroizing;

    use super::*;
    use crate::{applock::LockService, state::RECOVERY_AUTH_WINDOW};

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("mt-recovery-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn state_with_lock(dir: &Dir, pin: Option<&str>) -> AppState {
        let now = Instant::now();
        let lock = LockService::load(&dir.0, now);
        if let Some(pin) = pin {
            lock.set_pin(&Zeroizing::new(pin.to_owned())).unwrap();
        }
        AppState::new(None, lock)
    }

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

    #[test]
    fn with_a_valid_lock_the_pin_is_required_first() {
        let dir = Dir::new("pin");
        let state = state_with_lock(&dir, Some("493017"));
        let now = Instant::now();
        assert!(pin_required(&state));
        let denied = require_recovery_authority(&state, now).unwrap_err();
        assert_eq!(denied.code, ErrorCode::PinRequired);
        state.grant_recovery_auth(now);
        assert!(require_recovery_authority(&state, now).is_ok());
    }

    #[test]
    fn the_pin_confirmation_expires() {
        let dir = Dir::new("expiry");
        let state = state_with_lock(&dir, Some("493017"));
        let now = Instant::now();
        state.grant_recovery_auth(now);
        let almost = now + RECOVERY_AUTH_WINDOW - Duration::from_secs(1);
        assert!(require_recovery_authority(&state, almost).is_ok());
        let late = now + RECOVERY_AUTH_WINDOW + Duration::from_secs(1);
        assert_eq!(
            require_recovery_authority(&state, late).unwrap_err().code,
            ErrorCode::PinRequired
        );
        // Expiry is final: a later check inside the old window does not revive it.
        assert!(!state.recovery_auth_valid(almost));
    }

    #[test]
    fn a_resolved_problem_leaves_no_standing_permission() {
        let dir = Dir::new("cleared");
        let state = state_with_lock(&dir, Some("493017"));
        let now = Instant::now();
        state.grant_recovery_auth(now);
        state.set_problem(None);
        assert!(!state.recovery_auth_valid(now));
    }

    #[test]
    fn without_a_valid_lock_no_pin_can_be_asked_for() {
        // No lock at all.
        let dir = Dir::new("nolock");
        let state = state_with_lock(&dir, None);
        assert!(!pin_required(&state));
        assert!(require_recovery_authority(&state, Instant::now()).is_ok());

        // A damaged lock.json cannot be checked against, so it does not block either.
        let dir = Dir::new("brokenlock");
        std::fs::write(dir.0.join(crate::applock::LOCK_FILE), b"{broken").unwrap();
        let state = state_with_lock(&dir, None);
        assert!(state.applock().snapshot(Instant::now()).broken);
        assert!(!pin_required(&state));
        assert!(require_recovery_authority(&state, Instant::now()).is_ok());
    }

    #[test]
    fn a_wrong_pin_does_not_grant_anything_and_counts_against_the_limit() {
        let dir = Dir::new("wrongpin");
        let state = state_with_lock(&dir, Some("493017"));
        let now = Instant::now();
        let wrong = Zeroizing::new("000001".to_owned());
        assert!(state.applock().verify_pin(&wrong, now).is_err());
        assert!(!state.recovery_auth_valid(now));
        assert_eq!(state.applock().snapshot(now).failed_attempts, 1);
    }
}
