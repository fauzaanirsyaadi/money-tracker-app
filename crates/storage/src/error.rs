//! Storage errors. Variants never carry SQL text, row values or amounts.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// The key is wrong or the file is not an encrypted database.
    WrongKeyOrCorrupt,
    /// The database was written by a newer app version.
    SchemaTooNew { found: u32, supported: u32 },
    /// The linked SQLite has no SQLCipher: refusing to create an unencrypted file.
    EncryptionUnavailable,
    /// A UNIQUE constraint rejected the write (for example a duplicate name).
    Duplicate,
    /// Any other constraint (CHECK, FOREIGN KEY, NOT NULL, trigger) rejected a write.
    Constraint,
    /// The row does not exist.
    NotFound,
    /// The account or category is archived and cannot take new activity.
    Archived,
    /// The category's kind (income/expense) does not match the transaction.
    KindMismatch,
    /// The operation is not allowed on a transfer leg.
    TransferLeg,
    /// A currency different from the app's (or the account's) was supplied.
    CurrencyMismatch,
    /// A stored value could not be read back (unknown code, out-of-range amount).
    InvalidData,
    /// Any other SQLite failure, identified by its extended result code only.
    Sqlite { code: i32 },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKeyOrCorrupt => f.write_str("wrong key or corrupt database"),
            Self::SchemaTooNew { found, supported } => {
                write!(
                    f,
                    "database schema {found} is newer than supported {supported}"
                )
            }
            Self::EncryptionUnavailable => f.write_str("sqlcipher is not available"),
            Self::Duplicate => f.write_str("duplicate value"),
            Self::Constraint => f.write_str("constraint violated"),
            Self::NotFound => f.write_str("not found"),
            Self::Archived => f.write_str("archived"),
            Self::KindMismatch => f.write_str("category kind mismatch"),
            Self::TransferLeg => f.write_str("not allowed on a transfer leg"),
            Self::CurrencyMismatch => f.write_str("currency mismatch"),
            Self::InvalidData => f.write_str("invalid stored data"),
            Self::Sqlite { code } => write!(f, "sqlite error {code}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(err: rusqlite::Error) -> Self {
        match err {
            rusqlite::Error::SqliteFailure(e, _) => match e.code {
                rusqlite::ErrorCode::NotADatabase => Self::WrongKeyOrCorrupt,
                // SQLITE_CONSTRAINT_UNIQUE (2067) and _PRIMARYKEY (1555).
                rusqlite::ErrorCode::ConstraintViolation
                    if matches!(e.extended_code, 2067 | 1555) =>
                {
                    Self::Duplicate
                }
                rusqlite::ErrorCode::ConstraintViolation => Self::Constraint,
                _ => Self::Sqlite {
                    code: e.extended_code,
                },
            },
            _ => Self::Sqlite { code: -1 },
        }
    }
}
