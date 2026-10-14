//! The app lock: PIN verifier, failed-attempt counter and lock state.
//!
//! This is a gate on the UI, not on the data key (design decision K2): the
//! database key stays wrapped by the Keystore, so a forgotten PIN never makes
//! data unreadable. The verifier lives in `lock.json` beside the database, not
//! inside it, because the database is deliberately not opened while locked.
//!
//! Wrong guesses are counted and persisted *before* the PIN is checked, so
//! killing the process after a guess cannot give the guess back. Delays use the
//! monotonic clock and restart from the full delay after a process restart, so
//! changing the system clock never shortens a delay.

use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::{Duration, Instant, SystemTime},
};

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use mt_core::{AutoLock, PinProblem, is_plausible_pin, lockout_delay, validate_new_pin};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const LOCK_FILE: &str = "lock.json";
const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// No PIN has been set.
    NotConfigured,
    /// A PIN is already set.
    AlreadyConfigured,
    /// `lock.json` exists but cannot be trusted; the app stays locked.
    Broken,
    /// Too many wrong guesses; try again after the duration.
    LockedOut(Duration),
    WrongPin,
    InvalidPin(PinProblem),
    InvalidChoice,
    Io,
}

/// What the UI may know about the lock. Never contains the verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockSnapshot {
    pub configured: bool,
    pub locked: bool,
    pub broken: bool,
    pub biometric_enabled: bool,
    pub auto_lock_seconds: u32,
    pub failed_attempts: u32,
    pub retry_after: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    v: u32,
    /// PHC string: `$argon2id$v=19$m=...,t=...,p=...$salt$hash`.
    pin_hash: String,
    failed: u32,
    biometric: bool,
    auto_lock_secs: u32,
}

#[cfg_attr(desktop, allow(dead_code, reason = "set by the mobile lifecycle hook"))]
struct Suspension {
    mono: Instant,
    wall: SystemTime,
}

struct Inner {
    record: Option<Record>,
    broken: bool,
    locked: bool,
    lockout_until: Option<Instant>,
    suspended: Option<Suspension>,
}

pub struct LockService {
    path: PathBuf,
    inner: Mutex<Inner>,
}

#[cfg(not(test))]
fn hasher() -> Argon2<'static> {
    // OWASP's minimum for Argon2id: 19 MiB, 2 passes, 1 lane.
    #[allow(clippy::expect_used, reason = "constant parameters are always valid")]
    let params = Params::new(19 * 1024, 2, 1, Some(32)).expect("valid Argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

#[cfg(test)]
fn hasher() -> Argon2<'static> {
    #[allow(clippy::expect_used, reason = "constant parameters are always valid")]
    let params = Params::new(8, 1, 1, Some(32)).expect("valid Argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hash_pin(pin: &str) -> Result<String, LockError> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| LockError::Io)?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| LockError::Io)?;
    hasher()
        .hash_password(pin.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| LockError::Io)
}

fn pin_matches(hash: &str, pin: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| hasher().verify_password(pin.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

impl LockService {
    /// Reads `lock.json`. A missing file means no lock; an unreadable or
    /// unrecognised one fails closed.
    pub fn load(dir: &Path, now: Instant) -> Self {
        let path = dir.join(LOCK_FILE);
        let mut inner = Inner {
            record: None,
            broken: false,
            locked: false,
            lockout_until: None,
            suspended: None,
        };
        match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Record>(&bytes) {
                Ok(record)
                    if record.v == FORMAT
                        && AutoLock::from_seconds(record.auto_lock_secs).is_some() =>
                {
                    let delay = lockout_delay(record.failed);
                    if !delay.is_zero() {
                        inner.lockout_until = Some(now + delay);
                    }
                    inner.record = Some(record);
                    inner.locked = true;
                }
                _ => {
                    inner.broken = true;
                    inner.locked = true;
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {
                inner.broken = true;
                inner.locked = true;
            }
        }
        Self {
            path,
            inner: Mutex::new(inner),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn snapshot(&self, now: Instant) -> LockSnapshot {
        let inner = self.state();
        let retry_after = inner
            .lockout_until
            .and_then(|until| until.checked_duration_since(now))
            .filter(|left| !left.is_zero());
        LockSnapshot {
            configured: inner.record.is_some() || inner.broken,
            locked: inner.locked,
            broken: inner.broken,
            biometric_enabled: inner.record.as_ref().is_some_and(|r| r.biometric),
            auto_lock_seconds: inner
                .record
                .as_ref()
                .map_or(AutoLock::DEFAULT.seconds(), |r| r.auto_lock_secs),
            failed_attempts: inner.record.as_ref().map_or(0, |r| r.failed),
            retry_after,
        }
    }

    pub fn is_locked(&self) -> bool {
        self.state().locked
    }

    /// Checks `pin` against the stored verifier, counting the attempt.
    ///
    /// Success does not unlock: the caller opens the database first and then
    /// calls [`LockService::mark_unlocked`].
    pub fn verify_pin(&self, pin: &Zeroizing<String>, now: Instant) -> Result<(), LockError> {
        let mut inner = self.state();
        self.verify_locked(&mut inner, pin, now)
    }

    fn verify_locked(&self, inner: &mut Inner, pin: &str, now: Instant) -> Result<(), LockError> {
        if inner.broken {
            return Err(LockError::Broken);
        }
        let Some(record) = inner.record.as_mut() else {
            return Err(LockError::NotConfigured);
        };
        if let Some(left) = inner
            .lockout_until
            .and_then(|until| until.checked_duration_since(now))
            .filter(|d| !d.is_zero())
        {
            return Err(LockError::LockedOut(left));
        }

        // Count first, check second.
        record.failed = record.failed.saturating_add(1);
        let snapshot = record.clone();
        write_record(&self.path, &snapshot).map_err(|_| {
            // Could not persist the attempt, so it must not be allowed.
            LockError::Io
        })?;

        let ok = is_plausible_pin(pin) && pin_matches(&snapshot.pin_hash, pin);
        if ok {
            if let Some(record) = inner.record.as_mut() {
                record.failed = 0;
                // Best effort: a failed reset only costs the user a stale counter.
                let _ = write_record(&self.path, record);
            }
            inner.lockout_until = None;
            Ok(())
        } else {
            let delay = lockout_delay(snapshot.failed);
            inner.lockout_until = (!delay.is_zero()).then(|| now + delay);
            Err(LockError::WrongPin)
        }
    }

    pub fn mark_unlocked(&self) {
        let mut inner = self.state();
        inner.locked = false;
        inner.suspended = None;
    }

    pub fn lock_now(&self) {
        let mut inner = self.state();
        if inner.record.is_some() {
            inner.locked = true;
        }
    }

    /// Sets the first PIN. The caller must already be unlocked (no lock exists).
    pub fn set_pin(&self, pin: &Zeroizing<String>) -> Result<(), LockError> {
        let mut inner = self.state();
        if inner.broken || inner.record.is_some() {
            return Err(LockError::AlreadyConfigured);
        }
        validate_new_pin(pin).map_err(LockError::InvalidPin)?;
        let record = Record {
            v: FORMAT,
            pin_hash: hash_pin(pin)?,
            failed: 0,
            biometric: false,
            auto_lock_secs: AutoLock::DEFAULT.seconds(),
        };
        write_record(&self.path, &record).map_err(|_| LockError::Io)?;
        inner.record = Some(record);
        inner.locked = false;
        Ok(())
    }

    pub fn change_pin(
        &self,
        current: &Zeroizing<String>,
        new: &Zeroizing<String>,
        now: Instant,
    ) -> Result<(), LockError> {
        let mut inner = self.state();
        validate_new_pin(new).map_err(LockError::InvalidPin)?;
        self.verify_locked(&mut inner, current, now)?;
        let hash = hash_pin(new)?;
        let record = inner.record.as_mut().ok_or(LockError::NotConfigured)?;
        record.pin_hash = hash;
        record.failed = 0;
        let saved = record.clone();
        write_record(&self.path, &saved).map_err(|_| LockError::Io)
    }

    /// Removes the lock after confirming the PIN.
    pub fn remove(&self, current: &Zeroizing<String>, now: Instant) -> Result<(), LockError> {
        let mut inner = self.state();
        self.verify_locked(&mut inner, current, now)?;
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(LockError::Io),
        }
        inner.record = None;
        inner.locked = false;
        inner.lockout_until = None;
        Ok(())
    }

    pub fn set_biometric(&self, enabled: bool) -> Result<(), LockError> {
        self.update(|record| record.biometric = enabled)
    }

    pub fn set_auto_lock(&self, seconds: u32) -> Result<(), LockError> {
        let choice = AutoLock::from_seconds(seconds).ok_or(LockError::InvalidChoice)?;
        self.update(|record| record.auto_lock_secs = choice.seconds())
    }

    fn update(&self, change: impl FnOnce(&mut Record)) -> Result<(), LockError> {
        let mut inner = self.state();
        let record = inner.record.as_mut().ok_or(LockError::NotConfigured)?;
        let mut next = record.clone();
        change(&mut next);
        write_record(&self.path, &next).map_err(|_| LockError::Io)?;
        *record = next;
        Ok(())
    }

    /// The app left the foreground. Locks at once when the grace is zero.
    #[cfg_attr(
        desktop,
        allow(dead_code, reason = "called from the mobile lifecycle hook")
    )]
    pub fn on_suspended(&self, now: Instant, wall: SystemTime) {
        let mut inner = self.state();
        let Some(record) = inner.record.as_ref() else {
            return;
        };
        if inner.locked {
            return;
        }
        let grace = AutoLock::from_seconds(record.auto_lock_secs)
            .unwrap_or(AutoLock::DEFAULT)
            .grace();
        if grace.is_zero() {
            inner.locked = true;
        } else {
            inner.suspended = Some(Suspension { mono: now, wall });
        }
    }

    /// The app is back. Locks if it was away at least the grace, by either
    /// clock, or if the wall clock went backwards (an attempt to stretch it).
    #[cfg_attr(
        desktop,
        allow(dead_code, reason = "called from the mobile lifecycle hook")
    )]
    pub fn on_resumed(&self, now: Instant, wall: SystemTime) {
        let mut inner = self.state();
        let Some(seconds) = inner.record.as_ref().map(|r| r.auto_lock_secs) else {
            return;
        };
        let Some(away) = inner.suspended.take() else {
            return;
        };
        let grace = AutoLock::from_seconds(seconds)
            .unwrap_or(AutoLock::DEFAULT)
            .grace();
        let mono = now.saturating_duration_since(away.mono);
        let expired = match wall.duration_since(away.wall) {
            Ok(elapsed) => mono >= grace || elapsed >= grace,
            Err(_) => true,
        };
        if expired {
            inner.locked = true;
        }
    }
}

/// Writes, fsyncs and renames so a crash never leaves a truncated lock file.
fn write_record(path: &Path, record: &Record) -> io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!("mt-applock-{}-{n}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pin(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_owned())
    }

    fn fresh(dir: &Dir) -> (LockService, Instant) {
        let now = Instant::now();
        (LockService::load(&dir.0, now), now)
    }

    #[test]
    fn no_file_means_no_lock() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        let snap = service.snapshot(now);
        assert!(!snap.configured && !snap.locked && !snap.broken);
        assert_eq!(
            service.verify_pin(&pin("493017"), now),
            Err(LockError::NotConfigured)
        );
    }

    #[test]
    fn setting_a_pin_persists_and_next_start_is_locked() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        assert!(!service.is_locked(), "the user who just set it is unlocked");

        let (again, now) = (LockService::load(&dir.0, now), now);
        let snap = again.snapshot(now);
        assert!(snap.configured && snap.locked);
        assert_eq!(again.verify_pin(&pin("493017"), now), Ok(()));
        assert_eq!(
            again.verify_pin(&pin("493018"), now),
            Err(LockError::WrongPin)
        );
        // The PIN itself is never written.
        let text = fs::read_to_string(dir.0.join(LOCK_FILE)).unwrap();
        assert!(!text.contains("493017"));
        assert!(text.contains("$argon2id$"));
    }

    #[test]
    fn weak_pins_are_refused_at_setup() {
        let dir = Dir::new();
        let (service, _) = fresh(&dir);
        assert_eq!(
            service.set_pin(&pin("123456")),
            Err(LockError::InvalidPin(PinProblem::TooSimple))
        );
        assert!(!service.snapshot(Instant::now()).configured);
    }

    #[test]
    fn wrong_guesses_are_delayed_and_the_count_survives_restart() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();

        for _ in 0..4 {
            assert_eq!(
                service.verify_pin(&pin("000001"), now),
                Err(LockError::WrongPin)
            );
        }
        // The fifth miss starts the 30 s delay.
        assert_eq!(
            service.verify_pin(&pin("000001"), now),
            Err(LockError::WrongPin)
        );
        assert_eq!(
            service.verify_pin(&pin("493017"), now),
            Err(LockError::LockedOut(Duration::from_secs(30))),
            "even the right PIN waits"
        );
        assert_eq!(
            service.snapshot(now).retry_after,
            Some(Duration::from_secs(30))
        );

        // After the delay the right PIN works and resets the counter.
        let later = now + Duration::from_secs(31);
        assert_eq!(service.verify_pin(&pin("493017"), later), Ok(()));
        assert_eq!(service.snapshot(later).failed_attempts, 0);
        assert_eq!(service.snapshot(later).retry_after, None);
    }

    #[test]
    fn restarting_the_app_does_not_reset_the_delay() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        // Six misses, each after the previous delay has passed.
        for n in 0..6 {
            let at = now + Duration::from_secs(4000 * n);
            assert_eq!(
                service.verify_pin(&pin("000001"), at),
                Err(LockError::WrongPin)
            );
        }
        // A new process starts a fresh monotonic clock and the full delay.
        let restarted_at = Instant::now();
        let again = LockService::load(&dir.0, restarted_at);
        let snap = again.snapshot(restarted_at);
        assert_eq!(snap.failed_attempts, 6);
        assert_eq!(snap.retry_after, Some(Duration::from_secs(60)));
        assert_eq!(
            again.verify_pin(&pin("493017"), restarted_at),
            Err(LockError::LockedOut(Duration::from_secs(60)))
        );
    }

    #[test]
    fn a_guess_is_counted_even_if_the_process_dies_mid_check() {
        // The counter is written before the PIN is compared, so reading the file
        // right after a wrong guess already shows it.
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        let _ = service.verify_pin(&pin("000001"), now);
        let on_disk: Record =
            serde_json::from_slice(&fs::read(dir.0.join(LOCK_FILE)).unwrap()).unwrap();
        assert_eq!(on_disk.failed, 1);
    }

    #[test]
    fn implausible_entries_count_as_misses() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        assert_eq!(
            service.verify_pin(&pin("abc"), now),
            Err(LockError::WrongPin)
        );
        assert_eq!(service.snapshot(now).failed_attempts, 1);
    }

    #[test]
    fn a_corrupt_lock_file_fails_closed() {
        let dir = Dir::new();
        fs::write(dir.0.join(LOCK_FILE), b"{not json").unwrap();
        let (service, now) = fresh(&dir);
        let snap = service.snapshot(now);
        assert!(snap.broken && snap.locked && snap.configured);
        assert_eq!(
            service.verify_pin(&pin("493017"), now),
            Err(LockError::Broken)
        );
        assert_eq!(
            service.set_pin(&pin("493017")),
            Err(LockError::AlreadyConfigured)
        );
    }

    #[test]
    fn an_unknown_format_version_fails_closed() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        let text = fs::read_to_string(dir.0.join(LOCK_FILE))
            .unwrap()
            .replace("\"v\":1", "\"v\":2");
        fs::write(dir.0.join(LOCK_FILE), text).unwrap();
        assert!(LockService::load(&dir.0, now).snapshot(now).broken);
    }

    #[test]
    fn changing_the_pin_needs_the_current_one() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        assert_eq!(
            service.change_pin(&pin("000001"), &pin("718204"), now),
            Err(LockError::WrongPin)
        );
        assert_eq!(
            service.change_pin(&pin("493017"), &pin("111111"), now),
            Err(LockError::InvalidPin(PinProblem::TooSimple))
        );
        service
            .change_pin(&pin("493017"), &pin("718204"), now)
            .unwrap();
        assert_eq!(service.verify_pin(&pin("718204"), now), Ok(()));
        assert_eq!(
            service.verify_pin(&pin("493017"), now),
            Err(LockError::WrongPin)
        );
    }

    #[test]
    fn removing_the_lock_needs_the_pin_and_deletes_the_file() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        assert_eq!(
            service.remove(&pin("000001"), now),
            Err(LockError::WrongPin)
        );
        assert!(dir.0.join(LOCK_FILE).exists());
        service.remove(&pin("493017"), now).unwrap();
        assert!(!dir.0.join(LOCK_FILE).exists());
        assert!(!service.snapshot(now).configured);
    }

    #[test]
    fn backgrounding_locks_immediately_by_default() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.on_suspended(now, SystemTime::now());
        assert!(service.is_locked());
    }

    #[test]
    fn a_grace_period_locks_only_after_it_has_passed() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.set_auto_lock(60).unwrap();
        let wall = SystemTime::now();

        service.on_suspended(now, wall);
        assert!(!service.is_locked());
        service.on_resumed(
            now + Duration::from_secs(20),
            wall + Duration::from_secs(20),
        );
        assert!(!service.is_locked(), "back within the grace");

        service.on_suspended(now, wall);
        service.on_resumed(
            now + Duration::from_secs(61),
            wall + Duration::from_secs(61),
        );
        assert!(service.is_locked());
    }

    #[test]
    fn winding_the_clock_back_still_locks() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.set_auto_lock(300).unwrap();
        let wall = SystemTime::now();
        service.on_suspended(now, wall);
        service.on_resumed(
            now + Duration::from_secs(1),
            wall - Duration::from_secs(3600),
        );
        assert!(service.is_locked());
    }

    #[test]
    fn a_long_wall_clock_gap_locks_even_if_the_monotonic_clock_barely_moved() {
        // Deep sleep stops the monotonic clock on Linux.
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.set_auto_lock(30).unwrap();
        let wall = SystemTime::now();
        service.on_suspended(now, wall);
        service.on_resumed(
            now + Duration::from_secs(1),
            wall + Duration::from_secs(600),
        );
        assert!(service.is_locked());
    }

    #[test]
    fn settings_reject_unknown_auto_lock_values_and_need_a_pin() {
        let dir = Dir::new();
        let (service, _) = fresh(&dir);
        assert_eq!(service.set_biometric(true), Err(LockError::NotConfigured));
        service.set_pin(&pin("493017")).unwrap();
        assert_eq!(service.set_auto_lock(45), Err(LockError::InvalidChoice));
        service.set_biometric(true).unwrap();
        assert!(service.snapshot(Instant::now()).biometric_enabled);
    }
}
