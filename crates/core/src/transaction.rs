//! Transaction and category rules.
//!
//! The UI and the commands speak in *magnitudes* ("spent 25.000"); the sign is
//! derived from the kind here, once, so no other layer can get it backwards.

use std::fmt;

use time::Date;

use crate::{
    account::{TextProblem, clean_text},
    money::Money,
};

pub const MAX_NOTE_CHARS: usize = 500;
pub const MAX_CATEGORY_NAME_CHARS: usize = 80;

/// Direction of money for a category or an income/expense transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowKind {
    Income,
    Expense,
}

impl FlowKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Income => "income",
            Self::Expense => "expense",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "income" => Some(Self::Income),
            "expense" => Some(Self::Expense),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionError {
    /// Amounts are entered as positive magnitudes and must be above zero.
    NonPositiveAmount,
    NoteTooLong,
    /// Notes and names may not contain control characters.
    InvalidCharacters,
    EmptyName,
    NameTooLong,
    /// A transfer needs two different accounts.
    SameAccount,
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NonPositiveAmount => "amount must be above zero",
            Self::NoteTooLong => "note is too long",
            Self::InvalidCharacters => "text has invalid characters",
            Self::EmptyName => "name is empty",
            Self::NameTooLong => "name is too long",
            Self::SameAccount => "transfer needs two different accounts",
        })
    }
}

impl std::error::Error for TransactionError {}

/// Trims a note; an empty note is fine.
pub fn normalize_note(raw: &str) -> Result<String, TransactionError> {
    match clean_text(raw, MAX_NOTE_CHARS) {
        Ok(note) => Ok(note),
        Err(TextProblem::Empty) => Ok(String::new()),
        Err(TextProblem::TooLong) => Err(TransactionError::NoteTooLong),
        Err(TextProblem::ControlCharacter) => Err(TransactionError::InvalidCharacters),
    }
}

pub fn normalize_category_name(raw: &str) -> Result<String, TransactionError> {
    clean_text(raw, MAX_CATEGORY_NAME_CHARS).map_err(|problem| match problem {
        TextProblem::Empty => TransactionError::EmptyName,
        TextProblem::TooLong => TransactionError::NameTooLong,
        TextProblem::ControlCharacter => TransactionError::InvalidCharacters,
    })
}

/// A validated income or expense.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionDraft {
    account_id: i64,
    category_id: i64,
    kind: FlowKind,
    amount: Money,
    note: String,
    occurred_on: Date,
}

impl TransactionDraft {
    /// `amount` is the positive magnitude.
    pub fn new(
        account_id: i64,
        category_id: i64,
        kind: FlowKind,
        amount: Money,
        note: &str,
        occurred_on: Date,
    ) -> Result<Self, TransactionError> {
        if !amount.is_positive() {
            return Err(TransactionError::NonPositiveAmount);
        }
        Ok(Self {
            account_id,
            category_id,
            kind,
            amount,
            note: normalize_note(note)?,
            occurred_on,
        })
    }

    pub fn account_id(&self) -> i64 {
        self.account_id
    }

    pub fn category_id(&self) -> i64 {
        self.category_id
    }

    pub fn kind(&self) -> FlowKind {
        self.kind
    }

    pub fn amount(&self) -> Money {
        self.amount
    }

    pub fn note(&self) -> &str {
        &self.note
    }

    pub fn occurred_on(&self) -> Date {
        self.occurred_on
    }

    /// What is stored: positive for income, negative for expense.
    pub fn signed_amount(&self) -> Money {
        match self.kind {
            FlowKind::Income => self.amount,
            FlowKind::Expense => self.amount.negated(),
        }
    }
}

/// A validated transfer between two different accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDraft {
    from_account_id: i64,
    to_account_id: i64,
    amount: Money,
    note: String,
    occurred_on: Date,
}

impl TransferDraft {
    pub fn new(
        from_account_id: i64,
        to_account_id: i64,
        amount: Money,
        note: &str,
        occurred_on: Date,
    ) -> Result<Self, TransactionError> {
        if from_account_id == to_account_id {
            return Err(TransactionError::SameAccount);
        }
        if !amount.is_positive() {
            return Err(TransactionError::NonPositiveAmount);
        }
        Ok(Self {
            from_account_id,
            to_account_id,
            amount,
            note: normalize_note(note)?,
            occurred_on,
        })
    }

    pub fn from_account_id(&self) -> i64 {
        self.from_account_id
    }

    pub fn to_account_id(&self) -> i64 {
        self.to_account_id
    }

    pub fn amount(&self) -> Money {
        self.amount
    }

    pub fn note(&self) -> &str {
        &self.note
    }

    pub fn occurred_on(&self) -> Date {
        self.occurred_on
    }
}

/// A validated new category. `parent_id` makes it a sub-category (one level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryDraft {
    name: String,
    kind: FlowKind,
    parent_id: Option<i64>,
}

impl CategoryDraft {
    pub fn new(
        name: &str,
        kind: FlowKind,
        parent_id: Option<i64>,
    ) -> Result<Self, TransactionError> {
        Ok(Self {
            name: normalize_category_name(name)?,
            kind,
            parent_id,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> FlowKind {
        self.kind
    }

    pub fn parent_id(&self) -> Option<i64> {
        self.parent_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Currency, parse_iso_date};

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::IDR).unwrap()
    }

    fn day() -> Date {
        parse_iso_date("2026-10-14").unwrap()
    }

    #[test]
    fn expenses_are_stored_negative_and_income_positive() {
        let spend = TransactionDraft::new(1, 2, FlowKind::Expense, idr(25_000), "", day()).unwrap();
        assert_eq!(spend.signed_amount(), idr(-25_000));
        assert_eq!(spend.amount(), idr(25_000));
        let earn = TransactionDraft::new(1, 3, FlowKind::Income, idr(25_000), "", day()).unwrap();
        assert_eq!(earn.signed_amount(), idr(25_000));
    }

    #[test]
    fn amounts_must_be_above_zero() {
        for minor in [0, -1, -25_000] {
            assert_eq!(
                TransactionDraft::new(1, 2, FlowKind::Expense, idr(minor), "", day()),
                Err(TransactionError::NonPositiveAmount)
            );
        }
    }

    #[test]
    fn notes_are_trimmed_and_bounded() {
        assert_eq!(
            normalize_note("  makan siang  "),
            Ok("makan siang".to_owned())
        );
        assert_eq!(normalize_note("   "), Ok(String::new()));
        assert_eq!(
            normalize_note("a\nb"),
            Err(TransactionError::InvalidCharacters)
        );
        let max = "é".repeat(MAX_NOTE_CHARS);
        assert_eq!(normalize_note(&max), Ok(max.clone()));
        assert_eq!(
            normalize_note(&"é".repeat(MAX_NOTE_CHARS + 1)),
            Err(TransactionError::NoteTooLong)
        );
    }

    #[test]
    fn a_transfer_needs_two_accounts_and_a_positive_amount() {
        assert!(TransferDraft::new(1, 2, idr(100), "", day()).is_ok());
        assert_eq!(
            TransferDraft::new(1, 1, idr(100), "", day()),
            Err(TransactionError::SameAccount)
        );
        assert_eq!(
            TransferDraft::new(1, 2, idr(0), "", day()),
            Err(TransactionError::NonPositiveAmount)
        );
        assert_eq!(
            TransferDraft::new(1, 2, idr(-5), "", day()),
            Err(TransactionError::NonPositiveAmount)
        );
    }

    #[test]
    fn category_names_follow_the_same_rules_as_account_names() {
        assert_eq!(
            normalize_category_name(" Makanan "),
            Ok("Makanan".to_owned())
        );
        assert_eq!(
            normalize_category_name(""),
            Err(TransactionError::EmptyName)
        );
        assert_eq!(
            normalize_category_name(&"x".repeat(81)),
            Err(TransactionError::NameTooLong)
        );
    }

    #[test]
    fn flow_kind_codes_round_trip() {
        for kind in [FlowKind::Income, FlowKind::Expense] {
            assert_eq!(FlowKind::from_code(kind.code()), Some(kind));
        }
        assert_eq!(FlowKind::from_code("transfer_out"), None);
    }
}
