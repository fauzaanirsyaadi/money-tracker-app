//! Account commands.
//!
//! Money crosses the IPC boundary as a decimal string of minor units
//! (`"1250000"`, `"-500"`), never as a JSON number. The UI formats it using the
//! `exponent` that comes with each account.

use std::time::{SystemTime, UNIX_EPOCH};

use mt_core::{AccountDraft, AccountKind, Currency, Money};
use mt_storage::{Account, Db};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{error::AppError, state::AppState};

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    pub id: i64,
    pub name: String,
    pub kind: &'static str,
    pub currency: String,
    pub exponent: u8,
    pub opening_balance_minor: String,
    pub balance_minor: String,
    pub archived: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInput {
    pub name: String,
    pub kind: String,
    pub opening_balance_minor: String,
}

impl From<&Account> for AccountDto {
    fn from(account: &Account) -> Self {
        let currency = account.opening_balance.currency();
        Self {
            id: account.id,
            name: account.name.clone(),
            kind: account.kind.code(),
            currency: currency.code().to_owned(),
            exponent: currency.exponent(),
            opening_balance_minor: account.opening_balance.minor().to_string(),
            balance_minor: account.balance.minor().to_string(),
            archived: account.archived,
        }
    }
}

/// Parses `-?[0-9]{1,16}`; the range itself is checked by `Money`.
pub(crate) fn parse_minor(text: &str) -> Result<i64, AppError> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || digits.len() > 16 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AppError::invalid(
            "amount must be a whole number of minor units",
        ));
    }
    text.parse()
        .map_err(|_| AppError::invalid("amount must be a whole number of minor units"))
}

fn draft_from(db: &Db, input: &AccountInput) -> Result<AccountDraft, AppError> {
    let kind = AccountKind::from_code(&input.kind)
        .ok_or_else(|| AppError::invalid("unknown account kind"))?;
    let currency: Currency = db.app_currency()?;
    let opening = Money::from_minor(parse_minor(&input.opening_balance_minor)?, currency)?;
    Ok(AccountDraft::new(&input.name, kind, opening)?)
}

fn fetch(db: &Db, id: i64) -> Result<AccountDto, AppError> {
    let account = db.account(id)?.ok_or(mt_storage::StorageError::NotFound)?;
    Ok(AccountDto::from(&account))
}

pub(crate) fn list(db: &Db, include_archived: bool) -> Result<Vec<AccountDto>, AppError> {
    Ok(db
        .accounts(include_archived)?
        .iter()
        .map(AccountDto::from)
        .collect())
}

pub(crate) fn create(
    db: &mut Db,
    input: &AccountInput,
    now_unix: i64,
) -> Result<AccountDto, AppError> {
    let draft = draft_from(db, input)?;
    let id = db.create_account(&draft, now_unix)?;
    fetch(db, id)
}

pub(crate) fn update(db: &mut Db, id: i64, input: &AccountInput) -> Result<AccountDto, AppError> {
    let draft = draft_from(db, input)?;
    db.update_account(id, &draft)?;
    fetch(db, id)
}

pub(crate) fn archive(db: &mut Db, id: i64, archived: bool) -> Result<AccountDto, AppError> {
    db.set_account_archived(id, archived)?;
    fetch(db, id)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[tauri::command]
pub async fn accounts_list(
    state: State<'_, AppState>,
    include_archived: bool,
) -> Result<Vec<AccountDto>, AppError> {
    state.with_db(|db| list(db, include_archived))
}

#[tauri::command]
pub async fn account_create(
    state: State<'_, AppState>,
    input: AccountInput,
) -> Result<AccountDto, AppError> {
    state.with_db(|db| create(db, &input, now_unix()))
}

#[tauri::command]
pub async fn account_update(
    state: State<'_, AppState>,
    id: i64,
    input: AccountInput,
) -> Result<AccountDto, AppError> {
    state.with_db(|db| update(db, id, &input))
}

#[tauri::command]
pub async fn account_archive(
    state: State<'_, AppState>,
    id: i64,
    archived: bool,
) -> Result<AccountDto, AppError> {
    state.with_db(|db| archive(db, id, archived))
}

// Amounts, names and notes must never reach a log through `{:?}`.
mt_core::redacted_debug!(AccountDto, AccountInput);

#[cfg(test)]
mod tests {
    use mt_storage::DbKey;

    use super::*;
    use crate::error::ErrorCode;

    fn db() -> Db {
        Db::open_in_memory(&DbKey::new([2u8; 32])).unwrap()
    }

    fn input(name: &str, kind: &str, opening: &str) -> AccountInput {
        AccountInput {
            name: name.to_owned(),
            kind: kind.to_owned(),
            opening_balance_minor: opening.to_owned(),
        }
    }

    #[test]
    fn minor_amounts_are_strict_strings() {
        assert_eq!(parse_minor("0"), Ok(0));
        assert_eq!(parse_minor("-500"), Ok(-500));
        assert_eq!(parse_minor("9007199254740991"), Ok(9_007_199_254_740_991));
        for bad in [
            "",
            "-",
            "1.5",
            "1e3",
            " 1",
            "+1",
            "0x10",
            "١٢",
            "12345678901234567",
            "--1",
        ] {
            assert_eq!(
                parse_minor(bad).unwrap_err().code,
                ErrorCode::InvalidInput,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn create_returns_a_dto_with_string_money() {
        let mut db = db();
        let dto = create(&mut db, &input(" Dompet ", "cash", "150000"), 7).unwrap();
        assert_eq!(dto.name, "Dompet");
        assert_eq!(dto.kind, "cash");
        assert_eq!((dto.currency.as_str(), dto.exponent), ("IDR", 0));
        assert_eq!(dto.opening_balance_minor, "150000");
        assert_eq!(dto.balance_minor, "150000");
        assert!(!dto.archived);
    }

    #[test]
    fn dto_serialises_in_camel_case() {
        let mut db = db();
        let dto = create(&mut db, &input("A", "bank", "-1"), 0).unwrap();
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["openingBalanceMinor"], "-1");
        assert_eq!(json["balanceMinor"], "-1");
        assert!(json.get("opening_balance_minor").is_none());
    }

    #[test]
    fn invalid_input_is_rejected_with_stable_codes() {
        let mut db = db();
        let code = |r: Result<AccountDto, AppError>| r.unwrap_err().code;
        assert_eq!(
            code(create(&mut db, &input("", "cash", "0"), 0)),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(&mut db, &input("A", "savings", "0"), 0)),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(&mut db, &input("A", "cash", "1.5"), 0)),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(&mut db, &input("A", "cash", "9007199254740992"), 0)),
            ErrorCode::AmountOutOfRange
        );
    }

    #[test]
    fn duplicate_names_report_name_in_use() {
        let mut db = db();
        create(&mut db, &input("BCA", "bank", "0"), 0).unwrap();
        let err = create(&mut db, &input("bca", "bank", "0"), 0).unwrap_err();
        assert_eq!(err.code, ErrorCode::NameInUse);
    }

    #[test]
    fn update_and_archive_round_trip() {
        let mut db = db();
        let id = create(&mut db, &input("Lama", "cash", "100"), 0)
            .unwrap()
            .id;
        let updated = update(&mut db, id, &input("Baru", "ewallet", "250")).unwrap();
        assert_eq!(
            (
                updated.name.as_str(),
                updated.kind,
                updated.balance_minor.as_str()
            ),
            ("Baru", "ewallet", "250")
        );

        assert!(archive(&mut db, id, true).unwrap().archived);
        assert!(list(&db, false).unwrap().is_empty());
        assert_eq!(list(&db, true).unwrap().len(), 1);
        assert!(!archive(&mut db, id, false).unwrap().archived);
    }

    #[test]
    fn unknown_ids_report_not_found() {
        let mut db = db();
        assert_eq!(
            archive(&mut db, 5, true).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            update(&mut db, 5, &input("A", "cash", "0"))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn error_payloads_carry_no_user_data() {
        let mut db = db();
        let secret = "Rekening-Rahasia-12345";
        create(&mut db, &input(secret, "bank", "0"), 0).unwrap();
        let err = create(&mut db, &input(secret, "bank", "0"), 0).unwrap_err();
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains(secret), "{json}");
    }

    #[test]
    fn unavailable_storage_is_reported_not_panicked() {
        let state = AppState::new(
            None,
            crate::applock::LockService::load(
                std::path::Path::new("/nonexistent/mt-test"),
                std::time::Instant::now(),
            ),
        );
        let result = state.with_db(|db| list(db, false));
        assert_eq!(result.unwrap_err().code, ErrorCode::Unavailable);
    }
}
