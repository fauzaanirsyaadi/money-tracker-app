//! Reports. Every figure is computed by SQLite (sums, grouping) or by exact
//! integer arithmetic here; the UI only formats and draws.
//!
//! Conventions shared by all reports:
//! * Only `income` and `expense` rows count as income and spending. Transfers
//!   move money between the user's own accounts and never appear in them.
//! * Amounts are positive magnitudes (spending 25 000 is `25000`), except net
//!   worth and net, which keep their sign.
//! * Archived accounts still hold money, so their balances count in net worth.

use mt_core::{Currency, Date, Money, YearMonth, format_iso_date};
use rusqlite::params;

use crate::{accounts::Account, db::Db, error::StorageError};

/// Upper bound for a monthly trend, so one call cannot scan a lifetime.
pub const MAX_TREND_MONTHS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategorySpending {
    pub category_id: i64,
    pub name: String,
    /// Includes sub-categories.
    pub spent: Money,
    pub count: u32,
    /// Share of all spending in the period, basis points (10 000 = 100 %).
    /// Each share is rounded on its own, so the column may total 99.99-100.01 %.
    pub share_bp: i64,
    pub children: Vec<ChildSpending>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSpending {
    pub category_id: i64,
    pub name: String,
    pub spent: Money,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendingReport {
    pub from: Date,
    pub to: Date,
    pub total: Money,
    pub categories: Vec<CategorySpending>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthSummary {
    pub month: YearMonth,
    pub income: Money,
    pub expense: Money,
    /// `income - expense` (negative when more was spent than earned).
    pub net: Money,
    /// Net worth at the end of the month: opening balances plus every
    /// transaction up to and including that month.
    pub net_worth: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetWorth {
    pub total: Money,
    /// Active accounts first, in display order.
    pub accounts: Vec<Account>,
}

impl Db {
    /// Spending by category for `from..=to`, largest first. Sub-category
    /// spending rolls up into its parent, which also lists the children.
    pub fn spending_by_category(
        &self,
        from: Date,
        to: Date,
    ) -> Result<SpendingReport, StorageError> {
        let currency: Currency = self.app_currency()?;
        let money =
            |minor: i64| Money::from_minor(minor, currency).map_err(|_| StorageError::InvalidData);
        let (from_text, to_text) = (format_iso_date(from), format_iso_date(to));

        // One row per leaf category that has spending in the period.
        let mut statement = self.conn().prepare(
            "SELECT c.id, c.name, c.parent_id, p.name, SUM(-t.amount_minor), COUNT(*)
             FROM transactions t
             JOIN categories c ON c.id = t.category_id
             LEFT JOIN categories p ON p.id = c.parent_id
             WHERE t.kind = 'expense' AND t.occurred_on >= ?1 AND t.occurred_on <= ?2
             GROUP BY c.id",
        )?;
        let rows = statement
            .query_map(params![from_text, to_text], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        struct Root {
            id: i64,
            name: String,
            spent: i128,
            count: u32,
            children: Vec<ChildSpending>,
        }
        let mut roots: Vec<Root> = Vec::new();
        for (id, name, parent_id, parent_name, spent, count) in rows {
            let count = u32::try_from(count).unwrap_or(u32::MAX);
            let (root_id, root_name) = match (parent_id, parent_name) {
                (Some(pid), Some(pname)) => (pid, pname),
                _ => (id, name.clone()),
            };
            let at = match roots.iter().position(|r| r.id == root_id) {
                Some(at) => at,
                None => {
                    roots.push(Root {
                        id: root_id,
                        name: root_name,
                        spent: 0,
                        count: 0,
                        children: Vec::new(),
                    });
                    roots.len() - 1
                }
            };
            let root = &mut roots[at];
            root.spent += i128::from(spent);
            root.count = root.count.saturating_add(count);
            if parent_id.is_some() {
                root.children.push(ChildSpending {
                    category_id: id,
                    name,
                    spent: money(spent)?,
                    count,
                });
            }
        }

        let total: i128 = roots.iter().map(|r| r.spent).sum();
        let total_money = money(i64::try_from(total).map_err(|_| StorageError::InvalidData)?)?;
        roots.sort_by(|a, b| b.spent.cmp(&a.spent).then_with(|| a.name.cmp(&b.name)));

        let categories = roots
            .into_iter()
            .map(|mut root| {
                root.children.sort_by(|a, b| {
                    b.spent
                        .minor()
                        .cmp(&a.spent.minor())
                        .then_with(|| a.name.cmp(&b.name))
                });
                let spent =
                    money(i64::try_from(root.spent).map_err(|_| StorageError::InvalidData)?)?;
                let share_bp = if total_money.is_zero() {
                    0
                } else {
                    spent
                        .checked_mul_ratio(10_000, total_money.minor())
                        .map_err(|_| StorageError::InvalidData)?
                        .minor()
                };
                Ok(CategorySpending {
                    category_id: root.id,
                    name: root.name,
                    spent,
                    count: root.count,
                    share_bp,
                    children: root.children,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        Ok(SpendingReport {
            from,
            to,
            total: total_money,
            categories,
        })
    }

    /// Income, spending, net and end-of-month net worth for every month in
    /// `from..=to` (months without activity appear with zero flows).
    pub fn monthly_trend(
        &self,
        from: YearMonth,
        to: YearMonth,
    ) -> Result<Vec<MonthSummary>, StorageError> {
        if to < from {
            return Err(StorageError::Constraint);
        }
        let span = usize::try_from(to.index() - from.index() + 1).unwrap_or(usize::MAX);
        if span > MAX_TREND_MONTHS {
            return Err(StorageError::Constraint);
        }
        let currency: Currency = self.app_currency()?;
        let money = |value: i128| {
            i64::try_from(value)
                .ok()
                .and_then(|minor| Money::from_minor(minor, currency).ok())
                .ok_or(StorageError::InvalidData)
        };

        // Net worth just before the first month: every opening balance plus
        // everything dated earlier (transfers cancel out on their own).
        let before: i64 = self.conn().query_row(
            "SELECT COALESCE((SELECT SUM(opening_balance_minor) FROM accounts), 0)
                  + COALESCE((SELECT SUM(amount_minor) FROM transactions WHERE occurred_on < ?1), 0)",
            params![format_iso_date(from.first_day())],
            |r| r.get(0),
        )?;

        let mut statement = self.conn().prepare(
            "SELECT substr(occurred_on, 1, 7),
                    COALESCE(SUM(CASE WHEN kind = 'income'  THEN  amount_minor END), 0),
                    COALESCE(SUM(CASE WHEN kind = 'expense' THEN -amount_minor END), 0),
                    SUM(amount_minor)
             FROM transactions
             WHERE occurred_on >= ?1 AND occurred_on <= ?2
             GROUP BY substr(occurred_on, 1, 7)",
        )?;
        let rows = statement
            .query_map(
                params![
                    format_iso_date(from.first_day()),
                    format_iso_date(to.last_day())
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;

        let mut running = i128::from(before);
        let mut out = Vec::with_capacity(span);
        for offset in 0..span {
            let month = from
                .plus_months(i32::try_from(offset).unwrap_or(0))
                .map_err(|_| StorageError::InvalidData)?;
            let key = month.to_string();
            let (income, expense, all) = rows
                .iter()
                .find(|row| row.0 == key)
                .map_or((0, 0, 0), |row| (row.1, row.2, row.3));
            running += i128::from(all);
            out.push(MonthSummary {
                month,
                income: money(i128::from(income))?,
                expense: money(i128::from(expense))?,
                net: money(i128::from(income) - i128::from(expense))?,
                net_worth: money(running)?,
            });
        }
        Ok(out)
    }

    /// Total balance across all accounts (archived included) and the accounts.
    pub fn net_worth(&self) -> Result<NetWorth, StorageError> {
        let accounts = self.accounts(true)?;
        let total: i128 = accounts.iter().map(|a| i128::from(a.balance.minor())).sum();
        let currency = self.app_currency()?;
        let total = i64::try_from(total)
            .ok()
            .and_then(|minor| Money::from_minor(minor, currency).ok())
            .ok_or(StorageError::InvalidData)?;
        Ok(NetWorth { total, accounts })
    }
}

#[cfg(test)]
mod tests {
    use mt_core::{
        AccountDraft, AccountKind, CategoryDraft, FlowKind, TransactionDraft, TransferDraft,
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
        shopping: i64,
        salary: i64,
    }

    fn d(text: &str) -> Date {
        parse_iso_date(text).unwrap()
    }

    fn m(text: &str) -> YearMonth {
        YearMonth::parse(text).unwrap()
    }

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::IDR).unwrap()
    }

    fn fx() -> Fx {
        let mut db = Db::open_in_memory(&Zeroizing::new([11u8; 32])).unwrap();
        let cash = db
            .create_account(
                &AccountDraft::new("Tunai", AccountKind::Cash, idr(100_000)).unwrap(),
                0,
            )
            .unwrap();
        let bank = db
            .create_account(
                &AccountDraft::new("BCA", AccountKind::Bank, idr(900_000)).unwrap(),
                0,
            )
            .unwrap();
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
        let shopping = find(&db, "Belanja", FlowKind::Expense);
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
            shopping,
            salary,
        }
    }

    fn expense(db: &mut Db, account: i64, category: i64, minor: i64, date: &str) {
        let draft = TransactionDraft::new(
            account,
            category,
            FlowKind::Expense,
            idr(minor),
            "",
            d(date),
        )
        .unwrap();
        db.create_transaction(&draft, 0).unwrap();
    }

    fn income(db: &mut Db, account: i64, category: i64, minor: i64, date: &str) {
        let draft =
            TransactionDraft::new(account, category, FlowKind::Income, idr(minor), "", d(date))
                .unwrap();
        db.create_transaction(&draft, 0).unwrap();
    }

    // --- spending by category ---------------------------------------------------------

    #[test]
    fn groups_by_category_largest_first_with_shares() {
        let mut f = fx();
        expense(&mut f.db, f.cash, f.transport, 30_000, "2026-10-02");
        expense(&mut f.db, f.cash, f.shopping, 50_000, "2026-10-03");
        expense(&mut f.db, f.bank, f.shopping, 70_000, "2026-10-04");
        expense(&mut f.db, f.bank, f.food, 50_000, "2026-10-05");
        let report =
            f.db.spending_by_category(d("2026-10-01"), d("2026-10-31"))
                .unwrap();
        assert_eq!(report.total, idr(200_000));
        let names: Vec<&str> = report.categories.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Belanja", "Makanan", "Transportasi"]);
        let shopping = &report.categories[0];
        assert_eq!(
            (shopping.spent, shopping.count, shopping.share_bp),
            (idr(120_000), 2, 6_000)
        );
        assert_eq!(report.categories[2].share_bp, 1_500);
    }

    #[test]
    fn sub_categories_roll_up_into_the_parent_and_are_listed() {
        let mut f = fx();
        expense(&mut f.db, f.cash, f.food, 40_000, "2026-10-02");
        expense(&mut f.db, f.cash, f.coffee, 25_000, "2026-10-03");
        expense(&mut f.db, f.cash, f.coffee, 5_000, "2026-10-04");
        let report =
            f.db.spending_by_category(d("2026-10-01"), d("2026-10-31"))
                .unwrap();
        assert_eq!(report.categories.len(), 1);
        let food = &report.categories[0];
        assert_eq!(
            (food.name.as_str(), food.spent, food.count),
            ("Makanan", idr(70_000), 3)
        );
        assert_eq!(food.children.len(), 1);
        assert_eq!(
            (
                food.children[0].name.as_str(),
                food.children[0].spent,
                food.children[0].count
            ),
            ("Kopi", idr(30_000), 2)
        );
    }

    #[test]
    fn a_sub_category_alone_still_appears_under_its_parent() {
        let mut f = fx();
        expense(&mut f.db, f.cash, f.coffee, 9_000, "2026-10-02");
        let report =
            f.db.spending_by_category(d("2026-10-01"), d("2026-10-31"))
                .unwrap();
        assert_eq!(report.categories[0].name, "Makanan");
        assert_eq!(report.categories[0].spent, idr(9_000));
    }

    #[test]
    fn the_period_is_inclusive_and_excludes_income_and_transfers() {
        let mut f = fx();
        expense(&mut f.db, f.cash, f.food, 1_000, "2026-09-30");
        expense(&mut f.db, f.cash, f.food, 2_000, "2026-10-01");
        expense(&mut f.db, f.cash, f.food, 4_000, "2026-10-31");
        expense(&mut f.db, f.cash, f.food, 8_000, "2026-11-01");
        income(&mut f.db, f.bank, f.salary, 5_000_000, "2026-10-15");
        let transfer =
            TransferDraft::new(f.bank, f.cash, idr(77_000), "", d("2026-10-16")).unwrap();
        f.db.create_transfer(&transfer, 0).unwrap();
        let report =
            f.db.spending_by_category(d("2026-10-01"), d("2026-10-31"))
                .unwrap();
        assert_eq!(report.total, idr(6_000));
    }

    #[test]
    fn an_empty_period_is_an_empty_report() {
        let f = fx();
        let report =
            f.db.spending_by_category(d("2026-10-01"), d("2026-10-31"))
                .unwrap();
        assert!(report.categories.is_empty());
        assert_eq!(report.total, idr(0));
    }

    #[test]
    fn equal_totals_sort_by_name_and_shares_are_rounded_individually() {
        let mut f = fx();
        expense(&mut f.db, f.cash, f.transport, 1, "2026-10-02");
        expense(&mut f.db, f.cash, f.shopping, 1, "2026-10-02");
        expense(&mut f.db, f.cash, f.food, 1, "2026-10-02");
        let report =
            f.db.spending_by_category(d("2026-10-01"), d("2026-10-31"))
                .unwrap();
        let names: Vec<&str> = report.categories.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Belanja", "Makanan", "Transportasi"]);
        assert!(report.categories.iter().all(|c| c.share_bp == 3_333));
    }

    // --- monthly trend --------------------------------------------------------------------

    #[test]
    fn trend_reports_income_expense_and_net_per_month() {
        let mut f = fx();
        income(&mut f.db, f.bank, f.salary, 5_000_000, "2026-08-25");
        expense(&mut f.db, f.cash, f.food, 300_000, "2026-08-10");
        expense(&mut f.db, f.cash, f.food, 200_000, "2026-09-10");
        income(&mut f.db, f.bank, f.salary, 5_000_000, "2026-09-25");
        income(&mut f.db, f.bank, f.salary, 100_000, "2026-10-01");
        let trend = f.db.monthly_trend(m("2026-08"), m("2026-10")).unwrap();
        let row = |i: usize| {
            (
                trend[i].month.to_string(),
                trend[i].income.minor(),
                trend[i].expense.minor(),
                trend[i].net.minor(),
            )
        };
        assert_eq!(row(0), ("2026-08".into(), 5_000_000, 300_000, 4_700_000));
        assert_eq!(row(1), ("2026-09".into(), 5_000_000, 200_000, 4_800_000));
        assert_eq!(row(2), ("2026-10".into(), 100_000, 0, 100_000));
    }

    #[test]
    fn months_without_activity_appear_with_zero_flows_and_carry_net_worth() {
        let mut f = fx();
        income(&mut f.db, f.bank, f.salary, 1_000_000, "2026-06-15");
        let trend = f.db.monthly_trend(m("2026-06"), m("2026-09")).unwrap();
        assert_eq!(trend.len(), 4);
        // Opening balances 1 000 000 + 1 000 000 income = 2 000 000, carried forward.
        let worths: Vec<i64> = trend.iter().map(|t| t.net_worth.minor()).collect();
        assert_eq!(worths, [2_000_000, 2_000_000, 2_000_000, 2_000_000]);
        assert_eq!(trend[2].income, idr(0));
        assert_eq!(trend[2].net, idr(0));
    }

    #[test]
    fn net_worth_includes_everything_before_the_first_month() {
        let mut f = fx();
        expense(&mut f.db, f.cash, f.food, 40_000, "2026-03-05"); // long before the window
        income(&mut f.db, f.bank, f.salary, 500_000, "2026-04-05");
        expense(&mut f.db, f.cash, f.food, 10_000, "2026-09-02");
        let trend = f.db.monthly_trend(m("2026-09"), m("2026-09")).unwrap();
        // 1 000 000 opening - 40 000 + 500 000 - 10 000
        assert_eq!(trend[0].net_worth, idr(1_450_000));
    }

    #[test]
    fn transfers_do_not_change_flows_or_net_worth() {
        let mut f = fx();
        let transfer =
            TransferDraft::new(f.bank, f.cash, idr(250_000), "", d("2026-10-05")).unwrap();
        f.db.create_transfer(&transfer, 0).unwrap();
        let trend = f.db.monthly_trend(m("2026-10"), m("2026-10")).unwrap();
        assert_eq!(
            (trend[0].income, trend[0].expense, trend[0].net),
            (idr(0), idr(0), idr(0))
        );
        assert_eq!(trend[0].net_worth, idr(1_000_000));
    }

    #[test]
    fn the_last_net_worth_point_matches_the_net_worth_report() {
        let mut f = fx();
        income(&mut f.db, f.bank, f.salary, 2_000_000, "2026-10-01");
        expense(&mut f.db, f.cash, f.shopping, 123_456, "2026-10-09");
        let trend = f.db.monthly_trend(m("2026-10"), m("2026-10")).unwrap();
        assert_eq!(trend[0].net_worth, f.db.net_worth().unwrap().total);
    }

    #[test]
    fn trend_ranges_are_validated() {
        let f = fx();
        assert_eq!(
            f.db.monthly_trend(m("2026-10"), m("2026-09")),
            Err(StorageError::Constraint)
        );
        assert_eq!(
            f.db.monthly_trend(m("2010-01"), m("2026-10")),
            Err(StorageError::Constraint),
            "more than {MAX_TREND_MONTHS} months"
        );
        assert_eq!(
            f.db.monthly_trend(m("2016-11"), m("2026-10"))
                .unwrap()
                .len(),
            120
        );
    }

    // --- net worth ------------------------------------------------------------------------------

    #[test]
    fn net_worth_sums_every_account_including_archived_and_negative_ones() {
        let mut f = fx();
        expense(&mut f.db, f.cash, f.food, 30_000, "2026-10-02");
        let card =
            f.db.create_account(
                &AccountDraft::new("Kartu", AccountKind::Bank, idr(-400_000)).unwrap(),
                0,
            )
            .unwrap();
        f.db.set_account_archived(f.bank, true).unwrap();
        let report = f.db.net_worth().unwrap();
        // 100 000 - 30 000 + 900 000 - 400 000
        assert_eq!(report.total, idr(570_000));
        assert_eq!(report.accounts.len(), 3);
        assert!(
            report
                .accounts
                .iter()
                .any(|a| a.id == card && a.balance == idr(-400_000))
        );
        // Active accounts come first.
        assert!(report.accounts.last().unwrap().archived);
    }

    #[test]
    fn net_worth_is_unchanged_by_a_transfer() {
        let mut f = fx();
        let before = f.db.net_worth().unwrap().total;
        let transfer =
            TransferDraft::new(f.bank, f.cash, idr(111_000), "", d("2026-10-05")).unwrap();
        f.db.create_transfer(&transfer, 0).unwrap();
        assert_eq!(f.db.net_worth().unwrap().total, before);
    }
}
