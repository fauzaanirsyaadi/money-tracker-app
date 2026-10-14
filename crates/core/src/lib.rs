//! Domain logic for Money Tracker.
//!
//! This crate performs no I/O and depends on neither storage nor UI.

mod currency;
mod error;
mod money;
mod recurrence;

pub use currency::Currency;
pub use error::MoneyError;
pub use money::{MAX_MINOR, Money};
pub use recurrence::{
    CatchUp, Frequency, Schedule, ScheduleError, format_iso_date, parse_iso_date,
};
// Re-exported so callers do not need their own `time` dependency.
pub use time::Date;
