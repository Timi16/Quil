//! Reading contract view methods.
//!
//! The API never signs anything and holds no keys. [`Chain`] is the one seam
//! to NEAR, so the rest of the service is tested without a network.

use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;

/// Calls a contract view method and returns its JSON result.
pub trait Chain: Send + Sync {
    fn view(&self, contract: &str, method: &str, args: Value) -> Result<Value, String>;
}

/// A NEAR JSON-RPC endpoint.
pub struct Rpc {
    url: String,
}

impl Rpc {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl Chain for Rpc {
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
        let response: Value = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build()
            .post(&self.url)
            .send_json(request)
            .map_err(|e| format!("rpc unreachable: {e}"))?
            .into_json()
            .map_err(|e| format!("rpc sent invalid JSON: {e}"))?;
        decode_view(&response)
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
