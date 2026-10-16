//! Process-wide state shared by commands.

use std::{
    path::PathBuf,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

use mt_storage::{Db, DbKey, LogicalSummary};

use crate::{applock::LockService, error::AppError, vault::RecoveryReason};

/// How long a correct PIN keeps the destructive recovery steps available.
pub const RECOVERY_AUTH_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Why the data cannot be used right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageProblem {
    /// Data is lost or unreadable: restore from a backup, or start fresh.
    Recovery(RecoveryReason),
    /// A failure that may pass; only "try again" is offered.
    Retry,
}

/// A restore the user has started but not yet confirmed.
#[derive(Default)]
pub struct RestoreSession {
    /// The chosen backup, copied into the private cache (still encrypted).
    pub picked: Option<PathBuf>,
    /// A fully verified database built from it, waiting for confirmation.
    pub staged: Option<StagedRestore>,
    /// Key the staged database was built with, when it will *replace*
    /// unreadable data (recovery) rather than reuse the live key.
    pub fresh_key: Option<DbKey>,
}

pub struct StagedRestore {
    pub path: PathBuf,
    pub summary: LogicalSummary,
}

/// Holds the open database and the app lock.
///
/// The database exists only while the app is unlocked: it is opened after a
/// successful unlock and dropped (closing the connection) when the app locks.
pub struct AppState {
    db: Mutex<Option<Db>>,
    lock: LockService,
    restore: Mutex<RestoreSession>,
    problem: Mutex<Option<StorageProblem>>,
    /// When the PIN was last confirmed on the recovery screen (in memory only).
    recovery_auth: Mutex<Option<Instant>>,
}

impl AppState {
    pub fn new(db: Option<Db>, lock: LockService) -> Self {
        Self {
            db: Mutex::new(db),
            lock,
            restore: Mutex::new(RestoreSession::default()),
            problem: Mutex::new(None),
            recovery_auth: Mutex::new(None),
        }
    }

    pub fn problem(&self) -> Option<StorageProblem> {
        *self.problem.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_problem(&self, problem: Option<StorageProblem>) {
        *self.problem.lock().unwrap_or_else(PoisonError::into_inner) = problem;
        if problem.is_none() {
            // A resolved problem leaves no standing permission behind.
            self.clear_recovery_auth();
        }
    }

    /// Records that the PIN was just confirmed for the destructive recovery steps.
    pub fn grant_recovery_auth(&self, now: Instant) {
        *self
            .recovery_auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(now);
    }

    /// Whether a PIN confirmation is still fresh. An expired one is forgotten.
    pub fn recovery_auth_valid(&self, now: Instant) -> bool {
        let mut guard = self
            .recovery_auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match *guard {
            Some(at) if now.saturating_duration_since(at) < RECOVERY_AUTH_WINDOW => true,
            Some(_) => {
                *guard = None;
                false
            }
            None => false,
        }
    }

    pub fn clear_recovery_auth(&self) {
        *self
            .recovery_auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Whether the data is lost or unreadable and recovery is on offer.
    pub fn recovering(&self) -> bool {
        matches!(self.problem(), Some(StorageProblem::Recovery(_)))
    }

    pub fn applock(&self) -> &LockService {
        &self.lock
    }

    /// Runs `f` with the database, or fails with `locked` / `unavailable`.
    pub fn with_db<T>(
        &self,
        f: impl FnOnce(&mut Db) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut guard = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_mut() {
            Some(db) => f(db),
            None if self.lock.is_locked() => Err(AppError::locked()),
            None => Err(AppError::unavailable()),
        }
    }

    pub fn install_db(&self, db: Db) {
        *self.db.lock().unwrap_or_else(PoisonError::into_inner) = Some(db);
    }

    /// Drops the connection. Call whenever the lock engages. Any unfinished
    /// restore is abandoned and its files are deleted.
    pub fn close_db(&self) {
        *self.db.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.clear_restore();
    }

    /// Removes the connection without touching a pending restore.
    pub fn take_db(&self) -> Option<Db> {
        self.db
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    pub fn restore_session<T>(&self, f: impl FnOnce(&mut RestoreSession) -> T) -> T {
        f(&mut self.restore.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Forgets a pending restore and deletes the files it created.
    pub fn clear_restore(&self) {
        let session =
            std::mem::take(&mut *self.restore.lock().unwrap_or_else(PoisonError::into_inner));
        if let Some(path) = session.picked {
            let _ = std::fs::remove_file(path);
        }
        if let Some(staged) = session.staged {
            crate::backup::remove_database(&staged.path);
        }
    }
}
