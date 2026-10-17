//! Report commands. All figures come from `mt-storage` (SQL aggregation); this
//! layer only shapes them for the UI. Money is a string of minor units.

use mt_core::{Date, YearMonth, parse_iso_date};
use mt_storage::{CategorySpending, ChildSpending, Db, MonthSummary, NetWorth, SpendingReport};
use serde::Serialize;
use tauri::State;

use crate::{accounts::AccountDto, error::AppError, state::AppState};

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildSpendingDto {
    pub category_id: i64,
    pub name: String,
    pub spent_minor: String,
    pub count: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorySpendingDto {
    pub category_id: i64,
    pub name: String,
    pub spent_minor: String,
    pub count: u32,
    /// Basis points of all spending in the period (10 000 = 100 %).
    pub share_bp: i64,
    pub children: Vec<ChildSpendingDto>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendingReportDto {
    pub from: String,
    pub to: String,
    pub currency: String,
    pub exponent: u8,
    pub total_minor: String,
    pub categories: Vec<CategorySpendingDto>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthDto {
    pub month: String,
    pub income_minor: String,
    pub expense_minor: String,
    /// `income - expense`, signed.
    pub net_minor: String,
    /// Net worth at the end of the month, signed.
    pub net_worth_minor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendDto {
    pub currency: String,
    pub exponent: u8,
    pub months: Vec<MonthDto>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetWorthDto {
    pub currency: String,
    pub exponent: u8,
    pub total_minor: String,
    pub accounts: Vec<AccountDto>,
}

fn date(text: &str) -> Result<Date, AppError> {
    parse_iso_date(text).map_err(|_| AppError::invalid("date must be YYYY-MM-DD"))
}

impl From<&ChildSpending> for ChildSpendingDto {
    fn from(c: &ChildSpending) -> Self {
        Self {
            category_id: c.category_id,
            name: c.name.clone(),
            spent_minor: c.spent.minor().to_string(),
            count: c.count,
        }
    }
}

impl From<&CategorySpending> for CategorySpendingDto {
    fn from(c: &CategorySpending) -> Self {
        Self {
            category_id: c.category_id,
            name: c.name.clone(),
            spent_minor: c.spent.minor().to_string(),
            count: c.count,
            share_bp: c.share_bp,
            children: c.children.iter().map(ChildSpendingDto::from).collect(),
        }
    }
}

impl From<&SpendingReport> for SpendingReportDto {
    fn from(r: &SpendingReport) -> Self {
        Self {
            from: mt_core::format_iso_date(r.from),
            to: mt_core::format_iso_date(r.to),
            currency: r.total.currency().code().to_owned(),
            exponent: r.total.currency().exponent(),
            total_minor: r.total.minor().to_string(),
            categories: r.categories.iter().map(CategorySpendingDto::from).collect(),
        }
    }
}

impl From<&MonthSummary> for MonthDto {
    fn from(m: &MonthSummary) -> Self {
        Self {
            month: m.month.to_string(),
            income_minor: m.income.minor().to_string(),
            expense_minor: m.expense.minor().to_string(),
            net_minor: m.net.minor().to_string(),
            net_worth_minor: m.net_worth.minor().to_string(),
        }
    }
}

impl From<&NetWorth> for NetWorthDto {
    fn from(n: &NetWorth) -> Self {
        Self {
            currency: n.total.currency().code().to_owned(),
            exponent: n.total.currency().exponent(),
            total_minor: n.total.minor().to_string(),
            accounts: n.accounts.iter().map(AccountDto::from).collect(),
        }
    }
}

pub(crate) fn spending(db: &Db, from: &str, to: &str) -> Result<SpendingReportDto, AppError> {
    let (from, to) = (date(from)?, date(to)?);
    if to < from {
        return Err(AppError::invalid("period ends before it starts"));
    }
    Ok(SpendingReportDto::from(&db.spending_by_category(from, to)?))
}

pub(crate) fn trend(db: &Db, from: &str, to: &str) -> Result<TrendDto, AppError> {
    let (from, to) = (YearMonth::parse(from)?, YearMonth::parse(to)?);
    let currency = db.app_currency()?;
    let months = db.monthly_trend(from, to)?;
    Ok(TrendDto {
        currency: currency.code().to_owned(),
        exponent: currency.exponent(),
        months: months.iter().map(MonthDto::from).collect(),
    })
}

pub(crate) fn net_worth(db: &Db) -> Result<NetWorthDto, AppError> {
    Ok(NetWorthDto::from(&db.net_worth()?))
}

#[tauri::command]
pub async fn report_spending(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<SpendingReportDto, AppError> {
    state.with_db(|db| spending(db, &from, &to))
}

#[tauri::command]
pub async fn report_trend(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<TrendDto, AppError> {
    state.with_db(|db| trend(db, &from, &to))
}

#[tauri::command]
pub async fn report_net_worth(state: State<'_, AppState>) -> Result<NetWorthDto, AppError> {
    state.with_db(|db| net_worth(db))
}

// Amounts, names and notes must never reach a log through `{:?}`.
mt_core::redacted_debug!(
    ChildSpendingDto,
    CategorySpendingDto,
    SpendingReportDto,
    MonthDto,
    NetWorthDto
);

#[cfg(test)]
mod tests {
    use mt_core::{
        AccountDraft, AccountKind, Currency, FlowKind, Money, TransactionDraft, TransferDraft,
    };
    use mt_storage::DbKey;

    use super::*;
    use crate::error::ErrorCode;

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::IDR).unwrap()
    }

    fn setup() -> (Db, i64, i64, i64) {
        let mut db = Db::open_in_memory(&DbKey::new([13u8; 32])).unwrap();
        let cash = db
            .create_account(
                &AccountDraft::new("Tunai", AccountKind::Cash, idr(500_000)).unwrap(),
                0,
            )
            .unwrap();
        let bank = db
            .create_account(
                &AccountDraft::new("BCA", AccountKind::Bank, idr(0)).unwrap(),
                0,
            )
            .unwrap();
        let cats = db.categories(false).unwrap();
        let food = cats.iter().find(|c| c.name == "Makanan").unwrap().id;
        (db, cash, bank, food)
    }

    fn spend(db: &mut Db, account: i64, category: i64, minor: i64, date: &str) {
        let draft = TransactionDraft::new(
            account,
            category,
            FlowKind::Expense,
            idr(minor),
            "",
            parse_iso_date(date).unwrap(),
        )
        .unwrap();
        db.create_transaction(&draft, 0).unwrap();
    }

    #[test]
    fn spending_report_serialises_strings_and_shares() {
        let (mut db, cash, _, food) = setup();
        spend(&mut db, cash, food, 75_000, "2026-10-02");
        let transport = db
            .categories(false)
            .unwrap()
            .iter()
            .find(|c| c.name == "Transportasi")
            .unwrap()
            .id;
        spend(&mut db, cash, transport, 25_000, "2026-10-03");
        let report = spending(&db, "2026-10-01", "2026-10-31").unwrap();
        assert_eq!(report.total_minor, "100000");
        assert_eq!(report.categories[0].name, "Makanan");
        assert_eq!(report.categories[0].share_bp, 7_500);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["categories"][0]["spentMinor"], "75000");
        assert_eq!(json["totalMinor"], "100000");
        assert_eq!(
            (json["currency"].as_str(), json["exponent"].as_u64()),
            (Some("IDR"), Some(0))
        );
    }

    #[test]
    fn spending_rejects_bad_or_reversed_periods() {
        let (db, ..) = setup();
        let code = |r: Result<SpendingReportDto, AppError>| r.unwrap_err().code;
        assert_eq!(
            code(spending(&db, "2026-10-31", "2026-10-01")),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(spending(&db, "bulan ini", "2026-10-31")),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(spending(&db, "2026-02-30", "2026-03-01")),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn trend_returns_every_month_in_the_range_with_signed_net_worth() {
        let (mut db, cash, _, food) = setup();
        spend(&mut db, cash, food, 600_000, "2026-09-10"); // more than the 500 000 opening
        let trend = trend(&db, "2026-08", "2026-10").unwrap();
        assert_eq!(trend.months.len(), 3);
        assert_eq!(trend.months[0].month, "2026-08");
        assert_eq!(trend.months[1].expense_minor, "600000");
        assert_eq!(trend.months[1].net_minor, "-600000");
        assert_eq!(trend.months[1].net_worth_minor, "-100000");
        assert_eq!(trend.months[2].net_worth_minor, "-100000");
        assert_eq!((trend.currency.as_str(), trend.exponent), ("IDR", 0));
    }

    #[test]
    fn trend_rejects_invalid_ranges() {
        let (db, ..) = setup();
        let code = |r: Result<TrendDto, AppError>| r.unwrap_err().code;
        assert_eq!(
            code(trend(&db, "2026-13", "2026-12")),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(trend(&db, "2026-10", "2026-09")),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(trend(&db, "2000-01", "2026-12")),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn net_worth_lists_accounts_and_the_total() {
        let (mut db, cash, bank, _) = setup();
        let transfer = TransferDraft::new(
            cash,
            bank,
            idr(200_000),
            "",
            parse_iso_date("2026-10-01").unwrap(),
        )
        .unwrap();
        db.create_transfer(&transfer, 0).unwrap();
        let worth = net_worth(&db).unwrap();
        assert_eq!(worth.total_minor, "500000");
        let by_name: Vec<(&str, &str)> = worth
            .accounts
            .iter()
            .map(|a| (a.name.as_str(), a.balance_minor.as_str()))
            .collect();
        assert_eq!(by_name, [("Tunai", "300000"), ("BCA", "200000")]);
        let json = serde_json::to_value(&worth).unwrap();
        assert_eq!(json["totalMinor"], "500000");
    }
}
