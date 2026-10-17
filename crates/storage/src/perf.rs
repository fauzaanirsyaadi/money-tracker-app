//! Query plans and timings at 100 000 rows (Phase 8).
//!
//! The plan checks run on every test run, on exactly the SQL the app executes
//! (`page_query`, the report constants, the account SELECT). The timings are in an
//! ignored test, because they need a release build to mean anything:
//!
//!     cargo test -p mt-storage --release --test-threads=1 -- --ignored --nocapture perf

use mt_core::{AccountDraft, AccountKind, Currency, Money, parse_iso_date};
use rusqlite::params_from_iter;

use crate::{
    Db, DbKey,
    accounts::SELECT as ACCOUNTS_SELECT,
    reports::{SPENDING_SQL, TREND_BEFORE_SQL, TREND_SQL},
    transactions::{Cursor, KindFilter, TransactionFilter, page_query},
};

fn db_with(rows: usize) -> Db {
    let mut db = Db::open_in_memory(&DbKey::new([4u8; 32])).unwrap();
    let idr = |m| Money::from_minor(m, Currency::IDR).unwrap();
    for n in 0..5 {
        db.create_account(
            &AccountDraft::new(&format!("Rekening {n}"), AccountKind::Bank, idr(1_000_000))
                .unwrap(),
            0,
        )
        .unwrap();
    }
    let expense_categories: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE kind = 'expense'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(expense_categories > 0);
    // Rows are generated inside SQLite so that 100 000 of them take a moment, not a minute.
    db.conn_mut()
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
    db
}

/// The plan of `sql`, one line per step.
fn plan(db: &Db, sql: &str, args: Vec<rusqlite::types::Value>) -> String {
    let mut statement = db
        .conn()
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    let rows = statement
        .query_map(params_from_iter(args), |r| r.get::<_, String>(3))
        .unwrap();
    rows.map(|row| row.unwrap()).collect::<Vec<_>>().join(" | ")
}

fn date(text: &str) -> mt_core::Date {
    parse_iso_date(text).unwrap()
}

fn filters() -> Vec<(&'static str, TransactionFilter, Option<Cursor>)> {
    let base = TransactionFilter::default();
    vec![
        ("newest first", base.clone(), None),
        (
            "after a cursor",
            base.clone(),
            Some(Cursor {
                occurred_on: date("2024-06-01"),
                id: 50_000,
            }),
        ),
        (
            "one account",
            TransactionFilter {
                account_id: Some(2),
                ..base.clone()
            },
            None,
        ),
        (
            "one category (with its children)",
            TransactionFilter {
                category_id: Some(3),
                ..base.clone()
            },
            None,
        ),
        (
            "a month",
            TransactionFilter {
                from: Some(date("2024-03-01")),
                to: Some(date("2024-03-31")),
                ..base.clone()
            },
            None,
        ),
        (
            "expenses only",
            TransactionFilter {
                kind: Some(KindFilter::Expense),
                ..base.clone()
            },
            None,
        ),
        (
            "text search",
            TransactionFilter {
                search: Some("belanja 777".into()),
                ..base
            },
            None,
        ),
    ]
}

#[test]
fn the_list_queries_use_their_indexes_and_never_sort() {
    let db = db_with(5_000);
    let mut report = String::new();
    for (name, filter, cursor) in filters() {
        let (sql, args) = page_query(&filter, cursor, 50);
        let steps = plan(&db, &sql, args);
        report.push_str(&format!("{name}: {steps}\n"));
        // Ordered by (occurred_on DESC, id DESC): an index must deliver that order.
        // The category filter is the known exception: its "this category or one of
        // its children" condition makes SQLite combine two index searches and sort
        // the union (measured in `perf_100k_rows`; see ARCHITECTURE section 33).
        if name != "one category (with its children)" {
            assert!(!steps.contains("TEMP B-TREE"), "{name} sorts: {steps}");
        }
    }
    let by_name = |wanted: &str| {
        let (_, filter, cursor) = filters().into_iter().find(|(n, ..)| *n == wanted).unwrap();
        let (sql, args) = page_query(&filter, cursor, 50);
        plan(&db, &sql, args)
    };
    assert!(by_name("newest first").contains("idx_tx_date"), "{report}");
    assert!(
        by_name("after a cursor").contains("idx_tx_date"),
        "{report}"
    );
    assert!(by_name("a month").contains("idx_tx_date"), "{report}");
    assert!(
        by_name("one account").contains("idx_tx_account"),
        "{report}"
    );
    // The category filter never scans the table: it searches idx_tx_category.
    let category = by_name("one category (with its children)");
    assert!(
        category.contains("idx_tx_category") && !category.contains("SCAN t"),
        "{category}"
    );
}

#[test]
fn the_report_queries_use_the_date_index_for_the_range() {
    let db = db_with(5_000);
    let from = rusqlite::types::Value::from("2024-01-01".to_owned());
    let to = rusqlite::types::Value::from("2024-12-31".to_owned());
    let spending = plan(&db, SPENDING_SQL, vec![from.clone(), to.clone()]);
    assert!(spending.contains("idx_tx_date"), "{spending}");
    let trend = plan(&db, TREND_SQL, vec![from.clone(), to]);
    assert!(trend.contains("idx_tx_date"), "{trend}");
    let before = plan(&db, TREND_BEFORE_SQL, vec![from]);
    assert!(before.contains("idx_tx_date"), "{before}");
}

#[test]
fn the_balance_subquery_is_answered_from_the_covering_index_alone() {
    let db = db_with(5_000);
    let steps = plan(&db, ACCOUNTS_SELECT, vec![]);
    // "COVERING INDEX": the table is never read for a balance.
    assert!(
        steps.contains("COVERING INDEX idx_tx_account_amount"),
        "{steps}"
    );
}

#[test]
#[ignore = "timings: run with --release --ignored --nocapture"]
#[allow(
    clippy::print_stdout,
    reason = "the numbers are the point of this test"
)]
fn perf_100k_rows() {
    use std::time::Instant;

    let rows = 100_000;
    let started = Instant::now();
    let db = db_with(rows);
    println!("load {rows} rows: {:?}", started.elapsed());
    let time = |name: &str, f: &dyn Fn()| {
        let started = Instant::now();
        for _ in 0..5 {
            f();
        }
        println!("{name}: {:?} per call", started.elapsed() / 5);
    };
    for (name, filter, cursor) in filters() {
        time(&format!("page 50, {name}"), &|| {
            db.transactions(&filter, cursor, 50).unwrap();
        });
    }
    time("accounts (balance per account)", &|| {
        db.accounts(true).unwrap();
    });
    time("net worth", &|| {
        db.net_worth().unwrap();
    });
    time("spending report, one month", &|| {
        db.spending_by_category(date("2024-03-01"), date("2024-03-31"))
            .unwrap();
    });
    time("spending report, one year", &|| {
        db.spending_by_category(date("2024-01-01"), date("2024-12-31"))
            .unwrap();
    });
    time("monthly trend, 12 months", &|| {
        db.monthly_trend(
            mt_core::YearMonth::new(2024, 1).unwrap(),
            mt_core::YearMonth::new(2024, 12).unwrap(),
        )
        .unwrap();
    });
}
