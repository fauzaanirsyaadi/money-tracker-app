//! Process-wide state shared by commands.

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock, PoisonError},
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
    /// Published by the start-up thread as soon as `lock.json` is read; see
    /// [`AppState::pending`].
    lock: OnceLock<LockService>,
    /// Set once start-up has finished (lock loaded, storage inspected). Everything
    /// that answers questions about the data waits for it.
    ready: OnceLock<()>,
    restore: Mutex<RestoreSession>,
    problem: Mutex<Option<StorageProblem>>,
    /// Serialises everything that changes which files are the live data or
    /// whether they are open: a restore commit, `initialize`, start fresh and the
    /// unlock that opens the database. See [`AppState::storage_mutex`].
    storage_op: tauri::async_runtime::Mutex<()>,
    /// When the PIN was last confirmed for a guarded action, and the lock
    /// generation it was made under (in memory only).
    step_up: Mutex<Option<(Instant, u64)>>,
}

impl AppState {
    /// A ready state, for tests.
    #[cfg(test)]
    pub fn new(db: Option<Db>, lock: LockService) -> Self {
        let state = Self::empty(db);
        state.publish_lock(lock);
        state.mark_ready();
        state
    }

    /// A state that exists before anything has been read from disk.
    ///
    /// Start-up used to read the data directory, the lock file and the storage
    /// inside Tauri's `setup` hook, which holds the window back; most of that time
    /// is spent waiting for the (busy) main thread. Now `setup` only creates this
    /// and a thread fills it in. Until then **nothing answers as unlocked**:
    /// [`AppState::applock`] and everything built on it block (they run on
    /// async-runtime threads, never on the main thread, which must stay free to
    /// serve the Kotlin calls start-up itself makes), and the main-thread
    /// lifecycle hook uses [`AppState::applock_if_ready`], which never blocks.
    pub fn pending() -> Self {
        Self::empty(None)
    }

    fn empty(db: Option<Db>) -> Self {
        Self {
            db: Mutex::new(db),
            lock: OnceLock::new(),
            ready: OnceLock::new(),
            restore: Mutex::new(RestoreSession::default()),
            problem: Mutex::new(None),
            storage_op: tauri::async_runtime::Mutex::new(()),
            step_up: Mutex::new(None),
        }
    }

    /// Start-up, step 1: the lock has been read. (A second call is ignored.)
    pub fn publish_lock(&self, lock: LockService) {
        let _ = self.lock.set(lock);
    }

    /// Start-up, last step: storage has been inspected and any problem recorded.
    pub fn mark_ready(&self) {
        let _ = self.ready.set(());
    }

    #[cfg(test)]
    pub fn is_ready(&self) -> bool {
        self.ready.get().is_some()
    }

    /// Blocks until start-up has finished.
    pub fn wait_ready(&self) {
        self.ready.wait();
    }

    pub fn problem(&self) -> Option<StorageProblem> {
        self.wait_ready();
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
    ///
    /// `generation` is the lock generation read **before** the PIN was checked:
    /// if the lock engaged while the check ran, the confirmation belongs to a
    /// lock that no longer exists and is born stale.
    pub fn grant_step_up(&self, now: Instant, generation: u64) {
        *self.step_up.lock().unwrap_or_else(PoisonError::into_inner) = Some((now, generation));
    }

    /// Whether a PIN confirmation is still fresh. It lapses after
    /// [`STEP_UP_WINDOW`] and as soon as the lock engages; a lapsed one is forgotten.
    pub fn step_up_valid(&self, now: Instant) -> bool {
        let mut guard = self.step_up.lock().unwrap_or_else(PoisonError::into_inner);
        match *guard {
            Some((at, generation))
                if now.saturating_duration_since(at) < STEP_UP_WINDOW
                    && self
                        .lock
                        .get()
                        .is_some_and(|lock| lock.lock_generation() == generation) =>
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

    /// The one lock around every storage-changing operation (F14).
    ///
    /// Take it with `.lock().await` at the top of an **async command**, before
    /// the work is handed to `spawn_blocking`, and keep the guard until that work
    /// is done. Rules that keep it from ever deadlocking:
    /// * only async commands take it, never a synchronous command or the
    ///   lifecycle hook, which run on the main thread: the main thread must stay
    ///   free to serve the Kotlin calls the blocked work is waiting for;
    /// * the functions it protects (`initialize`, `commit_recovery`, ...) never
    ///   take it themselves, so nothing takes it twice;
    /// * re-check what the command assumed (is there still something to recover,
    ///   is a restore still staged) *after* taking it, since another command may
    ///   have finished first.
    pub fn storage_mutex(&self) -> &tauri::async_runtime::Mutex<()> {
        &self.storage_op
    }

    /// The app lock, once start-up has finished. **Blocks until then**: call it
    /// from async commands and background threads, never from the main thread.
    pub fn applock(&self) -> &LockService {
        self.wait_ready();
        self.lock.wait()
    }

    /// The lock as soon as it is loaded, for start-up code that runs before
    /// readiness is declared (it would otherwise wait for itself).
    pub(crate) fn applock_early(&self) -> &LockService {
        self.lock.wait()
    }

    /// The lock if start-up has finished, otherwise `None`. Never blocks: this is
    /// what the main-thread lifecycle hook uses.
    #[cfg_attr(
        desktop,
        allow(dead_code, reason = "used by the mobile lifecycle hook")
    )]
    pub fn applock_if_ready(&self) -> Option<&LockService> {
        self.ready.get()?;
        self.lock.get()
    }

    /// Runs `f` with the database, or fails with `locked` / `unavailable`.
    pub fn with_db<T>(
        &self,
        f: impl FnOnce(&mut Db) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        // The lock flag decides, not whether a connection happens to exist: the
        // lifecycle hook sets the flag first and closes the database a moment
        // later, and no command may slip through that moment.
        let lock = self.applock();
        if lock.is_locked() {
            return Err(AppError::locked());
        }
        let mut guard = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_mut() {
            // Checked again under the database lock: a lock that engaged while
            // this call waited for it must not run the closure.
            Some(db) if !lock.is_locked() => f(db),
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

    // --- F14: one storage operation at a time, and never a deadlock ---------------

    /// Many commands race for the storage mutex while "Kotlin" is served by a
    /// main-thread stand-in that, like the real main thread, never takes it. The
    /// blocked work waits on that thread (the lesson of the earlier ANR): if the
    /// mutex were ever held by, or waited on from, the main thread this would hang.
    #[test]
    fn concurrent_storage_operations_serialise_and_never_deadlock() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };

        let (state, dir) = open_state("concurrent", true);
        state.applock().mark_unlocked();
        let state = Arc::new(state);

        // The stand-in for the UI thread: answers requests, and also does the
        // synchronous things the lifecycle hook and sync commands do.
        let (to_main, requests) = mpsc::channel::<mpsc::Sender<()>>();
        let main_state = Arc::clone(&state);
        let main = std::thread::spawn(move || {
            while let Ok(reply) = requests.recv() {
                let _ = main_state.with_db(|_| Ok(()));
                main_state
                    .applock()
                    .on_suspended(Instant::now(), std::time::SystemTime::now());
                main_state
                    .applock()
                    .on_resumed(Instant::now(), std::time::SystemTime::now());
                let _ = reply.send(());
            }
        });

        let inside = Arc::new(AtomicUsize::new(0));
        let widest = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));
        let (finished, wait) = mpsc::channel::<()>();

        let runner = {
            let (state, inside, widest, done) = (
                Arc::clone(&state),
                Arc::clone(&inside),
                Arc::clone(&widest),
                Arc::clone(&done),
            );
            std::thread::spawn(move || {
                tauri::async_runtime::block_on(async move {
                    let mut tasks = Vec::new();
                    for _ in 0..24 {
                        let (state, inside, widest, done, to_main) = (
                            Arc::clone(&state),
                            Arc::clone(&inside),
                            Arc::clone(&widest),
                            Arc::clone(&done),
                            to_main.clone(),
                        );
                        tasks.push(tauri::async_runtime::spawn(async move {
                            // Taken in the async command, held across the blocking work.
                            let _op = state.storage_mutex().lock().await;
                            tauri::async_runtime::spawn_blocking(move || {
                                let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                                widest.fetch_max(now, Ordering::SeqCst);
                                // "Call into Kotlin": wait for the main thread.
                                let (reply, answered) = mpsc::channel();
                                to_main.send(reply).unwrap();
                                answered.recv().unwrap();
                                std::thread::sleep(std::time::Duration::from_millis(2));
                                inside.fetch_sub(1, Ordering::SeqCst);
                                done.fetch_add(1, Ordering::SeqCst);
                            })
                            .await
                            .unwrap();
                        }));
                    }
                    for task in tasks {
                        task.await.unwrap();
                    }
                });
                let _ = finished.send(());
            })
        };

        wait.recv_timeout(std::time::Duration::from_secs(30))
            .expect("storage operations deadlocked");
        runner.join().unwrap();
        assert_eq!(done.load(Ordering::SeqCst), 24);
        assert_eq!(widest.load(Ordering::SeqCst), 1, "operations overlapped");
        drop(state);
        let _ = main;
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Wiring, checked in the source (the command bodies need a running app): the
    /// storage-changing async commands take the mutex before anything else, and
    /// the main-thread lifecycle hook never does.
    #[test]
    fn the_storage_commands_take_the_mutex_and_the_main_thread_never_does() {
        fn body<'a>(source: &'a str, name: &str) -> &'a str {
            let start = source
                .find(&format!("pub async fn {name}("))
                .unwrap_or_else(|| panic!("{name} not found"));
            let rest = &source[start..];
            let end = rest.find("\n#[tauri::command]").unwrap_or(rest.len());
            &rest[..end]
        }
        for (source, name, then) in [
            (
                include_str!("recovery.rs"),
                "recovery_retry",
                "spawn_blocking",
            ),
            (
                include_str!("recovery.rs"),
                "recovery_start_fresh",
                "recovering()",
            ),
            (
                include_str!("backup_cmd.rs"),
                "backup_restore_commit",
                "require_unlocked_or_recovering",
            ),
            (include_str!("lock.rs"), "lock_unlock_pin", "blocking("),
            (
                include_str!("lock.rs"),
                "lock_unlock_biometric",
                "blocking(",
            ),
        ] {
            let text = body(source, name);
            let taken = text
                .find("storage_mutex()")
                .unwrap_or_else(|| panic!("{name} does not take the storage mutex"));
            let first = text.find(then).unwrap_or_else(|| panic!("{name}: {then}"));
            assert!(taken < first, "{name} must take the mutex before {then}");
        }
        // The lifecycle hook and the synchronous lock helpers run on the main thread.
        assert!(!include_str!("lib.rs").contains("storage_mutex"));
        assert!(!include_str!("applock.rs").contains("storage_mutex"));
    }

    // --- start-up is off the main thread, and nothing answers before it is done ------

    fn blocked_for(wait: std::time::Duration, f: impl FnOnce() + Send + 'static) -> bool {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            f();
            let _ = done.send(());
        });
        finished.recv_timeout(wait).is_err()
    }

    #[test]
    fn a_pending_state_blocks_every_answer_until_start_up_is_ready_then_answers_locked() {
        use std::sync::Arc;
        let dir = dir("pending");
        let lock = LockService::load(&dir, Instant::now());
        lock.set_pin(&zeroize::Zeroizing::new("493017".to_owned()))
            .unwrap();
        drop(lock);
        let state = Arc::new(AppState::pending());
        assert!(!state.is_ready());
        assert!(
            state.applock_if_ready().is_none(),
            "the main-thread accessor never blocks"
        );

        // Each of these would have answered "unlocked" or "no problem" on a state
        // that was not waiting; they must all be stuck.
        let wait = std::time::Duration::from_millis(200);
        let (s1, s2, s3) = (Arc::clone(&state), Arc::clone(&state), Arc::clone(&state));
        assert!(blocked_for(wait, move || {
            let _ = s1.applock().is_locked();
        }));
        assert!(blocked_for(wait, move || {
            let _ = s2.problem();
        }));
        assert!(blocked_for(wait, move || {
            let _ = s3.with_db(|_| Ok(()));
        }));

        // Start-up finishes on another thread; the same calls now answer, and
        // the app is locked (a PIN exists), so nothing is open.
        let starter = Arc::clone(&state);
        let starter_dir = dir.clone();
        std::thread::spawn(move || {
            starter.publish_lock(LockService::load(&starter_dir, Instant::now()));
            starter.mark_ready();
        });
        let answer = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    (
                        state.applock().is_locked(),
                        state.problem(),
                        state.with_db(|_| Ok(())).unwrap_err().code,
                    )
                })
                .join()
                .unwrap()
        });
        assert_eq!(answer, (true, None, crate::error::ErrorCode::Locked));
        assert!(state.applock_if_ready().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_start_up_steps_themselves_do_not_wait_for_readiness() {
        // `initialize` runs before ready is declared; it must be able to reach the lock.
        let dir = dir("early");
        let state = AppState::pending();
        state.publish_lock(LockService::load(&dir, Instant::now()));
        assert!(!state.applock_early().is_locked());
        assert!(!state.is_ready());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_step_up_cannot_exist_before_the_lock_does() {
        let state = AppState::pending();
        state.grant_step_up(Instant::now(), 0);
        assert!(!state.step_up_valid(Instant::now()));
    }

    /// Wiring, checked in the source: `setup` does no slow work and the main-thread
    /// lifecycle hook never takes the blocking accessor.
    #[test]
    fn setup_is_light_and_the_main_thread_hook_never_waits() {
        let lib = include_str!("lib.rs");
        let setup =
            &lib[lib.find(".setup(|app|").unwrap()..lib.find("// Leaving the foreground").unwrap()];
        for slow in [
            "app_data_dir",
            "LockService::load",
            "initialize(",
            "clean_leftovers",
        ] {
            assert!(!setup.contains(slow), "setup must not call {slow}");
        }
        assert!(setup.contains("AppState::pending()"));
        let hook = &lib[lib.find("// Leaving the foreground").unwrap()
            ..lib.find("tauri::generate_handler!").unwrap()];
        assert!(hook.contains("applock_if_ready()"));
        assert!(
            !hook.contains(".applock()"),
            "the hook runs on the main thread and must not block"
        );
    }
}
