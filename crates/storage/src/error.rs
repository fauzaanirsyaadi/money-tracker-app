//! Storage errors. Variants never carry SQL text, row values or amounts.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// The key is wrong or the file is not an encrypted database.
    WrongKeyOrCorrupt,
    /// The database was written by a newer app version.
    SchemaTooNew { found: u32, supported: u32 },
    /// A constraint (CHECK, UNIQUE, FOREIGN KEY, NOT NULL) rejected a write.
    Constraint,
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
            Self::Constraint => f.write_str("constraint violated"),
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
                rusqlite::ErrorCode::ConstraintViolation => Self::Constraint,
                _ => Self::Sqlite {
                    code: e.extended_code,
                },
            },
            _ => Self::Sqlite { code: -1 },
        }
    }
}
