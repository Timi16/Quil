//! The new routes over real HTTP: config, history, collateral, the paid feed
//! and the web app's static files.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine;
use quill_api::chain::{decode_tx, Chain};
use quill_api::feed::Ledger;
use quill_api::intents::{OneClick, OneClickError};
use quill_api::service::Contracts;
use quill_api::{app, AppState, Config};
use quill_oracle::Fetch;
use serde_json::{json, Value};

const PAYER: &str = "agent.testnet";
const GOOD_TX: &str = "9xQ7m3cY2zYbX1pW4uR8tS6vN5kL3jH2gF1dD9aB7cE";
const CHEAP_TX: &str = "8xQ7m3cY2zYbX1pW4uR8tS6vN5kL3jH2gF1dD9aB7cE";
const UNKNOWN_TX: &str = "7xQ7m3cY2zYbX1pW4uR8tS6vN5kL3jH2gF1dD9aB7cE";

struct NoSources;
impl Fetch for NoSources {
    fn get(&self, _url: &str) -> Result<String, String> {
        Err("offline".into())
    }
    fn post_json(&self, _url: &str, _body: &str) -> Result<String, String> {
        Err("offline".into())
    }
}

#[derive(Default)]
struct FakeChain {
    views: HashMap<String, Value>,
    txs: HashMap<(String, String), Value>,
    calls: Mutex<Vec<String>>,
}

impl Chain for FakeChain {
    fn view(&self, _contract: &str, method: &str, args: Value) -> Result<Value, String> {
        self.calls.lock().unwrap().push(format!("{method}{args}"));
        Ok(self.views.get(method).cloned().unwrap_or(Value::Null))
    }
    fn transaction(&self, hash: &str, sender: &str) -> Result<Option<Value>, String> {
        Ok(self
            .txs
            .get(&(hash.to_string(), sender.to_string()))
            .cloned())
    }
}

/// Nothing from 1Click is needed by these tests except a broken quote body.
struct NoOneClick;
impl OneClick for NoOneClick {
    fn tokens(&self) -> Result<Value, OneClickError> {
        Ok(
            json!([{ "assetId": "nep141:wrap.near", "symbol": "wNEAR", "blockchain": "near", "decimals": 24 }]),
        )
    }
    fn quote(&self, _request: &Value) -> Result<Value, OneClickError> {
        Err(OneClickError::Unreachable("not in this test".into()))
    }
    fn status(&self, _deposit_address: &str) -> Result<Value, OneClickError> {
        Err(OneClickError::Status(404, "not found".into()))
    }
}

fn payment(amount: &str) -> Value {
    let args = base64::engine::general_purpose::STANDARD
        .encode(json!({ "receiver_id": "quill.testnet", "amount": amount }).to_string());
    json!({
        "status": { "SuccessValue": "" },
        "transaction": { "signer_id": PAYER, "receiver_id": "usdc.quill.testnet",
            "actions": [{ "FunctionCall": { "method_name": "ft_transfer", "args": args, "deposit": "1" } }] },
        "receipts_outcome": []
    })
}

fn chain() -> Arc<FakeChain> {
    let mut c = FakeChain::default();
    let fixing = json!({ "index": "H100", "slot": 498471, "value": "2643750", "sources": 4, "inputs": "sha256:abc", "accepted": true });
    c.views.insert("latest".into(), fixing.clone());
    c.views.insert("fixings_between".into(), json!([fixing]));
    c.views.insert(
        "code_ref".into(),
        json!({ "wasm_url": "https://example.org/quill-oracle.wasm", "wasm_hash": "c163a26f", "build_target": "wasm32-wasip2" }),
    );
    c.txs
        .insert((GOOD_TX.into(), PAYER.into()), payment("10000"));
    c.txs.insert((CHEAP_TX.into(), PAYER.into()), payment("1"));
    Arc::new(c)
}

fn config(feed: bool) -> Config {
    Config {
        usdc: Some("usdc.quill.testnet".into()),
        feed_payee: feed.then(|| "quill.testnet".to_string()),
        ..Config::default()
    }
}

fn state(chain: Arc<FakeChain>, feed: bool) -> Arc<AppState> {
    AppState::with(
        Some((
            chain as Arc<dyn Chain>,
            Contracts {
                claim_market: "market.quill.testnet".into(),
                fixing_oracle: "oracle.quill.testnet".into(),
            },
        )),
        Arc::new(NoSources),
        config(feed),
        Arc::new(NoOneClick),
        Ledger::in_memory(),
    )
}

async fn serve(state: Arc<AppState>, app_dir: Option<PathBuf>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app(state, app_dir)).await.unwrap() });
    address
}

/// Status, content type and body text.
async fn call(
    method: &'static str,
    url: String,
    headers: Vec<(&'static str, &'static str)>,
    body: Option<&'static str>,
) -> (u16, String, String) {
    tokio::task::spawn_blocking(move || {
        let mut request = ureq::request(method, &url);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        let result = match body {
            Some(b) => request
                .set("content-type", "application/json")
                .send_string(b),
            None => request.call(),
        };
        let response = match result {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => panic!("{e}"),
        };
        let status = response.status();
        let kind = response.content_type().to_string();
        (status, kind, response.into_string().unwrap())
    })
    .await
    .unwrap()
}

async fn get(url: String) -> (u16, Value) {
    let (status, _, body) = call("GET", url, vec![], None).await;
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn config_tells_the_app_where_everything_is() {
    let base = serve(state(chain(), true), None).await;
    let (status, body) = get(format!("{base}/v1/config")).await;
    assert_eq!(status, 200);
    assert_eq!(body["network"], "testnet");
    assert_eq!(body["claim_market"], "market.quill.testnet");
    assert_eq!(body["fixing_oracle"], "oracle.quill.testnet");
    assert_eq!(body["usdc"], "usdc.quill.testnet");
    assert_eq!(body["feed_payee"], "quill.testnet");
    assert_eq!(body["feed_price"], "0.010000");
}

#[tokio::test(flavor = "multi_thread")]
async fn history_defaults_to_48_hours_and_checks_its_bounds() {
    let c = chain();
    let base = serve(state(c.clone(), true), None).await;

    let (status, body) = get(format!("{base}/v1/fixings/H100/history")).await;
    assert_eq!(status, 200);
    assert_eq!(body[0]["value"], "2.643750");
    let asked = c
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| call.starts_with("fixings_between") && call.contains("\"hours\":48"));
    assert!(asked, "the oracle should be asked for 48 hours");

    for bad in ["0", "169", "lots"] {
        let (status, body) = get(format!("{base}/v1/fixings/H100/history?hours={bad}")).await;
        assert_eq!(status, 400, "hours={bad}");
        assert!(body["error"].is_string());
    }
    let (status, _) = get(format!("{base}/v1/fixings/H1%2000/history")).await;
    assert_eq!(status, 400, "an index id with a space is refused");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_payment_the_feed_answers_402_with_the_terms() {
    let base = serve(state(chain(), true), None).await;
    let (status, body) = get(format!("{base}/v1/feed/H100")).await;
    assert_eq!(status, 402);
    assert_eq!(body["price"], "0.010000");
    assert_eq!(body["price_micro"], "10000");
    assert_eq!(body["asset"], "usdc.quill.testnet");
    assert_eq!(body["pay_to"], "quill.testnet");
    assert!(body["how"].as_str().unwrap().contains("X-Payment"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_valid_payment_buys_exactly_one_read() {
    let base = serve(state(chain(), true), None).await;
    let paid = vec![("X-Payment", GOOD_TX), ("X-Payer", PAYER)];

    let (status, _, body) = call("GET", format!("{base}/v1/feed/H100"), paid.clone(), None).await;
    assert_eq!(status, 200, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["fixing"]["value"], "2.643750");
    assert_eq!(body["code_hash"], "c163a26f");
    assert_eq!(body["paid"]["amount"], "0.010000");

    let (status, _, again) = call("GET", format!("{base}/v1/feed/H100"), paid, None).await;
    assert_eq!(status, 402);
    assert!(again.contains("already used"), "{again}");
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_payments_get_402_with_the_reason() {
    let base = serve(state(chain(), true), None).await;
    let cases: [(&str, &str, &str); 4] = [
        (CHEAP_TX, PAYER, "less than the price"),
        (UNKNOWN_TX, PAYER, "payment not found"),
        (GOOD_TX, "thief.testnet", "payment not found"), // the RPC finds it only with the real sender
        ("not-a-hash", PAYER, "not a transaction hash"),
    ];
    for (tx, payer, reason) in cases {
        let (status, _, body) = call(
            "GET",
            format!("{base}/v1/feed/H100"),
            vec![("X-Payment", tx), ("X-Payer", payer)],
            None,
        )
        .await;
        assert_eq!(status, 402, "{tx} from {payer}");
        assert!(body.contains(reason), "{body}");
    }
    // A refused payment is not spent: the good one still works afterwards.
    let (status, _, _) = call(
        "GET",
        format!("{base}/v1/feed/H100"),
        vec![("X-Payment", GOOD_TX), ("X-Payer", PAYER)],
        None,
    )
    .await;
    assert_eq!(status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_feed_is_503_when_no_payee_is_configured() {
    let base = serve(state(chain(), false), None).await;
    let (status, body) = get(format!("{base}/v1/feed/H100")).await;
    assert_eq!(status, 503);
    assert!(body["error"].as_str().unwrap().contains("FEED_PAYEE"));
}

#[tokio::test(flavor = "multi_thread")]
async fn collateral_routes_answer_in_json_even_when_the_request_is_bad() {
    let base = serve(state(chain(), true), None).await;
    let (status, body) = get(format!("{base}/v1/collateral/tokens")).await;
    assert_eq!(status, 200);
    assert_eq!(body[0]["symbol"], "wNEAR");

    let (status, kind, body) = call(
        "POST",
        format!("{base}/v1/collateral/quote"),
        vec![],
        Some("{ not json"),
    )
    .await;
    assert_eq!(status, 400);
    assert!(
        kind.contains("json") && body.contains("bad quote request"),
        "{kind} {body}"
    );

    let (status, body) = get(format!("{base}/v1/collateral/status/0xDepositAddress0001")).await;
    assert_eq!(status, 404);
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_web_app_is_served_beside_the_api() {
    let dir = std::env::temp_dir().join(format!("quill-app-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><title>Quill</title>",
    )
    .unwrap();
    std::fs::write(dir.join("app.css"), "body{}").unwrap();
    let base = serve(state(chain(), true), Some(dir.clone())).await;

    let (status, kind, body) = call("GET", format!("{base}/"), vec![], None).await;
    assert_eq!((status, kind.as_str()), (200, "text/html"));
    assert!(body.contains("Quill"));

    let (status, kind, _) = call("GET", format!("{base}/app.css"), vec![], None).await;
    assert_eq!((status, kind.as_str()), (200, "text/css"));

    // The API still answers, and an unknown API path is a JSON 404, not the app.
    let (status, body) = get(format!("{base}/v1/config")).await;
    assert_eq!((status, body["network"].as_str()), (200, Some("testnet")));
    let (status, kind, _) = call("GET", format!("{base}/v1/nope"), vec![], None).await;
    assert_eq!(status, 404);
    assert!(kind.contains("json"));
    let (status, _) = get(format!("{base}/health")).await;
    assert_eq!(status, 200);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_transaction_is_none_and_other_rpc_errors_are_errors() {
    let unknown =
        json!({ "error": { "cause": { "name": "UNKNOWN_TRANSACTION" }, "name": "HANDLER_ERROR" } });
    assert_eq!(decode_tx(&unknown), Ok(None));
    let timeout = json!({ "error": { "cause": { "name": "TIMEOUT_ERROR" } } });
    assert!(decode_tx(&timeout).is_err());
    let found = json!({ "result": { "status": { "SuccessValue": "" } } });
    assert_eq!(
        decode_tx(&found).unwrap().unwrap()["status"]["SuccessValue"],
        ""
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn another_site_may_call_the_feed_with_payment_headers() {
    let base = serve(state(chain(), true), None).await;
    let (status, headers) = tokio::task::spawn_blocking(move || {
        let r = ureq::request("OPTIONS", &format!("{base}/v1/feed/H100"))
            .set("origin", "https://agent.example")
            .set("access-control-request-method", "GET")
            .set("access-control-request-headers", "x-payment,x-payer")
            .call()
            .unwrap();
        let allow = |name: &str| r.header(name).unwrap_or_default().to_ascii_lowercase();
        (
            r.status(),
            [
                allow("access-control-allow-origin"),
                allow("access-control-allow-headers"),
            ],
        )
    })
    .await
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(headers[0], "*");
    assert!(
        headers[1].contains("x-payment") || headers[1] == "*",
        "{}",
        headers[1]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_chain_the_feed_does_not_take_payments() {
    let unconfigured = AppState::with(
        None,
        Arc::new(NoSources),
        config(true),
        Arc::new(NoOneClick),
        Ledger::in_memory(),
    );
    let base = serve(unconfigured, None).await;
    let (status, _, _) = call(
        "GET",
        format!("{base}/v1/feed/H100"),
        vec![("X-Payment", GOOD_TX), ("X-Payer", PAYER)],
        None,
    )
    .await;
    assert_eq!(status, 503);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_payment_for_an_index_with_no_fixing_is_not_spent() {
    let mut c = FakeChain::default();
    c.txs
        .insert((GOOD_TX.into(), PAYER.into()), payment("10000"));
    let base = serve(state(Arc::new(c), true), None).await;
    let paid = vec![("X-Payment", GOOD_TX), ("X-Payer", PAYER)];
    let (status, _, _) = call("GET", format!("{base}/v1/feed/DOGE"), paid, None).await;
    assert_eq!(status, 404, "nothing to sell, so nothing is charged");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_payment_sent_twice_at_once_buys_one_read() {
    let base = serve(state(chain(), true), None).await;
    let paid = || vec![("X-Payment", GOOD_TX), ("X-Payer", PAYER)];
    let (a, b) = tokio::join!(
        call("GET", format!("{base}/v1/feed/H100"), paid(), None),
        call("GET", format!("{base}/v1/feed/H100"), paid(), None),
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 402]);
}
