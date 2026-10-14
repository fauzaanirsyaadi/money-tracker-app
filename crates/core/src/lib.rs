//! Domain logic for Money Tracker.
//!
//! This crate performs no I/O and depends on neither storage nor UI.

mod currency;
mod error;
mod money;

pub use currency::Currency;
pub use error::MoneyError;
pub use money::{MAX_MINOR, Money};
