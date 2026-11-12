//! Checking feed payments against the shape the NEAR RPC `tx` method returns.

use base64::Engine;
use quill_api::feed::{valid_tx_hash, verify, Ledger, Paid, Terms};
use quill_api::service::ApiError;
use serde_json::{json, Value};

const PAYER: &str = "agent.testnet";

fn terms() -> Terms {
    Terms {
        asset: "usdc.quill.testnet".into(),
        pay_to: "quill.testnet".into(),
        price: 10_000,
    }
}

fn args(receiver: &str, amount: &str) -> String {
    base64::engine::general_purpose::STANDARD
        .encode(json!({ "receiver_id": receiver, "amount": amount }).to_string())
}

/// A successful `ft_transfer` as the RPC reports it.
fn payment(receiver: &str, amount: &str) -> Value {
    json!({
        "status": { "SuccessValue": "" },
        "transaction": {
            "signer_id": PAYER,
            "receiver_id": "usdc.quill.testnet",
            "hash": "9xQ7m3cY2zYbX1pW4uR8tS6vN5kL3jH2gF1dD9aB7cE",
            "actions": [{ "FunctionCall": {
                "method_name": "ft_transfer", "args": args(receiver, amount),
                "gas": 30_000_000_000_000u64, "deposit": "1" } }]
        },
        "receipts_outcome": [
            { "outcome": { "status": { "SuccessValue": "" }, "logs": [] } }
        ]
    })
}

fn refused(result: Result<Paid, ApiError>, expected: &str) {
    match result {
        Err(ApiError::BadRequest(why)) => assert!(why.contains(expected), "{why}"),
        other => panic!("expected a refusal containing `{expected}`, got {other:?}"),
    }
}

#[test]
fn a_correct_payment_is_accepted_with_its_amount() {
    assert_eq!(
        verify(&payment("quill.testnet", "10000"), PAYER, &terms()),
        Ok(Paid {
            payer: PAYER.into(),
            amount: 10_000
        })
    );
    // Paying more is fine.
    assert_eq!(
        verify(&payment("quill.testnet", "25000"), PAYER, &terms()).map(|p| p.amount),
        Ok(25_000)
    );
}

#[test]
fn paying_less_than_the_price_is_refused() {
    refused(
        verify(&payment("quill.testnet", "9999"), PAYER, &terms()),
        "less than the price",
    );
}

#[test]
fn paying_someone_else_is_refused() {
    refused(
        verify(&payment("mallory.testnet", "10000"), PAYER, &terms()),
        "someone else",
    );
}

#[test]
fn someone_elses_transaction_is_refused() {
    refused(
        verify(
            &payment("quill.testnet", "10000"),
            "thief.testnet",
            &terms(),
        ),
        "not signed by the payer",
    );
}

#[test]
fn a_transfer_of_another_token_is_refused() {
    let mut tx = payment("quill.testnet", "10000");
    tx["transaction"]["receiver_id"] = json!("fake-usdc.testnet");
    refused(verify(&tx, PAYER, &terms()), "feed's token");
}

#[test]
fn a_failed_transaction_is_refused() {
    let mut tx = payment("quill.testnet", "10000");
    tx["status"] = json!({ "Failure": { "ActionError": {} } });
    refused(verify(&tx, PAYER, &terms()), "did not succeed");
}

#[test]
fn a_failed_receipt_is_refused_even_if_the_transaction_reports_success() {
    let mut tx = payment("quill.testnet", "10000");
    tx["receipts_outcome"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "outcome": { "status": { "Failure": { "ActionError": {} } } } }));
    refused(
        verify(&tx, PAYER, &terms()),
        "part of the transaction failed",
    );
}

#[test]
fn only_a_single_plain_ft_transfer_counts() {
    let mut call = payment("quill.testnet", "10000");
    call["transaction"]["actions"][0]["FunctionCall"]["method_name"] = json!("ft_transfer_call");
    refused(verify(&call, PAYER, &terms()), "not an ft_transfer");

    let mut two = payment("quill.testnet", "10000");
    let action = two["transaction"]["actions"][0].clone();
    two["transaction"]["actions"]
        .as_array_mut()
        .unwrap()
        .push(action);
    refused(verify(&two, PAYER, &terms()), "exactly one action");

    let mut none = payment("quill.testnet", "10000");
    none["transaction"]["actions"] = json!([]);
    refused(verify(&none, PAYER, &terms()), "exactly one action");
}

#[test]
fn unreadable_arguments_are_refused() {
    let mut garbage = payment("quill.testnet", "10000");
    garbage["transaction"]["actions"][0]["FunctionCall"]["args"] = json!("not base64 !!");
    refused(
        verify(&garbage, PAYER, &terms()),
        "arguments cannot be read",
    );

    let mut number = payment("quill.testnet", "10000");
    number["transaction"]["actions"][0]["FunctionCall"]["args"] =
        json!(base64::engine::general_purpose::STANDARD
            .encode(json!({ "receiver_id": "quill.testnet", "amount": 10000 }).to_string()));
    refused(verify(&number, PAYER, &terms()), "amount cannot be read");
}

#[test]
fn a_receipt_buys_one_read() {
    let ledger = Ledger::in_memory();
    assert!(ledger.spend("hash-a"));
    assert!(!ledger.spend("hash-a"));
    assert!(ledger.spend("hash-b"));
}

#[test]
fn spent_receipts_survive_a_restart() {
    let dir = std::env::temp_dir().join(format!("quill-ledger-{}", std::process::id()));
    let file = dir.join("spent.txt");
    let _ = std::fs::remove_dir_all(&dir);

    let ledger = Ledger::open(file.clone());
    assert!(ledger.spend("hash-a"));
    drop(ledger);

    let reopened = Ledger::open(file);
    assert!(
        !reopened.spend("hash-a"),
        "a restart must not make a receipt spendable again"
    );
    assert!(reopened.spend("hash-b"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transaction_hashes_are_base58_of_the_right_length() {
    assert!(valid_tx_hash("9xQ7m3cY2zYbX1pW4uR8tS6vN5kL3jH2gF1dD9aB7cE"));
    assert!(!valid_tx_hash(""));
    assert!(!valid_tx_hash("short"));
    assert!(
        !valid_tx_hash("0xQ7m3cY2zYbX1pW4uR8tS6vN5kL3jH2gF1dD9aB7cE"),
        "0 is not base58"
    );
    assert!(!valid_tx_hash(
        "9xQ7m3cY2zYbX1pW4uR8tS6vN5kL3jH2gF1dD9aB7c/"
    ));
}
