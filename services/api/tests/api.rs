//! The API's answers, against a fake chain that returns what the contracts return.

use std::collections::HashMap;
use std::sync::Arc;

use quill_api::chain::{decode_view, Chain};
use quill_api::service::{self, ApiError, Contracts};
use quill_api::{router, AppState};
use quill_oracle::Fetch;
use serde_json::{json, Value};

const NOW: u64 = 490_010;

/// Canned view results keyed by `method` + compact args.
#[derive(Default)]
struct FakeChain {
    views: HashMap<String, Value>,
    down: bool,
    /// Contract whose every call fails, as an undeployed or broken contract would.
    broken: Option<&'static str>,
}

impl FakeChain {
    fn with(mut self, method: &str, args: Value, result: Value) -> Self {
        self.views.insert(format!("{method}{args}"), result);
        self
    }
}

impl Chain for FakeChain {
    fn view(&self, contract: &str, method: &str, args: Value) -> Result<Value, String> {
        if self.down {
            return Err("rpc unreachable".into());
        }
        if self.broken == Some(contract) {
            return Err("contract error: CodeDoesNotExist".into());
        }
        Ok(self
            .views
            .get(&format!("{method}{args}"))
            .cloned()
            .unwrap_or(Value::Null))
    }
}

fn contracts() -> Contracts {
    Contracts {
        claim_market: "claims.quill.testnet".into(),
        fixing_oracle: "fixing.quill.testnet".into(),
    }
}

fn raw_market(id: u32, expiry_slot: u64, settled_at: Option<&str>, void: bool) -> Value {
    json!({ "id": id, "index": "H100", "floor": "1500000", "cap": "5000000", "expiry_slot": expiry_slot,
            "long_supply": "100000000", "short_supply": "100000000", "locked": "350000000",
            "settled_at": settled_at, "void": void })
}

fn fixing(value: &str) -> Value {
    json!({ "index": "H100", "slot": NOW - 1, "value": value, "sources": 4, "inputs": "sha256:abc", "accepted": true })
}

fn open_market_chain() -> FakeChain {
    FakeChain::default()
        .with("market_count", json!({}), json!(1))
        .with(
            "get_market",
            json!({ "market_id": 0 }),
            raw_market(0, NOW + 20, None, false),
        )
        .with("latest", json!({ "index": "H100" }), fixing("3060000"))
}

#[test]
fn an_open_market_is_priced_at_the_latest_fixing() {
    let m = service::market(&open_market_chain(), &contracts(), 0, NOW).unwrap();
    assert_eq!(m.status, "open");
    assert_eq!(
        (m.floor.as_str(), m.cap.as_str(), m.width.as_str()),
        ("1.500000", "5.000000", "3.500000")
    );
    assert_eq!(m.index_value.as_deref(), Some("3.060000"));
    assert_eq!(m.long_value.as_deref(), Some("1.560000"));
    assert_eq!(m.short_value.as_deref(), Some("1.940000"));
    assert_eq!(m.collateral_locked, "350.000000");
    assert_eq!(m.expires_at, (NOW + 20) * 3600);
}

#[test]
fn a_market_with_no_fixing_yet_has_no_price() {
    let chain = FakeChain::default().with(
        "get_market",
        json!({ "market_id": 0 }),
        raw_market(0, NOW + 20, None, false),
    );
    let m = service::market(&chain, &contracts(), 0, NOW).unwrap();
    assert_eq!(
        (m.index_value, m.long_value, m.short_value),
        (None, None, None)
    );
}

#[test]
fn status_follows_expiry_and_settlement() {
    let status = |raw: Value, now: u64| {
        let chain = FakeChain::default().with("get_market", json!({ "market_id": 0 }), raw);
        service::market(&chain, &contracts(), 0, now).unwrap()
    };
    assert_eq!(
        status(raw_market(0, NOW + 1, None, false), NOW).status,
        "open"
    );
    assert_eq!(
        status(raw_market(0, NOW, None, false), NOW).status,
        "expired"
    );

    let settled = status(raw_market(0, NOW - 5, Some("9000000"), false), NOW);
    // Settled above the cap: LONG gets the whole band, SHORT nothing.
    assert_eq!(
        (
            settled.status,
            settled.long_value.as_deref(),
            settled.short_value.as_deref()
        ),
        ("settled", Some("3.500000"), Some("0.000000"))
    );

    let void = status(raw_market(0, NOW - 5, None, true), NOW);
    assert_eq!(
        (void.status, void.index_value, void.long_value.as_deref()),
        ("void", None, Some("1.750000"))
    );
}

#[test]
fn missing_things_are_404_not_500() {
    let chain = FakeChain::default();
    assert_eq!(
        service::market(&chain, &contracts(), 7, NOW),
        Err(ApiError::NotFound("no market 7".into()))
    );
    assert!(matches!(
        service::latest_fixing(&chain, &contracts(), "H100"),
        Err(ApiError::NotFound(_))
    ));
}

#[test]
fn bad_input_is_rejected_before_it_reaches_the_chain() {
    let chain = FakeChain {
        down: true,
        ..Default::default()
    };
    assert!(matches!(
        service::latest_fixing(&chain, &contracts(), ""),
        Err(ApiError::BadRequest(_))
    ));
    for junk in [
        "",
        "a",
        "Alice.Testnet",
        "bob testnet",
        "x';--",
        &"a".repeat(65),
    ] {
        assert!(
            matches!(
                service::portfolio(&chain, &contracts(), junk, NOW),
                Err(ApiError::BadRequest(_))
            ),
            "{junk:?}"
        );
    }
}

#[test]
fn a_dead_rpc_or_a_changed_contract_is_a_502() {
    let down = FakeChain {
        down: true,
        ..Default::default()
    };
    assert!(matches!(
        service::markets(&down, &contracts(), NOW),
        Err(ApiError::Upstream(_))
    ));

    let changed = FakeChain::default().with(
        "get_market",
        json!({ "market_id": 0 }),
        json!({ "id": 0, "surprise": true }),
    );
    assert!(matches!(
        service::market(&changed, &contracts(), 0, NOW),
        Err(ApiError::Upstream(_))
    ));

    let bad_number = FakeChain::default().with("get_market", json!({ "market_id": 0 }), {
        let mut m = raw_market(0, NOW + 1, None, false);
        m["locked"] = json!("lots");
        m
    });
    assert!(matches!(
        service::market(&bad_number, &contracts(), 0, NOW),
        Err(ApiError::Upstream(_))
    ));
}

#[test]
fn markets_are_listed_newest_first() {
    let chain = open_market_chain()
        .with("market_count", json!({}), json!(2))
        .with(
            "get_market",
            json!({ "market_id": 1 }),
            raw_market(1, NOW + 40, None, false),
        );
    let ids: Vec<u32> = service::markets(&chain, &contracts(), NOW)
        .unwrap()
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, [1, 0]);
}

#[test]
fn a_portfolio_values_each_side_and_skips_empty_markets() {
    let balance = |market: u32, side: &str| json!({ "market_id": market, "side": side, "account_id": "alice.testnet" });
    let chain = open_market_chain()
        .with("market_count", json!({}), json!(2))
        .with(
            "get_market",
            json!({ "market_id": 1 }),
            raw_market(1, NOW - 5, Some("3060000"), false),
        )
        // Market 0 (open): 100 LONG only. Market 1 (settled): nothing.
        .with("balance_of", balance(0, "Long"), json!("100000000"))
        .with("balance_of", balance(0, "Short"), json!("0"))
        .with("balance_of", balance(1, "Long"), json!("0"))
        .with("balance_of", balance(1, "Short"), json!("0"));
    let positions = service::portfolio(&chain, &contracts(), "alice.testnet", NOW).unwrap();
    assert_eq!(positions.len(), 1);
    let p = &positions[0];
    assert_eq!(
        (p.market, p.long.as_str(), p.short.as_str()),
        (0, "100.000000", "0.000000")
    );
    assert_eq!(p.worth_now.as_deref(), Some("156.000000"));
    assert_eq!(p.max_payout, "350.000000");
    assert!(!p.claimable);
}

#[test]
fn a_settled_position_is_claimable() {
    let balance = |side: &str| json!({ "market_id": 0, "side": side, "account_id": "bob.testnet" });
    let chain = FakeChain::default()
        .with("market_count", json!({}), json!(1))
        .with(
            "get_market",
            json!({ "market_id": 0 }),
            raw_market(0, NOW - 5, Some("3060000"), false),
        )
        .with("balance_of", balance("Long"), json!("0"))
        .with("balance_of", balance("Short"), json!("100000000"));
    let p = &service::portfolio(&chain, &contracts(), "bob.testnet", NOW).unwrap()[0];
    assert_eq!(
        (p.status, p.worth_now.as_deref(), p.claimable),
        ("settled", Some("194.000000"), true)
    );
}

#[test]
fn amounts_are_exact_decimal_strings() {
    assert_eq!(service::decimal(0), "0.000000");
    assert_eq!(service::decimal(1), "0.000001");
    assert_eq!(service::decimal(3_060_000), "3.060000");
    assert_eq!(
        service::decimal(u128::MAX),
        "340282366920938463463374607431768.211455"
    );
}

#[test]
fn rpc_responses_are_decoded_or_explained() {
    let ok = json!({ "result": { "result": br#"{"a":1}"#.to_vec() } });
    assert_eq!(decode_view(&ok), Ok(json!({ "a": 1 })));
    let null = json!({ "result": { "result": b"null".to_vec() } });
    assert_eq!(decode_view(&null), Ok(Value::Null));
    assert!(decode_view(&json!({ "error": { "message": "nope" } }))
        .unwrap_err()
        .starts_with("rpc error"));
    assert!(
        decode_view(&json!({ "result": { "error": "wasm execution failed" } }))
            .unwrap_err()
            .starts_with("contract error")
    );
    assert!(decode_view(&json!({ "result": {} })).is_err());
    assert!(decode_view(&json!({ "result": { "result": [999] } })).is_err());
    assert!(decode_view(&json!({ "result": { "result": b"not json".to_vec() } })).is_err());
}

// ----- over real HTTP -----

/// Serves a fixed NEAR AI price list and fails every other source.
struct OnlyNearAi;
impl Fetch for OnlyNearAi {
    fn get(&self, url: &str) -> Result<String, String> {
        if url.contains("near.ai") {
            Ok(r#"{"models":[{"modelId":"deepseek-ai/DeepSeek-V4.1-Flash-1017","outputCostPerToken":{"amount":1200,"scale":9,"currency":"USD"}}]}"#.into())
        } else {
            Err("offline".into())
        }
    }
    fn post_json(&self, _url: &str, _body: &str) -> Result<String, String> {
        Err("offline".into())
    }
}

async fn serve(state: Arc<AppState>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    address
}

async fn get(url: String) -> (u16, Value) {
    tokio::task::spawn_blocking(move || match ureq::get(&url).call() {
        Ok(response) => (response.status(), response.into_json().unwrap()),
        Err(ureq::Error::Status(code, response)) => (code, response.into_json().unwrap()),
        Err(other) => panic!("{other}"),
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn without_contracts_chain_routes_are_503_and_the_live_index_still_works() {
    let base = serve(AppState::new(None, Arc::new(OnlyNearAi))).await;

    let (status, body) = get(format!("{base}/health")).await;
    assert_eq!(
        (status, &body["status"], &body["chain"]),
        (200, &json!("ok"), &Value::Null)
    );

    let (status, body) = get(format!("{base}/v1/markets")).await;
    assert_eq!(status, 503);
    assert!(body["error"].as_str().unwrap().contains("not deployed"));

    let (status, body) = get(format!("{base}/v1/index/live")).await;
    assert_eq!(status, 200);
    assert_eq!(
        body["values"],
        json!([{ "index": "INF_OUT", "value": "1.200000", "sources": 1 }])
    );
    assert!(
        body["problems"].as_array().unwrap().len() >= 2,
        "GPU indexes report why they are missing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn with_contracts_the_routes_answer_from_the_chain() {
    let chain: Arc<dyn Chain> = Arc::new(open_market_chain());
    let base = serve(AppState::new(
        Some((chain, contracts())),
        Arc::new(OnlyNearAi),
    ))
    .await;

    let (status, body) = get(format!("{base}/v1/markets/0")).await;
    assert_eq!(
        (status, &body["index_value"], &body["long_value"]),
        (200, &json!("3.060000"), &json!("1.560000"))
    );

    let (status, body) = get(format!("{base}/v1/markets")).await;
    assert_eq!((status, body.as_array().unwrap().len()), (200, 1));

    let (status, body) = get(format!("{base}/v1/fixings/H100/latest")).await;
    assert_eq!(
        (status, &body["value"], &body["sources"]),
        (200, &json!("3.060000"), &json!(4))
    );

    assert_eq!(get(format!("{base}/v1/markets/9")).await.0, 404);
    assert_eq!(get(format!("{base}/v1/fixings/DOGE/latest")).await.0, 404);
    assert_eq!(
        get(format!("{base}/v1/portfolio/Not%20An%20Account"))
            .await
            .0,
        400
    );
    assert_eq!(get(format!("{base}/v1/markets/abc")).await.0, 400);
}

#[test]
fn a_missing_oracle_hides_the_price_not_the_market() {
    let mut chain = open_market_chain();
    chain.broken = Some("fixing.quill.testnet");
    let m = service::market(&chain, &contracts(), 0, NOW).unwrap();
    assert_eq!(
        (m.status, m.collateral_locked.as_str()),
        ("open", "350.000000")
    );
    assert_eq!((m.index_value, m.long_value), (None, None));
    // Asking for the fixing directly still reports the problem.
    assert!(matches!(
        service::latest_fixing(&chain, &contracts(), "H100"),
        Err(ApiError::Upstream(_))
    ));
}
