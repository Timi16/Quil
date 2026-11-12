//! Collateral from any chain through NEAR Intents (the 1Click API).
//!
//! The API only relays quotes and statuses. The quote's recipient is the
//! user's own NEAR account and the refund goes to their own wallet on the
//! origin chain, so Quill never holds the funds in between. 1Click runs on
//! mainnet only.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::service::{valid_account, ApiError};

pub const ONECLICK_URL: &str = "https://1click.chaindefuser.com";
/// Circle USDC on NEAR mainnet: what collateral arrives as.
pub const NEAR_USDC: &str =
    "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
/// Largest quote accepted: 1,000,000 USDC.
pub const MAX_AMOUNT_OUT: u128 = 1_000_000 * 1_000_000;
/// Slippage tolerated on the origin side, in basis points.
pub const SLIPPAGE_BPS: u32 = 100;
/// How long a real quote's deposit address stays valid.
pub const QUOTE_TTL_SECS: u64 = 30 * 60;

/// Why 1Click did not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OneClickError {
    /// It answered with an HTTP error status and this message.
    Status(u16, String),
    /// It could not be reached, or sent something that is not JSON.
    Unreachable(String),
}

impl std::fmt::Display for OneClickError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code, message) => write!(f, "1Click answered {code}: {message}"),
            Self::Unreachable(why) => write!(f, "1Click unreachable: {why}"),
        }
    }
}

impl From<OneClickError> for ApiError {
    fn from(e: OneClickError) -> Self {
        ApiError::Upstream(e.to_string())
    }
}

/// The three 1Click calls Quill uses.
pub trait OneClick: Send + Sync {
    fn tokens(&self) -> Result<Value, OneClickError>;
    fn quote(&self, request: &Value) -> Result<Value, OneClickError>;
    fn status(&self, deposit_address: &str) -> Result<Value, OneClickError>;
}

/// The real 1Click API. A JWT is optional; without one 1Click adds a 0.2% fee.
pub struct HttpOneClick {
    jwt: Option<String>,
    /// One agent for every call, so connections are reused.
    agent: ureq::Agent,
}

impl HttpOneClick {
    pub fn new(jwt: Option<String>) -> Self {
        Self {
            jwt,
            // EXACT_OUTPUT quotes can take tens of seconds.
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(60))
                .build(),
        }
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        let request = self.agent.request(method, &format!("{ONECLICK_URL}{path}"));
        match &self.jwt {
            Some(jwt) => request.set("authorization", &format!("Bearer {jwt}")),
            None => request,
        }
    }

    fn read(response: Result<ureq::Response, ureq::Error>) -> Result<Value, OneClickError> {
        match response {
            Ok(r) => r
                .into_json()
                .map_err(|e| OneClickError::Unreachable(format!("invalid JSON: {e}"))),
            Err(ureq::Error::Status(code, r)) => {
                let body: Value = r.into_json().unwrap_or(Value::Null);
                let message = body["message"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| body.to_string());
                Err(OneClickError::Status(code, message))
            }
            Err(e) => Err(OneClickError::Unreachable(e.to_string())),
        }
    }
}

impl OneClick for HttpOneClick {
    fn tokens(&self) -> Result<Value, OneClickError> {
        Self::read(self.request("GET", "/v0/tokens").call())
    }

    fn quote(&self, request: &Value) -> Result<Value, OneClickError> {
        Self::read(self.request("POST", "/v0/quote").send_json(request))
    }

    fn status(&self, deposit_address: &str) -> Result<Value, OneClickError> {
        Self::read(
            self.request("GET", "/v0/status")
                .query("depositAddress", deposit_address)
                .call(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouteToken {
    pub asset_id: String,
    pub symbol: String,
    pub blockchain: String,
    pub decimals: u32,
    /// USD price as 1Click reports it, for display only.
    pub price: Option<f64>,
}

/// Every token 1Click can route from, sorted by symbol then chain.
pub fn tokens(oneclick: &dyn OneClick) -> Result<Vec<RouteToken>, ApiError> {
    let raw = oneclick.tokens()?;
    let list = raw
        .as_array()
        .ok_or_else(|| ApiError::Upstream("1Click token list is not a list".into()))?;
    let mut tokens: Vec<RouteToken> = list
        .iter()
        .filter_map(|t| {
            Some(RouteToken {
                asset_id: t["assetId"].as_str()?.to_string(),
                symbol: t["symbol"].as_str()?.to_string(),
                blockchain: t["blockchain"].as_str()?.to_string(),
                decimals: u32::try_from(t["decimals"].as_u64()?).ok()?,
                price: t["price"].as_f64(),
            })
        })
        .collect();
    tokens.sort_by(|a, b| {
        a.symbol
            .cmp(&b.symbol)
            .then(a.blockchain.cmp(&b.blockchain))
    });
    Ok(tokens)
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuoteRequest {
    /// 1Click asset id of what the user holds, e.g. `nep141:base-0x8335…omft.near` (USDC on Base).
    pub origin_asset: String,
    /// Exact USDC to arrive on NEAR, in micro-USDC.
    pub amount_out: String,
    /// The user's NEAR account that receives the USDC.
    pub recipient: String,
    /// The user's own address on the origin chain, refunded if anything fails.
    pub refund_to: String,
    /// `true`: price only, no deposit address. Defaults to `true`.
    #[serde(default = "yes")]
    pub dry: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Quote {
    pub dry: bool,
    pub deposit_address: Option<String>,
    pub amount_in: String,
    pub amount_in_formatted: String,
    pub amount_out: String,
    pub amount_out_formatted: String,
    pub deadline: String,
    pub time_estimate_secs: Option<u64>,
}

/// Ask 1Click how much of `origin_asset` delivers exactly `amount_out` USDC to `recipient`.
pub fn quote(
    oneclick: &dyn OneClick,
    request: &QuoteRequest,
    now_secs: u64,
) -> Result<Quote, ApiError> {
    let amount: u128 = request.amount_out.parse().map_err(|_| {
        ApiError::BadRequest("amount_out must be a whole number of micro-USDC".into())
    })?;
    if amount == 0 || amount > MAX_AMOUNT_OUT {
        return Err(ApiError::BadRequest(
            "amount_out must be between 0.000001 and 1,000,000 USDC".into(),
        ));
    }
    if !valid_account(&request.recipient) {
        return Err(ApiError::BadRequest(format!(
            "`{}` is not a NEAR account id",
            request.recipient
        )));
    }
    if !plausible_address(&request.refund_to) {
        return Err(ApiError::BadRequest(
            "refund_to must be your own address on the origin chain".into(),
        ));
    }
    if !plausible_address(&request.origin_asset) || !request.origin_asset.contains(':') {
        return Err(ApiError::BadRequest(
            "origin_asset must be a 1Click asset id".into(),
        ));
    }

    let deadline = iso8601(now_secs + QUOTE_TTL_SECS);
    let body = json!({
        "dry": request.dry,
        "swapType": "EXACT_OUTPUT",
        "slippageTolerance": SLIPPAGE_BPS,
        "originAsset": request.origin_asset,
        "depositType": "ORIGIN_CHAIN",
        "destinationAsset": NEAR_USDC,
        "amount": amount.to_string(),
        "refundTo": request.refund_to,
        "refundType": "ORIGIN_CHAIN",
        "recipient": request.recipient,
        "recipientType": "DESTINATION_CHAIN",
        "deadline": deadline,
    });
    let answer = oneclick.quote(&body)?;
    let q = &answer["quote"];
    let text = |field: &str| -> Result<String, ApiError> {
        q[field]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| ApiError::Upstream(format!("1Click quote has no `{field}`")))
    };
    let deposit_address = q["depositAddress"].as_str().map(str::to_string);
    if !request.dry && deposit_address.is_none() {
        return Err(ApiError::Upstream(
            "1Click returned a real quote without a deposit address".into(),
        ));
    }
    Ok(Quote {
        dry: request.dry,
        deposit_address,
        amount_in: text("amountIn")?,
        amount_in_formatted: text("amountInFormatted")?,
        amount_out: text("amountOut")?,
        amount_out_formatted: text("amountOutFormatted")?,
        deadline,
        time_estimate_secs: q["timeEstimate"].as_u64(),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub status: String,
    pub tx_hashes: Vec<String>,
}

pub fn status(oneclick: &dyn OneClick, deposit_address: &str) -> Result<Status, ApiError> {
    if !plausible_address(deposit_address) {
        return Err(ApiError::BadRequest("not a deposit address".into()));
    }
    let raw = oneclick.status(deposit_address).map_err(|e| match e {
        OneClickError::Status(404, _) => ApiError::NotFound(format!(
            "1Click does not know deposit address {deposit_address}"
        )),
        e => e.into(),
    })?;
    let status = raw["status"]
        .as_str()
        .ok_or_else(|| ApiError::Upstream("1Click status has no `status`".into()))?;
    let details = &raw["swapDetails"];
    let mut tx_hashes = Vec::new();
    for side in [
        "originChainTxHashes",
        "destinationChainTxHashes",
        "intentHashes",
        "nearTxHashes",
    ] {
        for item in details[side].as_array().into_iter().flatten() {
            if let Some(hash) = item.as_str().or_else(|| item["hash"].as_str()) {
                if !tx_hashes.iter().any(|h| h == hash) {
                    tx_hashes.push(hash.to_string());
                }
            }
        }
    }
    Ok(Status {
        status: status.to_string(),
        tx_hashes,
    })
}

/// Unix seconds as `YYYY-MM-DDTHH:MM:SS.000Z`.
pub fn iso8601(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// An address or asset id from another chain: printable, no spaces, sane length.
fn plausible_address(text: &str) -> bool {
    (8..=160).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'-' | b'_'))
}
