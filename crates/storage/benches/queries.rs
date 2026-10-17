//! Query and logical-backup benchmarks at 100 000 transactions.
//!
//!     cargo bench -p mt-storage --features bench-support
//!
//! Numbers from a developer machine are **relative only**: use them to compare a change
//! against the one before it (criterion keeps the previous run), never as a promise about a
//! phone. Results are summarised in docs/PERFORMANCE.md.

#![allow(
    clippy::unwrap_used,
    reason = "a benchmark that fails to set up has failed"
)]

use std::{hint::black_box, io::Cursor, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};
use mt_core::parse_iso_date;
use mt_storage::{Db, DbKey, TransactionFilter};

const ROWS: usize = 100_000;

fn key() -> DbKey {
    DbKey::new([4u8; 32])
}

fn seeded() -> Db {
    let mut db = Db::open_in_memory(&key()).unwrap();
    db.seed_demo(ROWS);
    db
}

fn date(text: &str) -> mt_core::Date {
    parse_iso_date(text).unwrap()
}

fn lists(c: &mut Criterion) {
    let db = seeded();
    let mut group = c.benchmark_group("list page of 50 (100k rows)");
    let base = TransactionFilter::default();
    let cursor = mt_storage::Cursor {
        occurred_on: date("2024-06-01"),
        id: 50_000,
    };
    let cases: Vec<(&str, TransactionFilter, Option<mt_storage::Cursor>)> = vec![
        ("newest first", base.clone(), None),
        ("after a cursor", base.clone(), Some(cursor)),
        (
            "one account",
            TransactionFilter {
                account_id: Some(2),
                ..base.clone()
            },
            None,
        ),
        (
            "one category with children",
            TransactionFilter {
                category_id: Some(3),
                ..base.clone()
            },
            None,
        ),
        (
            "one month",
            TransactionFilter {
                from: Some(date("2024-03-01")),
                to: Some(date("2024-03-31")),
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
    ];
    for (name, filter, cursor) in cases {
        group.bench_function(name, |b| {
            b.iter(|| black_box(db.transactions(&filter, cursor, 50).unwrap()))
        });
    }
    group.finish();
}

fn balances_and_reports(c: &mut Criterion) {
    let db = seeded();
    let mut group = c.benchmark_group("balances and reports (100k rows)");
    group.bench_function("account list (a balance each)", |b| {
        b.iter(|| black_box(db.accounts(true).unwrap()))
    });
    group.bench_function("net worth", |b| {
        b.iter(|| black_box(db.net_worth().unwrap()))
    });
    group.bench_function("spending, one month", |b| {
        b.iter(|| {
            black_box(
                db.spending_by_category(date("2024-03-01"), date("2024-03-31"))
                    .unwrap(),
            )
        });
    });
    group.bench_function("spending, one year", |b| {
        b.iter(|| {
            black_box(
                db.spending_by_category(date("2024-01-01"), date("2024-12-31"))
                    .unwrap(),
            )
        });
    });
    group.bench_function("monthly trend, 12 months", |b| {
        b.iter(|| {
            black_box(
                db.monthly_trend(
                    mt_core::YearMonth::new(2024, 1).unwrap(),
                    mt_core::YearMonth::new(2024, 12).unwrap(),
                )
                .unwrap(),
            )
        });
    });
    group.finish();
}

/// Logical export and restore, without encryption (that is benchmarked in mt-backup).
fn logical_backup(c: &mut Criterion) {
    let db = seeded();
    let mut dump = Vec::new();
    db.export_logical(&mut dump).unwrap();

    let mut group = c.benchmark_group("logical backup (100k rows)");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(15));
    group.bench_function("export", |b| {
        b.iter(|| black_box(db.export_logical(&mut std::io::sink()).unwrap()));
    });
    let dir = std::env::temp_dir().join(format!("mt-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    group.bench_function("restore into a new database and verify", |b| {
        b.iter(|| {
            let path = dir.join("restored.db");
            let (restored, summary) =
                Db::restore_logical(&path, &key(), Cursor::new(&dump)).unwrap();
            drop(restored);
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
            }
            black_box(summary)
        });
    });
    group.finish();
    let _ = std::fs::remove_dir_all(dir);
}

criterion_group!(benches, lists, balances_and_reports, logical_backup);
criterion_main!(benches);
