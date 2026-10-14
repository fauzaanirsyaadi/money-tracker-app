//! Encrypted SQLite storage for Money Tracker.
//!
//! Knows nothing about Tauri or the UI. Depends on `mt-core` only.

mod accounts;
mod db;
mod error;
mod recurring;

pub use accounts::Account;
pub use db::{Db, DbKey};
pub use error::StorageError;
pub use recurring::CatchUpReport;
