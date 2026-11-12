//! Reading contract view methods.
//!
//! The API never signs anything and holds no keys. [`Chain`] is the one seam
//! to NEAR, so the rest of the service is tested without a network.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};

/// Calls a contract view method and returns its JSON result.
pub trait Chain: Send + Sync {
    fn view(&self, contract: &str, method: &str, args: Value) -> Result<Value, String>;

    /// A final transaction and its receipts (the RPC `tx` method), or `Ok(None)`
    /// if the node does not know it.
    fn transaction(&self, _hash: &str, _sender: &str) -> Result<Option<Value>, String> {
        Err("this chain cannot look up transactions".into())
    }
}

/// NEAR JSON-RPC, with fallback endpoints: a node that is down or rate
/// limiting (HTTP 429, 5xx) is skipped for the next one. A JSON-RPC error
/// from a working node is an answer, not a reason to ask another.
pub struct Rpc {
    urls: Vec<String>,
    /// One agent for every call, so connections are reused.
    agent: ureq::Agent,
}

impl Rpc {
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_fallbacks(vec![url.into()])
    }

    /// Endpoints in order of preference. Panics on an empty list.
    pub fn with_fallbacks(urls: Vec<String>) -> Self {
        assert!(!urls.is_empty(), "at least one RPC endpoint");
        Self {
            urls,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build(),
        }
    }

    fn post(&self, request: Value) -> Result<Value, String> {
        let mut failures = Vec::new();
        for url in &self.urls {
            match self.agent.post(url).send_json(&request) {
                Ok(response) => {
                    return response
                        .into_json()
                        .map_err(|e| format!("rpc {url} sent invalid JSON: {e}"))
                }
                Err(ureq::Error::Status(code, _)) if code == 429 || code >= 500 => {
                    failures.push(format!("{url}: status code {code}"))
                }
                Err(ureq::Error::Status(code, _)) => {
                    return Err(format!("rpc {url} refused the request: status code {code}"))
                }
                Err(e) => failures.push(format!("{url}: {e}")),
            }
        }
        Err(format!("rpc unreachable: {}", failures.join("; ")))
    }
}

/// Remembers view results for a few seconds, so a page that asks for the same
/// markets many times, or many viewers at once, costs the RPC one call.
/// Errors are never cached, and transactions are always looked up fresh.
pub struct Cached<C> {
    inner: C,
    ttl: Duration,
    entries: Mutex<HashMap<String, (Instant, Value)>>,
}

/// Most view results kept at once; the cache is emptied when it fills.
pub const CACHE_ENTRIES: usize = 4096;

impl<C: Chain> Cached<C> {
    pub fn new(inner: C, ttl: Duration) -> Self {
        Self {
            inner,
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn inner(&self) -> &C {
        &self.inner
    }
}

impl<C: Chain> Chain for Cached<C> {
    fn view(&self, contract: &str, method: &str, args: Value) -> Result<Value, String> {
        let key = format!("{contract}\u{0}{method}\u{0}{args}");
        let now = Instant::now();
        if let Some((at, value)) = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            if now.duration_since(*at) < self.ttl {
                return Ok(value.clone());
            }
        }
        let value = self.inner.view(contract, method, args)?;
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries.len() >= CACHE_ENTRIES {
            entries.clear();
        }
        entries.insert(key, (now, value.clone()));
        Ok(value)
    }

    fn transaction(&self, hash: &str, sender: &str) -> Result<Option<Value>, String> {
        self.inner.transaction(hash, sender)
    }
}

impl Chain for Rpc {
    fn transaction(&self, hash: &str, sender: &str) -> Result<Option<Value>, String> {
        let response = self.post(json!({
            "jsonrpc": "2.0",
            "id": "quill-api",
            "method": "tx",
            "params": { "tx_hash": hash, "sender_account_id": sender, "wait_until": "FINAL" }
        }))?;
        decode_tx(&response)
    }

    fn view(&self, contract: &str, method: &str, args: Value) -> Result<Value, String> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": "quill-api",
            "method": "query",
            "params": {
                "request_type": "call_function",
                "finality": "final",
                "account_id": contract,
                "method_name": method,
                "args_base64": base64::engine::general_purpose::STANDARD.encode(args.to_string()),
            }
        });
        decode_view(&self.post(request)?)
    }
}

/// Pull the method's JSON result out of a `call_function` response.
pub fn decode_view(response: &Value) -> Result<Value, String> {
    if let Some(error) = response.get("error") {
        return Err(format!("rpc error: {error}"));
    }
    let result = &response["result"];
    // A view that panics comes back as a successful RPC call carrying an `error` string.
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return Err(format!("contract error: {error}"));
    }
    let bytes: Vec<u8> = result["result"]
        .as_array()
        .ok_or("rpc response has no result bytes")?
        .iter()
        .map(|b| {
            b.as_u64()
                .and_then(|b| u8::try_from(b).ok())
                .ok_or("result is not a byte array")
        })
        .collect::<Result<_, _>>()?;
    serde_json::from_slice(&bytes).map_err(|e| format!("view result is not JSON: {e}"))
}

/// Pull the transaction out of a `tx` response. An unknown transaction is
/// `Ok(None)`, not an error: the payer may simply have the hash wrong.
pub fn decode_tx(response: &Value) -> Result<Option<Value>, String> {
    if let Some(error) = response.get("error") {
        let name = error["cause"]["name"].as_str().unwrap_or_default();
        if matches!(name, "UNKNOWN_TRANSACTION" | "INVALID_TRANSACTION") {
            return Ok(None);
        }
        return Err(format!("rpc error: {error}"));
    }
    match response.get("result") {
        Some(result) if !result.is_null() => Ok(Some(result.clone())),
        _ => Err("rpc response has no result".into()),
    }
}
