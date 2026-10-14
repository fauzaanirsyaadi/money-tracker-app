//! Domain logic for Money Tracker.
//!
//! This crate performs no I/O and depends on neither storage nor UI.

mod account;
mod currency;
mod error;
mod money;
mod recurrence;
mod transaction;

pub use account::{
    AccountDraft, AccountError, AccountKind, MAX_NAME_CHARS, normalize_account_name,
};
pub use currency::Currency;
pub use error::MoneyError;
pub use money::{MAX_MINOR, Money};
pub use recurrence::{
    CatchUp, Frequency, Schedule, ScheduleError, format_iso_date, parse_iso_date,
};
pub use transaction::{
    CategoryDraft, FlowKind, MAX_CATEGORY_NAME_CHARS, MAX_NOTE_CHARS, TransactionDraft,
    TransactionError, TransferDraft, normalize_category_name, normalize_note,
};
// Re-exported so callers do not need their own `time` dependency.
pub use time::Date;
