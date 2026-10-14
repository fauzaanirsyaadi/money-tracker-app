//! Encrypted SQLite storage for Money Tracker.
//!
//! Knows nothing about Tauri or the UI. Depends on `mt-core` only.

mod db;
mod error;

pub use db::{Db, DbKey};
pub use error::StorageError;
