//! App-lock commands.
//!
//! Everything here is reachable while locked except where noted; data commands
//! are not (they fail with `locked` because the database is closed).
//! Unlocking by biometrics is decided natively: the WebView can only *ask* for
//! the prompt, never report that it succeeded.

use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_mt_platform::{
    BiometricAvailability, EnrollOutcome, MtPlatformExt, UnlockOutcome,
};
use zeroize::Zeroizing;

use crate::{
    applock::{LockError, LockSnapshot},
    error::{AppError, ErrorCode},
    state::{AppState, StorageProblem},
    vault::{self, RecoveryReason},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockStatusDto {
    pub configured: bool,
    pub locked: bool,
    /// The lock file is damaged; the app stays locked until data is restored.
    pub broken: bool,
    pub biometric_enabled: bool,
    /// `available`, `none_enrolled` or `unavailable`.
    pub biometric: &'static str,
    pub auto_lock_seconds: u32,
    pub failed_attempts: u32,
    pub retry_after_ms: Option<u64>,
    /// `ok`, `retry`, or why the data needs recovery: `database_missing`,
    /// `key_missing`, `key_unusable`, `database_unreadable`.
    pub storage: &'static str,
    /// The PIN was confirmed within the last minute for guarded actions.
    pub step_up_active: bool,
}

impl LockStatusDto {
    fn new(
        snapshot: LockSnapshot,
        biometric: BiometricAvailability,
        storage: &'static str,
        step_up_active: bool,
    ) -> Self {
        Self {
            configured: snapshot.configured,
            locked: snapshot.locked,
            broken: snapshot.broken,
            biometric_enabled: snapshot.biometric_enabled,
            biometric: match biometric {
                BiometricAvailability::Available => "available",
                BiometricAvailability::NoneEnrolled => "none_enrolled",
                BiometricAvailability::Unavailable => "unavailable",
            },
            auto_lock_seconds: snapshot.auto_lock_seconds,
            failed_attempts: snapshot.failed_attempts,
            retry_after_ms: snapshot
                .retry_after
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            storage,
            step_up_active,
        }
    }
}

impl From<LockError> for AppError {
    fn from(err: LockError) -> Self {
        match err {
            LockError::NotConfigured => Self::invalid("no PIN is set"),
            LockError::AlreadyConfigured => Self::invalid("a PIN is already set"),
            LockError::Broken => Self::unavailable(),
            LockError::LockedOut(_) => {
                Self::new(ErrorCode::LockedOut, "too many wrong PINs, try again later")
            }
            LockError::WrongPin => Self::new(ErrorCode::WrongPin, "wrong PIN"),
            LockError::InvalidPin(problem) => Self::invalid(match problem {
                mt_core::PinProblem::Length => "PIN must be 6 to 16 digits",
                mt_core::PinProblem::NotDigits => "PIN may only contain digits",
                mt_core::PinProblem::TooSimple => "PIN is too easy to guess",
            }),
            LockError::InvalidChoice => Self::invalid("unsupported auto-lock choice"),
            LockError::Io => Self::new(ErrorCode::Internal, "could not save the lock settings"),
        }
    }
}

pub(crate) fn storage_label(problem: Option<StorageProblem>) -> &'static str {
    match problem {
        None => "ok",
        Some(StorageProblem::Retry) => "retry",
        Some(StorageProblem::Recovery(reason)) => match reason {
            RecoveryReason::DatabaseMissing => "database_missing",
            RecoveryReason::KeyMissing => "key_missing",
            RecoveryReason::KeyUnusable => "key_unusable",
            RecoveryReason::DatabaseUnreadable => "database_unreadable",
        },
    }
}

pub(crate) async fn status_of(app: &AppHandle) -> LockStatusDto {
    let state = app.state::<AppState>();
    let snapshot = state.applock().snapshot(Instant::now());
    let biometric = app
        .mt_platform()
        .biometric_availability()
        .await
        .unwrap_or(BiometricAvailability::Unavailable);
    LockStatusDto::new(
        snapshot,
        biometric,
        storage_label(state.problem()),
        state.step_up_valid(Instant::now()),
    )
}

/// Whether a valid, readable app lock exists to ask a PIN of. A damaged
/// `lock.json` cannot be checked against, so it does not block.
pub fn pin_required(state: &AppState) -> bool {
    let lock = state.applock().snapshot(Instant::now());
    lock.configured && !lock.broken
}

/// Guarded actions need a PIN confirmed within [`crate::state::STEP_UP_WINDOW`]
/// while a valid lock exists. Pure refusal: nothing is counted or changed here;
/// guesses are made (and limited) in [`lock_step_up`] through the same counter
/// as unlocking.
pub fn require_step_up(state: &AppState, now: Instant) -> Result<(), AppError> {
    if !pin_required(state) || state.step_up_valid(now) {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::PinRequired,
            "enter the PIN to continue",
        ))
    }
}

pub(crate) fn require_unlocked(state: &AppState) -> Result<(), AppError> {
    if state.applock().is_locked() {
        Err(AppError::locked())
    } else {
        Ok(())
    }
}

/// Opens the database and only then marks the app unlocked, so a storage
/// failure leaves it locked instead of half-open.
fn open_and_unlock(app: &AppHandle) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    let db = match vault::open_database(app) {
        Ok(db) => db,
        Err(err) => {
            // The PIN was right but the data is not usable: say why instead of
            // looking like a wrong PIN, and offer recovery only for real loss.
            state.set_problem(Some(crate::recovery::problem_of(&err)));
            return Err(AppError::unavailable());
        }
    };
    state.install_db(db);
    state.applock().mark_unlocked();
    if state.applock().is_locked() {
        // The app left the foreground while it was being unlocked and the lock
        // applies at once: nothing stays open behind it.
        state.close_db();
        return Err(AppError::locked());
    }
    Ok(())
}

/// Runs a blocking, CPU-heavy lock operation (Argon2) off the async threads.
async fn blocking<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&AppHandle) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || work(&app))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal, "internal error"))?
}

#[tauri::command]
pub async fn lock_status(app: AppHandle) -> Result<LockStatusDto, AppError> {
    Ok(status_of(&app).await)
}

#[tauri::command]
pub async fn lock_unlock_pin(
    app: AppHandle,
    pin: Zeroizing<String>,
) -> Result<LockStatusDto, AppError> {
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        state.applock().verify_pin(&pin, Instant::now())?;
        open_and_unlock(app)
    })
    .await?;
    Ok(status_of(&app).await)
}

/// What a biometric prompt's answer means for the lock. Pure on purpose, so the
/// failure paths are tested without a device.
///
/// This touches **only** the lock settings. A key invalidated by a newly
/// enrolled fingerprint is the biometric key's business: it switches biometric
/// unlock off and the PIN stays the way in. It is never a storage problem, never
/// reaches recovery, and never touches the database key or its files.
pub(crate) fn settle_biometric_unlock(
    lock: &crate::applock::LockService,
    outcome: UnlockOutcome,
) -> Result<(), AppError> {
    match outcome {
        UnlockOutcome::Token(token) if lock.biometric_token_matches(&token) => Ok(()),
        // The key opened but released something other than what was enrolled:
        // the pair is inconsistent, so biometric unlock is off until re-enabled.
        UnlockOutcome::Token(_) | UnlockOutcome::Invalidated => {
            let _ = lock.disable_biometric();
            Err(AppError::new(
                ErrorCode::BiometricInvalidated,
                "biometric unlock was switched off, use the PIN",
            ))
        }
        UnlockOutcome::Cancelled => Err(AppError::new(ErrorCode::Cancelled, "cancelled")),
        UnlockOutcome::Failed => Err(AppError::unavailable()),
    }
}

#[tauri::command]
pub async fn lock_unlock_biometric(app: AppHandle) -> Result<LockStatusDto, AppError> {
    let state = app.state::<AppState>();
    let snapshot = state.applock().snapshot(Instant::now());
    if snapshot.broken || !snapshot.biometric_enabled {
        return Err(AppError::invalid("biometric unlock is not enabled"));
    }
    let Some(blob) = state.applock().biometric_blob() else {
        return Err(AppError::invalid("biometric unlock is not enabled"));
    };
    let outcome = app
        .mt_platform()
        .biometric_unlock(
            "Buka Money Tracker",
            "Pindai sidik jari atau wajah",
            "Gunakan PIN",
            &blob,
        )
        .await
        .unwrap_or(UnlockOutcome::Failed);
    let invalidated = matches!(outcome, UnlockOutcome::Invalidated);
    settle_biometric_unlock(state.applock(), outcome).inspect_err(|_| {
        if invalidated {
            // The key is useless now; remove it. Best effort, only the biometric key.
            tauri::async_runtime::spawn({
                let app = app.clone();
                async move {
                    let _ = app.mt_platform().biometric_forget().await;
                }
            });
        }
    })?;
    let app_for_open = app.clone();
    blocking(&app, move |_| open_and_unlock(&app_for_open)).await?;
    Ok(status_of(&app).await)
}

/// Sets the first PIN. Needs the app to be unlocked (it always is when none exists).
#[tauri::command]
pub async fn lock_setup(app: AppHandle, pin: Zeroizing<String>) -> Result<LockStatusDto, AppError> {
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        require_unlocked(&state)?;
        Ok(state.applock().set_pin(&pin)?)
    })
    .await?;
    Ok(status_of(&app).await)
}

#[tauri::command]
pub async fn lock_change_pin(
    app: AppHandle,
    current: Zeroizing<String>,
    new: Zeroizing<String>,
) -> Result<LockStatusDto, AppError> {
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        require_unlocked(&state)?;
        Ok(state.applock().change_pin(&current, &new, Instant::now())?)
    })
    .await?;
    Ok(status_of(&app).await)
}

/// Turns the lock off after confirming the PIN.
#[tauri::command]
pub async fn lock_remove(
    app: AppHandle,
    current: Zeroizing<String>,
) -> Result<LockStatusDto, AppError> {
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        require_unlocked(&state)?;
        Ok(state.applock().remove(&current, Instant::now())?)
    })
    .await?;
    // The lock is gone, so is the key that opened it. Best effort; only that key.
    let _ = app.mt_platform().biometric_forget().await;
    Ok(status_of(&app).await)
}

/// Confirms the PIN for guarded actions (export, restore, biometric and
/// auto-lock changes, the destructive recovery steps) for the next minute.
///
/// A guess here is a guess like any other: it goes through `verify_pin`, so it
/// counts against the same persisted limit and backoff as unlocking and cannot
/// be used to try PINs more often. Allowed while unlocked, or while the data is
/// being recovered (the app is locked then but there is no database to unlock).
#[tauri::command]
pub async fn lock_step_up(
    app: AppHandle,
    state: State<'_, AppState>,
    pin: Zeroizing<String>,
) -> Result<LockStatusDto, AppError> {
    if state.applock().is_locked() && !state.recovering() {
        return Err(AppError::locked());
    }
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        let now = Instant::now();
        if pin_required(&state) {
            state.applock().verify_pin(&pin, now)?;
        }
        state.grant_step_up(now);
        Ok(())
    })
    .await?;
    Ok(status_of(&app).await)
}

/// Whether moving from `current` to `wanted` seconds of grace weakens the lock
/// (the app may then stay readable in the background for longer).
pub(crate) fn weakens_auto_lock(current: u32, wanted: u32) -> Result<bool, AppError> {
    use mt_core::AutoLock;
    let wanted = AutoLock::from_seconds(wanted)
        .ok_or_else(|| AppError::invalid("unsupported auto-lock choice"))?;
    // A stored value this build does not know is treated as the strictest, so
    // that any change from it needs the PIN.
    let current = AutoLock::from_seconds(current).unwrap_or(AutoLock::Immediately);
    Ok(wanted.grace() > current.grace())
}

/// Enables or disables biometric unlock. Enabling proves it works first.
#[tauri::command]
pub async fn lock_biometric_set(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<LockStatusDto, AppError> {
    require_unlocked(&state)?;
    require_step_up(&state, Instant::now())?;
    if enabled {
        let availability = app.mt_platform().biometric_availability().await;
        if availability.ok() != Some(BiometricAvailability::Available) {
            return Err(AppError::invalid("no fingerprint or face is enrolled"));
        }
        // A key of its own is made and proven with a real biometric; the token it
        // releases is kept only as a hash. Anything but success leaves biometric
        // unlock off (a half-made key must not pair with an old blob).
        match app
            .mt_platform()
            .biometric_enroll(
                "Aktifkan buka kunci biometrik",
                "Pindai untuk memastikan berfungsi",
                "Batal",
            )
            .await
            .unwrap_or(EnrollOutcome::Failed)
        {
            EnrollOutcome::Enrolled(enrollment) => {
                state
                    .applock()
                    .enable_biometric(&enrollment.token, &enrollment.blob)?;
            }
            EnrollOutcome::Cancelled => {
                let _ = state.applock().disable_biometric();
                let _ = app.mt_platform().biometric_forget().await;
                return Err(AppError::new(ErrorCode::Cancelled, "cancelled"));
            }
            EnrollOutcome::Failed => {
                let _ = state.applock().disable_biometric();
                let _ = app.mt_platform().biometric_forget().await;
                return Err(AppError::unavailable());
            }
        }
    } else {
        // Off first (that is what counts), then the key is removed.
        state.applock().disable_biometric()?;
        let _ = app.mt_platform().biometric_forget().await;
    }
    Ok(status_of(&app).await)
}

#[tauri::command]
pub async fn lock_auto_set(
    app: AppHandle,
    state: State<'_, AppState>,
    seconds: u32,
) -> Result<LockStatusDto, AppError> {
    require_unlocked(&state)?;
    // Shortening the grace is always fine; lengthening it weakens the lock.
    let current = state.applock().snapshot(Instant::now()).auto_lock_seconds;
    if weakens_auto_lock(current, seconds)? {
        require_step_up(&state, Instant::now())?;
    }
    state.applock().set_auto_lock(seconds)?;
    Ok(status_of(&app).await)
}

/// Locks now: the database is closed and the PIN is needed again.
#[tauri::command]
pub async fn lock_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LockStatusDto, AppError> {
    state.applock().lock_now();
    if state.applock().is_locked() {
        state.close_db();
    }
    Ok(status_of(&app).await)
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::{applock::LockService, state::STEP_UP_WINDOW};
    use tauri_plugin_mt_platform::UnlockOutcome;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("mt-stepup-{}-{tag}", std::process::id()));
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

    const PIN: &str = "493017";

    fn pin(text: &str) -> Zeroizing<String> {
        Zeroizing::new(text.to_owned())
    }

    fn state_with_lock(dir: &Dir, with_pin: bool) -> AppState {
        let lock = LockService::load(&dir.0, Instant::now());
        if with_pin {
            lock.set_pin(&pin(PIN)).unwrap();
        }
        AppState::new(None, lock)
    }

    /// What `lock_step_up` does, without the Tauri plumbing around it.
    fn step_up(state: &AppState, attempt: &str, now: Instant) -> Result<(), AppError> {
        if pin_required(state) {
            state.applock().verify_pin(&pin(attempt), now)?;
        }
        state.grant_step_up(now);
        Ok(())
    }

    #[test]
    fn a_guarded_action_is_refused_without_a_step_up_and_allowed_with_one() {
        let dir = Dir::new("basic");
        let state = state_with_lock(&dir, true);
        let now = Instant::now();
        assert!(pin_required(&state));
        assert_eq!(
            require_step_up(&state, now).unwrap_err().code,
            ErrorCode::PinRequired
        );
        step_up(&state, PIN, now).unwrap();
        assert!(require_step_up(&state, now).is_ok());
    }

    #[test]
    fn the_step_up_lapses_after_its_window_and_stays_lapsed() {
        let dir = Dir::new("expiry");
        let state = state_with_lock(&dir, true);
        let now = Instant::now();
        step_up(&state, PIN, now).unwrap();
        assert_eq!(STEP_UP_WINDOW, Duration::from_secs(60));
        let almost = now + STEP_UP_WINDOW - Duration::from_secs(1);
        assert!(require_step_up(&state, almost).is_ok());
        let late = now + STEP_UP_WINDOW + Duration::from_secs(1);
        assert_eq!(
            require_step_up(&state, late).unwrap_err().code,
            ErrorCode::PinRequired
        );
        // Expiry is final: checking again "inside the old window" does not revive it.
        assert!(!state.step_up_valid(almost));
    }

    #[test]
    fn wrong_pins_grant_nothing_and_count_against_the_unlock_limit() {
        let dir = Dir::new("shared-counter");
        let state = state_with_lock(&dir, true);
        let now = Instant::now();
        for expected in 1..=4 {
            let err = step_up(&state, "000001", now).unwrap_err();
            assert_eq!(err.code, ErrorCode::WrongPin);
            assert!(!state.step_up_valid(now));
            assert_eq!(state.applock().snapshot(now).failed_attempts, expected);
        }
        // The fifth miss starts the same backoff the unlock screen would impose...
        assert_eq!(
            step_up(&state, "000001", now).unwrap_err().code,
            ErrorCode::WrongPin
        );
        // ...and while it runs even the right PIN is refused, here as at unlock.
        let err = step_up(&state, PIN, now).unwrap_err();
        assert_eq!(err.code, ErrorCode::LockedOut);
        assert!(!state.step_up_valid(now));
        assert!(matches!(
            state.applock().verify_pin(&pin(PIN), now),
            Err(LockError::LockedOut(_))
        ));
        // After the delay the right PIN works again.
        let later = now + Duration::from_secs(31);
        step_up(&state, PIN, later).unwrap();
        assert!(require_step_up(&state, later).is_ok());
    }

    #[test]
    fn misses_at_unlock_and_at_step_up_add_up() {
        let dir = Dir::new("mixed");
        let state = state_with_lock(&dir, true);
        let now = Instant::now();
        state.applock().verify_pin(&pin("000001"), now).unwrap_err();
        state.applock().verify_pin(&pin("000002"), now).unwrap_err();
        step_up(&state, "000003", now).unwrap_err();
        step_up(&state, "000004", now).unwrap_err();
        assert_eq!(state.applock().snapshot(now).failed_attempts, 4);
    }

    #[test]
    fn the_step_up_does_not_survive_the_lock_engaging() {
        let dir = Dir::new("lock-clears");
        let state = state_with_lock(&dir, true);
        let now = Instant::now();
        state.applock().mark_unlocked();
        step_up(&state, PIN, now).unwrap();
        state.applock().lock_now();
        state.applock().mark_unlocked();
        assert_eq!(
            require_step_up(&state, now).unwrap_err().code,
            ErrorCode::PinRequired,
            "a confirmation made before a lock must not count after it"
        );
    }

    #[test]
    fn a_resolved_problem_leaves_no_standing_permission() {
        let dir = Dir::new("cleared");
        let state = state_with_lock(&dir, true);
        let now = Instant::now();
        step_up(&state, PIN, now).unwrap();
        state.set_problem(None);
        assert!(!state.step_up_valid(now));
    }

    #[test]
    fn without_a_valid_lock_no_pin_can_be_asked_for() {
        let dir = Dir::new("nolock");
        let state = state_with_lock(&dir, false);
        assert!(!pin_required(&state));
        assert!(require_step_up(&state, Instant::now()).is_ok());

        // A damaged lock.json cannot be checked against, so it does not block either.
        let dir = Dir::new("brokenlock");
        std::fs::write(dir.0.join(crate::applock::LOCK_FILE), b"{broken").unwrap();
        let state = state_with_lock(&dir, false);
        assert!(state.applock().snapshot(Instant::now()).broken);
        assert!(!pin_required(&state));
        assert!(require_step_up(&state, Instant::now()).is_ok());
    }

    #[test]
    fn only_a_longer_grace_counts_as_weakening() {
        // (current, wanted, weakens)
        for (current, wanted, weakens) in [
            (0, 30, true),
            (0, 300, true),
            (30, 60, true),
            (60, 300, true),
            (300, 60, false),
            (300, 0, false),
            (60, 60, false),
            (0, 0, false),
        ] {
            assert_eq!(
                weakens_auto_lock(current, wanted).unwrap(),
                weakens,
                "{current} -> {wanted}"
            );
        }
        assert_eq!(
            weakens_auto_lock(0, 45).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        // A stored value this build does not know counts as strictest.
        assert!(weakens_auto_lock(999, 30).unwrap());
    }

    /// The command bodies need a running Tauri app, so their wiring is checked
    /// in the source: every guarded command must ask for the step-up before it
    /// does anything else that matters. (The device scripts exercise the real
    /// commands end to end.)
    #[test]
    fn every_guarded_command_asks_for_the_step_up_first() {
        fn body<'a>(source: &'a str, name: &str) -> &'a str {
            let start = source
                .find(&format!("pub async fn {name}("))
                .unwrap_or_else(|| panic!("{name} not found"));
            let rest = &source[start..];
            let end = rest.find("\n#[tauri::command]").unwrap_or(rest.len());
            &rest[..end]
        }
        let lock = include_str!("lock.rs");
        let backup = include_str!("backup_cmd.rs");
        let recovery = include_str!("recovery.rs");

        // (source, command, a marker for the first thing it does after the guard)
        for (source, name, then) in [
            (lock, "lock_biometric_set", "biometric_availability"),
            (lock, "lock_auto_set", "set_auto_lock"),
            (backup, "backup_export", "validate_passphrase"),
            (backup, "backup_restore_commit", "staged.take()"),
            (recovery, "recovery_start_fresh", "spawn_blocking"),
        ] {
            let text = body(source, name);
            let guard = text
                .find("require_step_up")
                .unwrap_or_else(|| panic!("{name} has no step-up guard"));
            let first = text.find(then).unwrap_or_else(|| panic!("{name}: {then}"));
            assert!(guard < first, "{name} must check the step-up before {then}");
        }
    }

    // --- F8: the biometric failure path is separate from the storage one ----------

    fn enrolled(dir: &Dir) -> AppState {
        let state = state_with_lock(dir, true);
        state
            .applock()
            .enable_biometric(&[7; 32], &[9; 40])
            .unwrap();
        state
    }

    #[test]
    fn the_enrolled_token_unlocks() {
        let dir = Dir::new("bio-ok");
        let state = enrolled(&dir);
        let outcome = UnlockOutcome::Token(Zeroizing::new(vec![7; 32]));
        assert!(settle_biometric_unlock(state.applock(), outcome).is_ok());
        assert!(state.applock().snapshot(Instant::now()).biometric_enabled);
    }

    #[test]
    fn a_new_fingerprint_switches_biometrics_off_and_nothing_else() {
        let dir = Dir::new("bio-invalidated");
        // The data and its key exist; they must come out byte for byte the same.
        std::fs::write(dir.0.join("money.db"), b"database bytes").unwrap();
        std::fs::write(dir.0.join("db.key"), [1u8, 2, 3, 4]).unwrap();
        let state = enrolled(&dir);
        state.applock().mark_unlocked();

        let err = settle_biometric_unlock(state.applock(), UnlockOutcome::Invalidated).unwrap_err();
        assert_eq!(err.code, ErrorCode::BiometricInvalidated);

        // Biometric unlock is off and the blob is gone; the PIN still works.
        let snapshot = state.applock().snapshot(Instant::now());
        assert!(!snapshot.biometric_enabled);
        assert_eq!(state.applock().biometric_blob(), None);
        assert!(
            state
                .applock()
                .verify_pin(&pin(PIN), Instant::now())
                .is_ok()
        );

        // Not a storage problem, so no recovery screen, and the data is untouched.
        assert_eq!(state.problem(), None);
        assert!(!state.recovering());
        assert_eq!(
            std::fs::read(dir.0.join("money.db")).unwrap(),
            b"database bytes"
        );
        assert_eq!(std::fs::read(dir.0.join("db.key")).unwrap(), [1u8, 2, 3, 4]);
        // The user can re-enable it afterwards.
        state
            .applock()
            .enable_biometric(&[5; 32], &[6; 40])
            .unwrap();
        assert!(state.applock().snapshot(Instant::now()).biometric_enabled);
    }

    #[test]
    fn a_token_that_is_not_the_enrolled_one_switches_biometrics_off() {
        let dir = Dir::new("bio-mismatch");
        let state = enrolled(&dir);
        let outcome = UnlockOutcome::Token(Zeroizing::new(vec![8; 32]));
        let err = settle_biometric_unlock(state.applock(), outcome).unwrap_err();
        assert_eq!(err.code, ErrorCode::BiometricInvalidated);
        assert!(!state.applock().snapshot(Instant::now()).biometric_enabled);
    }

    #[test]
    fn a_cancel_or_a_transient_failure_changes_nothing() {
        let dir = Dir::new("bio-transient");
        let state = enrolled(&dir);
        let cancelled = settle_biometric_unlock(state.applock(), UnlockOutcome::Cancelled);
        assert_eq!(cancelled.unwrap_err().code, ErrorCode::Cancelled);
        let failed = settle_biometric_unlock(state.applock(), UnlockOutcome::Failed);
        assert_eq!(failed.unwrap_err().code, ErrorCode::Unavailable);
        // Still enabled: unknown failures are transient, as everywhere in the app.
        assert!(state.applock().snapshot(Instant::now()).biometric_enabled);
        assert_eq!(state.problem(), None);
    }

    #[test]
    fn the_biometric_error_is_its_own_code() {
        // Not the storage vocabulary: the recovery labels never mention it.
        assert_eq!(
            serde_json::to_string(&ErrorCode::BiometricInvalidated).unwrap(),
            "\"biometric_invalidated\""
        );
        for problem in [None, Some(StorageProblem::Retry)] {
            assert!(!storage_label(problem).contains("biometric"));
        }
    }
}
