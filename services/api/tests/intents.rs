//! NEAR Intents collateral against a fake 1Click that answers in the real API's shapes.

use std::sync::Mutex;

use quill_api::intents::{
    iso8601, quote, status, tokens, OneClick, OneClickError, QuoteRequest, NEAR_USDC,
};
use quill_api::service::ApiError;
use serde_json::{json, Value};

const BASE_USDC: &str = "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near";
const NOW: u64 = 1_762_905_600; // 2025-11-12T00:00:00Z

/// One way to spoil a good quote request.
type Change = Box<dyn Fn(&mut QuoteRequest)>;

#[derive(Default)]
struct FakeOneClick {
    tokens: Value,
    quote: Option<Result<Value, OneClickError>>,
    status: Option<Result<Value, OneClickError>>,
    sent: Mutex<Option<Value>>,
}

impl OneClick for FakeOneClick {
    fn tokens(&self) -> Result<Value, OneClickError> {
        Ok(self.tokens.clone())
    }
    fn quote(&self, request: &Value) -> Result<Value, OneClickError> {
        *self.sent.lock().unwrap() = Some(request.clone());
        self.quote.clone().expect("no quote prepared")
    }
    fn status(&self, _deposit_address: &str) -> Result<Value, OneClickError> {
        self.status.clone().expect("no status prepared")
    }
}

fn request(dry: bool) -> QuoteRequest {
    QuoteRequest {
        origin_asset: BASE_USDC.into(),
        amount_out: "350000000".into(),
        recipient: "hedger.testnet".into(),
        refund_to: "0x4838B106FCe9647Bdf1E7877BF73cE8B0BAD5f97".into(),
        dry,
    }
}

fn answer(deposit: Option<&str>) -> Value {
    let mut q = json!({ "amountIn": "351060000", "amountInFormatted": "351.06", "amountOut": "350000000",
                        "amountOutFormatted": "350.0", "timeEstimate": 20 });
    if let Some(address) = deposit {
        q["depositAddress"] = json!(address);
    }
    json!({ "quote": q, "quoteRequest": {} })
}

#[test]
fn tokens_are_parsed_sorted_and_malformed_entries_skipped() {
    let fake = FakeOneClick {
        tokens: json!([
            { "assetId": "nep141:wrap.near", "symbol": "wNEAR", "blockchain": "near", "decimals": 24, "price": 2.5 },
            { "assetId": BASE_USDC, "symbol": "USDC", "blockchain": "base", "decimals": 6, "price": 1.0 },
            { "assetId": "nep141:eth-usdc", "symbol": "USDC", "blockchain": "eth", "decimals": 6 },
            { "symbol": "BROKEN" },
        ]),
        ..Default::default()
    };
    let list = tokens(&fake).unwrap();
    let names: Vec<_> = list
        .iter()
        .map(|t| format!("{}/{}", t.symbol, t.blockchain))
        .collect();
    assert_eq!(names, ["USDC/base", "USDC/eth", "wNEAR/near"]);
    assert_eq!(
        list[1].price, None,
        "a missing price stays missing, never 0"
    );
}

#[test]
fn a_token_list_that_is_not_a_list_is_an_upstream_error() {
    let fake = FakeOneClick {
        tokens: json!({ "message": "rate limited" }),
        ..Default::default()
    };
    assert!(matches!(tokens(&fake), Err(ApiError::Upstream(_))));
}

#[test]
fn a_quote_asks_for_exact_usdc_on_near_for_the_users_own_account() {
    let fake = FakeOneClick {
        quote: Some(Ok(answer(None))),
        ..Default::default()
    };
    let q = quote(&fake, &request(true), NOW).unwrap();
    assert_eq!(q.amount_in, "351060000");
    assert_eq!(q.amount_out_formatted, "350.0");
    assert_eq!(q.deposit_address, None);
    assert_eq!(q.time_estimate_secs, Some(20));
    assert_eq!(q.deadline, "2025-11-12T00:30:00.000Z");

    let sent = fake.sent.lock().unwrap().clone().unwrap();
    assert_eq!(sent["swapType"], "EXACT_OUTPUT");
    assert_eq!(sent["destinationAsset"], NEAR_USDC);
    assert_eq!(
        sent["recipient"], "hedger.testnet",
        "the USDC goes to the user, never to Quill"
    );
    assert_eq!(sent["recipientType"], "DESTINATION_CHAIN");
    assert_eq!(
        sent["refundTo"],
        "0x4838B106FCe9647Bdf1E7877BF73cE8B0BAD5f97"
    );
    assert_eq!(sent["refundType"], "ORIGIN_CHAIN");
    assert_eq!(sent["amount"], "350000000");
    assert_eq!(sent["dry"], true);
}

#[test]
fn a_real_quote_must_come_with_a_deposit_address() {
    let missing = FakeOneClick {
        quote: Some(Ok(answer(None))),
        ..Default::default()
    };
    assert!(matches!(
        quote(&missing, &request(false), NOW),
        Err(ApiError::Upstream(_))
    ));

    let present = FakeOneClick {
        quote: Some(Ok(answer(Some("0xDepositAddress0001")))),
        ..Default::default()
    };
    let q = quote(&present, &request(false), NOW).unwrap();
    assert_eq!(q.deposit_address.as_deref(), Some("0xDepositAddress0001"));
    assert!(!q.dry);
}

#[test]
fn bad_quote_requests_are_refused_before_1click_is_asked() {
    let fake = FakeOneClick::default(); // panics if asked
    let cases: Vec<(&str, Change)> = vec![
        ("zero", Box::new(|r| r.amount_out = "0".into())),
        ("decimal", Box::new(|r| r.amount_out = "1.5".into())),
        ("negative", Box::new(|r| r.amount_out = "-5".into())),
        (
            "too large",
            Box::new(|r| r.amount_out = "1000000000001".into()),
        ),
        (
            "recipient",
            Box::new(|r| r.recipient = "Not An Account".into()),
        ),
        (
            "recipient dot",
            Box::new(|r| r.recipient = ".hedger.testnet".into()),
        ),
        ("refund", Box::new(|r| r.refund_to = "0x 12".into())),
        ("refund short", Box::new(|r| r.refund_to = "0x12".into())),
        (
            "asset",
            Box::new(|r| r.origin_asset = "usdc-on-base".into()),
        ),
    ];
    for (name, change) in cases {
        let mut r = request(true);
        change(&mut r);
        assert!(
            matches!(quote(&fake, &r, NOW), Err(ApiError::BadRequest(_))),
            "{name} should be refused"
        );
    }
}

#[test]
fn a_1click_failure_is_an_upstream_error_with_its_message() {
    let fake = FakeOneClick {
        quote: Some(Err(OneClickError::Status(
            400,
            "Amount is too low for bridge".into(),
        ))),
        ..Default::default()
    };
    match quote(&fake, &request(true), NOW) {
        Err(ApiError::Upstream(why)) => {
            assert_eq!(why, "1Click answered 400: Amount is too low for bridge")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn only_a_404_means_an_unknown_deposit_address() {
    for (error, not_found) in [
        (OneClickError::Status(404, "nope".into()), true),
        (OneClickError::Status(500, "nope".into()), false),
        (OneClickError::Unreachable("timed out".into()), false),
    ] {
        let fake = FakeOneClick {
            status: Some(Err(error.clone())),
            ..Default::default()
        };
        let result = status(&fake, "0xDepositAddress0001");
        assert_eq!(
            matches!(result, Err(ApiError::NotFound(_))),
            not_found,
            "{error}"
        );
        assert!(result.is_err());
    }
}

#[test]
fn a_status_without_a_status_field_is_an_upstream_error() {
    let fake = FakeOneClick {
        status: Some(Ok(json!({ "swapDetails": {} }))),
        ..Default::default()
    };
    assert!(matches!(
        status(&fake, "0xDepositAddress0001"),
        Err(ApiError::Upstream(_))
    ));
}

#[test]
fn a_quote_answer_missing_a_field_is_an_upstream_error() {
    let mut partial = answer(None);
    partial["quote"].as_object_mut().unwrap().remove("amountIn");
    let fake = FakeOneClick {
        quote: Some(Ok(partial)),
        ..Default::default()
    };
    match quote(&fake, &request(true), NOW) {
        Err(ApiError::Upstream(why)) => assert!(why.contains("amountIn")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_dry_quote_is_the_default() {
    let body = json!({ "origin_asset": BASE_USDC, "amount_out": "1", "recipient": "a.testnet", "refund_to": "0x4838B106FCe9647Bdf1E7877BF73cE8B0BAD5f97" });
    let parsed: QuoteRequest = serde_json::from_value(body).unwrap();
    assert!(parsed.dry);
}

#[test]
fn status_collects_every_transaction_hash_once() {
    let fake = FakeOneClick {
        status: Some(Ok(json!({
            "status": "SUCCESS",
            "swapDetails": {
                "originChainTxHashes": [{ "hash": "0xaaa", "explorerUrl": "" }],
                "destinationChainTxHashes": [{ "hash": "Bbb111" }, { "hash": "Bbb111" }],
                "intentHashes": ["Ccc222"],
            }
        }))),
        ..Default::default()
    };
    let s = status(&fake, "0xDepositAddress0001").unwrap();
    assert_eq!(s.status, "SUCCESS");
    assert_eq!(s.tx_hashes, ["0xaaa", "Bbb111", "Ccc222"]);
}

#[test]
fn an_unknown_deposit_address_is_404_and_a_bad_one_400() {
    let fake = FakeOneClick {
        status: Some(Err(OneClickError::Status(
            404,
            "Deposit address not found".into(),
        ))),
        ..Default::default()
    };
    assert!(matches!(
        status(&fake, "0xDepositAddress0001"),
        Err(ApiError::NotFound(_))
    ));
    assert!(matches!(status(&fake, "a b"), Err(ApiError::BadRequest(_))));
}

#[test]
fn iso8601_matches_known_dates() {
    assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(iso8601(NOW + 3725), "2025-11-12T01:02:05.000Z");
    assert_eq!(
        iso8601(951_782_400),
        "2000-02-29T00:00:00.000Z",
        "leap day in a century leap year"
    );
    assert_eq!(
        iso8601(4_107_542_399),
        "2100-02-28T23:59:59.000Z",
        "2100 is not a leap year"
    );
}
