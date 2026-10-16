//! App-lock commands.
//!
//! Everything here is reachable while locked except where noted; data commands
//! are not (they fail with `locked` because the database is closed).
//! Unlocking by biometrics is decided natively: the WebView can only *ask* for
//! the prompt, never report that it succeeded.

use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_mt_platform::{BiometricAvailability, BiometricOutcome, MtPlatformExt};
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
}

impl LockStatusDto {
    fn new(
        snapshot: LockSnapshot,
        biometric: BiometricAvailability,
        storage: &'static str,
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
    LockStatusDto::new(snapshot, biometric, storage_label(state.problem()))
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
pub async fn lock_unlock_pin(app: AppHandle, pin: String) -> Result<LockStatusDto, AppError> {
    let pin = Zeroizing::new(pin);
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        state.applock().verify_pin(&pin, Instant::now())?;
        open_and_unlock(app)
    })
    .await?;
    Ok(status_of(&app).await)
}

#[tauri::command]
pub async fn lock_unlock_biometric(app: AppHandle) -> Result<LockStatusDto, AppError> {
    let snapshot = app.state::<AppState>().applock().snapshot(Instant::now());
    if snapshot.broken || !snapshot.biometric_enabled {
        return Err(AppError::invalid("biometric unlock is not enabled"));
    }
    match app
        .mt_platform()
        .authenticate(
            "Buka Money Tracker",
            "Pindai sidik jari atau wajah",
            "Gunakan PIN",
        )
        .await
        .unwrap_or(BiometricOutcome::Failed)
    {
        BiometricOutcome::Success => {
            let app_for_open = app.clone();
            blocking(&app, move |_| open_and_unlock(&app_for_open)).await?;
            Ok(status_of(&app).await)
        }
        BiometricOutcome::Cancelled => Err(AppError::new(ErrorCode::Cancelled, "cancelled")),
        BiometricOutcome::Failed => Err(AppError::unavailable()),
    }
}

/// Sets the first PIN. Needs the app to be unlocked (it always is when none exists).
#[tauri::command]
pub async fn lock_setup(app: AppHandle, pin: String) -> Result<LockStatusDto, AppError> {
    let pin = Zeroizing::new(pin);
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
    current: String,
    new: String,
) -> Result<LockStatusDto, AppError> {
    let (current, new) = (Zeroizing::new(current), Zeroizing::new(new));
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
pub async fn lock_remove(app: AppHandle, current: String) -> Result<LockStatusDto, AppError> {
    let current = Zeroizing::new(current);
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        require_unlocked(&state)?;
        Ok(state.applock().remove(&current, Instant::now())?)
    })
    .await?;
    Ok(status_of(&app).await)
}

/// Enables or disables biometric unlock. Enabling proves it works first.
#[tauri::command]
pub async fn lock_biometric_set(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<LockStatusDto, AppError> {
    require_unlocked(&state)?;
    if enabled {
        let availability = app.mt_platform().biometric_availability().await;
        if availability.ok() != Some(BiometricAvailability::Available) {
            return Err(AppError::invalid("no fingerprint or face is enrolled"));
        }
        match app
            .mt_platform()
            .authenticate(
                "Aktifkan buka kunci biometrik",
                "Pindai untuk memastikan berfungsi",
                "Batal",
            )
            .await
            .unwrap_or(BiometricOutcome::Failed)
        {
            BiometricOutcome::Success => {}
            BiometricOutcome::Cancelled => {
                return Err(AppError::new(ErrorCode::Cancelled, "cancelled"));
            }
            BiometricOutcome::Failed => return Err(AppError::unavailable()),
        }
    }
    state.applock().set_biometric(enabled)?;
    Ok(status_of(&app).await)
}

#[tauri::command]
pub async fn lock_auto_set(
    app: AppHandle,
    state: State<'_, AppState>,
    seconds: u32,
) -> Result<LockStatusDto, AppError> {
    require_unlocked(&state)?;
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
