//! Monthly budgets per expense category, with progress computed from the
//! month's actual spending.
//!
//! Spending for a budget on a parent category includes its sub-categories, so
//! a "Makanan" budget also counts "Kopi". Only expenses count; transfers never
//! do (they only move money between the user's own accounts).

use mt_core::{
    BudgetDraft, BudgetProgress, Currency, FlowKind, Money, YearMonth, format_iso_date, progress,
};
use rusqlite::{OptionalExtension, params};

use crate::{db::Db, error::StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetRecord {
    pub id: i64,
    pub category_id: i64,
    pub category_name: String,
    pub month: YearMonth,
    pub progress: BudgetProgress,
}

impl Db {
    /// Sets the limit for a category and month, replacing any earlier one.
    /// Returns the budget's id.
    pub fn set_budget(&mut self, draft: &BudgetDraft) -> Result<i64, StorageError> {
        if draft.limit().currency() != self.app_currency()? {
            return Err(StorageError::CurrencyMismatch);
        }
        let tx = self.conn_mut().transaction()?;
        let category: Option<(String, i64)> = tx
            .query_row(
                "SELECT kind, archived FROM categories WHERE id = ?1",
                params![draft.category_id()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (kind, archived) = category.ok_or(StorageError::NotFound)?;
        if kind != FlowKind::Expense.code() {
            return Err(StorageError::KindMismatch);
        }
        if archived != 0 {
            return Err(StorageError::Archived);
        }
        tx.execute(
            "INSERT INTO budgets(category_id, month, limit_minor) VALUES (?1, ?2, ?3)
             ON CONFLICT(category_id, month) DO UPDATE SET limit_minor = excluded.limit_minor",
            params![
                draft.category_id(),
                draft.month().to_string(),
                draft.limit().minor()
            ],
        )?;
        let id = tx.query_row(
            "SELECT id FROM budgets WHERE category_id = ?1 AND month = ?2",
            params![draft.category_id(), draft.month().to_string()],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn delete_budget(&mut self, id: i64) -> Result<(), StorageError> {
        let removed = self
            .conn()
            .execute("DELETE FROM budgets WHERE id = ?1", params![id])?;
        if removed == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    /// Budgets of one month with their progress, ordered by category name.
    pub fn budgets(&self, month: YearMonth) -> Result<Vec<BudgetRecord>, StorageError> {
        let currency: Currency = self.app_currency()?;
        let mut statement = self.conn().prepare(
            "SELECT b.id, b.category_id, c.name, b.limit_minor,
                    COALESCE((
                      SELECT SUM(-t.amount_minor) FROM transactions t
                      WHERE t.kind = 'expense'
                        AND t.occurred_on >= ?1 AND t.occurred_on <= ?2
                        AND (t.category_id = b.category_id
                             OR t.category_id IN
                                (SELECT id FROM categories WHERE parent_id = b.category_id))
                    ), 0)
             FROM budgets b
             JOIN categories c ON c.id = b.category_id
             WHERE b.month = ?3
             ORDER BY c.name, b.id",
        )?;
        let rows = statement.query_map(
            params![
                format_iso_date(month.first_day()),
                format_iso_date(month.last_day()),
                month.to_string()
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )?;
        rows.map(|row| {
            let (id, category_id, category_name, limit, spent) = row?;
            let money =
                |minor| Money::from_minor(minor, currency).map_err(|_| StorageError::InvalidData);
            Ok(BudgetRecord {
                id,
                category_id,
                category_name,
                month,
                progress: progress(money(limit)?, money(spent)?)
                    .map_err(|_| StorageError::InvalidData)?,
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use mt_core::{
        AccountDraft, AccountKind, BudgetStatus, CategoryDraft, TransactionDraft, TransferDraft,
        parse_iso_date,
    };
    use zeroize::Zeroizing;

    use super::*;

    struct Fx {
        db: Db,
        cash: i64,
        bank: i64,
        food: i64,
        coffee: i64,
        transport: i64,
        salary: i64,
    }

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::IDR).unwrap()
    }

    fn month(text: &str) -> YearMonth {
        YearMonth::parse(text).unwrap()
    }

    fn fx() -> Fx {
        let mut db = Db::open_in_memory(&Zeroizing::new([7u8; 32])).unwrap();
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
        let transport = find(&db, "Transportasi", FlowKind::Expense);
        let salary = find(&db, "Gaji", FlowKind::Income);
        let coffee = db
            .create_category(&CategoryDraft::new("Kopi", FlowKind::Expense, Some(food)).unwrap())
            .unwrap();
        Fx {
            db,
            cash,
            bank,
            food,
            coffee,
            transport,
            salary,
        }
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

    fn budget(category: i64, m: &str, limit: i64) -> BudgetDraft {
        BudgetDraft::new(category, month(m), idr(limit)).unwrap()
    }

    #[test]
    fn progress_follows_spending_in_the_month_only() {
        let mut f = fx();
        f.db.set_budget(&budget(f.food, "2026-10", 1_000_000))
            .unwrap();
        spend(&mut f.db, f.cash, f.food, 250_000, "2026-10-01");
        spend(&mut f.db, f.bank, f.food, 150_000, "2026-10-31"); // last day counts
        spend(&mut f.db, f.cash, f.food, 999_999, "2026-09-30"); // other months do not
        spend(&mut f.db, f.cash, f.food, 999_999, "2026-11-01");
        let list = f.db.budgets(month("2026-10")).unwrap();
        assert_eq!(list.len(), 1);
        let p = list[0].progress;
        assert_eq!(p.spent, idr(400_000));
        assert_eq!(p.remaining, idr(600_000));
        assert_eq!((p.used_bp, p.status), (4_000, BudgetStatus::Ok));
        assert_eq!(list[0].category_name, "Makanan");
    }

    #[test]
    fn a_parent_budget_counts_its_sub_categories() {
        let mut f = fx();
        f.db.set_budget(&budget(f.food, "2026-10", 100_000))
            .unwrap();
        spend(&mut f.db, f.cash, f.food, 30_000, "2026-10-02");
        spend(&mut f.db, f.cash, f.coffee, 25_000, "2026-10-03");
        spend(&mut f.db, f.cash, f.transport, 99_000, "2026-10-03");
        let p = f.db.budgets(month("2026-10")).unwrap()[0].progress;
        assert_eq!(p.spent, idr(55_000));
    }

    #[test]
    fn a_child_budget_does_not_count_its_parent_spending() {
        let mut f = fx();
        f.db.set_budget(&budget(f.coffee, "2026-10", 50_000))
            .unwrap();
        spend(&mut f.db, f.cash, f.food, 30_000, "2026-10-02");
        spend(&mut f.db, f.cash, f.coffee, 10_000, "2026-10-03");
        assert_eq!(
            f.db.budgets(month("2026-10")).unwrap()[0].progress.spent,
            idr(10_000)
        );
    }

    #[test]
    fn income_and_transfers_never_count_as_spending() {
        let mut f = fx();
        f.db.set_budget(&budget(f.food, "2026-10", 100_000))
            .unwrap();
        let income = TransactionDraft::new(
            f.cash,
            f.salary,
            FlowKind::Income,
            idr(5_000_000),
            "",
            parse_iso_date("2026-10-01").unwrap(),
        )
        .unwrap();
        f.db.create_transaction(&income, 0).unwrap();
        let transfer = TransferDraft::new(
            f.cash,
            f.bank,
            idr(70_000),
            "",
            parse_iso_date("2026-10-02").unwrap(),
        )
        .unwrap();
        f.db.create_transfer(&transfer, 0).unwrap();
        assert_eq!(
            f.db.budgets(month("2026-10")).unwrap()[0].progress.spent,
            idr(0)
        );
    }

    #[test]
    fn warning_and_over_follow_the_thresholds() {
        let mut f = fx();
        f.db.set_budget(&budget(f.food, "2026-10", 100_000))
            .unwrap();
        spend(&mut f.db, f.cash, f.food, 79_999, "2026-10-01");
        assert_eq!(
            f.db.budgets(month("2026-10")).unwrap()[0].progress.status,
            BudgetStatus::Ok
        );
        spend(&mut f.db, f.cash, f.food, 1, "2026-10-02");
        assert_eq!(
            f.db.budgets(month("2026-10")).unwrap()[0].progress.status,
            BudgetStatus::Warning
        );
        spend(&mut f.db, f.cash, f.food, 20_001, "2026-10-03");
        let p = f.db.budgets(month("2026-10")).unwrap()[0].progress;
        assert_eq!((p.status, p.remaining), (BudgetStatus::Over, idr(-1)));
    }

    #[test]
    fn setting_again_replaces_the_limit_and_keeps_one_row() {
        let mut f = fx();
        let first =
            f.db.set_budget(&budget(f.food, "2026-10", 100_000))
                .unwrap();
        let second =
            f.db.set_budget(&budget(f.food, "2026-10", 250_000))
                .unwrap();
        assert_eq!(first, second);
        let list = f.db.budgets(month("2026-10")).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].progress.limit, idr(250_000));
        // Another month is independent.
        f.db.set_budget(&budget(f.food, "2026-11", 1)).unwrap();
        assert_eq!(f.db.budgets(month("2026-11")).unwrap().len(), 1);
    }

    #[test]
    fn only_active_expense_categories_can_have_budgets() {
        let mut f = fx();
        assert_eq!(
            f.db.set_budget(&budget(f.salary, "2026-10", 1)),
            Err(StorageError::KindMismatch)
        );
        assert_eq!(
            f.db.set_budget(&budget(9_999, "2026-10", 1)),
            Err(StorageError::NotFound)
        );
        f.db.conn()
            .execute(
                "UPDATE categories SET archived = 1 WHERE id = ?1",
                params![f.transport],
            )
            .unwrap();
        assert_eq!(
            f.db.set_budget(&budget(f.transport, "2026-10", 1)),
            Err(StorageError::Archived)
        );
        let usd = BudgetDraft::new(
            f.food,
            month("2026-10"),
            Money::from_minor(5, Currency::USD).unwrap(),
        )
        .unwrap();
        assert_eq!(f.db.set_budget(&usd), Err(StorageError::CurrencyMismatch));
    }

    #[test]
    fn delete_removes_only_that_budget() {
        let mut f = fx();
        let a = f.db.set_budget(&budget(f.food, "2026-10", 1)).unwrap();
        f.db.set_budget(&budget(f.transport, "2026-10", 1)).unwrap();
        f.db.delete_budget(a).unwrap();
        let names: Vec<String> =
            f.db.budgets(month("2026-10"))
                .unwrap()
                .into_iter()
                .map(|b| b.category_name)
                .collect();
        assert_eq!(names, ["Transportasi"]);
        assert_eq!(f.db.delete_budget(a), Err(StorageError::NotFound));
    }

    #[test]
    fn a_month_without_budgets_is_empty() {
        let f = fx();
        assert!(f.db.budgets(month("2030-01")).unwrap().is_empty());
    }
}
