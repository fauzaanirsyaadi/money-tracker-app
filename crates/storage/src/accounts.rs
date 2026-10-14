//! Accounts: create, edit, archive and list with computed balances.
//!
//! A balance is `opening_balance + SUM(transactions)` evaluated by SQLite on
//! every read; there is no stored balance to drift out of date.

use mt_core::{AccountDraft, AccountKind, Currency, Money};
use rusqlite::{OptionalExtension, params};

use crate::{db::Db, error::StorageError};

const DEFAULT_CURRENCY: Currency = Currency::IDR;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub kind: AccountKind,
    pub opening_balance: Money,
    /// `opening_balance` plus every transaction on the account.
    pub balance: Money,
    pub archived: bool,
    pub sort_order: i64,
    pub created_at: i64,
}

struct RawAccount {
    id: i64,
    name: String,
    kind: String,
    currency: String,
    opening: i64,
    archived: i64,
    sort_order: i64,
    created_at: i64,
    balance: i64,
}

const SELECT: &str = "
    SELECT a.id, a.name, a.kind, a.currency, a.opening_balance_minor, a.archived,
           a.sort_order, a.created_at,
           a.opening_balance_minor
             + COALESCE((SELECT SUM(t.amount_minor) FROM transactions t
                         WHERE t.account_id = a.id), 0)
    FROM accounts a";

impl RawAccount {
    fn into_account(self) -> Result<Account, StorageError> {
        let currency =
            Currency::from_code(&self.currency).map_err(|_| StorageError::InvalidData)?;
        Ok(Account {
            id: self.id,
            name: self.name,
            kind: AccountKind::from_code(&self.kind).ok_or(StorageError::InvalidData)?,
            opening_balance: Money::from_minor(self.opening, currency)
                .map_err(|_| StorageError::InvalidData)?,
            // A sum beyond +/-(2^53-1) is reported, never wrapped or clamped.
            balance: Money::from_minor(self.balance, currency)
                .map_err(|_| StorageError::InvalidData)?,
            archived: self.archived != 0,
            sort_order: self.sort_order,
            created_at: self.created_at,
        })
    }
}

impl Db {
    /// The single currency of this installation (decision K1). Until a first-run
    /// choice exists in the UI, an unset value means IDR.
    pub fn app_currency(&self) -> Result<Currency, StorageError> {
        let code: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = 'currency'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match code {
            Some(code) => Currency::from_code(&code).map_err(|_| StorageError::InvalidData),
            None => Ok(DEFAULT_CURRENCY),
        }
    }

    /// Creates an account and returns its id.
    pub fn create_account(
        &mut self,
        draft: &AccountDraft,
        now_unix: i64,
    ) -> Result<i64, StorageError> {
        if draft.opening_balance().currency() != self.app_currency()? {
            return Err(StorageError::CurrencyMismatch);
        }
        let conn = self.conn();
        conn.execute(
            "INSERT INTO accounts(name, kind, currency, opening_balance_minor, sort_order, created_at)
             VALUES (?1, ?2, ?3, ?4,
                     (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM accounts), ?5)",
            params![
                draft.name(),
                draft.kind().code(),
                draft.opening_balance().currency().code(),
                draft.opening_balance().minor(),
                now_unix
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Replaces name, kind and opening balance. The currency cannot change.
    pub fn update_account(&mut self, id: i64, draft: &AccountDraft) -> Result<(), StorageError> {
        let existing = self.account(id)?.ok_or(StorageError::NotFound)?;
        if existing.opening_balance.currency() != draft.opening_balance().currency() {
            return Err(StorageError::CurrencyMismatch);
        }
        self.conn().execute(
            "UPDATE accounts SET name = ?1, kind = ?2, opening_balance_minor = ?3 WHERE id = ?4",
            params![
                draft.name(),
                draft.kind().code(),
                draft.opening_balance().minor(),
                id
            ],
        )?;
        Ok(())
    }

    /// Archives or restores an account. Accounts are never deleted.
    ///
    /// Restoring fails with `Duplicate` if an active account already uses the name.
    pub fn set_account_archived(&mut self, id: i64, archived: bool) -> Result<(), StorageError> {
        let changed = self.conn().execute(
            "UPDATE accounts SET archived = ?1 WHERE id = ?2",
            params![i64::from(archived), id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    pub fn account(&self, id: i64) -> Result<Option<Account>, StorageError> {
        let raw = self
            .conn()
            .query_row(&format!("{SELECT} WHERE a.id = ?1"), params![id], read_raw)
            .optional()?;
        raw.map(RawAccount::into_account).transpose()
    }

    /// Active accounts first, then archived ones when requested.
    pub fn accounts(&self, include_archived: bool) -> Result<Vec<Account>, StorageError> {
        let sql = format!(
            "{SELECT} WHERE (?1 = 1 OR a.archived = 0) ORDER BY a.archived, a.sort_order, a.id"
        );
        let mut statement = self.conn().prepare(&sql)?;
        let rows = statement.query_map(params![i64::from(include_archived)], read_raw)?;
        rows.map(|row| row?.into_account()).collect()
    }
}

fn read_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawAccount> {
    Ok(RawAccount {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        currency: row.get(3)?,
        opening: row.get(4)?,
        archived: row.get(5)?,
        sort_order: row.get(6)?,
        created_at: row.get(7)?,
        balance: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use mt_core::MAX_MINOR;
    use zeroize::Zeroizing;

    use super::*;

    fn db() -> Db {
        let db = Db::open_in_memory(&Zeroizing::new([4u8; 32])).unwrap();
        db.conn()
            .execute_batch("INSERT INTO categories(name,kind) VALUES('Food','expense');")
            .unwrap();
        db
    }

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::IDR).unwrap()
    }

    fn draft(name: &str, kind: AccountKind, opening: i64) -> AccountDraft {
        AccountDraft::new(name, kind, idr(opening)).unwrap()
    }

    fn spend(db: &Db, account: i64, minor: i64) {
        db.conn()
            .execute(
                "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,created_at)
                 VALUES(?1, 1, ?2, ?3, '2026-10-14', 0)",
                params![account, minor, if minor < 0 { "expense" } else { "income" }],
            )
            .unwrap();
    }

    #[test]
    fn creates_and_reads_back_an_account() {
        let mut db = db();
        let id = db
            .create_account(&draft("Dompet", AccountKind::Cash, 50_000), 1_700_000_000)
            .unwrap();
        let account = db.account(id).unwrap().unwrap();
        assert_eq!(account.name, "Dompet");
        assert_eq!(account.kind, AccountKind::Cash);
        assert_eq!(account.opening_balance, idr(50_000));
        assert_eq!(account.balance, idr(50_000));
        assert!(!account.archived);
        assert_eq!(account.created_at, 1_700_000_000);
    }

    #[test]
    fn balance_is_opening_plus_transactions() {
        let mut db = db();
        let id = db
            .create_account(&draft("BCA", AccountKind::Bank, 1_000_000), 0)
            .unwrap();
        spend(&db, id, -250_000);
        spend(&db, id, -100_000);
        spend(&db, id, 40_000);
        let account = db.account(id).unwrap().unwrap();
        assert_eq!(account.balance, idr(690_000));
        assert_eq!(account.opening_balance, idr(1_000_000));
    }

    #[test]
    fn balances_are_per_account_and_may_be_negative() {
        let mut db = db();
        let card = db
            .create_account(&draft("Kartu", AccountKind::Bank, 0), 0)
            .unwrap();
        let cash = db
            .create_account(&draft("Tunai", AccountKind::Cash, 10_000), 0)
            .unwrap();
        spend(&db, card, -75_000);
        spend(&db, cash, -1_000);
        assert_eq!(db.account(card).unwrap().unwrap().balance, idr(-75_000));
        assert_eq!(db.account(cash).unwrap().unwrap().balance, idr(9_000));
    }

    #[test]
    fn a_negative_opening_balance_is_allowed() {
        let mut db = db();
        let id = db
            .create_account(&draft("Utang", AccountKind::Bank, -500_000), 0)
            .unwrap();
        assert_eq!(db.account(id).unwrap().unwrap().balance, idr(-500_000));
    }

    #[test]
    fn list_orders_active_accounts_by_creation_and_hides_archived() {
        let mut db = db();
        let a = db
            .create_account(&draft("Alpha", AccountKind::Cash, 0), 0)
            .unwrap();
        let b = db
            .create_account(&draft("Beta", AccountKind::Bank, 0), 0)
            .unwrap();
        let c = db
            .create_account(&draft("Gamma", AccountKind::Ewallet, 0), 0)
            .unwrap();
        db.set_account_archived(b, true).unwrap();

        let ids = |accounts: Vec<Account>| accounts.into_iter().map(|x| x.id).collect::<Vec<_>>();
        assert_eq!(ids(db.accounts(false).unwrap()), [a, c]);
        // Archived accounts come last when included.
        assert_eq!(ids(db.accounts(true).unwrap()), [a, c, b]);
    }

    #[test]
    fn archived_accounts_keep_their_balance() {
        let mut db = db();
        let id = db
            .create_account(&draft("Lama", AccountKind::Bank, 100), 0)
            .unwrap();
        spend(&db, id, 900);
        db.set_account_archived(id, true).unwrap();
        let account = db.account(id).unwrap().unwrap();
        assert!(account.archived);
        assert_eq!(account.balance, idr(1_000));
        db.set_account_archived(id, false).unwrap();
        assert!(!db.account(id).unwrap().unwrap().archived);
    }

    #[test]
    fn update_changes_name_kind_and_opening_balance() {
        let mut db = db();
        let id = db
            .create_account(&draft("Old", AccountKind::Cash, 1_000), 0)
            .unwrap();
        spend(&db, id, -300);
        db.update_account(id, &draft("New name", AccountKind::Ewallet, 2_000))
            .unwrap();
        let account = db.account(id).unwrap().unwrap();
        assert_eq!(account.name, "New name");
        assert_eq!(account.kind, AccountKind::Ewallet);
        assert_eq!(account.opening_balance, idr(2_000));
        assert_eq!(account.balance, idr(1_700));
    }

    #[test]
    fn active_names_are_unique_ignoring_case_and_archiving_frees_them() {
        let mut db = db();
        let first = db
            .create_account(&draft("Tunai", AccountKind::Cash, 0), 0)
            .unwrap();
        assert_eq!(
            db.create_account(&draft("tunai", AccountKind::Cash, 0), 0),
            Err(StorageError::Duplicate)
        );
        db.set_account_archived(first, true).unwrap();
        let second = db
            .create_account(&draft("Tunai", AccountKind::Cash, 0), 0)
            .unwrap();
        // Restoring the first would now clash with the active "Tunai".
        assert_eq!(
            db.set_account_archived(first, false),
            Err(StorageError::Duplicate)
        );
        // Renaming into a clash is rejected too.
        let other = db
            .create_account(&draft("Lain", AccountKind::Bank, 0), 0)
            .unwrap();
        assert_eq!(
            db.update_account(other, &draft("TUNAI", AccountKind::Bank, 0)),
            Err(StorageError::Duplicate)
        );
        assert!(db.account(second).unwrap().is_some());
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let mut db = db();
        assert_eq!(db.account(99).unwrap(), None);
        assert_eq!(
            db.set_account_archived(99, true),
            Err(StorageError::NotFound)
        );
        assert_eq!(
            db.update_account(99, &draft("X", AccountKind::Cash, 0)),
            Err(StorageError::NotFound)
        );
    }

    #[test]
    fn currency_is_fixed() {
        let mut db = db();
        let usd = AccountDraft::new(
            "Dollar",
            AccountKind::Cash,
            Money::from_minor(100, Currency::USD).unwrap(),
        )
        .unwrap();
        // The installation currency is IDR (K1).
        assert_eq!(
            db.create_account(&usd, 0),
            Err(StorageError::CurrencyMismatch)
        );

        let id = db
            .create_account(&draft("Rupiah", AccountKind::Cash, 0), 0)
            .unwrap();
        assert_eq!(
            db.update_account(id, &usd),
            Err(StorageError::CurrencyMismatch)
        );
    }

    #[test]
    fn a_balance_beyond_the_money_range_is_an_error_not_a_wrap() {
        let mut db = db();
        let id = db
            .create_account(&draft("Besar", AccountKind::Bank, MAX_MINOR), 0)
            .unwrap();
        spend(&db, id, 1);
        assert_eq!(db.account(id), Err(StorageError::InvalidData));
        assert_eq!(db.accounts(true), Err(StorageError::InvalidData));
    }

    #[test]
    fn new_accounts_append_to_the_end() {
        let mut db = db();
        let a = db
            .create_account(&draft("A", AccountKind::Cash, 0), 0)
            .unwrap();
        let b = db
            .create_account(&draft("B", AccountKind::Cash, 0), 0)
            .unwrap();
        let orders: Vec<i64> = db
            .accounts(true)
            .unwrap()
            .iter()
            .map(|x| x.sort_order)
            .collect();
        assert!(orders.windows(2).all(|w| w[0] < w[1]), "{a} {b} {orders:?}");
    }
}
