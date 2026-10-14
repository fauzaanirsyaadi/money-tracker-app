//! Transactions: create, edit, delete, transfer, and the paged, filtered list.
//!
//! * An income/expense is one row whose sign follows its kind.
//! * A transfer is two rows sharing `transfer_group_id`, which is the id of the
//!   outgoing leg. Both legs are written, and removed, in one transaction.
//! * Lists page by keyset `(occurred_on DESC, id DESC)`, never by OFFSET, so a
//!   page costs the same on row 100 000 as on row 10.

use mt_core::{
    Currency, Date, FlowKind, Money, TransactionDraft, TransferDraft, format_iso_date,
    parse_iso_date,
};
use rusqlite::{
    Connection, OptionalExtension, TransactionBehavior, params, params_from_iter, types::Value,
};

use crate::{db::Db, error::StorageError};

pub const DEFAULT_PAGE: usize = 50;
pub const MAX_PAGE: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Income,
    Expense,
    TransferOut,
    TransferIn,
}

impl RecordKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Income => "income",
            Self::Expense => "expense",
            Self::TransferOut => "transfer_out",
            Self::TransferIn => "transfer_in",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "income" => Some(Self::Income),
            "expense" => Some(Self::Expense),
            "transfer_out" => Some(Self::TransferOut),
            "transfer_in" => Some(Self::TransferIn),
            _ => None,
        }
    }

    pub fn is_transfer(self) -> bool {
        matches!(self, Self::TransferOut | Self::TransferIn)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRecord {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub kind: RecordKind,
    /// Signed: positive money in, negative money out.
    pub amount: Money,
    pub note: String,
    pub occurred_on: Date,
    pub transfer_group_id: Option<i64>,
    pub recurring_rule_id: Option<i64>,
}

/// Which transactions to list; every field narrows the result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransactionFilter {
    pub account_id: Option<i64>,
    /// A parent category also matches its sub-categories.
    pub category_id: Option<i64>,
    pub kind: Option<KindFilter>,
    /// Inclusive bounds.
    pub from: Option<Date>,
    pub to: Option<Date>,
    /// Case-insensitive (ASCII) substring of the note or the category name.
    pub search: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFilter {
    Income,
    Expense,
    /// Both legs of every transfer.
    Transfer,
}

/// Position after the last row of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub occurred_on: Date,
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<TransactionRecord>,
    /// Pass back as the cursor to get the next page; `None` on the last page.
    pub next: Option<Cursor>,
}

pub(crate) fn check_account(
    conn: &Connection,
    id: i64,
    currency: Currency,
    still_allowed: Option<i64>,
) -> Result<(), StorageError> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT currency, archived FROM accounts WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (account_currency, archived) = row.ok_or(StorageError::NotFound)?;
    if account_currency != currency.code() {
        return Err(StorageError::CurrencyMismatch);
    }
    if archived != 0 && still_allowed != Some(id) {
        return Err(StorageError::Archived);
    }
    Ok(())
}

pub(crate) fn check_category(
    conn: &Connection,
    id: i64,
    kind: FlowKind,
    still_allowed: Option<i64>,
) -> Result<(), StorageError> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT kind, archived FROM categories WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (category_kind, archived) = row.ok_or(StorageError::NotFound)?;
    if category_kind != kind.code() {
        return Err(StorageError::KindMismatch);
    }
    if archived != 0 && still_allowed != Some(id) {
        return Err(StorageError::Archived);
    }
    Ok(())
}

/// Escapes `%`, `_` and `\` so user text is matched literally by LIKE.
fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

impl Db {
    /// Records an income or expense and returns its id.
    pub fn create_transaction(
        &mut self,
        draft: &TransactionDraft,
        now_unix: i64,
    ) -> Result<i64, StorageError> {
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let currency = draft.amount().currency();
        check_account(&tx, draft.account_id(), currency, None)?;
        check_category(&tx, draft.category_id(), draft.kind(), None)?;
        tx.execute(
            "INSERT INTO transactions
               (account_id, category_id, amount_minor, kind, note, occurred_on, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                draft.account_id(),
                draft.category_id(),
                draft.signed_amount().minor(),
                draft.kind().code(),
                draft.note(),
                format_iso_date(draft.occurred_on()),
                now_unix
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Replaces an income/expense. Transfer legs cannot be edited this way.
    pub fn update_transaction(
        &mut self,
        id: i64,
        draft: &TransactionDraft,
    ) -> Result<(), StorageError> {
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, i64, Option<i64>)> = tx
            .query_row(
                "SELECT kind, account_id, category_id FROM transactions WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (kind, old_account, old_category) = existing.ok_or(StorageError::NotFound)?;
        if RecordKind::from_code(&kind).is_none_or(RecordKind::is_transfer) {
            return Err(StorageError::TransferLeg);
        }
        // An archived account or category may stay on an old record, but a
        // record cannot be moved onto one.
        check_account(
            &tx,
            draft.account_id(),
            draft.amount().currency(),
            Some(old_account),
        )?;
        check_category(&tx, draft.category_id(), draft.kind(), old_category)?;
        tx.execute(
            "UPDATE transactions
                SET account_id = ?1, category_id = ?2, amount_minor = ?3, kind = ?4,
                    note = ?5, occurred_on = ?6
              WHERE id = ?7",
            params![
                draft.account_id(),
                draft.category_id(),
                draft.signed_amount().minor(),
                draft.kind().code(),
                draft.note(),
                format_iso_date(draft.occurred_on()),
                id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes a transaction. Deleting either leg of a transfer removes both.
    /// Returns how many rows were removed.
    pub fn delete_transaction(&mut self, id: i64) -> Result<usize, StorageError> {
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let group: Option<Option<i64>> = tx
            .query_row(
                "SELECT transfer_group_id FROM transactions WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        let removed = match group.ok_or(StorageError::NotFound)? {
            // One statement removes both legs: the foreign key is checked at its end.
            Some(group_id) => tx.execute(
                "DELETE FROM transactions WHERE transfer_group_id = ?1",
                params![group_id],
            )?,
            None => tx.execute("DELETE FROM transactions WHERE id = ?1", params![id])?,
        };
        tx.commit()?;
        Ok(removed)
    }

    /// Moves money between two accounts and returns the transfer's group id.
    pub fn create_transfer(
        &mut self,
        draft: &TransferDraft,
        now_unix: i64,
    ) -> Result<i64, StorageError> {
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let currency = draft.amount().currency();
        check_account(&tx, draft.from_account_id(), currency, None)?;
        check_account(&tx, draft.to_account_id(), currency, None)?;

        // The outgoing leg names itself as the group, so it needs its id first.
        let group_id: i64 = tx.query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM transactions",
            [],
            |r| r.get(0),
        )?;
        let date = format_iso_date(draft.occurred_on());
        let amount = draft.amount().minor();
        tx.execute(
            "INSERT INTO transactions
               (id, account_id, amount_minor, kind, note, occurred_on, transfer_group_id, created_at)
             VALUES (?1, ?2, ?3, 'transfer_out', ?4, ?5, ?1, ?6)",
            params![group_id, draft.from_account_id(), -amount, draft.note(), date, now_unix],
        )?;
        tx.execute(
            "INSERT INTO transactions
               (account_id, amount_minor, kind, note, occurred_on, transfer_group_id, created_at)
             VALUES (?1, ?2, 'transfer_in', ?3, ?4, ?5, ?6)",
            params![
                draft.to_account_id(),
                amount,
                draft.note(),
                date,
                group_id,
                now_unix
            ],
        )?;
        tx.commit()?;
        Ok(group_id)
    }

    pub fn transaction(&self, id: i64) -> Result<Option<TransactionRecord>, StorageError> {
        let raw = self
            .conn()
            .query_row(&format!("{SELECT} WHERE t.id = ?1"), params![id], read_raw)
            .optional()?;
        raw.map(Raw::into_record).transpose()
    }

    /// One page of transactions, newest first. `limit` is clamped to
    /// `1..=MAX_PAGE`.
    pub fn transactions(
        &self,
        filter: &TransactionFilter,
        after: Option<Cursor>,
        limit: usize,
    ) -> Result<Page, StorageError> {
        let limit = limit.clamp(1, MAX_PAGE);
        let mut clauses: Vec<&str> = Vec::new();
        let mut args: Vec<Value> = Vec::new();

        if let Some(account) = filter.account_id {
            clauses.push("t.account_id = ?");
            args.push(account.into());
        }
        if let Some(category) = filter.category_id {
            clauses.push(
                "(t.category_id = ? OR t.category_id IN
                    (SELECT id FROM categories WHERE parent_id = ?))",
            );
            args.push(category.into());
            args.push(category.into());
        }
        match filter.kind {
            Some(KindFilter::Income) => clauses.push("t.kind = 'income'"),
            Some(KindFilter::Expense) => clauses.push("t.kind = 'expense'"),
            Some(KindFilter::Transfer) => clauses.push("t.kind IN ('transfer_out','transfer_in')"),
            None => {}
        }
        if let Some(from) = filter.from {
            clauses.push("t.occurred_on >= ?");
            args.push(format_iso_date(from).into());
        }
        if let Some(to) = filter.to {
            clauses.push("t.occurred_on <= ?");
            args.push(format_iso_date(to).into());
        }
        if let Some(text) = filter
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            clauses.push("(t.note LIKE ? ESCAPE '\\' OR c.name LIKE ? ESCAPE '\\')");
            let pattern = like_pattern(text);
            args.push(pattern.clone().into());
            args.push(pattern.into());
        }
        if let Some(cursor) = after {
            clauses.push("(t.occurred_on < ? OR (t.occurred_on = ? AND t.id < ?))");
            let date = format_iso_date(cursor.occurred_on);
            args.push(date.clone().into());
            args.push(date.into());
            args.push(cursor.id.into());
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        // One extra row tells whether another page exists.
        let sql = format!(
            "{SELECT}{where_sql} ORDER BY t.occurred_on DESC, t.id DESC LIMIT {}",
            limit + 1
        );
        let mut statement = self.conn().prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(args), read_raw)?;
        let mut items = rows
            .map(|row| row?.into_record())
            .collect::<Result<Vec<_>, _>>()?;

        let next = if items.len() > limit {
            items.truncate(limit);
            items.last().map(|last| Cursor {
                occurred_on: last.occurred_on,
                id: last.id,
            })
        } else {
            None
        };
        Ok(Page { items, next })
    }
}

const SELECT: &str = "
    SELECT t.id, t.account_id, a.name, a.currency, t.category_id, c.name, t.kind,
           t.amount_minor, t.note, t.occurred_on, t.transfer_group_id, t.recurring_rule_id
    FROM transactions t
    JOIN accounts a ON a.id = t.account_id
    LEFT JOIN categories c ON c.id = t.category_id";

struct Raw {
    id: i64,
    account_id: i64,
    account_name: String,
    currency: String,
    category_id: Option<i64>,
    category_name: Option<String>,
    kind: String,
    amount: i64,
    note: String,
    occurred_on: String,
    transfer_group_id: Option<i64>,
    recurring_rule_id: Option<i64>,
}

fn read_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<Raw> {
    Ok(Raw {
        id: row.get(0)?,
        account_id: row.get(1)?,
        account_name: row.get(2)?,
        currency: row.get(3)?,
        category_id: row.get(4)?,
        category_name: row.get(5)?,
        kind: row.get(6)?,
        amount: row.get(7)?,
        note: row.get(8)?,
        occurred_on: row.get(9)?,
        transfer_group_id: row.get(10)?,
        recurring_rule_id: row.get(11)?,
    })
}

impl Raw {
    fn into_record(self) -> Result<TransactionRecord, StorageError> {
        let currency =
            Currency::from_code(&self.currency).map_err(|_| StorageError::InvalidData)?;
        Ok(TransactionRecord {
            id: self.id,
            account_id: self.account_id,
            account_name: self.account_name,
            category_id: self.category_id,
            category_name: self.category_name,
            kind: RecordKind::from_code(&self.kind).ok_or(StorageError::InvalidData)?,
            amount: Money::from_minor(self.amount, currency)
                .map_err(|_| StorageError::InvalidData)?,
            note: self.note,
            occurred_on: parse_iso_date(&self.occurred_on)
                .map_err(|_| StorageError::InvalidData)?,
            transfer_group_id: self.transfer_group_id,
            recurring_rule_id: self.recurring_rule_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use mt_core::{AccountDraft, AccountKind};
    use zeroize::Zeroizing;

    use super::*;

    struct Fixture {
        db: Db,
        cash: i64,
        bank: i64,
        food: i64,
        coffee: i64,
        transport: i64,
        salary: i64,
    }

    fn d(text: &str) -> Date {
        parse_iso_date(text).unwrap()
    }

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::IDR).unwrap()
    }

    fn fixture() -> Fixture {
        let mut db = Db::open_in_memory(&Zeroizing::new([8u8; 32])).unwrap();
        let account = |db: &mut Db, name: &str, kind| {
            db.create_account(&AccountDraft::new(name, kind, idr(0)).unwrap(), 0)
                .unwrap()
        };
        let cash = account(&mut db, "Tunai", AccountKind::Cash);
        let bank = account(&mut db, "BCA", AccountKind::Bank);
        let category = |db: &Db, name: &str, kind: FlowKind| {
            db.categories(false)
                .unwrap()
                .into_iter()
                .find(|c| c.name == name && c.kind == kind)
                .unwrap()
                .id
        };
        let food = category(&db, "Makanan", FlowKind::Expense);
        let transport = category(&db, "Transportasi", FlowKind::Expense);
        let salary = category(&db, "Gaji", FlowKind::Income);
        let coffee = db
            .create_category(
                &mt_core::CategoryDraft::new("Kopi", FlowKind::Expense, Some(food)).unwrap(),
            )
            .unwrap();
        Fixture {
            db,
            cash,
            bank,
            food,
            coffee,
            transport,
            salary,
        }
    }

    fn spend(db: &mut Db, account: i64, category: i64, minor: i64, note: &str, date: &str) -> i64 {
        let draft = TransactionDraft::new(
            account,
            category,
            FlowKind::Expense,
            idr(minor),
            note,
            d(date),
        )
        .unwrap();
        db.create_transaction(&draft, 1).unwrap()
    }

    fn earn(db: &mut Db, account: i64, category: i64, minor: i64, date: &str) -> i64 {
        let draft =
            TransactionDraft::new(account, category, FlowKind::Income, idr(minor), "", d(date))
                .unwrap();
        db.create_transaction(&draft, 1).unwrap()
    }

    fn ids(page: &Page) -> Vec<i64> {
        page.items.iter().map(|t| t.id).collect()
    }

    fn all(db: &Db, filter: &TransactionFilter) -> Vec<TransactionRecord> {
        db.transactions(filter, None, MAX_PAGE).unwrap().items
    }

    // --- create / read -----------------------------------------------------------

    #[test]
    fn expenses_are_stored_negative_and_change_the_balance() {
        let mut f = fixture();
        let id = spend(
            &mut f.db,
            f.cash,
            f.food,
            25_000,
            "  nasi goreng ",
            "2026-10-14",
        );
        let record = f.db.transaction(id).unwrap().unwrap();
        assert_eq!(record.amount, idr(-25_000));
        assert_eq!(record.kind, RecordKind::Expense);
        assert_eq!(record.note, "nasi goreng");
        assert_eq!(record.category_name.as_deref(), Some("Makanan"));
        assert_eq!(record.account_name, "Tunai");
        assert_eq!(record.occurred_on, d("2026-10-14"));
        assert_eq!(f.db.account(f.cash).unwrap().unwrap().balance, idr(-25_000));
    }

    #[test]
    fn income_raises_the_balance() {
        let mut f = fixture();
        earn(&mut f.db, f.bank, f.salary, 5_000_000, "2026-10-01");
        spend(&mut f.db, f.bank, f.food, 100_000, "", "2026-10-02");
        assert_eq!(
            f.db.account(f.bank).unwrap().unwrap().balance,
            idr(4_900_000)
        );
    }

    // --- rules ---------------------------------------------------------------------

    #[test]
    fn category_kind_must_match_the_transaction_kind() {
        let mut f = fixture();
        let wrong = TransactionDraft::new(
            f.cash,
            f.salary,
            FlowKind::Expense,
            idr(1),
            "",
            d("2026-10-14"),
        )
        .unwrap();
        assert_eq!(
            f.db.create_transaction(&wrong, 0),
            Err(StorageError::KindMismatch)
        );
        let wrong = TransactionDraft::new(
            f.cash,
            f.food,
            FlowKind::Income,
            idr(1),
            "",
            d("2026-10-14"),
        )
        .unwrap();
        assert_eq!(
            f.db.create_transaction(&wrong, 0),
            Err(StorageError::KindMismatch)
        );
    }

    #[test]
    fn unknown_archived_and_foreign_currency_targets_are_rejected() {
        let mut f = fixture();
        let missing_account =
            TransactionDraft::new(999, f.food, FlowKind::Expense, idr(1), "", d("2026-10-14"))
                .unwrap();
        assert_eq!(
            f.db.create_transaction(&missing_account, 0),
            Err(StorageError::NotFound)
        );
        let missing_category =
            TransactionDraft::new(f.cash, 999, FlowKind::Expense, idr(1), "", d("2026-10-14"))
                .unwrap();
        assert_eq!(
            f.db.create_transaction(&missing_category, 0),
            Err(StorageError::NotFound)
        );

        f.db.set_account_archived(f.bank, true).unwrap();
        let archived = TransactionDraft::new(
            f.bank,
            f.food,
            FlowKind::Expense,
            idr(1),
            "",
            d("2026-10-14"),
        )
        .unwrap();
        assert_eq!(
            f.db.create_transaction(&archived, 0),
            Err(StorageError::Archived)
        );

        let usd = Money::from_minor(100, Currency::USD).unwrap();
        let foreign =
            TransactionDraft::new(f.cash, f.food, FlowKind::Expense, usd, "", d("2026-10-14"))
                .unwrap();
        assert_eq!(
            f.db.create_transaction(&foreign, 0),
            Err(StorageError::CurrencyMismatch)
        );
    }

    #[test]
    fn nothing_is_written_when_validation_fails() {
        let mut f = fixture();
        let bad = TransactionDraft::new(
            f.cash,
            f.salary,
            FlowKind::Expense,
            idr(1),
            "",
            d("2026-10-14"),
        )
        .unwrap();
        let _ = f.db.create_transaction(&bad, 0);
        assert!(all(&f.db, &TransactionFilter::default()).is_empty());
    }

    // --- update / delete -------------------------------------------------------------

    #[test]
    fn update_rewrites_every_field_and_the_balance_follows() {
        let mut f = fixture();
        let id = spend(&mut f.db, f.cash, f.food, 10_000, "old", "2026-10-01");
        let draft = TransactionDraft::new(
            f.bank,
            f.transport,
            FlowKind::Expense,
            idr(40_000),
            "ojek",
            d("2026-10-05"),
        )
        .unwrap();
        f.db.update_transaction(id, &draft).unwrap();
        let record = f.db.transaction(id).unwrap().unwrap();
        assert_eq!(
            (
                record.account_id,
                record.category_id,
                record.amount,
                record.note.as_str(),
                record.occurred_on
            ),
            (
                f.bank,
                Some(f.transport),
                idr(-40_000),
                "ojek",
                d("2026-10-05")
            )
        );
        assert_eq!(f.db.account(f.cash).unwrap().unwrap().balance, idr(0));
        assert_eq!(f.db.account(f.bank).unwrap().unwrap().balance, idr(-40_000));
    }

    #[test]
    fn income_can_be_edited_into_an_expense_with_a_matching_category() {
        let mut f = fixture();
        let id = earn(&mut f.db, f.cash, f.salary, 10_000, "2026-10-01");
        let draft = TransactionDraft::new(
            f.cash,
            f.food,
            FlowKind::Expense,
            idr(10_000),
            "",
            d("2026-10-01"),
        )
        .unwrap();
        f.db.update_transaction(id, &draft).unwrap();
        assert_eq!(f.db.transaction(id).unwrap().unwrap().amount, idr(-10_000));
    }

    #[test]
    fn an_old_record_may_keep_its_archived_account_but_cannot_move_onto_one() {
        let mut f = fixture();
        let id = spend(&mut f.db, f.bank, f.food, 1_000, "", "2026-10-01");
        f.db.set_account_archived(f.bank, true).unwrap();
        f.db.set_account_archived(f.cash, true).unwrap();
        let same = TransactionDraft::new(
            f.bank,
            f.food,
            FlowKind::Expense,
            idr(2_000),
            "fix",
            d("2026-10-01"),
        )
        .unwrap();
        assert_eq!(f.db.update_transaction(id, &same), Ok(()));
        let moved = TransactionDraft::new(
            f.cash,
            f.food,
            FlowKind::Expense,
            idr(2_000),
            "",
            d("2026-10-01"),
        )
        .unwrap();
        assert_eq!(
            f.db.update_transaction(id, &moved),
            Err(StorageError::Archived)
        );
    }

    #[test]
    fn delete_removes_the_row_and_reports_unknown_ids() {
        let mut f = fixture();
        let id = spend(&mut f.db, f.cash, f.food, 5_000, "", "2026-10-01");
        assert_eq!(f.db.delete_transaction(id), Ok(1));
        assert_eq!(f.db.transaction(id).unwrap(), None);
        assert_eq!(f.db.delete_transaction(id), Err(StorageError::NotFound));
        assert_eq!(f.db.account(f.cash).unwrap().unwrap().balance, idr(0));
    }

    // --- transfers ----------------------------------------------------------------------

    #[test]
    fn a_transfer_moves_money_and_leaves_net_worth_unchanged() {
        let mut f = fixture();
        earn(&mut f.db, f.bank, f.salary, 1_000_000, "2026-10-01");
        let draft =
            TransferDraft::new(f.bank, f.cash, idr(300_000), "tarik tunai", d("2026-10-02"))
                .unwrap();
        let group = f.db.create_transfer(&draft, 0).unwrap();

        let out = f.db.transaction(group).unwrap().unwrap();
        assert_eq!(
            (out.kind, out.amount, out.account_id),
            (RecordKind::TransferOut, idr(-300_000), f.bank)
        );
        assert_eq!(out.transfer_group_id, Some(group));
        assert_eq!(out.category_id, None);

        assert_eq!(f.db.account(f.bank).unwrap().unwrap().balance, idr(700_000));
        assert_eq!(f.db.account(f.cash).unwrap().unwrap().balance, idr(300_000));
        let total: i64 =
            f.db.accounts(true)
                .unwrap()
                .iter()
                .map(|a| a.balance.minor())
                .sum();
        assert_eq!(total, 1_000_000);
    }

    #[test]
    fn transfers_between_the_same_account_or_to_archived_accounts_are_rejected() {
        let mut f = fixture();
        f.db.set_account_archived(f.cash, true).unwrap();
        let to_archived = TransferDraft::new(f.bank, f.cash, idr(1), "", d("2026-10-02")).unwrap();
        assert_eq!(
            f.db.create_transfer(&to_archived, 0),
            Err(StorageError::Archived)
        );
        assert!(all(&f.db, &TransactionFilter::default()).is_empty());
    }

    #[test]
    fn deleting_either_leg_removes_both() {
        let mut f = fixture();
        let draft = TransferDraft::new(f.bank, f.cash, idr(10), "", d("2026-10-02")).unwrap();
        let group = f.db.create_transfer(&draft, 0).unwrap();
        let legs = all(
            &f.db,
            &TransactionFilter {
                kind: Some(KindFilter::Transfer),
                ..Default::default()
            },
        );
        assert_eq!(legs.len(), 2);
        let incoming = legs
            .iter()
            .find(|l| l.kind == RecordKind::TransferIn)
            .unwrap()
            .id;
        assert_ne!(incoming, group);
        assert_eq!(f.db.delete_transaction(incoming), Ok(2));
        assert!(all(&f.db, &TransactionFilter::default()).is_empty());
    }

    #[test]
    fn transfer_legs_cannot_be_edited_as_ordinary_transactions() {
        let mut f = fixture();
        let draft = TransferDraft::new(f.bank, f.cash, idr(10), "", d("2026-10-02")).unwrap();
        let group = f.db.create_transfer(&draft, 0).unwrap();
        let edit = TransactionDraft::new(
            f.bank,
            f.food,
            FlowKind::Expense,
            idr(5),
            "",
            d("2026-10-02"),
        )
        .unwrap();
        assert_eq!(
            f.db.update_transaction(group, &edit),
            Err(StorageError::TransferLeg)
        );
    }

    #[test]
    fn consecutive_transfers_get_distinct_groups() {
        let mut f = fixture();
        let a =
            f.db.create_transfer(
                &TransferDraft::new(f.bank, f.cash, idr(1), "", d("2026-10-02")).unwrap(),
                0,
            )
            .unwrap();
        let b =
            f.db.create_transfer(
                &TransferDraft::new(f.cash, f.bank, idr(1), "", d("2026-10-02")).unwrap(),
                0,
            )
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(all(&f.db, &TransactionFilter::default()).len(), 4);
    }

    // --- listing, filters, paging ---------------------------------------------------------

    #[test]
    fn lists_newest_first_with_id_as_the_tie_breaker() {
        let mut f = fixture();
        let a = spend(&mut f.db, f.cash, f.food, 1, "a", "2026-10-01");
        let b = spend(&mut f.db, f.cash, f.food, 1, "b", "2026-10-03");
        let c = spend(&mut f.db, f.cash, f.food, 1, "c", "2026-10-03");
        let page =
            f.db.transactions(&TransactionFilter::default(), None, 10)
                .unwrap();
        assert_eq!(ids(&page), [c, b, a]);
        assert_eq!(page.next, None);
    }

    #[test]
    fn keyset_paging_visits_every_row_once_even_with_equal_dates() {
        let mut f = fixture();
        let mut expected = Vec::new();
        for n in 0..137 {
            // Twenty rows share each date so the id tie-breaker really matters.
            let date = format!("2026-09-{:02}", 1 + n / 20);
            expected.push(spend(&mut f.db, f.cash, f.food, 1 + n as i64, "", &date));
        }
        expected.reverse(); // newest first: later date, then higher id

        let mut seen = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        loop {
            let page =
                f.db.transactions(&TransactionFilter::default(), cursor, 25)
                    .unwrap();
            seen.extend(ids(&page));
            pages += 1;
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, expected);
        assert_eq!(pages, 6); // 137 / 25 -> 5 full pages + 12
    }

    #[test]
    fn page_size_is_clamped_and_next_is_absent_on_an_exact_fit() {
        let mut f = fixture();
        for n in 0..3 {
            spend(
                &mut f.db,
                f.cash,
                f.food,
                1,
                "",
                &format!("2026-10-0{}", n + 1),
            );
        }
        assert_eq!(
            f.db.transactions(&TransactionFilter::default(), None, 0)
                .unwrap()
                .items
                .len(),
            1
        );
        let exact =
            f.db.transactions(&TransactionFilter::default(), None, 3)
                .unwrap();
        assert_eq!(exact.items.len(), 3);
        assert_eq!(exact.next, None);
        assert!(
            f.db.transactions(&TransactionFilter::default(), None, 3)
                .unwrap()
                .next
                .is_none()
        );
        let partial =
            f.db.transactions(&TransactionFilter::default(), None, 2)
                .unwrap();
        assert!(partial.next.is_some());
    }

    #[test]
    fn filters_by_account_kind_and_date_range() {
        let mut f = fixture();
        let a = spend(&mut f.db, f.cash, f.food, 1, "", "2026-09-30");
        let b = spend(&mut f.db, f.cash, f.food, 1, "", "2026-10-01");
        let c = spend(&mut f.db, f.bank, f.food, 1, "", "2026-10-15");
        let d_inc = earn(&mut f.db, f.bank, f.salary, 9, "2026-10-31");
        let e = spend(&mut f.db, f.cash, f.food, 1, "", "2026-11-01");

        let only = |filter: TransactionFilter| -> Vec<i64> {
            all(&f.db, &filter).iter().map(|t| t.id).collect()
        };
        let month = TransactionFilter {
            from: Some(d("2026-10-01")),
            to: Some(d("2026-10-31")),
            ..Default::default()
        };
        assert_eq!(only(month), [d_inc, c, b], "bounds are inclusive");
        assert_eq!(
            only(TransactionFilter {
                account_id: Some(f.bank),
                ..Default::default()
            }),
            [d_inc, c]
        );
        assert_eq!(
            only(TransactionFilter {
                kind: Some(KindFilter::Income),
                ..Default::default()
            }),
            [d_inc]
        );
        assert_eq!(
            only(TransactionFilter {
                kind: Some(KindFilter::Expense),
                account_id: Some(f.cash),
                ..Default::default()
            }),
            [e, b, a]
        );
    }

    #[test]
    fn a_parent_category_filter_includes_its_sub_categories() {
        let mut f = fixture();
        let plain = spend(&mut f.db, f.cash, f.food, 1, "", "2026-10-01");
        let sub = spend(&mut f.db, f.cash, f.coffee, 1, "", "2026-10-02");
        let other = spend(&mut f.db, f.cash, f.transport, 1, "", "2026-10-03");
        let only = |category: i64| -> Vec<i64> {
            all(
                &f.db,
                &TransactionFilter {
                    category_id: Some(category),
                    ..Default::default()
                },
            )
            .iter()
            .map(|t| t.id)
            .collect()
        };
        assert_eq!(only(f.food), [sub, plain]);
        assert_eq!(only(f.coffee), [sub]);
        assert_eq!(only(f.transport), [other]);
    }

    #[test]
    fn search_matches_notes_and_category_names_literally() {
        let mut f = fixture();
        let a = spend(
            &mut f.db,
            f.cash,
            f.food,
            1,
            "Kopi susu 100% enak",
            "2026-10-01",
        );
        let b = spend(
            &mut f.db,
            f.cash,
            f.transport,
            1,
            "grab_ke kantor",
            "2026-10-02",
        );
        let c = spend(&mut f.db, f.cash, f.food, 1, "", "2026-10-03");
        let search = |text: &str| -> Vec<i64> {
            all(
                &f.db,
                &TransactionFilter {
                    search: Some(text.to_owned()),
                    ..Default::default()
                },
            )
            .iter()
            .map(|t| t.id)
            .collect()
        };
        assert_eq!(search("kopi"), [a], "case-insensitive note match");
        assert_eq!(search("makanan"), [c, a], "category name match");
        assert_eq!(search("100%"), [a], "% is literal");
        assert_eq!(search("grab_ke"), [b], "_ is literal");
        assert_eq!(search("%"), [a], "a lone % is not a wildcard");
        assert_eq!(search("_"), [b]);
        assert_eq!(search("a'; DROP TABLE transactions; --"), Vec::<i64>::new());
        assert_eq!(search("   ").len(), 3, "blank search means no filter");
    }

    #[test]
    fn filters_and_cursor_combine() {
        let mut f = fixture();
        let mut cash_ids = Vec::new();
        for n in 0..10 {
            cash_ids.push(spend(&mut f.db, f.cash, f.food, 1, "", "2026-10-01"));
            spend(
                &mut f.db,
                f.bank,
                f.food,
                1,
                "",
                &format!("2026-10-{:02}", 1 + n),
            );
        }
        let filter = TransactionFilter {
            account_id: Some(f.cash),
            ..Default::default()
        };
        let first = f.db.transactions(&filter, None, 4).unwrap();
        let second = f.db.transactions(&filter, first.next, 4).unwrap();
        let mut got = ids(&first);
        got.extend(ids(&second));
        cash_ids.reverse();
        assert_eq!(got, cash_ids[..8]);
        assert!(
            first
                .items
                .iter()
                .chain(&second.items)
                .all(|t| t.account_id == f.cash)
        );
    }
}
