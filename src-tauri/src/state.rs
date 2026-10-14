//! Process-wide state shared by commands.

use std::sync::{Mutex, PoisonError};

use mt_storage::Db;

use crate::error::AppError;

/// Holds the open database, or `None` while storage is unavailable.
///
/// Until the app lock exists (phase 7) the database is opened at startup; the
/// lock will later close it and clear the option when the app is backgrounded.
pub struct AppState {
    db: Mutex<Option<Db>>,
}

impl AppState {
    pub fn new(db: Option<Db>) -> Self {
        Self { db: Mutex::new(db) }
    }

    /// Runs `f` with the database, or fails with `unavailable`.
    pub fn with_db<T>(
        &self,
        f: impl FnOnce(&mut Db) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut guard = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_mut() {
            Some(db) => f(db),
            None => Err(AppError::unavailable()),
        }
    }
}
