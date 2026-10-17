//! Recurring-transaction catch-up, run when the app opens.
//!
//! No background service exists: this is the only place recurring rules turn
//! into transactions. Everything happens in one `IMMEDIATE` transaction, so a
//! crash leaves either all new transactions and advanced due dates, or none.

use mt_core::{
    Currency, Date, FlowKind, Frequency, Money, RecurringDraft, RecurringEdit, Schedule,
    format_iso_date, parse_iso_date,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::{
    db::Db,
    error::StorageError,
    transactions::{check_account, check_category},
};

/// Counts only; never amounts or notes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CatchUpReport {
    /// Rules that had at least one occurrence processed.
    pub rules_advanced: u32,
    /// Transactions created.
    pub transactions_created: u32,
    /// Rules that reached their end date and were deactivated.
    pub rules_finished: u32,
    /// Rules skipped because their stored state is inconsistent.
    pub rules_invalid: u32,
}

struct RuleRow {
    id: i64,
    account_id: i64,
    category_id: i64,
    kind: String,
    amount_minor: i64,
    note: String,
    freq: String,
    every_n: i64,
    anchor_day: i64,
    start_on: String,
    end_on: Option<String>,
    next_due: String,
}

impl Db {
    /// Creates every recurring transaction that became due up to `today`.
    ///
    /// * `now_unix` is stored as `created_at` (injected so tests are exact).
    /// * `limit_per_rule` caps work per rule per call (the architecture uses
    ///   1000); a rule that hits it simply continues on the next app open.
    /// * Rules on archived accounts or categories are left untouched.
    pub fn catch_up_recurring(
        &mut self,
        today: Date,
        now_unix: i64,
        limit_per_rule: usize,
    ) -> Result<CatchUpReport, StorageError> {
        let today_text = format_iso_date(today);
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut report = CatchUpReport::default();

        let rules: Vec<RuleRow> = {
            let mut statement = tx.prepare(
                "SELECT r.id, r.account_id, r.category_id, r.kind, r.amount_minor, r.note,
                        r.freq, r.every_n, r.anchor_day, r.start_on, r.end_on, r.next_due
                 FROM recurring_rules r
                 JOIN accounts a   ON a.id = r.account_id  AND a.archived = 0
                 JOIN categories c ON c.id = r.category_id AND c.archived = 0
                 WHERE r.active = 1 AND r.next_due <= ?1
                 ORDER BY r.id",
            )?;
            let rows = statement.query_map(params![today_text], |row| {
                Ok(RuleRow {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    category_id: row.get(2)?,
                    kind: row.get(3)?,
                    amount_minor: row.get(4)?,
                    note: row.get(5)?,
                    freq: row.get(6)?,
                    every_n: row.get(7)?,
                    anchor_day: row.get(8)?,
                    start_on: row.get(9)?,
                    end_on: row.get(10)?,
                    next_due: row.get(11)?,
                })
            })?;
            rows.collect::<Result<_, _>>()?
        };

        for rule in rules {
            let Some((schedule, next_due)) = schedule_of(&rule) else {
                report.rules_invalid += 1;
                continue;
            };
            let Ok(result) = schedule.catch_up(next_due, today, limit_per_rule) else {
                report.rules_invalid += 1;
                continue;
            };

            for date in &result.due {
                // OR IGNORE: the unique (rule, date) index makes a repeat a no-op.
                let created = tx.execute(
                    "INSERT OR IGNORE INTO transactions
                       (account_id, category_id, amount_minor, kind, note, occurred_on,
                        recurring_rule_id, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        rule.account_id,
                        rule.category_id,
                        rule.amount_minor,
                        rule.kind,
                        rule.note,
                        format_iso_date(*date),
                        rule.id,
                        now_unix
                    ],
                )?;
                report.transactions_created += u32::try_from(created).unwrap_or(0);
            }

            match result.next_due {
                Some(next) => {
                    tx.execute(
                        "UPDATE recurring_rules SET next_due = ?1 WHERE id = ?2",
                        params![format_iso_date(next), rule.id],
                    )?;
                }
                None => {
                    tx.execute(
                        "UPDATE recurring_rules SET active = 0 WHERE id = ?1",
                        params![rule.id],
                    )?;
                    report.rules_finished += 1;
                }
            }
            if !result.due.is_empty() || result.next_due.is_none() {
                report.rules_advanced += 1;
            }
        }

        tx.commit()?;
        Ok(report)
    }
}

/// A recurring rule as shown to the user.
#[derive(Clone, PartialEq, Eq)]
pub struct RecurringRecord {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub category_id: i64,
    pub category_name: String,
    pub kind: FlowKind,
    /// The positive magnitude of each occurrence.
    pub amount: Money,
    pub note: String,
    pub frequency: Frequency,
    pub every: u32,
    pub start_on: Date,
    pub end_on: Option<Date>,
    pub next_due: Date,
    pub active: bool,
}

impl Db {
    /// Creates a rule that first fires on its start date. Returns its id.
    pub fn create_recurring(&mut self, draft: &RecurringDraft) -> Result<i64, StorageError> {
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_account(&tx, draft.account_id(), draft.amount().currency(), None)?;
        check_category(&tx, draft.category_id(), draft.kind(), None)?;
        let schedule = draft.schedule();
        tx.execute(
            "INSERT INTO recurring_rules
               (account_id, category_id, kind, amount_minor, note, freq, every_n, anchor_day,
                start_on, end_on, next_due, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?9, 1)",
            params![
                draft.account_id(),
                draft.category_id(),
                draft.kind().code(),
                draft.signed_amount().minor(),
                draft.note(),
                schedule.frequency().code(),
                schedule.every(),
                schedule.anchor_day(),
                format_iso_date(schedule.start()),
                schedule.end().map(format_iso_date),
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Changes account, category, amount, note and end date. The schedule and
    /// kind are fixed; to change them, delete the rule and create a new one.
    pub fn update_recurring(&mut self, id: i64, edit: &RecurringEdit) -> Result<(), StorageError> {
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, i64, i64, String)> = tx
            .query_row(
                "SELECT kind, account_id, category_id, start_on FROM recurring_rules WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let (kind, old_account, old_category, start_on) = existing.ok_or(StorageError::NotFound)?;
        let kind = FlowKind::from_code(&kind).ok_or(StorageError::InvalidData)?;
        let start = parse_iso_date(&start_on).map_err(|_| StorageError::InvalidData)?;
        if edit.end_on().is_some_and(|end| end < start) {
            return Err(StorageError::Constraint);
        }
        check_account(
            &tx,
            edit.account_id(),
            edit.amount().currency(),
            Some(old_account),
        )?;
        check_category(&tx, edit.category_id(), kind, Some(old_category))?;
        let signed = match kind {
            FlowKind::Income => edit.amount().minor(),
            FlowKind::Expense => -edit.amount().minor(),
        };
        tx.execute(
            "UPDATE recurring_rules
                SET account_id = ?1, category_id = ?2, amount_minor = ?3, note = ?4, end_on = ?5
              WHERE id = ?6",
            params![
                edit.account_id(),
                edit.category_id(),
                signed,
                edit.note(),
                edit.end_on().map(format_iso_date),
                id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Pauses or resumes a rule. Resuming skips whatever fell due during the
    /// pause: the next occurrence on or after `today` becomes the due date, so
    /// a long break does not back-fill transactions the user chose not to have.
    pub fn set_recurring_active(
        &mut self,
        id: i64,
        active: bool,
        today: Date,
    ) -> Result<(), StorageError> {
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<RuleRow> = tx
            .query_row(RULE_SELECT_BY_ID, params![id], read_rule)
            .optional()?;
        let rule = row.ok_or(StorageError::NotFound)?;
        if !active {
            tx.execute(
                "UPDATE recurring_rules SET active = 0 WHERE id = ?1",
                params![id],
            )?;
            tx.commit()?;
            return Ok(());
        }
        let (schedule, next_due) = schedule_of(&rule).ok_or(StorageError::InvalidData)?;
        let resume = if next_due >= today {
            next_due
        } else {
            // A finished rule has no next occurrence and cannot be resumed.
            schedule
                .first_on_or_after(today)
                .ok_or(StorageError::Constraint)?
        };
        tx.execute(
            "UPDATE recurring_rules SET active = 1, next_due = ?1 WHERE id = ?2",
            params![format_iso_date(resume), id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes a rule. Transactions it already created stay (unlinked).
    pub fn delete_recurring(&mut self, id: i64) -> Result<(), StorageError> {
        let removed = self
            .conn()
            .execute("DELETE FROM recurring_rules WHERE id = ?1", params![id])?;
        if removed == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    /// All rules: active first, then by next due date.
    pub fn recurring_rules(&self) -> Result<Vec<RecurringRecord>, StorageError> {
        let currency: Currency = self.app_currency()?;
        let mut statement = self.conn().prepare(
            "SELECT r.id, r.account_id, r.category_id, r.kind, r.amount_minor, r.note,
                    r.freq, r.every_n, r.anchor_day, r.start_on, r.end_on, r.next_due,
                    r.active, a.name, c.name
             FROM recurring_rules r
             JOIN accounts a   ON a.id = r.account_id
             JOIN categories c ON c.id = r.category_id
             ORDER BY r.active DESC, r.next_due, r.id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                RuleRow {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    category_id: row.get(2)?,
                    kind: row.get(3)?,
                    amount_minor: row.get(4)?,
                    note: row.get(5)?,
                    freq: row.get(6)?,
                    every_n: row.get(7)?,
                    anchor_day: row.get(8)?,
                    start_on: row.get(9)?,
                    end_on: row.get(10)?,
                    next_due: row.get(11)?,
                },
                row.get::<_, i64>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
            ))
        })?;
        rows.map(|row| {
            let (rule, active, account_name, category_name) = row?;
            let bad = || StorageError::InvalidData;
            Ok(RecurringRecord {
                id: rule.id,
                account_id: rule.account_id,
                account_name,
                category_id: rule.category_id,
                category_name,
                kind: FlowKind::from_code(&rule.kind).ok_or_else(bad)?,
                amount: Money::from_minor(rule.amount_minor.abs(), currency).map_err(|_| bad())?,
                note: rule.note,
                frequency: Frequency::from_code(&rule.freq).ok_or_else(bad)?,
                every: u32::try_from(rule.every_n).map_err(|_| bad())?,
                start_on: parse_iso_date(&rule.start_on).map_err(|_| bad())?,
                end_on: match &rule.end_on {
                    Some(text) => Some(parse_iso_date(text).map_err(|_| bad())?),
                    None => None,
                },
                next_due: parse_iso_date(&rule.next_due).map_err(|_| bad())?,
                active: active != 0,
            })
        })
        .collect()
    }
}

const RULE_SELECT_BY_ID: &str = "SELECT id, account_id, category_id, kind, amount_minor, note,
        freq, every_n, anchor_day, start_on, end_on, next_due
     FROM recurring_rules WHERE id = ?1";

fn read_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuleRow> {
    Ok(RuleRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        category_id: row.get(2)?,
        kind: row.get(3)?,
        amount_minor: row.get(4)?,
        note: row.get(5)?,
        freq: row.get(6)?,
        every_n: row.get(7)?,
        anchor_day: row.get(8)?,
        start_on: row.get(9)?,
        end_on: row.get(10)?,
        next_due: row.get(11)?,
    })
}

fn schedule_of(rule: &RuleRow) -> Option<(Schedule, Date)> {
    let schedule = Schedule::new(
        Frequency::from_code(&rule.freq)?,
        u32::try_from(rule.every_n).ok()?,
        u8::try_from(rule.anchor_day).ok()?,
        parse_iso_date(&rule.start_on).ok()?,
        match &rule.end_on {
            Some(text) => Some(parse_iso_date(text).ok()?),
            None => None,
        },
    )
    .ok()?;
    Some((schedule, parse_iso_date(&rule.next_due).ok()?))
}

// Amounts, names and notes must never reach a log through `{:?}`.
mt_core::redacted_debug!(RecurringRecord);

#[cfg(test)]
mod tests {
    use crate::DbKey;

    use super::*;

    fn d(text: &str) -> Date {
        parse_iso_date(text).unwrap()
    }

    fn db() -> Db {
        let db = Db::open_in_memory(&DbKey::new([9u8; 32])).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO accounts(name,kind,currency,created_at) VALUES('Cash','cash','IDR',0);
                 INSERT INTO categories(name,kind) VALUES('Rent','expense');",
            )
            .unwrap();
        db
    }

    fn add_rule(db: &Db, freq: &str, anchor: i64, start: &str, end: Option<&str>) {
        let end = end.map_or("NULL".to_owned(), |e| format!("'{e}'"));
        db.conn()
            .execute_batch(&format!(
                "INSERT INTO recurring_rules(account_id,category_id,kind,amount_minor,note,freq,
                                             anchor_day,start_on,end_on,next_due)
                 VALUES(1,1,'expense',-1500000,'rent','{freq}',{anchor},'{start}',{end},'{start}');"
            ))
            .unwrap();
    }

    fn count(db: &Db, sql: &str) -> i64 {
        db.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn next_due(db: &Db) -> String {
        db.conn()
            .query_row(
                "SELECT next_due FROM recurring_rules WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn creates_missed_occurrences_and_advances_the_rule() {
        let mut db = db();
        add_rule(&db, "monthly", 31, "2026-01-31", None);
        let report = db.catch_up_recurring(d("2026-04-15"), 1_000, 1000).unwrap();
        assert_eq!(report.transactions_created, 3);
        assert_eq!(report.rules_advanced, 1);
        let dates: Vec<String> = db
            .conn()
            .prepare("SELECT occurred_on FROM transactions ORDER BY occurred_on")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(dates, ["2026-01-31", "2026-02-28", "2026-03-31"]);
        assert_eq!(next_due(&db), "2026-04-30");
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM transactions WHERE recurring_rule_id = 1 AND created_at = 1000"
            ),
            3
        );
    }

    #[test]
    fn created_rows_keep_amount_kind_and_category() {
        let mut db = db();
        add_rule(&db, "daily", 1, "2026-10-14", None);
        db.catch_up_recurring(d("2026-10-14"), 5, 10).unwrap();
        let (amount, kind, note, category): (i64, String, String, i64) = db
            .conn()
            .query_row(
                "SELECT amount_minor, kind, note, category_id FROM transactions",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (amount, kind.as_str(), note.as_str(), category),
            (-1_500_000, "expense", "rent", 1)
        );
    }

    #[test]
    fn a_second_run_creates_nothing() {
        let mut db = db();
        add_rule(&db, "monthly", 15, "2026-01-15", None);
        let first = db.catch_up_recurring(d("2026-04-20"), 1, 1000).unwrap();
        assert_eq!(first.transactions_created, 4);
        let second = db.catch_up_recurring(d("2026-04-20"), 2, 1000).unwrap();
        assert_eq!(second, CatchUpReport::default());
        assert_eq!(count(&db, "SELECT count(*) FROM transactions"), 4);
    }

    #[test]
    fn does_nothing_before_the_due_date() {
        let mut db = db();
        add_rule(&db, "monthly", 15, "2026-06-15", None);
        let report = db.catch_up_recurring(d("2026-06-14"), 1, 1000).unwrap();
        assert_eq!(report, CatchUpReport::default());
        assert_eq!(next_due(&db), "2026-06-15");
    }

    #[test]
    fn limit_spreads_a_long_backlog_over_several_opens() {
        let mut db = db();
        add_rule(&db, "daily", 1, "2026-01-01", None);
        let today = d("2026-01-25"); // 25 occurrences
        assert_eq!(
            db.catch_up_recurring(today, 1, 10)
                .unwrap()
                .transactions_created,
            10
        );
        assert_eq!(next_due(&db), "2026-01-11");
        assert_eq!(
            db.catch_up_recurring(today, 1, 10)
                .unwrap()
                .transactions_created,
            10
        );
        assert_eq!(
            db.catch_up_recurring(today, 1, 10)
                .unwrap()
                .transactions_created,
            5
        );
        assert_eq!(count(&db, "SELECT count(*) FROM transactions"), 25);
        assert_eq!(next_due(&db), "2026-01-26");
    }

    #[test]
    fn an_ended_rule_is_deactivated() {
        let mut db = db();
        add_rule(&db, "monthly", 15, "2026-01-15", Some("2026-03-15"));
        let report = db.catch_up_recurring(d("2026-12-31"), 1, 1000).unwrap();
        assert_eq!(report.transactions_created, 3);
        assert_eq!(report.rules_finished, 1);
        assert_eq!(
            count(&db, "SELECT active FROM recurring_rules WHERE id = 1"),
            0
        );
        // And it stays quiet afterwards.
        assert_eq!(
            db.catch_up_recurring(d("2027-01-01"), 1, 1000).unwrap(),
            CatchUpReport::default()
        );
    }

    #[test]
    fn archived_accounts_and_categories_are_skipped() {
        let mut db = db();
        add_rule(&db, "daily", 1, "2026-01-01", None);
        db.conn()
            .execute_batch("UPDATE accounts SET archived = 1")
            .unwrap();
        assert_eq!(
            db.catch_up_recurring(d("2026-01-10"), 1, 100).unwrap(),
            CatchUpReport::default()
        );
        assert_eq!(count(&db, "SELECT count(*) FROM transactions"), 0);
        // Unarchiving lets the backlog flow in.
        db.conn()
            .execute_batch("UPDATE accounts SET archived = 0")
            .unwrap();
        assert_eq!(
            db.catch_up_recurring(d("2026-01-10"), 1, 100)
                .unwrap()
                .transactions_created,
            10
        );
        db.conn()
            .execute_batch("UPDATE categories SET archived = 1")
            .unwrap();
        assert_eq!(
            db.catch_up_recurring(d("2026-02-10"), 1, 100).unwrap(),
            CatchUpReport::default()
        );
    }

    #[test]
    fn an_inconsistent_rule_is_reported_and_does_not_block_others() {
        let mut db = db();
        add_rule(&db, "monthly", 15, "2026-01-15", None);
        // next_due not on the schedule (the 16th).
        db.conn()
            .execute_batch("UPDATE recurring_rules SET next_due = '2026-02-16' WHERE id = 1;")
            .unwrap();
        add_rule(&db, "daily", 1, "2026-03-01", None);
        let report = db.catch_up_recurring(d("2026-03-03"), 1, 100).unwrap();
        assert_eq!(report.rules_invalid, 1);
        assert_eq!(report.transactions_created, 3);
        assert_eq!(next_due(&db), "2026-02-16"); // untouched
    }

    #[test]
    fn a_pre_existing_transaction_is_not_duplicated() {
        let mut db = db();
        add_rule(&db, "monthly", 15, "2026-01-15", None);
        db.conn()
            .execute_batch(
                "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,
                                          recurring_rule_id,created_at)
                 VALUES(1,1,-1500000,'expense','2026-01-15',1,0);",
            )
            .unwrap();
        let report = db.catch_up_recurring(d("2026-02-20"), 1, 100).unwrap();
        assert_eq!(report.transactions_created, 1); // only February
        assert_eq!(count(&db, "SELECT count(*) FROM transactions"), 2);
        assert_eq!(next_due(&db), "2026-03-15");
    }
}

#[cfg(test)]
mod crud_tests {
    use mt_core::{AccountDraft, AccountKind, CategoryDraft};

    use crate::DbKey;

    use super::*;

    struct Fx {
        db: Db,
        cash: i64,
        bank: i64,
        food: i64,
        rent: i64,
        salary: i64,
    }

    fn d(text: &str) -> Date {
        parse_iso_date(text).unwrap()
    }

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::IDR).unwrap()
    }

    fn fx() -> Fx {
        let mut db = Db::open_in_memory(&DbKey::new([5u8; 32])).unwrap();
        let mut account = |name: &str, kind| {
            db.create_account(&AccountDraft::new(name, kind, idr(0)).unwrap(), 0)
                .unwrap()
        };
        let cash = account("Tunai", AccountKind::Cash);
        let bank = account("BCA", AccountKind::Bank);
        let find = |db: &Db, name: &str, kind: FlowKind| {
            db.categories(false)
                .unwrap()
                .into_iter()
                .find(|c| c.name == name && c.kind == kind)
                .unwrap()
                .id
        };
        let food = find(&db, "Makanan", FlowKind::Expense);
        let salary = find(&db, "Gaji", FlowKind::Income);
        let rent = db
            .create_category(&CategoryDraft::new("Sewa", FlowKind::Expense, None).unwrap())
            .unwrap();
        Fx {
            db,
            cash,
            bank,
            food,
            rent,
            salary,
        }
    }

    fn rent_rule(f: &mut Fx, start: &str, end: Option<&str>) -> i64 {
        let draft = RecurringDraft::new(
            f.cash,
            f.rent,
            FlowKind::Expense,
            idr(1_500_000),
            "kos",
            Frequency::Monthly,
            1,
            d(start),
            end.map(d),
        )
        .unwrap();
        f.db.create_recurring(&draft).unwrap()
    }

    #[test]
    fn a_new_rule_is_active_and_due_on_its_start_date() {
        let mut f = fx();
        let id = rent_rule(&mut f, "2026-11-05", None);
        let rules = f.db.recurring_rules().unwrap();
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert_eq!(rule.id, id);
        assert_eq!(
            (rule.kind, rule.amount, rule.note.as_str()),
            (FlowKind::Expense, idr(1_500_000), "kos")
        );
        assert_eq!((rule.frequency, rule.every), (Frequency::Monthly, 1));
        assert_eq!(
            (rule.start_on, rule.next_due),
            (d("2026-11-05"), d("2026-11-05"))
        );
        assert!(rule.active);
        assert_eq!(
            (rule.account_name.as_str(), rule.category_name.as_str()),
            ("Tunai", "Sewa")
        );
    }

    #[test]
    fn income_rules_store_positive_amounts_and_report_magnitudes() {
        let mut f = fx();
        let draft = RecurringDraft::new(
            f.bank,
            f.salary,
            FlowKind::Income,
            idr(8_000_000),
            "",
            Frequency::Monthly,
            1,
            d("2026-10-25"),
            None,
        )
        .unwrap();
        f.db.create_recurring(&draft).unwrap();
        let stored: i64 =
            f.db.conn()
                .query_row("SELECT amount_minor FROM recurring_rules", [], |r| r.get(0))
                .unwrap();
        assert_eq!(stored, 8_000_000);
        assert_eq!(f.db.recurring_rules().unwrap()[0].amount, idr(8_000_000));
    }

    #[test]
    fn creation_validates_account_and_category() {
        let mut f = fx();
        let make = |account, category, kind| {
            RecurringDraft::new(
                account,
                category,
                kind,
                idr(1),
                "",
                Frequency::Daily,
                1,
                d("2026-10-14"),
                None,
            )
            .unwrap()
        };
        assert_eq!(
            f.db.create_recurring(&make(f.cash, f.salary, FlowKind::Expense)),
            Err(StorageError::KindMismatch)
        );
        assert_eq!(
            f.db.create_recurring(&make(999, f.food, FlowKind::Expense)),
            Err(StorageError::NotFound)
        );
        f.db.set_account_archived(f.bank, true).unwrap();
        assert_eq!(
            f.db.create_recurring(&make(f.bank, f.food, FlowKind::Expense)),
            Err(StorageError::Archived)
        );
        assert!(f.db.recurring_rules().unwrap().is_empty());
    }

    #[test]
    fn update_changes_editable_fields_and_keeps_the_schedule() {
        let mut f = fx();
        let id = rent_rule(&mut f, "2026-11-05", None);
        let edit = RecurringEdit::new(
            f.bank,
            f.rent,
            idr(1_750_000),
            " naik ",
            Some(d("2027-06-05")),
        )
        .unwrap();
        f.db.update_recurring(id, &edit).unwrap();
        let rule = &f.db.recurring_rules().unwrap()[0];
        assert_eq!(
            (rule.account_id, rule.amount, rule.note.as_str()),
            (f.bank, idr(1_750_000), "naik")
        );
        assert_eq!(rule.end_on, Some(d("2027-06-05")));
        assert_eq!(
            (rule.start_on, rule.next_due, rule.frequency),
            (d("2026-11-05"), d("2026-11-05"), Frequency::Monthly)
        );
        let stored: i64 =
            f.db.conn()
                .query_row("SELECT amount_minor FROM recurring_rules", [], |r| r.get(0))
                .unwrap();
        assert_eq!(stored, -1_750_000);
    }

    #[test]
    fn update_rejects_a_wrong_category_kind_an_early_end_and_unknown_ids() {
        let mut f = fx();
        let id = rent_rule(&mut f, "2026-11-05", None);
        let wrong_kind = RecurringEdit::new(f.cash, f.salary, idr(1), "", None).unwrap();
        assert_eq!(
            f.db.update_recurring(id, &wrong_kind),
            Err(StorageError::KindMismatch)
        );
        let early = RecurringEdit::new(f.cash, f.rent, idr(1), "", Some(d("2026-11-04"))).unwrap();
        assert_eq!(
            f.db.update_recurring(id, &early),
            Err(StorageError::Constraint)
        );
        let ok = RecurringEdit::new(f.cash, f.rent, idr(1), "", None).unwrap();
        assert_eq!(f.db.update_recurring(999, &ok), Err(StorageError::NotFound));
    }

    #[test]
    fn pausing_stops_generation_and_resuming_skips_the_gap() {
        let mut f = fx();
        let id = rent_rule(&mut f, "2026-01-15", None);
        // Caught up through March, then paused.
        f.db.catch_up_recurring(d("2026-03-20"), 1, 1000).unwrap();
        assert_eq!(f.db.recurring_rules().unwrap()[0].next_due, d("2026-04-15"));
        f.db.set_recurring_active(id, false, d("2026-03-20"))
            .unwrap();
        assert_eq!(
            f.db.catch_up_recurring(d("2026-09-01"), 1, 1000).unwrap(),
            CatchUpReport::default()
        );

        // Resume in October: April..September are not back-filled.
        f.db.set_recurring_active(id, true, d("2026-10-14"))
            .unwrap();
        let rule = &f.db.recurring_rules().unwrap()[0];
        assert!(rule.active);
        assert_eq!(rule.next_due, d("2026-10-15"));
        let report = f.db.catch_up_recurring(d("2026-10-14"), 1, 1000).unwrap();
        assert_eq!(report.transactions_created, 0);
        assert_eq!(
            f.db.catch_up_recurring(d("2026-10-15"), 1, 1000)
                .unwrap()
                .transactions_created,
            1
        );
    }

    #[test]
    fn resuming_before_the_due_date_keeps_it() {
        let mut f = fx();
        let id = rent_rule(&mut f, "2026-12-01", None);
        f.db.set_recurring_active(id, false, d("2026-10-01"))
            .unwrap();
        f.db.set_recurring_active(id, true, d("2026-10-01"))
            .unwrap();
        assert_eq!(f.db.recurring_rules().unwrap()[0].next_due, d("2026-12-01"));
    }

    #[test]
    fn a_finished_rule_cannot_be_resumed() {
        let mut f = fx();
        let id = rent_rule(&mut f, "2026-01-15", Some("2026-02-15"));
        f.db.catch_up_recurring(d("2026-12-31"), 1, 1000).unwrap();
        assert!(!f.db.recurring_rules().unwrap()[0].active);
        assert_eq!(
            f.db.set_recurring_active(id, true, d("2026-12-31")),
            Err(StorageError::Constraint)
        );
    }

    #[test]
    fn deleting_a_rule_keeps_the_transactions_it_created() {
        let mut f = fx();
        let id = rent_rule(&mut f, "2026-01-15", None);
        f.db.catch_up_recurring(d("2026-03-20"), 1, 1000).unwrap();
        f.db.delete_recurring(id).unwrap();
        assert!(f.db.recurring_rules().unwrap().is_empty());
        let (count, linked): (i64, i64) =
            f.db.conn()
                .query_row(
                    "SELECT count(*), count(recurring_rule_id) FROM transactions",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
        assert_eq!((count, linked), (3, 0));
        assert_eq!(f.db.delete_recurring(id), Err(StorageError::NotFound));
    }

    #[test]
    fn rules_list_active_first_then_by_due_date() {
        let mut f = fx();
        let late = rent_rule(&mut f, "2026-12-01", None);
        let early = rent_rule(&mut f, "2026-11-01", None);
        let paused = rent_rule(&mut f, "2026-10-01", None);
        f.db.set_recurring_active(paused, false, d("2026-09-01"))
            .unwrap();
        let order: Vec<i64> =
            f.db.recurring_rules()
                .unwrap()
                .iter()
                .map(|r| r.id)
                .collect();
        assert_eq!(order, [early, late, paused]);
    }
}
