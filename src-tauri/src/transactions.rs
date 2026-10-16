//! Category and transaction commands.
//!
//! As elsewhere, money is a decimal string of minor units. Amounts in inputs are
//! positive magnitudes; the kind decides the sign. Outputs carry the signed
//! amount, so the UI can colour and sum without knowing the rules.

use std::time::{SystemTime, UNIX_EPOCH};

use mt_core::{
    CategoryDraft, Date, FlowKind, Money, TransactionDraft, TransferDraft, format_iso_date,
    parse_iso_date,
};
use mt_storage::{
    Category, Cursor, DEFAULT_PAGE, Db, KindFilter, Page, TransactionFilter, TransactionRecord,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{accounts::parse_minor, error::AppError, state::AppState};

// ---------------------------------------------------------------- categories

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryDto {
    pub id: i64,
    pub name: String,
    pub kind: &'static str,
    pub parent_id: Option<i64>,
    pub archived: bool,
}

impl From<&Category> for CategoryDto {
    fn from(c: &Category) -> Self {
        Self {
            id: c.id,
            name: c.name.clone(),
            kind: c.kind.code(),
            parent_id: c.parent_id,
            archived: c.archived,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInput {
    pub name: String,
    pub kind: String,
    pub parent_id: Option<i64>,
}

fn flow_kind(code: &str) -> Result<FlowKind, AppError> {
    FlowKind::from_code(code).ok_or_else(|| AppError::invalid("kind must be income or expense"))
}

pub(crate) fn categories(db: &Db, include_archived: bool) -> Result<Vec<CategoryDto>, AppError> {
    Ok(db
        .categories(include_archived)?
        .iter()
        .map(CategoryDto::from)
        .collect())
}

pub(crate) fn create_category(db: &mut Db, input: &CategoryInput) -> Result<CategoryDto, AppError> {
    let draft = CategoryDraft::new(&input.name, flow_kind(&input.kind)?, input.parent_id)?;
    let id = db.create_category(&draft)?;
    let created = db
        .categories(true)?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or(AppError::new(
            crate::error::ErrorCode::Internal,
            "internal error",
        ))?;
    Ok(CategoryDto::from(&created))
}

// -------------------------------------------------------------- transactions

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionDto {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    /// `income`, `expense`, `transfer_out` or `transfer_in`.
    pub kind: &'static str,
    /// Signed minor units.
    pub amount_minor: String,
    pub currency: String,
    pub exponent: u8,
    pub note: String,
    pub occurred_on: String,
    pub transfer_group_id: Option<i64>,
    pub recurring: bool,
}

impl From<&TransactionRecord> for TransactionDto {
    fn from(t: &TransactionRecord) -> Self {
        Self {
            id: t.id,
            account_id: t.account_id,
            account_name: t.account_name.clone(),
            category_id: t.category_id,
            category_name: t.category_name.clone(),
            kind: t.kind.code(),
            amount_minor: t.amount.minor().to_string(),
            currency: t.amount.currency().code().to_owned(),
            exponent: t.amount.currency().exponent(),
            note: t.note.clone(),
            occurred_on: format_iso_date(t.occurred_on),
            transfer_group_id: t.transfer_group_id,
            recurring: t.recurring_rule_id.is_some(),
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionInput {
    pub account_id: i64,
    pub category_id: i64,
    /// `income` or `expense`.
    pub kind: String,
    /// Positive magnitude, minor units.
    pub amount_minor: String,
    pub note: String,
    pub occurred_on: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferInput {
    pub from_account_id: i64,
    pub to_account_id: i64,
    pub amount_minor: String,
    pub note: String,
    pub occurred_on: String,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterInput {
    pub account_id: Option<i64>,
    pub category_id: Option<i64>,
    /// `income`, `expense` or `transfer`.
    pub kind: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorDto {
    pub occurred_on: String,
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageDto {
    pub items: Vec<TransactionDto>,
    pub next: Option<CursorDto>,
}

impl From<&Page> for PageDto {
    fn from(page: &Page) -> Self {
        Self {
            items: page.items.iter().map(TransactionDto::from).collect(),
            next: page.next.map(|c| CursorDto {
                occurred_on: format_iso_date(c.occurred_on),
                id: c.id,
            }),
        }
    }
}

fn date(text: &str) -> Result<Date, AppError> {
    parse_iso_date(text).map_err(|_| AppError::invalid("date must be YYYY-MM-DD"))
}

fn magnitude(db: &Db, text: &str) -> Result<Money, AppError> {
    Ok(Money::from_minor(parse_minor(text)?, db.app_currency()?)?)
}

fn transaction_draft(db: &Db, input: &TransactionInput) -> Result<TransactionDraft, AppError> {
    Ok(TransactionDraft::new(
        input.account_id,
        input.category_id,
        flow_kind(&input.kind)?,
        magnitude(db, &input.amount_minor)?,
        &input.note,
        date(&input.occurred_on)?,
    )?)
}

fn fetch(db: &Db, id: i64) -> Result<TransactionDto, AppError> {
    let record = db
        .transaction(id)?
        .ok_or(mt_storage::StorageError::NotFound)?;
    Ok(TransactionDto::from(&record))
}

fn filter_from(input: &FilterInput) -> Result<TransactionFilter, AppError> {
    let kind = match input.kind.as_deref() {
        None | Some("") => None,
        Some("income") => Some(KindFilter::Income),
        Some("expense") => Some(KindFilter::Expense),
        Some("transfer") => Some(KindFilter::Transfer),
        Some(_) => return Err(AppError::invalid("unknown kind filter")),
    };
    let optional_date = |value: &Option<String>| -> Result<Option<Date>, AppError> {
        match value.as_deref() {
            None | Some("") => Ok(None),
            Some(text) => Ok(Some(date(text)?)),
        }
    };
    Ok(TransactionFilter {
        account_id: input.account_id,
        category_id: input.category_id,
        kind,
        from: optional_date(&input.from)?,
        to: optional_date(&input.to)?,
        search: input.search.clone(),
    })
}

pub(crate) fn list(
    db: &Db,
    filter: &FilterInput,
    cursor: Option<&CursorDto>,
    limit: Option<usize>,
) -> Result<PageDto, AppError> {
    let after = match cursor {
        Some(c) => Some(Cursor {
            occurred_on: date(&c.occurred_on)?,
            id: c.id,
        }),
        None => None,
    };
    let page = db.transactions(&filter_from(filter)?, after, limit.unwrap_or(DEFAULT_PAGE))?;
    Ok(PageDto::from(&page))
}

pub(crate) fn create(
    db: &mut Db,
    input: &TransactionInput,
    now_unix: i64,
) -> Result<TransactionDto, AppError> {
    let draft = transaction_draft(db, input)?;
    let id = db.create_transaction(&draft, now_unix)?;
    fetch(db, id)
}

pub(crate) fn update(
    db: &mut Db,
    id: i64,
    input: &TransactionInput,
) -> Result<TransactionDto, AppError> {
    let draft = transaction_draft(db, input)?;
    db.update_transaction(id, &draft)?;
    fetch(db, id)
}

pub(crate) fn delete(db: &mut Db, id: i64) -> Result<usize, AppError> {
    Ok(db.delete_transaction(id)?)
}

pub(crate) fn create_transfer(
    db: &mut Db,
    input: &TransferInput,
    now_unix: i64,
) -> Result<TransactionDto, AppError> {
    let draft = TransferDraft::new(
        input.from_account_id,
        input.to_account_id,
        magnitude(db, &input.amount_minor)?,
        &input.note,
        date(&input.occurred_on)?,
    )?;
    let group = db.create_transfer(&draft, now_unix)?;
    // The outgoing leg's id is the group id.
    fetch(db, group)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[tauri::command]
pub async fn categories_list(
    state: State<'_, AppState>,
    include_archived: bool,
) -> Result<Vec<CategoryDto>, AppError> {
    state.with_db(|db| categories(db, include_archived))
}

#[tauri::command]
pub async fn category_create(
    state: State<'_, AppState>,
    input: CategoryInput,
) -> Result<CategoryDto, AppError> {
    state.with_db(|db| create_category(db, &input))
}

#[tauri::command]
pub async fn transactions_list(
    state: State<'_, AppState>,
    filter: Option<FilterInput>,
    cursor: Option<CursorDto>,
    limit: Option<usize>,
) -> Result<PageDto, AppError> {
    state.with_db(|db| list(db, &filter.unwrap_or_default(), cursor.as_ref(), limit))
}

#[tauri::command]
pub async fn transaction_create(
    state: State<'_, AppState>,
    input: TransactionInput,
) -> Result<TransactionDto, AppError> {
    state.with_db(|db| create(db, &input, now_unix()))
}

#[tauri::command]
pub async fn transaction_update(
    state: State<'_, AppState>,
    id: i64,
    input: TransactionInput,
) -> Result<TransactionDto, AppError> {
    state.with_db(|db| update(db, id, &input))
}

/// Returns how many rows were removed (2 for a transfer).
#[tauri::command]
pub async fn transaction_delete(state: State<'_, AppState>, id: i64) -> Result<usize, AppError> {
    state.with_db(|db| delete(db, id))
}

#[tauri::command]
pub async fn transfer_create(
    state: State<'_, AppState>,
    input: TransferInput,
) -> Result<TransactionDto, AppError> {
    state.with_db(|db| create_transfer(db, &input, now_unix()))
}

// Amounts, names and notes must never reach a log through `{:?}`.
mt_core::redacted_debug!(
    CategoryDto,
    CategoryInput,
    TransactionDto,
    TransactionInput,
    TransferInput,
    FilterInput
);

#[cfg(test)]
mod tests {
    use mt_core::AccountKind;
    use zeroize::Zeroizing;

    use super::*;
    use crate::{accounts, error::ErrorCode};

    #[derive(Clone, Copy)]
    struct Ids {
        cash: i64,
        bank: i64,
        food: i64,
        salary: i64,
    }

    struct Ctx {
        db: Db,
        ids: Ids,
    }

    fn ctx() -> Ctx {
        let mut db = Db::open_in_memory(&Zeroizing::new([3u8; 32])).unwrap();
        let account = |db: &mut Db, name: &str, kind: &str| {
            accounts::create(
                db,
                &accounts::AccountInput {
                    name: name.to_owned(),
                    kind: kind.to_owned(),
                    opening_balance_minor: "0".to_owned(),
                },
                0,
            )
            .unwrap()
            .id
        };
        let cash = account(&mut db, "Tunai", "cash");
        let bank = account(&mut db, "BCA", "bank");
        let cats = categories(&db, false).unwrap();
        let find = |name: &str, kind: &str| {
            cats.iter()
                .find(|c| c.name == name && c.kind == kind)
                .unwrap()
                .id
        };
        let (food, salary) = (find("Makanan", "expense"), find("Gaji", "income"));
        let _ = AccountKind::Cash;
        Ctx {
            db,
            ids: Ids {
                cash,
                bank,
                food,
                salary,
            },
        }
    }

    fn input(c: &Ids, kind: &str, amount: &str, note: &str, date: &str) -> TransactionInput {
        TransactionInput {
            account_id: c.cash,
            category_id: if kind == "income" { c.salary } else { c.food },
            kind: kind.to_owned(),
            amount_minor: amount.to_owned(),
            note: note.to_owned(),
            occurred_on: date.to_owned(),
        }
    }

    #[test]
    fn expenses_come_back_signed_and_dated() {
        let mut c = ctx();
        let dto = create(
            &mut c.db,
            &input(&c.ids, "expense", "25000", " makan ", "2026-10-14"),
            1,
        )
        .unwrap();
        assert_eq!(dto.amount_minor, "-25000");
        assert_eq!(dto.kind, "expense");
        assert_eq!(dto.note, "makan");
        assert_eq!(dto.occurred_on, "2026-10-14");
        assert_eq!((dto.currency.as_str(), dto.exponent), ("IDR", 0));
        assert_eq!(dto.category_name.as_deref(), Some("Makanan"));
        assert!(!dto.recurring);
    }

    #[test]
    fn invalid_input_maps_to_stable_codes() {
        let mut c = ctx();
        let code = |r: Result<TransactionDto, AppError>| r.unwrap_err().code;
        assert_eq!(
            code(create(
                &mut c.db,
                &input(&c.ids, "expense", "0", "", "2026-10-14"),
                0
            )),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(
                &mut c.db,
                &input(&c.ids, "expense", "-5", "", "2026-10-14"),
                0
            )),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(
                &mut c.db,
                &input(&c.ids, "expense", "5", "", "14-10-2026"),
                0
            )),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(
                &mut c.db,
                &input(&c.ids, "expense", "5", "", "2026-02-30"),
                0
            )),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(
                &mut c.db,
                &input(&c.ids, "transfer", "5", "", "2026-10-14"),
                0
            )),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(create(
                &mut c.db,
                &input(&c.ids, "expense", "9007199254740992", "", "2026-10-14"),
                0
            )),
            ErrorCode::AmountOutOfRange
        );
        // An income category used for an expense.
        let mut wrong = input(&c.ids, "expense", "5", "", "2026-10-14");
        wrong.category_id = c.ids.salary;
        assert_eq!(code(create(&mut c.db, &wrong, 0)), ErrorCode::InvalidInput);
    }

    #[test]
    fn update_and_delete_round_trip() {
        let mut c = ctx();
        let id = create(
            &mut c.db,
            &input(&c.ids, "expense", "100", "a", "2026-10-01"),
            0,
        )
        .unwrap()
        .id;
        let updated = update(
            &mut c.db,
            id,
            &input(&c.ids, "expense", "250", "b", "2026-10-02"),
        )
        .unwrap();
        assert_eq!(
            (updated.amount_minor.as_str(), updated.note.as_str()),
            ("-250", "b")
        );
        assert_eq!(delete(&mut c.db, id).unwrap(), 1);
        assert_eq!(delete(&mut c.db, id).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn transfers_show_both_legs_and_delete_together() {
        let mut c = ctx();
        let out = create_transfer(
            &mut c.db,
            &TransferInput {
                from_account_id: c.ids.bank,
                to_account_id: c.ids.cash,
                amount_minor: "500000".into(),
                note: "tarik".into(),
                occurred_on: "2026-10-14".into(),
            },
            0,
        )
        .unwrap();
        assert_eq!(
            (out.kind, out.amount_minor.as_str()),
            ("transfer_out", "-500000")
        );
        assert_eq!(out.transfer_group_id, Some(out.id));

        let filter = FilterInput {
            kind: Some("transfer".into()),
            ..Default::default()
        };
        let page = list(&c.db, &filter, None, None).unwrap();
        let kinds: Vec<&str> = page.items.iter().map(|t| t.kind).collect();
        assert_eq!(kinds.len(), 2);
        assert!(kinds.contains(&"transfer_in") && kinds.contains(&"transfer_out"));

        let incoming = page
            .items
            .iter()
            .find(|t| t.kind == "transfer_in")
            .unwrap()
            .id;
        assert_eq!(delete(&mut c.db, incoming).unwrap(), 2);
        assert!(
            list(&c.db, &FilterInput::default(), None, None)
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[test]
    fn same_account_transfer_is_rejected() {
        let mut c = ctx();
        let err = create_transfer(
            &mut c.db,
            &TransferInput {
                from_account_id: c.ids.cash,
                to_account_id: c.ids.cash,
                amount_minor: "1".into(),
                note: String::new(),
                occurred_on: "2026-10-14".into(),
            },
            0,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn paging_works_through_the_dto_cursor() {
        let mut c = ctx();
        for n in 0..7 {
            create(
                &mut c.db,
                &input(&c.ids, "expense", &(100 + n).to_string(), "", "2026-10-14"),
                0,
            )
            .unwrap();
        }
        let first = list(&c.db, &FilterInput::default(), None, Some(3)).unwrap();
        assert_eq!(first.items.len(), 3);
        let second = list(&c.db, &FilterInput::default(), first.next.as_ref(), Some(3)).unwrap();
        let third = list(
            &c.db,
            &FilterInput::default(),
            second.next.as_ref(),
            Some(3),
        )
        .unwrap();
        assert_eq!(third.items.len(), 1);
        assert_eq!(third.next, None);
        let amounts: Vec<String> = [first, second, third]
            .iter()
            .flat_map(|p| p.items.iter().map(|t| t.amount_minor.clone()))
            .collect();
        assert_eq!(
            amounts,
            ["-106", "-105", "-104", "-103", "-102", "-101", "-100"]
        );
    }

    #[test]
    fn filters_come_through_the_dto() {
        let mut c = ctx();
        create(
            &mut c.db,
            &input(&c.ids, "expense", "10", "kopi", "2026-09-30"),
            0,
        )
        .unwrap();
        create(
            &mut c.db,
            &input(&c.ids, "expense", "20", "bakso", "2026-10-02"),
            0,
        )
        .unwrap();
        create(
            &mut c.db,
            &input(&c.ids, "income", "30", "", "2026-10-03"),
            0,
        )
        .unwrap();
        let run = |f: FilterInput| list(&c.db, &f, None, None).unwrap().items.len();
        assert_eq!(run(FilterInput::default()), 3);
        assert_eq!(
            run(FilterInput {
                from: Some("2026-10-01".into()),
                ..Default::default()
            }),
            2
        );
        assert_eq!(
            run(FilterInput {
                kind: Some("income".into()),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            run(FilterInput {
                search: Some("BAK".into()),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            list(
                &c.db,
                &FilterInput {
                    kind: Some("nope".into()),
                    ..Default::default()
                },
                None,
                None
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn dto_uses_camel_case_and_string_money() {
        let mut c = ctx();
        let dto = create(
            &mut c.db,
            &input(&c.ids, "expense", "5", "", "2026-10-14"),
            0,
        )
        .unwrap();
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["amountMinor"], "-5");
        assert_eq!(json["occurredOn"], "2026-10-14");
        assert!(json.get("amount_minor").is_none());
        let page =
            serde_json::to_value(list(&c.db, &FilterInput::default(), None, Some(1)).unwrap())
                .unwrap();
        assert!(page["next"].is_null());
    }

    #[test]
    fn categories_can_be_added_and_nested() {
        let mut c = ctx();
        let top = create_category(
            &mut c.db,
            &CategoryInput {
                name: " Hobi ".into(),
                kind: "expense".into(),
                parent_id: None,
            },
        )
        .unwrap();
        assert_eq!(top.name, "Hobi");
        let sub = create_category(
            &mut c.db,
            &CategoryInput {
                name: "Gitar".into(),
                kind: "expense".into(),
                parent_id: Some(top.id),
            },
        )
        .unwrap();
        assert_eq!(sub.parent_id, Some(top.id));
        let dup = create_category(
            &mut c.db,
            &CategoryInput {
                name: "hobi".into(),
                kind: "expense".into(),
                parent_id: None,
            },
        );
        assert_eq!(dup.unwrap_err().code, ErrorCode::NameInUse);
    }

    #[test]
    fn archived_accounts_report_archived() {
        let mut c = ctx();
        accounts::archive(&mut c.db, c.ids.bank, true).unwrap();
        let mut bad = input(&c.ids, "expense", "5", "", "2026-10-14");
        bad.account_id = c.ids.bank;
        assert_eq!(
            create(&mut c.db, &bad, 0).unwrap_err().code,
            ErrorCode::Archived
        );
    }
}
