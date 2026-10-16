//! Account rules: the kinds an account can have and what makes a valid draft.
//!
//! An account's balance is never stored; it is the opening balance plus the sum
//! of its transactions, computed in SQL by the storage layer.

use std::fmt;

use crate::money::Money;

pub const MAX_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    Cash,
    Bank,
    Ewallet,
}

impl AccountKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Cash => "cash",
            Self::Bank => "bank",
            Self::Ewallet => "ewallet",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "cash" => Some(Self::Cash),
            "bank" => Some(Self::Bank),
            "ewallet" => Some(Self::Ewallet),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountError {
    EmptyName,
    NameTooLong,
    /// Control characters (newlines, tabs, NUL, ...) are not allowed in names.
    InvalidCharacters,
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyName => "account name is empty",
            Self::NameTooLong => "account name is too long",
            Self::InvalidCharacters => "account name has invalid characters",
        })
    }
}

impl std::error::Error for AccountError {}

/// What can be wrong with a user-supplied piece of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextProblem {
    Empty,
    TooLong,
    /// Control characters (newlines, tabs, NUL, ...) are not allowed.
    ControlCharacter,
}

/// Trims, then checks emptiness, control characters and length (in characters,
/// matching SQLite's `length()`).
pub(crate) fn clean_text(raw: &str, max_chars: usize) -> Result<String, TextProblem> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(TextProblem::Empty);
    }
    if text.chars().any(char::is_control) {
        return Err(TextProblem::ControlCharacter);
    }
    if text.chars().count() > max_chars {
        return Err(TextProblem::TooLong);
    }
    Ok(text.to_owned())
}

/// Trims surrounding whitespace and validates an account name.
pub fn normalize_account_name(raw: &str) -> Result<String, AccountError> {
    clean_text(raw, MAX_NAME_CHARS).map_err(|problem| match problem {
        TextProblem::Empty => AccountError::EmptyName,
        TextProblem::TooLong => AccountError::NameTooLong,
        TextProblem::ControlCharacter => AccountError::InvalidCharacters,
    })
}

/// A validated new account. Its currency is the opening balance's currency.
#[derive(Clone, PartialEq, Eq)]
pub struct AccountDraft {
    name: String,
    kind: AccountKind,
    opening_balance: Money,
}

impl AccountDraft {
    pub fn new(
        name: &str,
        kind: AccountKind,
        opening_balance: Money,
    ) -> Result<Self, AccountError> {
        Ok(Self {
            name: normalize_account_name(name)?,
            kind,
            opening_balance,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> AccountKind {
        self.kind
    }

    pub fn opening_balance(&self) -> Money {
        self.opening_balance
    }
}

// Amounts, names and notes must never reach a log through `{:?}`.
crate::redacted_debug!(AccountDraft);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Currency;

    #[test]
    fn names_are_trimmed_and_validated() {
        assert_eq!(normalize_account_name("  BCA  "), Ok("BCA".to_owned()));
        assert_eq!(normalize_account_name("   "), Err(AccountError::EmptyName));
        assert_eq!(normalize_account_name(""), Err(AccountError::EmptyName));
        assert_eq!(
            normalize_account_name("a\nb"),
            Err(AccountError::InvalidCharacters)
        );
        assert_eq!(
            normalize_account_name("a\u{0}b"),
            Err(AccountError::InvalidCharacters)
        );
        assert_eq!(
            normalize_account_name("tab\there"),
            Err(AccountError::InvalidCharacters)
        );
    }

    #[test]
    fn length_is_counted_in_characters() {
        let eighty = "é".repeat(80);
        assert_eq!(normalize_account_name(&eighty), Ok(eighty.clone()));
        let too_long = "é".repeat(81);
        assert_eq!(
            normalize_account_name(&too_long),
            Err(AccountError::NameTooLong)
        );
        // 80 two-byte characters are 160 bytes: bytes must not be the measure.
        assert!(eighty.len() > MAX_NAME_CHARS);
    }

    #[test]
    fn kind_codes_round_trip() {
        for kind in [AccountKind::Cash, AccountKind::Bank, AccountKind::Ewallet] {
            assert_eq!(AccountKind::from_code(kind.code()), Some(kind));
        }
        assert_eq!(AccountKind::from_code("savings"), None);
        assert_eq!(AccountKind::from_code(""), None);
    }

    #[test]
    fn a_draft_carries_the_normalised_name_and_currency() {
        let opening = Money::from_minor(-250_000, Currency::IDR).unwrap();
        let draft = AccountDraft::new(" Kartu kredit ", AccountKind::Bank, opening).unwrap();
        assert_eq!(draft.name(), "Kartu kredit");
        assert_eq!(draft.opening_balance().currency(), Currency::IDR);
        assert!(draft.opening_balance().is_negative());
        assert_eq!(
            AccountDraft::new("", AccountKind::Cash, opening),
            Err(AccountError::EmptyName)
        );
    }
}
