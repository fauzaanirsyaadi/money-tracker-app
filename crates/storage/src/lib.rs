//! Encrypted SQLite storage for Money Tracker.
//!
//! Knows nothing about Tauri or the UI. Depends on `mt-core` only.

mod accounts;
mod backup;
mod budgets;
mod categories;
mod db;
mod error;
mod recurring;
mod reports;
mod transactions;

pub use accounts::Account;
pub use backup::{LogicalSummary, TABLE_ORDER};
pub use budgets::BudgetRecord;
pub use categories::Category;
pub use db::{Db, DbKey};
pub use error::StorageError;
pub use recurring::{CatchUpReport, RecurringRecord};
pub use reports::{
    CategorySpending, ChildSpending, MAX_TREND_MONTHS, MonthSummary, NetWorth, SpendingReport,
};
pub use transactions::{
    Cursor, DEFAULT_PAGE, KindFilter, MAX_PAGE, Page, RecordKind, TransactionFilter,
    TransactionRecord,
};
