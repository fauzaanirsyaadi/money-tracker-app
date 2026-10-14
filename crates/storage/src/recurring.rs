//! Recurring-transaction catch-up, run when the app opens.
//!
//! No background service exists: this is the only place recurring rules turn
//! into transactions. Everything happens in one `IMMEDIATE` transaction, so a
//! crash leaves either all new transactions and advanced due dates, or none.

use mt_core::{Date, Frequency, Schedule, format_iso_date, parse_iso_date};
use rusqlite::{TransactionBehavior, params};

use crate::{db::Db, error::StorageError};

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

#[cfg(test)]
mod tests {
    use zeroize::Zeroizing;

    use super::*;

    fn d(text: &str) -> Date {
        parse_iso_date(text).unwrap()
    }

    fn db() -> Db {
        let db = Db::open_in_memory(&Zeroizing::new([9u8; 32])).unwrap();
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
