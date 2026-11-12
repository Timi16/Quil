//! Pay-per-read price feed.
//!
//! HTTP 402 in the spirit of x402, settled on NEAR without a facilitator: the
//! reader sends `price` test USDC to `pay_to` with a plain `ft_transfer`, then
//! asks again with `X-Payment: <tx hash>` and `X-Payer: <their account>`. The
//! API looks the transaction up over RPC and checks every part of it. Each
//! transaction buys one read.
//!
//! This is not the x402 facilitator protocol (`@fastnear/x402`); it is the
//! same idea, verified directly against the chain.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use base64::Engine;
use serde_json::Value;

use crate::service::ApiError;

/// Which payment the feed accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terms {
    /// The token contract (test USDC on testnet).
    pub asset: String,
    /// Who must be paid.
    pub pay_to: String,
    /// Smallest accepted amount, in micro-USDC.
    pub price: u128,
}

/// A checked payment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paid {
    pub payer: String,
    pub amount: u128,
}

/// Check that `tx` (the RPC `tx` result) is a successful `ft_transfer` by
/// `payer` of at least the price to the payee, on the right token.
pub fn verify(tx: &Value, payer: &str, terms: &Terms) -> Result<Paid, ApiError> {
    let refuse = |why: &str| Err(ApiError::BadRequest(format!("payment not accepted: {why}")));

    let status = &tx["status"];
    if status.get("SuccessValue").is_none() {
        return refuse("the transaction did not succeed");
    }
    if tx["receipts_outcome"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|r| r["outcome"]["status"].get("Failure").is_some())
    {
        return refuse("part of the transaction failed");
    }

    let transaction = &tx["transaction"];
    if transaction["signer_id"] != payer {
        return refuse("it was not signed by the payer");
    }
    if transaction["receiver_id"] != terms.asset.as_str() {
        return refuse("it is not a transfer of the feed's token");
    }
    let actions = transaction["actions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let [action] = actions else {
        return refuse("it must contain exactly one action");
    };
    let call = &action["FunctionCall"];
    if call["method_name"] != "ft_transfer" {
        return refuse("it is not an ft_transfer");
    }
    let args = call["args"]
        .as_str()
        .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let Some(args) = args else {
        return refuse("its arguments cannot be read");
    };
    if args["receiver_id"] != terms.pay_to.as_str() {
        return refuse("it pays someone else");
    }
    let amount: u128 = match args["amount"].as_str().and_then(|a| a.parse().ok()) {
        Some(amount) => amount,
        None => return refuse("its amount cannot be read"),
    };
    if amount < terms.price {
        return refuse("it pays less than the price");
    }
    Ok(Paid {
        payer: payer.to_string(),
        amount,
    })
}

/// Remembers which transactions have been spent, in memory and in a file, so a
/// receipt buys exactly one read even across restarts.
pub struct Ledger {
    used: Mutex<HashSet<String>>,
    file: Option<PathBuf>,
}

impl Ledger {
    pub fn in_memory() -> Self {
        Self {
            used: Mutex::new(HashSet::new()),
            file: None,
        }
    }

    /// Load spent hashes from `file` (one per line), creating it if needed.
    pub fn open(file: PathBuf) -> Self {
        let used = std::fs::read_to_string(&file)
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            used: Mutex::new(used),
            file: Some(file),
        }
    }

    /// Mark `tx_hash` spent. Returns false if it already was.
    pub fn spend(&self, tx_hash: &str) -> bool {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        if !used.insert(tx_hash.to_string()) {
            return false;
        }
        if let Some(path) = &self.file {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{tx_hash}");
            }
        }
        true
    }
}

/// A NEAR transaction hash: base58, 43 or 44 characters.
pub fn valid_tx_hash(hash: &str) -> bool {
    (43..=44).contains(&hash.len())
        && hash
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() && !matches!(b, b'0' | b'O' | b'I' | b'l'))
}
