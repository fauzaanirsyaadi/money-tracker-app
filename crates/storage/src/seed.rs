//! Deterministic demo data, for the query-plan tests and the benchmarks only.
//!
//! Compiled for tests and under the `bench-support` feature; never part of the app.

use mt_core::{AccountDraft, AccountKind, Currency, Money};

use crate::Db;

impl Db {
    /// Five accounts and `rows` expense transactions spread over five years, built
    /// inside SQLite so that 100 000 rows take a moment, not a minute.
    #[doc(hidden)]
    #[allow(
        clippy::unwrap_used,
        reason = "demo data: a failure here is a failed run"
    )]
    pub fn seed_demo(&mut self, rows: usize) {
        let idr = |m| Money::from_minor(m, Currency::IDR).unwrap();
        for n in 0..5 {
            self.create_account(
                &AccountDraft::new(&format!("Rekening {n}"), AccountKind::Bank, idr(1_000_000))
                    .unwrap(),
                0,
            )
            .unwrap();
        }
        let expense_categories: i64 = self
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE kind = 'expense'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(expense_categories > 0);
        self.conn_mut()
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1),
                      c(id, k) AS (SELECT id, row_number() OVER (ORDER BY id) - 1
                                   FROM categories WHERE kind = 'expense')
                 INSERT INTO transactions(account_id, category_id, amount_minor, kind, note, occurred_on, created_at)
                 SELECT 1 + (n.i % 5),
                        c.id,
                        -(1000 + (n.i * 7919) % 500000),
                        'expense',
                        'catatan belanja ' || n.i,
                        date('2022-01-01', '+' || ((n.i * 37) % 1825) || ' days'),
                        0
                 FROM n JOIN c ON c.k = n.i % ?2",
                rusqlite::params![i64::try_from(rows).unwrap(), expense_categories],
            )
            .unwrap();
    }
}
