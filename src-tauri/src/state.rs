//! Process-wide state shared by commands.

use std::sync::{Mutex, PoisonError};

use mt_storage::Db;

use crate::{applock::LockService, error::AppError};

/// Holds the open database and the app lock.
///
/// The database exists only while the app is unlocked: it is opened after a
/// successful unlock and dropped (closing the connection) when the app locks.
pub struct AppState {
    db: Mutex<Option<Db>>,
    lock: LockService,
}

impl AppState {
    pub fn new(db: Option<Db>, lock: LockService) -> Self {
        Self {
            db: Mutex::new(db),
            lock,
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

    /// Drops the connection. Call whenever the lock engages.
    pub fn close_db(&self) {
        *self.db.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}
