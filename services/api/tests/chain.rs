//! The seam to NEAR: endpoint fallback over real HTTP, and the view cache.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use quill_api::chain::{Cached, Chain, Rpc};
use serde_json::{json, Value};

/// A view answer as the RPC encodes it: the JSON result as a byte array.
fn view_answer(result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": "quill-api", "result": { "result": result.to_string().into_bytes() } })
}

/// A local node that always answers with `status` and `body`, counting requests.
async fn node(status: StatusCode, body: Value) -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = Router::new().route(
        "/",
        post(move || {
            let (counter, body) = (counter.clone(), body.clone());
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                (status, Json(body))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, hits)
}

async fn view(rpc: Rpc) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || rpc.view("market.testnet", "market_count", json!({})))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limited_node_is_skipped_for_the_next() {
    let (limited, limited_hits) = node(StatusCode::TOO_MANY_REQUESTS, json!({})).await;
    let (healthy, healthy_hits) = node(StatusCode::OK, view_answer(json!(3))).await;
    assert_eq!(
        view(Rpc::with_fallbacks(vec![limited, healthy])).await,
        Ok(json!(3))
    );
    assert_eq!(limited_hits.load(Ordering::SeqCst), 1);
    assert_eq!(healthy_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_down_node_is_skipped_too() {
    let (broken, _) = node(StatusCode::BAD_GATEWAY, json!({})).await;
    let (healthy, _) = node(StatusCode::OK, view_answer(json!(1))).await;
    let nobody = "http://127.0.0.1:9/".to_string(); // the discard port: connection refused
    assert_eq!(
        view(Rpc::with_fallbacks(vec![nobody, broken, healthy])).await,
        Ok(json!(1))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_healthy_node_answers_and_the_rest_are_not_asked() {
    let (first, _) = node(StatusCode::OK, view_answer(json!(1))).await;
    let (second, second_hits) = node(StatusCode::OK, view_answer(json!(2))).await;
    assert_eq!(
        view(Rpc::with_fallbacks(vec![first, second])).await,
        Ok(json!(1))
    );
    assert_eq!(second_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_contract_error_is_an_answer_not_a_reason_to_ask_elsewhere() {
    let error = json!({ "result": { "error": "wasm execution failed" } });
    let (first, _) = node(StatusCode::OK, error).await;
    let (second, second_hits) = node(StatusCode::OK, view_answer(json!(2))).await;
    let result = view(Rpc::with_fallbacks(vec![first, second])).await;
    assert!(result.unwrap_err().starts_with("contract error"));
    assert_eq!(second_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn when_every_node_fails_the_error_names_each() {
    let (a, _) = node(StatusCode::TOO_MANY_REQUESTS, json!({})).await;
    let (b, _) = node(StatusCode::SERVICE_UNAVAILABLE, json!({})).await;
    let error = view(Rpc::with_fallbacks(vec![a.clone(), b.clone()]))
        .await
        .unwrap_err();
    assert!(error.contains(&a) && error.contains("429"), "{error}");
    assert!(error.contains(&b) && error.contains("503"), "{error}");
}

/// Counts calls and answers from a script.
struct Counting {
    calls: AtomicUsize,
    fail: bool,
}

impl Chain for Counting {
    fn view(&self, _contract: &str, method: &str, args: Value) -> Result<Value, String> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err("rpc unreachable".into());
        }
        Ok(json!({ "method": method, "args": args, "call": n }))
    }
    fn transaction(&self, _hash: &str, _sender: &str) -> Result<Option<Value>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
}

fn counting(fail: bool) -> Counting {
    Counting {
        calls: AtomicUsize::new(0),
        fail,
    }
}

#[test]
fn the_same_view_within_the_ttl_is_asked_once() {
    let cached = Cached::new(counting(false), Duration::from_secs(60));
    let a = cached
        .view("m", "get_market", json!({ "market_id": 0 }))
        .unwrap();
    let b = cached
        .view("m", "get_market", json!({ "market_id": 0 }))
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(cached_calls(&cached), 1);

    // Different args, method or contract are different questions.
    cached
        .view("m", "get_market", json!({ "market_id": 1 }))
        .unwrap();
    cached.view("m", "market_count", json!({})).unwrap();
    cached.view("other", "market_count", json!({})).unwrap();
    assert_eq!(cached_calls(&cached), 4);
}

#[test]
fn an_expired_answer_is_asked_again() {
    let cached = Cached::new(counting(false), Duration::ZERO);
    cached.view("m", "market_count", json!({})).unwrap();
    cached.view("m", "market_count", json!({})).unwrap();
    assert_eq!(cached_calls(&cached), 2);
}

#[test]
fn errors_are_never_cached() {
    let cached = Cached::new(counting(true), Duration::from_secs(60));
    assert!(cached.view("m", "market_count", json!({})).is_err());
    assert!(cached.view("m", "market_count", json!({})).is_err());
    assert_eq!(cached_calls(&cached), 2);
}

#[test]
fn transactions_are_always_looked_up_fresh() {
    let cached = Cached::new(counting(false), Duration::from_secs(60));
    cached.transaction("h", "a.testnet").unwrap();
    cached.transaction("h", "a.testnet").unwrap();
    assert_eq!(
        cached_calls(&cached),
        2,
        "a payment that lands later must be seen"
    );
}

fn cached_calls(cached: &Cached<Counting>) -> usize {
    cached.inner().calls.load(Ordering::SeqCst)
}
