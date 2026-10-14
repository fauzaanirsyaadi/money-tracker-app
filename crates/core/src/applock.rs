//! Rules for the app lock: PIN format, the delay after wrong guesses and the
//! auto-lock choices. Pure functions; hashing and storage live in the app.

use std::fmt;
use std::time::Duration;

/// Shortest and longest accepted PIN, in digits.
pub const PIN_MIN_DIGITS: usize = 6;
pub const PIN_MAX_DIGITS: usize = 16;

/// Wrong guesses allowed in a row before delays begin.
pub const FREE_ATTEMPTS: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinProblem {
    /// Fewer than [`PIN_MIN_DIGITS`] or more than [`PIN_MAX_DIGITS`] characters.
    Length,
    /// Something other than the digits 0-9.
    NotDigits,
    /// Every digit equal (`000000`) or a straight run (`123456`, `654321`).
    TooSimple,
}

impl fmt::Display for PinProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Length => "PIN must be 6 to 16 digits",
            Self::NotDigits => "PIN may only contain digits",
            Self::TooSimple => "PIN is too easy to guess",
        })
    }
}

impl std::error::Error for PinProblem {}

/// Checks a PIN chosen by the user. Never echoes the PIN.
pub fn validate_new_pin(pin: &str) -> Result<(), PinProblem> {
    if !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PinProblem::NotDigits);
    }
    if !(PIN_MIN_DIGITS..=PIN_MAX_DIGITS).contains(&pin.len()) {
        return Err(PinProblem::Length);
    }
    let digits: Vec<i16> = pin.bytes().map(|b| i16::from(b - b'0')).collect();
    let step = digits[1] - digits[0];
    let is_run = (-1..=1).contains(&step) && digits.windows(2).all(|w| w[1] - w[0] == step);
    if is_run {
        return Err(PinProblem::TooSimple);
    }
    Ok(())
}

/// Checks an entry that is only compared with the stored PIN. Any string that
/// could not have been set is rejected cheaply, before any hashing.
pub fn is_plausible_pin(pin: &str) -> bool {
    (PIN_MIN_DIGITS..=PIN_MAX_DIGITS).contains(&pin.len())
        && pin.bytes().all(|b| b.is_ascii_digit())
}

/// How long to wait before the next guess after `failed` wrong guesses in a row.
///
/// The count survives restarts, so closing the app does not reset it.
pub fn lockout_delay(failed: u32) -> Duration {
    let seconds = match failed {
        0..=FREE_ATTEMPTS => 0,
        5 => 30,
        6 => 60,
        7 => 5 * 60,
        8 => 15 * 60,
        _ => 60 * 60,
    };
    Duration::from_secs(seconds)
}

/// How long the app may stay in the background before it locks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoLock {
    Immediately,
    After30Seconds,
    After1Minute,
    After5Minutes,
}

impl AutoLock {
    pub const DEFAULT: Self = Self::Immediately;

    pub fn from_seconds(seconds: u32) -> Option<Self> {
        match seconds {
            0 => Some(Self::Immediately),
            30 => Some(Self::After30Seconds),
            60 => Some(Self::After1Minute),
            300 => Some(Self::After5Minutes),
            _ => None,
        }
    }

    pub fn seconds(self) -> u32 {
        match self {
            Self::Immediately => 0,
            Self::After30Seconds => 30,
            Self::After1Minute => 60,
            Self::After5Minutes => 300,
        }
    }

    pub fn grace(self) -> Duration {
        Duration::from_secs(u64::from(self.seconds()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_pins() {
        for pin in ["493017", "7391028456", "100200", "1234560"] {
            assert_eq!(validate_new_pin(pin), Ok(()), "{pin}");
        }
    }

    #[test]
    fn rejects_wrong_length_and_non_digits() {
        assert_eq!(validate_new_pin("49301"), Err(PinProblem::Length));
        assert_eq!(validate_new_pin(&"4".repeat(17)), Err(PinProblem::Length));
        assert_eq!(validate_new_pin(""), Err(PinProblem::Length));
        assert_eq!(validate_new_pin("49a017"), Err(PinProblem::NotDigits));
        assert_eq!(validate_new_pin("49 017"), Err(PinProblem::NotDigits));
        // Arabic-Indic digits are not ASCII digits.
        assert_eq!(validate_new_pin("٤٩٣٠١٧"), Err(PinProblem::NotDigits));
    }

    #[test]
    fn rejects_repeats_and_straight_runs() {
        for pin in [
            "000000",
            "111111",
            "123456",
            "654321",
            "234567",
            "9876543210",
        ] {
            assert_eq!(validate_new_pin(pin), Err(PinProblem::TooSimple), "{pin}");
        }
        // A run that breaks is fine.
        assert_eq!(validate_new_pin("123457"), Ok(()));
    }

    #[test]
    fn plausible_entries_skip_the_simplicity_rule() {
        assert!(is_plausible_pin("123456"));
        assert!(!is_plausible_pin("12345"));
        assert!(!is_plausible_pin("12345a"));
        assert!(!is_plausible_pin(&"1".repeat(40)));
    }

    #[test]
    fn delays_start_after_four_misses_and_only_grow() {
        let delays: Vec<u64> = (0..=12).map(|n| lockout_delay(n).as_secs()).collect();
        assert_eq!(
            delays,
            [0, 0, 0, 0, 0, 30, 60, 300, 900, 3600, 3600, 3600, 3600]
        );
        assert!(delays.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(lockout_delay(u32::MAX).as_secs(), 3600);
    }

    #[test]
    fn auto_lock_round_trips_and_rejects_other_values() {
        for choice in [
            AutoLock::Immediately,
            AutoLock::After30Seconds,
            AutoLock::After1Minute,
            AutoLock::After5Minutes,
        ] {
            assert_eq!(AutoLock::from_seconds(choice.seconds()), Some(choice));
        }
        assert_eq!(AutoLock::from_seconds(45), None);
        assert_eq!(AutoLock::DEFAULT.grace().as_secs(), 0);
    }
}
