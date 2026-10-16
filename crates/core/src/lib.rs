//! Domain logic for Money Tracker.
//!
//! This crate performs no I/O and depends on neither storage nor UI.

/// Implements `Debug` for types that hold amounts, names or notes, printing the
/// type name only. Use it instead of `#[derive(Debug)]` on such types; the
/// repository guard `scripts/check_debug_redaction.py` enforces that.
#[macro_export]
macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ::core::fmt::Debug for $ty {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    f.write_str(concat!(stringify!($ty), " { .. }"))
                }
            }
        )+
    };
}

mod account;
mod applock;
mod budget;
mod currency;
mod error;
mod money;
mod recurrence;
mod transaction;

pub use account::{
    AccountDraft, AccountError, AccountKind, MAX_NAME_CHARS, normalize_account_name,
};
pub use applock::{
    AutoLock, FREE_ATTEMPTS, PIN_MAX_DIGITS, PIN_MIN_DIGITS, PinProblem, is_plausible_pin,
    lockout_delay, validate_new_pin,
};
pub use budget::{
    BudgetDraft, BudgetError, BudgetProgress, BudgetStatus, WARN_AT_BP, YearMonth, progress,
};
pub use currency::Currency;
pub use error::MoneyError;
pub use money::{MAX_MINOR, Money};
pub use recurrence::{
    CatchUp, Frequency, RecurringDraft, RecurringEdit, RuleError, Schedule, ScheduleError,
    format_iso_date, parse_iso_date,
};
pub use transaction::{
    CategoryDraft, FlowKind, MAX_CATEGORY_NAME_CHARS, MAX_NOTE_CHARS, TransactionDraft,
    TransactionError, TransferDraft, normalize_category_name, normalize_note,
};
// Re-exported so callers do not need their own `time` dependency.
pub use time::Date;
