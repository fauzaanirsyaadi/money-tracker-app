//! A stray `{:?}` on any command input or output must never print an amount,
//! a name or a note (review item L1).

use mt_core::{AccountDraft, AccountKind, Currency, Money};
use mt_storage::DbKey;
use serde_json::json;
use zeroize::Zeroizing;

use crate::{
    accounts::{AccountDto, AccountInput},
    budgets::BudgetInput,
    recurring::RecurringInput,
    transactions::{FilterInput, TransactionInput, TransferInput},
};

fn clean(label: &str, text: &str) {
    for needle in ["987654321", "987654", "rahasia", "Rahasia", "gaji"] {
        assert!(!text.contains(needle), "{label} leaked {needle:?}: {text}");
    }
}

#[test]
fn command_inputs_never_print_their_contents() {
    let account: AccountInput = serde_json::from_value(json!({
        "name": "Tabungan Rahasia", "kind": "bank", "openingBalanceMinor": "987654321"
    }))
    .unwrap();
    clean("AccountInput", &format!("{account:?}"));

    let transaction: TransactionInput = serde_json::from_value(json!({
        "accountId": 1, "categoryId": 2, "kind": "expense", "amountMinor": "987654",
        "note": "gaji rahasia", "occurredOn": "2026-10-14"
    }))
    .unwrap();
    clean("TransactionInput", &format!("{transaction:?}"));

    let transfer: TransferInput = serde_json::from_value(json!({
        "fromAccountId": 1, "toAccountId": 2, "amountMinor": "987654",
        "note": "gaji rahasia", "occurredOn": "2026-10-14"
    }))
    .unwrap();
    clean("TransferInput", &format!("{transfer:?}"));

    let filter: FilterInput = serde_json::from_value(json!({ "search": "gaji rahasia" })).unwrap();
    clean("FilterInput", &format!("{filter:?}"));

    let budget: BudgetInput = serde_json::from_value(json!({
        "categoryId": 1, "month": "2026-10", "limitMinor": "987654321"
    }))
    .unwrap();
    clean("BudgetInput", &format!("{budget:?}"));

    let recurring: RecurringInput = serde_json::from_value(json!({
        "accountId": 1, "categoryId": 2, "kind": "expense", "amountMinor": "987654321",
        "note": "gaji rahasia", "frequency": "monthly", "every": 1,
        "startOn": "2026-10-01", "endOn": null
    }))
    .unwrap();
    clean("RecurringInput", &format!("{recurring:?}"));
}

#[test]
fn command_outputs_never_print_their_contents() {
    let mut db = mt_storage::Db::open_in_memory(&DbKey::new([5u8; 32])).unwrap();
    let id = db
        .create_account(
            &AccountDraft::new(
                "Tabungan Rahasia",
                AccountKind::Bank,
                Money::from_minor(987_654_321, Currency::IDR).unwrap(),
            )
            .unwrap(),
            0,
        )
        .unwrap();
    let account = db.account(id).unwrap().unwrap();
    let dto = AccountDto::from(&account);
    clean("Account", &format!("{account:?}"));
    clean("AccountDto", &format!("{dto:?}"));
}

#[test]
fn secrets_arrive_from_the_webview_directly_as_zeroizing_strings() {
    // Commands take `Zeroizing<String>` for the PIN and the passphrase, so no
    // un-wiped `String` copy is made while deserialising them.
    let pin: Zeroizing<String> = serde_json::from_str("\"493017\"").unwrap();
    assert_eq!(pin.as_str(), "493017");
    let args: serde_json::Value = serde_json::json!({ "current": "493017", "new": "718204" });
    let current: Zeroizing<String> = serde_json::from_value(args["current"].clone()).unwrap();
    assert_eq!(current.as_str(), "493017");
}
