//! Budget commands: a monthly limit per expense category, with progress.

use mt_core::{BudgetDraft, BudgetStatus, Money, YearMonth};
use mt_storage::{BudgetRecord, Db};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{accounts::parse_minor, error::AppError, state::AppState};

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetDto {
    pub id: i64,
    pub category_id: i64,
    pub category_name: String,
    pub month: String,
    pub currency: String,
    pub exponent: u8,
    pub limit_minor: String,
    pub spent_minor: String,
    /// `limit - spent`; negative once exceeded.
    pub remaining_minor: String,
    /// Share of the limit used, in basis points (10 000 = 100 %).
    pub used_bp: i64,
    /// `ok`, `warning` (>= 80 %) or `over` (> 100 %).
    pub status: &'static str,
}

impl From<&BudgetRecord> for BudgetDto {
    fn from(b: &BudgetRecord) -> Self {
        let currency = b.progress.limit.currency();
        Self {
            id: b.id,
            category_id: b.category_id,
            category_name: b.category_name.clone(),
            month: b.month.to_string(),
            currency: currency.code().to_owned(),
            exponent: currency.exponent(),
            limit_minor: b.progress.limit.minor().to_string(),
            spent_minor: b.progress.spent.minor().to_string(),
            remaining_minor: b.progress.remaining.minor().to_string(),
            used_bp: b.progress.used_bp,
            status: match b.progress.status {
                BudgetStatus::Ok => "ok",
                BudgetStatus::Warning => "warning",
                BudgetStatus::Over => "over",
            },
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetInput {
    pub category_id: i64,
    pub month: String,
    pub limit_minor: String,
}

pub(crate) fn list(db: &Db, month: &str) -> Result<Vec<BudgetDto>, AppError> {
    let month = YearMonth::parse(month)?;
    Ok(db.budgets(month)?.iter().map(BudgetDto::from).collect())
}

pub(crate) fn set(db: &mut Db, input: &BudgetInput) -> Result<BudgetDto, AppError> {
    let month = YearMonth::parse(&input.month)?;
    let limit = Money::from_minor(parse_minor(&input.limit_minor)?, db.app_currency()?)?;
    let draft = BudgetDraft::new(input.category_id, month, limit)?;
    db.set_budget(&draft)?;
    db.budgets(month)?
        .iter()
        .find(|b| b.category_id == input.category_id)
        .map(BudgetDto::from)
        .ok_or(AppError::new(
            crate::error::ErrorCode::Internal,
            "internal error",
        ))
}

pub(crate) fn delete(db: &mut Db, id: i64) -> Result<(), AppError> {
    Ok(db.delete_budget(id)?)
}

#[tauri::command]
pub async fn budgets_list(
    state: State<'_, AppState>,
    month: String,
) -> Result<Vec<BudgetDto>, AppError> {
    state.with_db(|db| list(db, &month))
}

#[tauri::command]
pub async fn budget_set(
    state: State<'_, AppState>,
    input: BudgetInput,
) -> Result<BudgetDto, AppError> {
    state.with_db(|db| set(db, &input))
}

#[tauri::command]
pub async fn budget_delete(state: State<'_, AppState>, id: i64) -> Result<(), AppError> {
    state.with_db(|db| delete(db, id))
}

// Amounts, names and notes must never reach a log through `{:?}`.
mt_core::redacted_debug!(BudgetDto, BudgetInput);

#[cfg(test)]
mod tests {
    use mt_core::{
        AccountDraft, AccountKind, Currency, FlowKind, TransactionDraft, parse_iso_date,
    };
    use mt_storage::DbKey;

    use super::*;
    use crate::error::ErrorCode;

    fn setup() -> (Db, i64, i64, i64) {
        let mut db = Db::open_in_memory(&DbKey::new([1u8; 32])).unwrap();
        let idr = |m| Money::from_minor(m, Currency::IDR).unwrap();
        let account = db
            .create_account(
                &AccountDraft::new("Tunai", AccountKind::Cash, idr(0)).unwrap(),
                0,
            )
            .unwrap();
        let cats = db.categories(false).unwrap();
        let food = cats.iter().find(|c| c.name == "Makanan").unwrap().id;
        let salary = cats.iter().find(|c| c.name == "Gaji").unwrap().id;
        (db, account, food, salary)
    }

    fn spend(db: &mut Db, account: i64, category: i64, minor: i64, date: &str) {
        let draft = TransactionDraft::new(
            account,
            category,
            FlowKind::Expense,
            Money::from_minor(minor, Currency::IDR).unwrap(),
            "",
            parse_iso_date(date).unwrap(),
        )
        .unwrap();
        db.create_transaction(&draft, 0).unwrap();
    }

    fn input(category: i64, month: &str, limit: &str) -> BudgetInput {
        BudgetInput {
            category_id: category,
            month: month.to_owned(),
            limit_minor: limit.to_owned(),
        }
    }

    #[test]
    fn a_budget_reports_progress_as_strings() {
        let (mut db, account, food, _) = setup();
        let dto = set(&mut db, &input(food, "2026-10", "1000000")).unwrap();
        assert_eq!(
            (
                dto.limit_minor.as_str(),
                dto.spent_minor.as_str(),
                dto.status
            ),
            ("1000000", "0", "ok")
        );
        spend(&mut db, account, food, 850_000, "2026-10-10");
        let dto = list(&db, "2026-10").unwrap().remove(0);
        assert_eq!(dto.spent_minor, "850000");
        assert_eq!(dto.remaining_minor, "150000");
        assert_eq!((dto.used_bp, dto.status), (8_500, "warning"));
        assert_eq!((dto.currency.as_str(), dto.exponent), ("IDR", 0));
    }

    #[test]
    fn exceeding_the_budget_gives_a_negative_remainder() {
        let (mut db, account, food, _) = setup();
        set(&mut db, &input(food, "2026-10", "100")).unwrap();
        spend(&mut db, account, food, 130, "2026-10-10");
        let dto = list(&db, "2026-10").unwrap().remove(0);
        assert_eq!((dto.remaining_minor.as_str(), dto.status), ("-30", "over"));
    }

    #[test]
    fn invalid_input_maps_to_stable_codes() {
        let (mut db, _, food, salary) = setup();
        let code = |r: Result<BudgetDto, AppError>| r.unwrap_err().code;
        assert_eq!(
            code(set(&mut db, &input(food, "2026-13", "100"))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(set(&mut db, &input(food, "Oktober", "100"))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(set(&mut db, &input(food, "2026-10", "0"))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(set(&mut db, &input(food, "2026-10", "-5"))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(set(&mut db, &input(salary, "2026-10", "5"))),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(set(&mut db, &input(9_999, "2026-10", "5"))),
            ErrorCode::NotFound
        );
        assert_eq!(list(&db, "nope").unwrap_err().code, ErrorCode::InvalidInput);
    }

    #[test]
    fn delete_and_serialisation() {
        let (mut db, _, food, _) = setup();
        let dto = set(&mut db, &input(food, "2026-10", "500")).unwrap();
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["limitMinor"], "500");
        assert_eq!(json["usedBp"], 0);
        assert_eq!(json["categoryName"], "Makanan");
        delete(&mut db, dto.id).unwrap();
        assert!(list(&db, "2026-10").unwrap().is_empty());
        assert_eq!(
            delete(&mut db, dto.id).unwrap_err().code,
            ErrorCode::NotFound
        );
    }
}
