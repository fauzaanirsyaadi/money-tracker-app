//! Recurring-rule commands, and the catch-up the UI triggers at app start.
//!
//! The device's *local* calendar date is only known to the UI, so it passes
//! `today`; the backend accepts it only if it is within two days of the UTC
//! date (time zones span UTC-12 to UTC+14). That keeps a wrong or hostile value
//! from generating years of transactions.

use std::time::{SystemTime, UNIX_EPOCH};

use mt_core::{
    Date, FlowKind, Frequency, Money, RecurringDraft, RecurringEdit, format_iso_date,
    parse_iso_date,
};
use mt_storage::{CatchUpReport, Db, RecurringRecord};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{accounts::parse_minor, error::AppError, state::AppState};

/// Per rule per app open; the remainder continues on the next open.
const CATCH_UP_LIMIT: usize = 1_000;

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurringDto {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub category_id: i64,
    pub category_name: String,
    pub kind: &'static str,
    /// Positive magnitude of each occurrence.
    pub amount_minor: String,
    pub currency: String,
    pub exponent: u8,
    pub note: String,
    pub frequency: &'static str,
    pub every: u32,
    pub start_on: String,
    pub end_on: Option<String>,
    pub next_due: String,
    pub active: bool,
}

impl From<&RecurringRecord> for RecurringDto {
    fn from(r: &RecurringRecord) -> Self {
        Self {
            id: r.id,
            account_id: r.account_id,
            account_name: r.account_name.clone(),
            category_id: r.category_id,
            category_name: r.category_name.clone(),
            kind: r.kind.code(),
            amount_minor: r.amount.minor().to_string(),
            currency: r.amount.currency().code().to_owned(),
            exponent: r.amount.currency().exponent(),
            note: r.note.clone(),
            frequency: r.frequency.code(),
            every: r.every,
            start_on: format_iso_date(r.start_on),
            end_on: r.end_on.map(format_iso_date),
            next_due: format_iso_date(r.next_due),
            active: r.active,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurringInput {
    pub account_id: i64,
    pub category_id: i64,
    pub kind: String,
    pub amount_minor: String,
    pub note: String,
    pub frequency: String,
    pub every: u32,
    pub start_on: String,
    pub end_on: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurringEditInput {
    pub account_id: i64,
    pub category_id: i64,
    pub amount_minor: String,
    pub note: String,
    pub end_on: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatchUpDto {
    pub rules_advanced: u32,
    pub transactions_created: u32,
    pub rules_finished: u32,
    pub rules_invalid: u32,
}

impl From<CatchUpReport> for CatchUpDto {
    fn from(r: CatchUpReport) -> Self {
        Self {
            rules_advanced: r.rules_advanced,
            transactions_created: r.transactions_created,
            rules_finished: r.rules_finished,
            rules_invalid: r.rules_invalid,
        }
    }
}

fn date(text: &str) -> Result<Date, AppError> {
    parse_iso_date(text).map_err(|_| AppError::invalid("date must be YYYY-MM-DD"))
}

fn optional_date(text: &Option<String>) -> Result<Option<Date>, AppError> {
    match text.as_deref() {
        None | Some("") => Ok(None),
        Some(value) => Ok(Some(date(value)?)),
    }
}

/// Accepts the UI's local date only if it is plausible next to the UTC clock.
pub(crate) fn plausible_today(text: &str, now_unix: i64) -> Result<Date, AppError> {
    const UNIX_EPOCH_JULIAN_DAY: i64 = 2_440_588;
    let today = date(text)?;
    let utc_julian = UNIX_EPOCH_JULIAN_DAY + now_unix.div_euclid(86_400);
    let skew = (i64::from(today.to_julian_day()) - utc_julian).abs();
    if skew > 2 {
        return Err(AppError::invalid("date does not match the device clock"));
    }
    Ok(today)
}

fn money(db: &Db, text: &str) -> Result<Money, AppError> {
    Ok(Money::from_minor(parse_minor(text)?, db.app_currency()?)?)
}

pub(crate) fn list(db: &Db) -> Result<Vec<RecurringDto>, AppError> {
    Ok(db
        .recurring_rules()?
        .iter()
        .map(RecurringDto::from)
        .collect())
}

fn fetch(db: &Db, id: i64) -> Result<RecurringDto, AppError> {
    db.recurring_rules()?
        .iter()
        .find(|r| r.id == id)
        .map(RecurringDto::from)
        .ok_or(mt_storage::StorageError::NotFound.into())
}

pub(crate) fn create(db: &mut Db, input: &RecurringInput) -> Result<RecurringDto, AppError> {
    let kind = FlowKind::from_code(&input.kind)
        .ok_or_else(|| AppError::invalid("kind must be income or expense"))?;
    let frequency = Frequency::from_code(&input.frequency)
        .ok_or_else(|| AppError::invalid("unknown frequency"))?;
    let draft = RecurringDraft::new(
        input.account_id,
        input.category_id,
        kind,
        money(db, &input.amount_minor)?,
        &input.note,
        frequency,
        input.every,
        date(&input.start_on)?,
        optional_date(&input.end_on)?,
    )?;
    let id = db.create_recurring(&draft)?;
    fetch(db, id)
}

pub(crate) fn update(
    db: &mut Db,
    id: i64,
    input: &RecurringEditInput,
) -> Result<RecurringDto, AppError> {
    let edit = RecurringEdit::new(
        input.account_id,
        input.category_id,
        money(db, &input.amount_minor)?,
        &input.note,
        optional_date(&input.end_on)?,
    )?;
    db.update_recurring(id, &edit)?;
    fetch(db, id)
}

pub(crate) fn set_active(
    db: &mut Db,
    id: i64,
    active: bool,
    today: &str,
    now_unix: i64,
) -> Result<RecurringDto, AppError> {
    db.set_recurring_active(id, active, plausible_today(today, now_unix)?)?;
    fetch(db, id)
}

pub(crate) fn delete(db: &mut Db, id: i64) -> Result<(), AppError> {
    Ok(db.delete_recurring(id)?)
}

pub(crate) fn catch_up(db: &mut Db, today: &str, now_unix: i64) -> Result<CatchUpDto, AppError> {
    let today = plausible_today(today, now_unix)?;
    Ok(db
        .catch_up_recurring(today, now_unix, CATCH_UP_LIMIT)?
        .into())
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[tauri::command]
pub async fn recurring_list(state: State<'_, AppState>) -> Result<Vec<RecurringDto>, AppError> {
    state.with_db(|db| list(db))
}

#[tauri::command]
pub async fn recurring_create(
    state: State<'_, AppState>,
    input: RecurringInput,
) -> Result<RecurringDto, AppError> {
    state.with_db(|db| create(db, &input))
}

#[tauri::command]
pub async fn recurring_update(
    state: State<'_, AppState>,
    id: i64,
    input: RecurringEditInput,
) -> Result<RecurringDto, AppError> {
    state.with_db(|db| update(db, id, &input))
}

#[tauri::command]
pub async fn recurring_set_active(
    state: State<'_, AppState>,
    id: i64,
    active: bool,
    today: String,
) -> Result<RecurringDto, AppError> {
    state.with_db(|db| set_active(db, id, active, &today, now_unix()))
}

#[tauri::command]
pub async fn recurring_delete(state: State<'_, AppState>, id: i64) -> Result<(), AppError> {
    state.with_db(|db| delete(db, id))
}

/// Run once when the app opens: records every occurrence that became due.
#[tauri::command]
pub async fn recurring_catch_up(
    state: State<'_, AppState>,
    today: String,
) -> Result<CatchUpDto, AppError> {
    state.with_db(|db| catch_up(db, &today, now_unix()))
}

// Amounts, names and notes must never reach a log through `{:?}`.
mt_core::redacted_debug!(RecurringDto, RecurringInput, RecurringEditInput);

#[cfg(test)]
mod tests {
    use mt_core::{AccountDraft, AccountKind, Currency};
    use zeroize::Zeroizing;

    use super::*;
    use crate::error::ErrorCode;

    // 2026-10-14 12:00:00 UTC
    const NOW: i64 = 1_791_979_200;

    fn setup() -> (Db, i64, i64) {
        let mut db = Db::open_in_memory(&Zeroizing::new([9u8; 32])).unwrap();
        let account = db
            .create_account(
                &AccountDraft::new(
                    "Tunai",
                    AccountKind::Cash,
                    Money::from_minor(0, Currency::IDR).unwrap(),
                )
                .unwrap(),
                0,
            )
            .unwrap();
        let category = db
            .categories(false)
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Tagihan")
            .unwrap()
            .id;
        (db, account, category)
    }

    fn input(account: i64, category: i64, start: &str) -> RecurringInput {
        RecurringInput {
            account_id: account,
            category_id: category,
            kind: "expense".into(),
            amount_minor: "150000".into(),
            note: "internet".into(),
            frequency: "monthly".into(),
            every: 1,
            start_on: start.into(),
            end_on: None,
        }
    }

    #[test]
    fn today_must_be_close_to_the_utc_clock() {
        assert!(plausible_today("2026-10-14", NOW).is_ok());
        assert!(
            plausible_today("2026-10-15", NOW).is_ok(),
            "ahead east of UTC"
        );
        assert!(plausible_today("2026-10-13", NOW).is_ok());
        assert!(plausible_today("2026-10-17", NOW).is_err());
        assert!(plausible_today("2026-10-11", NOW).is_err());
        assert!(plausible_today("2036-10-14", NOW).is_err());
        assert!(plausible_today("kemarin", NOW).is_err());
    }

    #[test]
    fn create_returns_the_rule_with_schedule_fields() {
        let (mut db, account, category) = setup();
        let dto = create(&mut db, &input(account, category, "2026-11-05")).unwrap();
        assert_eq!(
            (dto.frequency, dto.every, dto.kind),
            ("monthly", 1, "expense")
        );
        assert_eq!(
            (dto.amount_minor.as_str(), dto.next_due.as_str()),
            ("150000", "2026-11-05")
        );
        assert!(dto.active);
        assert_eq!(dto.end_on, None);
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["nextDue"], "2026-11-05");
        assert!(json["endOn"].is_null());
    }

    #[test]
    fn invalid_rules_are_rejected() {
        let (mut db, account, category) = setup();
        let code = |r: Result<RecurringDto, AppError>| r.unwrap_err().code;
        let mut bad = input(account, category, "2026-11-05");
        bad.every = 0;
        assert_eq!(code(create(&mut db, &bad)), ErrorCode::InvalidInput);
        let mut bad = input(account, category, "2026-11-05");
        bad.frequency = "hourly".into();
        assert_eq!(code(create(&mut db, &bad)), ErrorCode::InvalidInput);
        let mut bad = input(account, category, "2026-11-05");
        bad.end_on = Some("2026-11-04".into());
        assert_eq!(code(create(&mut db, &bad)), ErrorCode::InvalidInput);
        let mut bad = input(account, category, "2026-11-05");
        bad.amount_minor = "0".into();
        assert_eq!(code(create(&mut db, &bad)), ErrorCode::InvalidInput);
        let mut bad = input(account, category, "2026-11-05");
        bad.kind = "income".into(); // category is an expense category
        assert_eq!(code(create(&mut db, &bad)), ErrorCode::InvalidInput);
    }

    #[test]
    fn catch_up_records_missed_occurrences_once() {
        let (mut db, account, category) = setup();
        create(&mut db, &input(account, category, "2026-07-14")).unwrap();
        let first = catch_up(&mut db, "2026-10-14", NOW).unwrap();
        assert_eq!(first.transactions_created, 4); // Jul, Aug, Sep, Oct 14
        let second = catch_up(&mut db, "2026-10-14", NOW).unwrap();
        assert_eq!(
            second,
            CatchUpDto {
                rules_advanced: 0,
                transactions_created: 0,
                rules_finished: 0,
                rules_invalid: 0
            }
        );
        assert_eq!(list(&db).unwrap()[0].next_due, "2026-11-14");
    }

    #[test]
    fn catch_up_refuses_an_implausible_date() {
        let (mut db, account, category) = setup();
        create(&mut db, &input(account, category, "2026-01-01")).unwrap();
        assert_eq!(
            catch_up(&mut db, "2030-01-01", NOW).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            db.transactions(&Default::default(), None, 10)
                .unwrap()
                .items
                .len(),
            0
        );
    }

    #[test]
    fn pause_resume_and_delete() {
        let (mut db, account, category) = setup();
        let id = create(&mut db, &input(account, category, "2026-07-14"))
            .unwrap()
            .id;
        assert!(
            !set_active(&mut db, id, false, "2026-10-14", NOW)
                .unwrap()
                .active
        );
        let resumed = set_active(&mut db, id, true, "2026-10-14", NOW).unwrap();
        assert!(resumed.active);
        assert_eq!(resumed.next_due, "2026-10-14");
        delete(&mut db, id).unwrap();
        assert!(list(&db).unwrap().is_empty());
        assert_eq!(delete(&mut db, id).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn edit_changes_the_amount_but_not_the_schedule() {
        let (mut db, account, category) = setup();
        let id = create(&mut db, &input(account, category, "2026-11-05"))
            .unwrap()
            .id;
        let dto = update(
            &mut db,
            id,
            &RecurringEditInput {
                account_id: account,
                category_id: category,
                amount_minor: "175000".into(),
                note: "naik".into(),
                end_on: Some("2027-05-05".into()),
            },
        )
        .unwrap();
        assert_eq!(
            (dto.amount_minor.as_str(), dto.note.as_str()),
            ("175000", "naik")
        );
        assert_eq!(dto.end_on.as_deref(), Some("2027-05-05"));
        assert_eq!(
            (dto.start_on.as_str(), dto.frequency),
            ("2026-11-05", "monthly")
        );
    }
}
