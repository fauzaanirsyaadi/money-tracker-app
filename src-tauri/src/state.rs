//! Process-wide state shared by commands.

use std::{
    path::PathBuf,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

use mt_storage::{Db, DbKey, LogicalSummary};

use crate::{applock::LockService, error::AppError, vault::RecoveryReason};

/// How long a correct PIN keeps the guarded actions available (export, restore,
/// biometric and auto-lock changes, the destructive recovery steps).
///
/// Short on purpose: an unlocked phone handed to someone else must not stay one
/// tap away from exporting or replacing all the data.
pub const STEP_UP_WINDOW: Duration = Duration::from_secs(60);

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
    /// When the PIN was last confirmed for a guarded action, and the lock
    /// generation it was made under (in memory only).
    step_up: Mutex<Option<(Instant, u64)>>,
}

impl AppState {
    pub fn new(db: Option<Db>, lock: LockService) -> Self {
        Self {
            db: Mutex::new(db),
            lock,
            restore: Mutex::new(RestoreSession::default()),
            problem: Mutex::new(None),
            step_up: Mutex::new(None),
        }
    }

    pub fn problem(&self) -> Option<StorageProblem> {
        *self.problem.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_problem(&self, problem: Option<StorageProblem>) {
        *self.problem.lock().unwrap_or_else(PoisonError::into_inner) = problem;
        if problem.is_none() {
            // A resolved problem leaves no standing permission behind.
            self.clear_step_up();
        }
    }

    /// Records that the PIN was just confirmed for guarded actions.
    pub fn grant_step_up(&self, now: Instant) {
        let generation = self.lock.lock_generation();
        *self.step_up.lock().unwrap_or_else(PoisonError::into_inner) = Some((now, generation));
    }

    /// Whether a PIN confirmation is still fresh. It lapses after
    /// [`STEP_UP_WINDOW`] and as soon as the lock engages; a lapsed one is forgotten.
    pub fn step_up_valid(&self, now: Instant) -> bool {
        let mut guard = self.step_up.lock().unwrap_or_else(PoisonError::into_inner);
        match *guard {
            Some((at, generation))
                if now.saturating_duration_since(at) < STEP_UP_WINDOW
                    && generation == self.lock.lock_generation() =>
            {
                true
            }
            Some(_) => {
                *guard = None;
                false
            }
            None => false,
        }
    }

    pub fn clear_step_up(&self) {
        *self.step_up.lock().unwrap_or_else(PoisonError::into_inner) = None;
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
        // The lock flag decides, not whether a connection happens to exist: the
        // lifecycle hook sets the flag first and closes the database a moment
        // later, and no command may slip through that moment.
        if self.lock.is_locked() {
            return Err(AppError::locked());
        }
        let mut guard = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_mut() {
            // Checked again under the database lock: a lock that engaged while
            // this call waited for it must not run the closure.
            Some(db) if !self.lock.is_locked() => f(db),
            Some(_) => Err(AppError::locked()),
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

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use mt_storage::{Db, DbKey};

    use super::*;

    // --- F12: the lock flag, not the connection, decides ---------------------------

    fn dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mt-state-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open_state(tag: &str, with_pin: bool) -> (AppState, std::path::PathBuf) {
        let dir = dir(tag);
        let lock = LockService::load(&dir, Instant::now());
        if with_pin {
            lock.set_pin(&zeroize::Zeroizing::new("493017".to_owned()))
                .unwrap();
        }
        let db = Db::open_in_memory(&DbKey::new([3u8; 32])).unwrap();
        (AppState::new(Some(db), lock), dir)
    }

    #[test]
    fn data_commands_fail_with_locked_the_moment_the_flag_is_set_even_with_a_connection() {
        let (state, dir) = open_state("locked-flag", true);
        // Unlocked and open: the closure runs.
        assert_eq!(state.with_db(|_| Ok(7)).unwrap(), 7);

        // The flag is set but the lifecycle hook has not closed the database yet.
        state.applock().lock_now();
        let mut ran = false;
        let err = state
            .with_db(|_| {
                ran = true;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::Locked);
        assert!(!ran, "the closure must not run while locked");

        // After the close it is still `locked`, and after the unlock it works again.
        state.close_db();
        assert_eq!(
            state.with_db(|_| Ok(())).unwrap_err().code,
            crate::error::ErrorCode::Locked
        );
        state.install_db(Db::open_in_memory(&DbKey::new([3u8; 32])).unwrap());
        state.applock().mark_unlocked();
        assert!(state.with_db(|_| Ok(())).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_lock_a_missing_connection_is_unavailable_not_locked() {
        let (state, dir) = open_state("no-lock", false);
        assert!(state.with_db(|_| Ok(())).is_ok());
        state.close_db();
        assert_eq!(
            state.with_db(|_| Ok(())).unwrap_err().code,
            crate::error::ErrorCode::Unavailable
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
