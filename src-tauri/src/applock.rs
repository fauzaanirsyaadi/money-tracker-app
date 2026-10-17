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
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tauri_plugin_mt_platform::hex;
use zeroize::Zeroizing;

pub(crate) const LOCK_FILE: &str = "lock.json";
const FORMAT: u32 = 1;

/// Longest the system file picker may hold the app in the background before
/// the lock applies as usual. Short on purpose: it exists only so choosing a
/// backup file does not lock the user out of their own export. The window ends
/// earlier, the moment the picker returns or is cancelled.
pub const EXTERNAL_UI_MAX: Duration = Duration::from_secs(60);

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

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    v: u32,
    /// PHC string: `$argon2id$v=19$m=...,t=...,p=...$salt$hash`.
    pin_hash: String,
    failed: u32,
    biometric: bool,
    auto_lock_secs: u32,
    /// The token the biometric key sealed at enrolment (hex of `iv || ciphertext`).
    /// Useless without the Keystore key, which opens only for a fresh biometric.
    #[serde(default)]
    bio_blob: Option<String>,
    /// SHA-256 of the token (hex): what a token released by the key is checked against.
    #[serde(default)]
    bio_check: Option<String>,
}

impl Record {
    /// Biometric unlock needs both the switch and the material from an enrolment:
    /// a switch left on without it (an older file, a damaged field) means off.
    fn biometric_ready(&self) -> bool {
        self.biometric && self.bio_blob.is_some() && self.bio_check.is_some()
    }
}

#[cfg_attr(desktop, allow(dead_code, reason = "set by the mobile lifecycle hook"))]
struct Suspension {
    mono: Instant,
    wall: SystemTime,
    /// Time since boot, *including* time the device spent asleep (the monotonic
    /// clock stops in deep sleep, and the wall clock can be set by whoever holds
    /// the phone), when the platform can tell.
    boot: Option<Duration>,
    /// Set when the app left for a system picker that we launched: it may stay
    /// away until this moment regardless of the auto-lock choice.
    external_deadline: Option<Instant>,
}

impl Inner {
    fn engage(&mut self) {
        if !self.locked {
            self.generation = self.generation.wrapping_add(1);
        }
        self.locked = true;
    }
}

struct Inner {
    record: Option<Record>,
    broken: bool,
    locked: bool,
    /// Bumped every time the lock engages, so a PIN confirmation made before
    /// cannot outlive it (see `AppState::step_up_valid`).
    generation: u64,
    lockout_until: Option<Instant>,
    suspended: Option<Suspension>,
    /// Where "time since boot" comes from; replaced in tests.
    boot_clock: fn() -> Option<Duration>,
    /// End of the window opened by `begin_external_ui`. In memory only: a
    /// process that dies while the picker is open comes back locked.
    external_until: Option<Instant>,
}

/// Time since boot including deep sleep (`/proc/uptime`: CLOCK_BOOTTIME), or None
/// where the platform has no such file. Neither the monotonic clock (which stops
/// while the device sleeps) nor the wall clock (which the user can set) can stand in.
fn boot_elapsed() -> Option<Duration> {
    let text = fs::read_to_string("/proc/uptime").ok()?;
    let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
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

// Amounts, names and notes must never reach a log through `{:?}`.
mt_core::redacted_debug!(Record);

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
            generation: 0,
            lockout_until: None,
            suspended: None,
            boot_clock: boot_elapsed,
            external_until: None,
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

    /// How many times the lock has engaged since start.
    pub fn lock_generation(&self) -> u64 {
        self.state().generation
    }

    /// Whether the app is locked **and** the generation, read together under one
    /// lock acquisition: reading them separately leaves a gap in which the lock can
    /// engage, and a PIN confirmation stamped with the later generation would then
    /// belong to a lock the user never confirmed.
    pub fn locked_and_generation(&self) -> (bool, u64) {
        let inner = self.state();
        (inner.locked, inner.generation)
    }

    #[cfg(test)]
    pub(crate) fn set_boot_clock(&self, clock: fn() -> Option<Duration>) {
        self.state().boot_clock = clock;
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
            biometric_enabled: inner.record.as_ref().is_some_and(Record::biometric_ready),
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

    /// The unlock succeeded. If the app left the foreground *while* it was being
    /// unlocked (the suspension was recorded although the app was locked), that
    /// departure counts as if it had happened just now: with no grace the lock
    /// engages again at once, otherwise the time away is measured from when the
    /// app really left, when it comes back.
    pub fn mark_unlocked(&self) {
        let mut inner = self.state();
        inner.locked = false;
        let relock = match (&inner.suspended, inner.record.as_ref()) {
            (Some(away), Some(record)) => {
                let grace = AutoLock::from_seconds(record.auto_lock_secs)
                    .unwrap_or(AutoLock::DEFAULT)
                    .grace();
                away.external_deadline.is_none() && grace.is_zero()
            }
            _ => false,
        };
        if relock {
            inner.engage();
        }
    }

    pub fn lock_now(&self) {
        let mut inner = self.state();
        if inner.record.is_some() {
            inner.engage();
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
            bio_blob: None,
            bio_check: None,
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

    /// Forgets the lock entirely, deleting `lock.json`. For recovery, after the
    /// user confirmed that the old data (and its PIN) are being replaced.
    pub fn erase(&self) -> io::Result<()> {
        let mut inner = self.state();
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        inner.record = None;
        inner.broken = false;
        inner.locked = false;
        inner.lockout_until = None;
        inner.suspended = None;
        Ok(())
    }

    /// Turns biometric unlock on with the material from a fresh enrolment.
    pub fn enable_biometric(&self, token: &[u8], blob: &[u8]) -> Result<(), LockError> {
        let check = hex::encode(&Sha256::digest(token));
        let blob = hex::encode(blob);
        self.update(|record| {
            record.biometric = true;
            record.bio_blob = Some(blob);
            record.bio_check = Some(check);
        })
    }

    /// Turns biometric unlock off and forgets its material. The PIN is unaffected.
    pub fn disable_biometric(&self) -> Result<(), LockError> {
        self.update(|record| {
            record.biometric = false;
            record.bio_blob = None;
            record.bio_check = None;
        })
    }

    /// The sealed token to hand to the biometric prompt, when biometric unlock is on.
    pub fn biometric_blob(&self) -> Option<Vec<u8>> {
        let inner = self.state();
        let record = inner.record.as_ref().filter(|r| r.biometric_ready())?;
        hex::decode(record.bio_blob.as_deref()?)
    }

    /// Whether `token`, released by the biometric key, is the one enrolled.
    pub fn biometric_token_matches(&self, token: &[u8]) -> bool {
        let inner = self.state();
        let Some(expected) = inner
            .record
            .as_ref()
            .filter(|r| r.biometric_ready())
            .and_then(|r| r.bio_check.as_deref())
            .and_then(hex::decode)
        else {
            return false;
        };
        let actual = Sha256::digest(token);
        bool::from(expected.as_slice().ct_eq(actual.as_slice()))
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

    /// Opens a short window for a system picker we are about to launch. Call
    /// right before starting it and [`LockService::end_external_ui`] when it
    /// returns or is cancelled.
    pub fn begin_external_ui(&self, now: Instant) {
        self.state().external_until = Some(now + EXTERNAL_UI_MAX);
    }

    pub fn end_external_ui(&self) {
        self.state().external_until = None;
    }

    /// The app left the foreground. Locks at once when the grace is zero,
    /// unless a system picker we launched is open.
    #[cfg_attr(
        desktop,
        allow(dead_code, reason = "called from the mobile lifecycle hook")
    )]
    pub fn on_suspended(&self, now: Instant, wall: SystemTime) {
        let mut inner = self.state();
        let Some(seconds) = inner.record.as_ref().map(|r| r.auto_lock_secs) else {
            return;
        };
        let external_deadline = inner.external_until.filter(|until| now < *until);
        if inner.locked {
            // Already locked, but the departure still matters: an unlock may be
            // in progress and finish after the app is gone (see `mark_unlocked`).
            let boot = (inner.boot_clock)();
            inner.suspended = Some(Suspension {
                mono: now,
                wall,
                boot,
                external_deadline,
            });
            return;
        }
        let grace = AutoLock::from_seconds(seconds)
            .unwrap_or(AutoLock::DEFAULT)
            .grace();
        if external_deadline.is_none() && grace.is_zero() {
            inner.engage();
        } else {
            let boot = (inner.boot_clock)();
            inner.suspended = Some(Suspension {
                mono: now,
                wall,
                boot,
                external_deadline,
            });
        }
    }

    /// The app is back. Locks if it was away at least the grace, by either
    /// clock, or if the wall clock went backwards (an attempt to stretch it).
    /// After a system picker, the only limit is the picker's own deadline.
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
        let clock_went_back = wall.duration_since(away.wall).is_err();
        // Time away by the boot clock: None when it cannot be read (then the other
        // two clocks decide as before); a clock that went backwards means a reboot,
        // which locks.
        let boot_away = match (away.boot, (inner.boot_clock)()) {
            (Some(then), Some(now_boot)) => match now_boot.checked_sub(then) {
                Some(away_for) => Some(away_for),
                None => return inner.engage(),
            },
            _ => None,
        };
        if away.external_deadline.is_some() {
            // The picker we launched has returned: its window is spent, whether
            // or not the command awaiting it has reported back yet.
            inner.external_until = None;
        }
        let expired = if let Some(deadline) = away.external_deadline {
            now > deadline
                || clock_went_back
                || boot_away.is_some_and(|away_for| away_for > EXTERNAL_UI_MAX)
        } else {
            let grace = AutoLock::from_seconds(seconds)
                .unwrap_or(AutoLock::DEFAULT)
                .grace();
            let mono = now.saturating_duration_since(away.mono);
            let elapsed = wall.duration_since(away.wall).unwrap_or_default();
            clock_went_back
                || mono >= grace
                || elapsed >= grace
                || boot_away.is_some_and(|away_for| away_for >= grace)
        };
        if expired {
            inner.engage();
        }
    }
}

/// A crash or a power loss never leaves a truncated lock file (see `durable`).
fn write_record(path: &Path, record: &Record) -> io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
    crate::durable::write_atomic(path, &bytes)
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
    fn erase_forgets_the_lock_even_when_the_file_is_damaged() {
        let dir = Dir::new();
        fs::write(dir.0.join(LOCK_FILE), b"{broken").unwrap();
        let (service, now) = fresh(&dir);
        assert!(service.snapshot(now).broken);
        service.erase().unwrap();
        let snap = service.snapshot(now);
        assert!(!snap.configured && !snap.locked && !snap.broken);
        assert!(!dir.0.join(LOCK_FILE).exists());
        // Erasing again is harmless, and a PIN can be set afresh.
        service.erase().unwrap();
        service.set_pin(&pin("493017")).unwrap();
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
    fn a_picker_window_keeps_the_app_unlocked_while_it_is_open() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        // Auto-lock is immediate: without the window this would lock at once.
        let wall = SystemTime::now();
        service.begin_external_ui(now);
        service.on_suspended(now, wall);
        assert!(!service.is_locked(), "the picker must not lock the app");
        service.end_external_ui();
        service.on_resumed(
            now + Duration::from_secs(20),
            wall + Duration::from_secs(20),
        );
        assert!(!service.is_locked(), "back from the picker in time");
    }

    #[test]
    fn the_picker_window_is_one_minute() {
        assert_eq!(EXTERNAL_UI_MAX, Duration::from_secs(60));
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        let wall = SystemTime::now();
        service.begin_external_ui(now);
        service.on_suspended(now, wall);
        // Just inside the minute: still fine. Just outside: locked.
        let inside = Duration::from_secs(59);
        service.on_resumed(now + inside, wall + inside);
        assert!(!service.is_locked());

        service.begin_external_ui(now);
        service.on_suspended(now, wall);
        let outside = Duration::from_secs(61);
        service.on_resumed(now + outside, wall + outside);
        assert!(service.is_locked());
    }

    #[test]
    fn the_window_ends_when_the_picker_returns_even_before_the_command_reports() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        let wall = SystemTime::now();
        service.begin_external_ui(now);
        service.on_suspended(now, wall);
        service.on_resumed(now + Duration::from_secs(5), wall + Duration::from_secs(5));
        assert!(!service.is_locked());
        // `end_external_ui` has not run yet; leaving again must not ride on the old window.
        service.on_suspended(now + Duration::from_secs(6), wall + Duration::from_secs(6));
        assert!(service.is_locked(), "a spent window protects nothing");
    }

    #[test]
    fn a_cancelled_picker_leaves_no_window_behind() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        // The picker was opened and cancelled without the app ever being suspended.
        service.begin_external_ui(now);
        service.end_external_ui();
        service.on_suspended(now + Duration::from_secs(1), SystemTime::now());
        assert!(service.is_locked());
    }

    #[test]
    fn a_picker_window_is_capped() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        let wall = SystemTime::now();
        service.begin_external_ui(now);
        service.on_suspended(now, wall);
        let late = EXTERNAL_UI_MAX + Duration::from_secs(1);
        service.on_resumed(now + late, wall + late);
        assert!(service.is_locked(), "away longer than the cap");
    }

    #[test]
    fn leaving_after_the_picker_closed_locks_again() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.begin_external_ui(now);
        service.end_external_ui();
        service.on_suspended(now, SystemTime::now());
        assert!(
            service.is_locked(),
            "the window is gone once the picker returned"
        );
    }

    #[test]
    fn an_expired_window_does_not_protect_a_later_background() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.begin_external_ui(now);
        // Suspended long after the window ran out (the picker never returned).
        service.on_suspended(
            now + EXTERNAL_UI_MAX + Duration::from_secs(5),
            SystemTime::now(),
        );
        assert!(service.is_locked());
    }

    #[test]
    fn winding_the_clock_back_during_a_picker_still_locks() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        let wall = SystemTime::now();
        service.begin_external_ui(now);
        service.on_suspended(now, wall);
        service.on_resumed(
            now + Duration::from_secs(2),
            wall - Duration::from_secs(600),
        );
        assert!(service.is_locked());
    }

    #[test]
    fn the_picker_window_does_not_survive_a_new_process() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.begin_external_ui(now);
        // Process death: a new service starts locked and has no window.
        let again = LockService::load(&dir.0, now);
        assert!(again.is_locked());
        again.on_suspended(now, SystemTime::now());
        assert!(again.is_locked());
    }

    #[test]
    fn settings_reject_unknown_auto_lock_values_and_need_a_pin() {
        let dir = Dir::new();
        let (service, _) = fresh(&dir);
        assert_eq!(
            service.enable_biometric(&[1; 32], &[2; 40]),
            Err(LockError::NotConfigured)
        );
        service.set_pin(&pin("493017")).unwrap();
        assert_eq!(service.set_auto_lock(45), Err(LockError::InvalidChoice));
    }

    #[test]
    fn biometric_unlock_needs_the_enrolment_material_and_checks_the_token() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        assert!(!service.snapshot(now).biometric_enabled);
        assert_eq!(service.biometric_blob(), None);
        assert!(!service.biometric_token_matches(&[7; 32]));

        service.enable_biometric(&[7; 32], &[9; 40]).unwrap();
        assert!(service.snapshot(now).biometric_enabled);
        assert_eq!(service.biometric_blob(), Some(vec![9; 40]));
        assert!(service.biometric_token_matches(&[7; 32]));
        assert!(!service.biometric_token_matches(&[8; 32]));
        assert!(!service.biometric_token_matches(&[]));

        // It survives a restart, and the token itself is not in the file.
        let again = LockService::load(&dir.0, now);
        assert!(again.snapshot(now).biometric_enabled);
        assert!(again.biometric_token_matches(&[7; 32]));
        let text = fs::read_to_string(dir.0.join(LOCK_FILE)).unwrap();
        assert!(
            !text.contains(&hex::encode(&[7u8; 32])),
            "only a hash of the token is kept"
        );

        service.disable_biometric().unwrap();
        assert!(!service.snapshot(now).biometric_enabled);
        assert_eq!(service.biometric_blob(), None);
        assert!(!service.biometric_token_matches(&[7; 32]));
        // The PIN is untouched by any of this.
        assert!(service.verify_pin(&pin("493017"), now).is_ok());
    }

    #[test]
    fn a_switch_left_on_without_material_means_off() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        // An older file: `biometric: true` and no blob or check.
        let text = fs::read_to_string(dir.0.join(LOCK_FILE)).unwrap();
        let edited = text.replace("\"biometric\":false", "\"biometric\":true");
        assert_ne!(text, edited);
        fs::write(dir.0.join(LOCK_FILE), edited).unwrap();
        let again = LockService::load(&dir.0, now);
        assert!(!again.snapshot(now).biometric_enabled);
        assert!(!again.biometric_token_matches(&[0; 32]));
    }

    // --- F10: leaving while the unlock is still in progress ----------------------

    #[test]
    fn leaving_during_an_unlock_with_no_grace_locks_again_when_it_finishes() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.lock_now();
        assert!(service.is_locked());
        // Suspended while locked (the PIN is being checked), then the unlock completes.
        service.on_suspended(now, SystemTime::now());
        service.mark_unlocked();
        assert!(
            service.is_locked(),
            "the app is in the background: it must not stay open"
        );
    }

    #[test]
    fn leaving_during_an_unlock_with_a_grace_is_measured_from_when_it_left() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.set_auto_lock(30).unwrap();
        let wall = SystemTime::now();

        // Back within the grace: stays unlocked.
        service.lock_now();
        service.on_suspended(now, wall);
        service.mark_unlocked();
        assert!(!service.is_locked());
        let soon = Duration::from_secs(5);
        service.on_resumed(now + soon, wall + soon);
        assert!(!service.is_locked());

        // Back after the grace, counted from the departure (not from the unlock).
        service.lock_now();
        service.on_suspended(now, wall);
        service.mark_unlocked();
        assert!(!service.is_locked());
        let late = Duration::from_secs(31);
        service.on_resumed(now + late, wall + late);
        assert!(service.is_locked());
    }

    #[test]
    fn an_unlock_that_nobody_left_stays_unlocked() {
        let dir = Dir::new();
        let (service, _) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.lock_now();
        service.mark_unlocked();
        assert!(!service.is_locked());
    }

    #[test]
    fn a_departure_that_was_resumed_before_the_unlock_ended_does_not_relock() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.lock_now();
        let wall = SystemTime::now();
        service.on_suspended(now, wall);
        service.on_resumed(now + Duration::from_secs(1), wall + Duration::from_secs(1));
        service.mark_unlocked();
        assert!(!service.is_locked(), "the app is back in front");
    }

    #[test]
    fn a_picker_departure_during_an_unlock_still_gets_its_window() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.lock_now();
        service.begin_external_ui(now);
        service.on_suspended(now, SystemTime::now());
        service.mark_unlocked();
        assert!(!service.is_locked());
    }

    /// Time of one PIN check with the production Argon2id cost (the tests above use a
    /// tiny cost on purpose). Run: cargo test -p money-tracker-app --release
    /// -- --ignored --nocapture perf_pin_hash
    #[test]
    #[ignore = "timing: needs --release --ignored --nocapture"]
    #[allow(
        clippy::print_stdout,
        reason = "the numbers are the point of this test"
    )]
    fn perf_pin_hash_with_the_production_cost() {
        let params = Params::new(19 * 1024, 2, 1, Some(32)).unwrap();
        let production = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let salt = SaltString::encode_b64(&[7u8; 16]).unwrap();
        let hash = production
            .hash_password(b"493017", &salt)
            .unwrap()
            .to_string();
        let parsed = PasswordHash::new(&hash).unwrap();
        let started = Instant::now();
        for _ in 0..5 {
            production.verify_password(b"493017", &parsed).unwrap();
        }
        println!(
            "PIN check (19 MiB, t=2, p=1): {:?} per attempt",
            started.elapsed() / 5
        );
    }

    // --- the boot clock: deep sleep and a wall clock set back -------------------------

    thread_local! {
        static BOOT: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
    }

    fn fake_boot() -> Option<Duration> {
        BOOT.with(std::cell::Cell::get)
    }

    fn boot_at(seconds: u64) {
        BOOT.with(|b| b.set(Some(Duration::from_secs(seconds))));
    }

    #[test]
    fn a_wall_clock_set_back_by_less_than_the_time_away_no_longer_stretches_the_grace() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.set_auto_lock(300).unwrap();
        service.set_boot_clock(fake_boot);
        let wall = SystemTime::now();
        service.mark_unlocked();

        // The phone sleeps for an hour (the monotonic clock barely moves in deep sleep),
        // and whoever holds it sets the wall clock back by that hour. By the wall and
        // monotonic clocks nothing happened; the boot clock knows better.
        boot_at(1_000);
        service.on_suspended(now, wall);
        boot_at(1_000 + 3_600);
        service.on_resumed(now + Duration::from_secs(2), wall);
        assert!(
            service.is_locked(),
            "an hour away must lock a five-minute grace"
        );
    }

    #[test]
    fn the_boot_clock_does_not_lock_a_short_absence_nor_change_anything_when_unreadable() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.set_auto_lock(300).unwrap();
        service.set_boot_clock(fake_boot);
        let wall = SystemTime::now();

        service.mark_unlocked();
        boot_at(500);
        service.on_suspended(now, wall);
        boot_at(520); // twenty seconds away
        service.on_resumed(
            now + Duration::from_secs(20),
            wall + Duration::from_secs(20),
        );
        assert!(!service.is_locked());

        // Unreadable (no /proc/uptime): the other two clocks decide, as before.
        BOOT.with(|b| b.set(None));
        service.on_suspended(now, wall);
        service.on_resumed(
            now + Duration::from_secs(20),
            wall + Duration::from_secs(20),
        );
        assert!(!service.is_locked());
        service.on_suspended(now, wall);
        service.on_resumed(
            now + Duration::from_secs(400),
            wall + Duration::from_secs(400),
        );
        assert!(service.is_locked());
    }

    #[test]
    fn a_boot_clock_that_went_backwards_means_a_reboot_and_locks() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.set_auto_lock(300).unwrap();
        service.set_boot_clock(fake_boot);
        service.mark_unlocked();
        boot_at(5_000);
        service.on_suspended(now, SystemTime::now());
        boot_at(30);
        service.on_resumed(now + Duration::from_secs(1), SystemTime::now());
        assert!(service.is_locked());
    }

    #[test]
    fn the_picker_window_is_also_bounded_by_the_boot_clock() {
        let dir = Dir::new();
        let (service, now) = fresh(&dir);
        service.set_pin(&pin("493017")).unwrap();
        service.set_boot_clock(fake_boot);
        service.mark_unlocked();
        let wall = SystemTime::now();
        boot_at(100);
        service.begin_external_ui(now);
        service.on_suspended(now, wall);
        boot_at(100 + 3_600);
        service.on_resumed(now + Duration::from_secs(2), wall); // the clocks the user can set say 2 s
        assert!(service.is_locked(), "an hour away is not a picker visit");
    }
}
