//! Process-wide state shared by commands.

use std::{
    path::PathBuf,
    sync::{Mutex, PoisonError},
};

use mt_storage::{Db, LogicalSummary};

use crate::{applock::LockService, error::AppError};

/// A restore the user has started but not yet confirmed.
#[derive(Default)]
pub struct RestoreSession {
    /// The chosen backup, copied into the private cache (still encrypted).
    pub picked: Option<PathBuf>,
    /// A fully verified database built from it, waiting for confirmation.
    pub staged: Option<StagedRestore>,
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
}

impl AppState {
    pub fn new(db: Option<Db>, lock: LockService) -> Self {
        Self {
            db: Mutex::new(db),
            lock,
            restore: Mutex::new(RestoreSession::default()),
        }
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
