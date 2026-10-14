//! Error type for the domain crate.

use std::fmt;

/// Errors raised by `Money` and `Currency` operations.
///
/// Variants carry no amounts, so an error is always safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoneyError {
    /// Result falls outside `-MAX_MINOR..=MAX_MINOR` (includes `i64` overflow).
    Overflow,
    /// Two amounts in different currencies were combined.
    CurrencyMismatch,
    /// Currency code or exponent is not valid.
    InvalidCurrency,
    /// Text is not a valid decimal amount for the currency.
    InvalidAmount,
    /// Allocation ratios are empty or all zero.
    InvalidRatios,
    /// Division by zero.
    DivisionByZero,
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::Overflow => "amount out of range",
            Self::CurrencyMismatch => "currency mismatch",
            Self::InvalidCurrency => "invalid currency",
            Self::InvalidAmount => "invalid amount",
            Self::InvalidRatios => "invalid allocation ratios",
            Self::DivisionByZero => "division by zero",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for MoneyError {}
