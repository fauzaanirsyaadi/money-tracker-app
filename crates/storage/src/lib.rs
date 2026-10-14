//! Encrypted SQLite storage for Money Tracker.
//!
//! Knows nothing about Tauri or the UI. Depends on `mt-core` only.

mod accounts;
mod categories;
mod db;
mod error;
mod recurring;
mod transactions;

pub use accounts::Account;
pub use categories::Category;
pub use db::{Db, DbKey};
pub use error::StorageError;
pub use recurring::CatchUpReport;
pub use transactions::{
    Cursor, DEFAULT_PAGE, KindFilter, MAX_PAGE, Page, RecordKind, TransactionFilter,
    TransactionRecord,
};
