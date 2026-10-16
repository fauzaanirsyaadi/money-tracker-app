#![allow(clippy::unwrap_used, reason = "test helpers build fixtures")]
//! A stray `{:?}` must never print an amount, a name or a note (review item L1).

use mt_core::{
    AccountDraft, AccountKind, BudgetDraft, CategoryDraft, Currency, Date, FlowKind, Money,
    TransactionDraft, TransferDraft, YearMonth, progress,
};
use time::Month;

const AMOUNT: i64 = 123_456_789;
const SECRET_NOTE: &str = "gaji rahasia bulan ini";
const SECRET_NAME: &str = "Tabungan Rahasia";

fn money(minor: i64) -> Money {
    Money::from_minor(minor, Currency::IDR).unwrap()
}

fn date() -> Date {
    Date::from_calendar_date(2026, Month::October, 14).unwrap()
}

fn assert_clean(label: &str, text: &str) {
    for needle in [
        "123456789",
        "123_456_789",
        "456789",
        SECRET_NOTE,
        SECRET_NAME,
        "rahasia",
    ] {
        assert!(!text.contains(needle), "{label} leaked {needle:?}: {text}");
    }
}

#[test]
fn money_prints_its_currency_but_never_the_amount() {
    let text = format!("{:?}", money(AMOUNT));
    assert_eq!(text, "Money(<redacted> IDR)");
    assert_clean("Money", &text);
    // Also inside containers and in the alternate (pretty) form.
    assert_clean(
        "Vec<Money>",
        &format!("{:?}", vec![money(AMOUNT), money(-AMOUNT)]),
    );
    assert_clean("Option<Money>", &format!("{:#?}", Some(money(AMOUNT))));
    assert_clean(
        "Result<Money, _>",
        &format!("{:?}", Ok::<_, ()>(money(AMOUNT))),
    );
}

#[test]
fn every_value_object_that_holds_money_or_text_is_redacted() {
    let transaction =
        TransactionDraft::new(1, 2, FlowKind::Expense, money(AMOUNT), SECRET_NOTE, date()).unwrap();
    assert_clean(
        "TransactionDraft",
        &format!("{transaction:?} {transaction:#?}"),
    );

    let transfer = TransferDraft::new(1, 2, money(AMOUNT), SECRET_NOTE, date()).unwrap();
    assert_clean("TransferDraft", &format!("{transfer:?}"));

    let account = AccountDraft::new(SECRET_NAME, AccountKind::Bank, money(AMOUNT)).unwrap();
    assert_clean("AccountDraft", &format!("{account:?}"));

    let category = CategoryDraft::new(SECRET_NAME, FlowKind::Expense, None).unwrap();
    assert_clean("CategoryDraft", &format!("{category:?}"));

    let month = YearMonth::new(2026, 10).unwrap();
    let budget = BudgetDraft::new(1, month, money(AMOUNT)).unwrap();
    assert_clean("BudgetDraft", &format!("{budget:?}"));
    let progress = progress(money(AMOUNT), money(AMOUNT / 2)).unwrap();
    assert_clean("BudgetProgress", &format!("{progress:?}"));
}

#[test]
fn redacted_types_still_say_what_they_are() {
    let account = AccountDraft::new(SECRET_NAME, AccountKind::Bank, money(1)).unwrap();
    assert_eq!(format!("{account:?}"), "AccountDraft { .. }");
}

#[test]
fn errors_carry_no_values_and_stay_derived() {
    // Error types hold no amounts, so they keep their derived Debug.
    let overflow = Money::from_minor(i64::MAX, Currency::IDR).unwrap_err();
    assert_eq!(format!("{overflow:?}"), "Overflow");
}
