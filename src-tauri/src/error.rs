//! The error type every command returns to the UI.
//!
//! `code` is a stable machine-readable string the UI switches on; `message` is
//! a fixed English sentence. Neither ever contains amounts, names or notes.

use mt_core::{AccountError, BudgetError, MoneyError, RuleError, TransactionError};
use mt_storage::StorageError;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Storage could not be opened (or is locked).
    Unavailable,
    InvalidInput,
    NotFound,
    NameInUse,
    /// The account or category is archived.
    Archived,
    CurrencyMismatch,
    AmountOutOfRange,
    /// The app is locked; unlock before using data commands.
    Locked,
    WrongPin,
    /// Too many wrong PINs in a row; try again later.
    LockedOut,
    /// The user dismissed a prompt or picker.
    Cancelled,
    /// The backup passphrase is too weak to use.
    WeakPassphrase,
    /// Wrong passphrase for this backup (or it was altered).
    WrongPassphrase,
    /// The chosen file is not a Money Tracker backup.
    NotABackup,
    /// The backup comes from a newer version of the app.
    UnsupportedBackup,
    /// The backup is incomplete or inconsistent.
    BackupDamaged,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: &'static str,
}

impl AppError {
    pub const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    pub const fn unavailable() -> Self {
        Self::new(ErrorCode::Unavailable, "storage is not available")
    }

    pub const fn locked() -> Self {
        Self::new(ErrorCode::Locked, "the app is locked")
    }

    pub const fn invalid(message: &'static str) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }
}

impl From<StorageError> for AppError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::NotFound => Self::new(ErrorCode::NotFound, "not found"),
            StorageError::Duplicate => Self::new(ErrorCode::NameInUse, "name is already in use"),
            StorageError::CurrencyMismatch => {
                Self::new(ErrorCode::CurrencyMismatch, "currency does not match")
            }
            StorageError::Archived => {
                Self::new(ErrorCode::Archived, "account or category is archived")
            }
            StorageError::KindMismatch => Self::invalid("category does not match the kind"),
            StorageError::TransferLeg => {
                Self::invalid("a transfer leg cannot be edited; delete and recreate the transfer")
            }
            StorageError::Constraint => Self::invalid("value rejected by a rule"),
            StorageError::InvalidData => {
                Self::new(ErrorCode::AmountOutOfRange, "stored value is out of range")
            }
            _ => Self::new(ErrorCode::Internal, "internal error"),
        }
    }
}

impl From<AccountError> for AppError {
    fn from(err: AccountError) -> Self {
        Self::invalid(match err {
            AccountError::EmptyName => "account name is empty",
            AccountError::NameTooLong => "account name is too long",
            AccountError::InvalidCharacters => "account name has invalid characters",
        })
    }
}

impl From<MoneyError> for AppError {
    fn from(err: MoneyError) -> Self {
        match err {
            MoneyError::Overflow => {
                Self::new(ErrorCode::AmountOutOfRange, "amount is out of range")
            }
            _ => Self::invalid("invalid amount"),
        }
    }
}

impl From<TransactionError> for AppError {
    fn from(err: TransactionError) -> Self {
        Self::invalid(match err {
            TransactionError::NonPositiveAmount => "amount must be above zero",
            TransactionError::NoteTooLong => "note is too long",
            TransactionError::InvalidCharacters => "text has invalid characters",
            TransactionError::EmptyName => "name is empty",
            TransactionError::NameTooLong => "name is too long",
            TransactionError::SameAccount => "transfer needs two different accounts",
        })
    }
}

impl From<BudgetError> for AppError {
    fn from(err: BudgetError) -> Self {
        Self::invalid(match err {
            BudgetError::InvalidMonth => "month must be YYYY-MM",
            BudgetError::NonPositiveLimit => "budget limit must be above zero",
        })
    }
}

impl From<RuleError> for AppError {
    fn from(err: RuleError) -> Self {
        Self::invalid(match err {
            RuleError::NonPositiveAmount => "amount must be above zero",
            RuleError::EndBeforeStart => "end date is before the start date",
            RuleError::InvalidNote => "invalid note",
            RuleError::Schedule(_) => "invalid schedule",
        })
    }
}
