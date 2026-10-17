//! A stray `{:?}` on anything read back from the database must never print an
//! amount, a name or a note (review item L1).

use crate::DbKey;
use mt_core::{
    AccountDraft, AccountKind, BudgetDraft, CategoryDraft, Currency, FlowKind, Money,
    TransactionDraft, TransferDraft, YearMonth, parse_iso_date,
};

use crate::{Db, TransactionFilter};

const NOTE: &str = "gaji rahasia bulan ini";
const ACCOUNT: &str = "Tabungan Rahasia";
const CATEGORY: &str = "Kategori Rahasia";

fn money(minor: i64) -> Money {
    Money::from_minor(minor, Currency::IDR).unwrap()
}

fn clean(label: &str, text: &str) {
    for needle in ["987654321", "9876543", "rahasia", "Rahasia", "123456"] {
        assert!(!text.contains(needle), "{label} leaked {needle:?}: {text}");
    }
}

#[test]
fn records_read_from_the_database_never_print_their_contents() {
    let mut db = Db::open_in_memory(&DbKey::new([3u8; 32])).unwrap();
    let date = parse_iso_date("2026-10-14").unwrap();

    let a = db
        .create_account(
            &AccountDraft::new(ACCOUNT, AccountKind::Bank, money(987_654_321)).unwrap(),
            0,
        )
        .unwrap();
    let b = db
        .create_account(
            &AccountDraft::new("Dompet", AccountKind::Cash, money(123_456)).unwrap(),
            0,
        )
        .unwrap();
    let category = db
        .create_category(&CategoryDraft::new(CATEGORY, FlowKind::Expense, None).unwrap())
        .unwrap();
    db.create_transaction(
        &TransactionDraft::new(a, category, FlowKind::Expense, money(987_654), NOTE, date).unwrap(),
        1,
    )
    .unwrap();
    db.create_transfer(
        &TransferDraft::new(a, b, money(123_456), NOTE, date).unwrap(),
        2,
    )
    .unwrap();
    db.set_budget(
        &BudgetDraft::new(
            category,
            YearMonth::new(2026, 10).unwrap(),
            money(9_876_543),
        )
        .unwrap(),
    )
    .unwrap();

    let accounts = db.accounts(true).unwrap();
    clean("Account", &format!("{accounts:?} {:#?}", accounts[0]));

    let page = db
        .transactions(&TransactionFilter::default(), None, 50)
        .unwrap();
    assert!(!page.items.is_empty());
    clean(
        "TransactionRecord",
        &format!("{:?} {:#?}", page.items, page.items[0]),
    );

    clean("Category", &format!("{:?}", db.categories(true).unwrap()));
    clean(
        "BudgetRecord",
        &format!(
            "{:?}",
            db.budgets(YearMonth::new(2026, 10).unwrap()).unwrap()
        ),
    );
    clean("NetWorth", &format!("{:?}", db.net_worth().unwrap()));
    let report = db.spending_by_category(date, date).unwrap();
    clean("SpendingReport", &format!("{report:?}"));
    let trend = db
        .monthly_trend(
            YearMonth::new(2026, 10).unwrap(),
            YearMonth::new(2026, 10).unwrap(),
        )
        .unwrap();
    clean("MonthSummary", &format!("{trend:?}"));

    let mut dump = Vec::new();
    let summary = db.export_logical(&mut dump).unwrap();
    clean("LogicalSummary", &format!("{summary:?}"));
}
