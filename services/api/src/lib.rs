//! Quill's read API.
//!
//! A cache and a convenience, never an authority: it signs nothing, holds no
//! keys and every number it serves can be re-read from the chain. Anything
//! that decides money in the app should be read from NEAR directly.
//!
//! | Route | Returns |
//! |---|---|
//! | `GET /health` | Whether the chain is configured |
//! | `GET /v1/index/live` | Index values computed now from the public sources (not on-chain) |
//! | `GET /v1/markets` | Every market, newest first |
//! | `GET /v1/markets/{id}` | One market |
//! | `GET /v1/fixings/{index}/latest` | The latest accepted on-chain fixing |
//! | `GET /v1/fixings/{index}/history?hours=N` | Up to 168 hours of fixings, newest first |
//! | `GET /v1/portfolio/{account}` | An account's positions |
//! | `GET /v1/config` | Network and contract ids, for the web app |
//! | `GET /v1/collateral/tokens` | What NEAR Intents can bring in as collateral |
//! | `POST /v1/collateral/quote` | A 1Click quote for USDC delivered to a NEAR account |
//! | `GET /v1/collateral/status/{deposit_address}` | Where a 1Click deposit is |
//! | `GET /v1/feed/{index}` | The latest fixing, paid per read (HTTP 402) |
//!
//! Anything else is served from the web app's folder when one is configured.

pub mod chain;
pub mod feed;
pub mod intents;
pub mod service;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use chain::Chain;
use feed::{Ledger, Terms};
use intents::{OneClick, RouteToken};
use quill_oracle::Fetch;
use service::{decimal, ApiError, Contracts, LiveIndex};

/// How long a live index is reused before the sources are asked again.
pub const LIVE_TTL: Duration = Duration::from_secs(60);
/// The indexes the live endpoint computes.
pub const LIVE_INDEXES: [&str; 3] = ["H100", "B200", "INF_OUT"];

/// How long the 1Click token list is reused.
pub const TOKENS_TTL: Duration = Duration::from_secs(600);
/// Default hours of fixing history.
pub const DEFAULT_HISTORY_HOURS: u64 = 48;

/// What the web app needs to know about this deployment. Public by design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Config {
    pub network: String,
    pub rpc: String,
    /// The collateral token contract.
    pub usdc: Option<String>,
    pub usdc_symbol: String,
    /// Who is paid for feed reads. `None` turns the paid feed off.
    pub feed_payee: Option<String>,
    /// Price of one feed read, in micro-USDC.
    pub feed_price: u128,
    /// Block explorer base URL; a transaction is at `{explorer}/txns/{hash}`.
    pub explorer: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            network: "testnet".into(),
            rpc: "https://rpc.testnet.near.org".into(),
            usdc: None,
            usdc_symbol: "tUSDC".into(),
            feed_payee: None,
            feed_price: 10_000,
            explorer: "https://testnet.nearblocks.io".into(),
        }
    }
}

/// Where the API gets its answers.
pub struct AppState {
    /// NEAR and the contract ids. `None` until the contracts are deployed.
    pub chain: Option<(Arc<dyn Chain>, Contracts)>,
    /// The public price sources.
    pub fetch: Arc<dyn Fetch + Send + Sync>,
    pub config: Config,
    /// NEAR Intents (1Click), for collateral from other chains.
    pub oneclick: Arc<dyn OneClick>,
    /// Spent feed payments.
    pub ledger: Ledger,
    live: Mutex<Option<(Instant, LiveIndex)>>,
    tokens: Mutex<Option<(Instant, Vec<RouteToken>)>>,
}

impl AppState {
    /// Chain and price sources only: the paid feed off, real 1Click.
    pub fn new(
        chain: Option<(Arc<dyn Chain>, Contracts)>,
        fetch: Arc<dyn Fetch + Send + Sync>,
    ) -> Arc<Self> {
        Self::with(
            chain,
            fetch,
            Config::default(),
            Arc::new(intents::HttpOneClick::new(None)),
            Ledger::in_memory(),
        )
    }

    pub fn with(
        chain: Option<(Arc<dyn Chain>, Contracts)>,
        fetch: Arc<dyn Fetch + Send + Sync>,
        config: Config,
        oneclick: Arc<dyn OneClick>,
        ledger: Ledger,
    ) -> Arc<Self> {
        Arc::new(Self {
            chain,
            fetch,
            config,
            oneclick,
            ledger,
            live: Mutex::new(None),
            tokens: Mutex::new(None),
        })
    }

    fn terms(&self) -> Option<Terms> {
        Some(Terms {
            asset: self.config.usdc.clone()?,
            pay_to: self.config.feed_payee.clone()?,
            price: self.config.feed_price,
        })
    }
}

fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/v1/config", get(config))
        .route("/v1/index/live", get(live))
        .route("/v1/markets", get(markets))
        .route("/v1/markets/{id}", get(market))
        .route("/v1/fixings/{index}/latest", get(latest_fixing))
        .route("/v1/fixings/{index}/history", get(fixing_history))
        .route("/v1/portfolio/{account}", get(portfolio))
        .route("/v1/collateral/tokens", get(collateral_tokens))
        .route("/v1/collateral/quote", post(collateral_quote))
        .route("/v1/collateral/status/{address}", get(collateral_status))
        .route("/v1/feed/{index}", get(paid_feed))
}

/// Any origin may read the API and buy from the feed: the data is public and
/// there is nothing a cross-site request could abuse (no cookies, no keys).
fn cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers(Any)
}

/// The API alone.
pub fn router(state: Arc<AppState>) -> Router {
    routes().fallback(not_found).layer(cors()).with_state(state)
}

/// The API plus the web app's static files from `app_dir` at `/`. Unknown
/// `/v1/…` paths still get a JSON 404, never the app's HTML.
pub fn app(state: Arc<AppState>, app_dir: Option<PathBuf>) -> Router {
    match app_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            routes()
                .route("/v1/{*rest}", get(not_found).post(not_found))
                .fallback_service(ServeDir::new(dir).not_found_service(ServeFile::new(index)))
                .layer(cors())
                .with_state(state)
        }
        None => router(state),
    }
}

async fn not_found() -> ApiError {
    ApiError::NotFound("no such route".into())
}

/// The current hourly slot.
pub fn now_slot() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 3600)
        .unwrap_or(0)
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "The contracts are not deployed yet. Set CLAIM_MARKET_ID and FIXING_ORACLE_ID."
                    .to_string(),
            ),
            ApiError::NotFound(what) => (StatusCode::NOT_FOUND, what),
            ApiError::BadRequest(why) => (StatusCode::BAD_REQUEST, why),
            ApiError::Upstream(why) => (StatusCode::BAD_GATEWAY, why),
            ApiError::Unavailable(why) => (StatusCode::SERVICE_UNAVAILABLE, why),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

/// Run blocking work (RPC and source fetches use blocking HTTP) off the async threads.
async fn blocking<T, F>(work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| ApiError::Upstream(format!("worker failed: {e}")))?
}

/// Run `read` against the chain, or say the chain is not configured.
async fn on_chain<T, F>(state: Arc<AppState>, read: F) -> Result<Json<T>, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&dyn Chain, &Contracts) -> Result<T, ApiError> + Send + 'static,
{
    blocking(move || {
        let (chain, contracts) = state.chain.as_ref().ok_or(ApiError::NotConfigured)?;
        read(chain.as_ref(), contracts)
    })
    .await
    .map(Json)
}

async fn health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "chain": state.chain.as_ref().map(|(_, c)| json!({ "claim_market": c.claim_market, "fixing_oracle": c.fixing_oracle })),
        "slot": now_slot(),
    }))
}

async fn live(State(state): State<Arc<AppState>>) -> Result<Json<LiveIndex>, ApiError> {
    blocking(move || {
        if let Some((at, cached)) = state
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if at.elapsed() < LIVE_TTL {
                return Ok(cached.clone());
            }
        }
        let fresh = service::live_index(state.fetch.as_ref(), now_slot(), &LIVE_INDEXES);
        if fresh.values.is_empty() {
            // Do not cache a total outage: the next request should try again.
            return Err(ApiError::Upstream(format!(
                "no price source answered: {}",
                fresh.problems.join("; ")
            )));
        }
        *state.live.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), fresh.clone()));
        Ok(fresh)
    })
    .await
    .map(Json)
}

async fn markets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<service::Market>>, ApiError> {
    on_chain(state, |chain, contracts| {
        service::markets(chain, contracts, now_slot())
    })
    .await
}

async fn market(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<service::Market>, ApiError> {
    // Parsed here, not by the extractor, so a bad id gets the same JSON error shape as everything else.
    let id: u32 = id
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("`{id}` is not a market id")))?;
    on_chain(state, move |chain, contracts| {
        service::market(chain, contracts, id, now_slot())
    })
    .await
}

async fn latest_fixing(
    State(state): State<Arc<AppState>>,
    Path(index): Path<String>,
) -> Result<Json<service::Fixing>, ApiError> {
    on_chain(state, move |chain, contracts| {
        service::latest_fixing(chain, contracts, &index)
    })
    .await
}

async fn portfolio(
    State(state): State<Arc<AppState>>,
    Path(account): Path<String>,
) -> Result<Json<Vec<service::Position>>, ApiError> {
    on_chain(state, move |chain, contracts| {
        service::portfolio(chain, contracts, &account, now_slot())
    })
    .await
}

async fn config(State(state): State<Arc<AppState>>) -> Json<Value> {
    let c = &state.config;
    let contracts = state.chain.as_ref().map(|(_, c)| c);
    Json(json!({
        "network": c.network,
        "rpc": c.rpc,
        "claim_market": contracts.map(|c| &c.claim_market),
        "fixing_oracle": contracts.map(|c| &c.fixing_oracle),
        "usdc": c.usdc,
        "usdc_symbol": c.usdc_symbol,
        "feed_payee": c.feed_payee,
        "feed_price": decimal(c.feed_price),
        "explorer": c.explorer,
        "slot": now_slot(),
    }))
}

async fn fixing_history(
    State(state): State<Arc<AppState>>,
    Path(index): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<service::Fixing>>, ApiError> {
    let hours = match query.get("hours") {
        Some(h) => h
            .parse()
            .map_err(|_| ApiError::BadRequest(format!("`{h}` is not a number of hours")))?,
        None => DEFAULT_HISTORY_HOURS,
    };
    on_chain(state, move |chain, contracts| {
        service::fixing_history(chain, contracts, &index, now_slot(), hours)
    })
    .await
}

// ----- collateral from any chain (NEAR Intents) -----

async fn collateral_tokens(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RouteToken>>, ApiError> {
    blocking(move || {
        if let Some((at, cached)) = state
            .tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if at.elapsed() < TOKENS_TTL {
                return Ok(cached.clone());
            }
        }
        let fresh = intents::tokens(state.oneclick.as_ref())?;
        *state.tokens.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), fresh.clone()));
        Ok(fresh)
    })
    .await
    .map(Json)
}

async fn collateral_quote(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<intents::Quote>, ApiError> {
    // Parsed by hand so a bad body gets the same JSON error as everything else.
    let request: intents::QuoteRequest = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("bad quote request: {e}")))?;
    blocking(move || intents::quote(state.oneclick.as_ref(), &request, unix_now()))
        .await
        .map(Json)
}

async fn collateral_status(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Result<Json<intents::Status>, ApiError> {
    blocking(move || intents::status(state.oneclick.as_ref(), &address))
        .await
        .map(Json)
}

// ----- the paid feed -----

/// `402 Payment Required`, with what to pay and how.
fn payment_required(terms: &Terms, network: &str, why: Option<String>) -> Response {
    let body = json!({
        "error": why.unwrap_or_else(|| "payment required".into()),
        "price": decimal(terms.price),
        "price_micro": terms.price.to_string(),
        "asset": terms.asset,
        "pay_to": terms.pay_to,
        "network": network,
        "how": format!(
            "Call ft_transfer on {} with receiver_id {} and amount {} (1 yoctoNEAR attached), then repeat this request with headers X-Payment: <transaction hash> and X-Payer: <your account>. Each transaction buys one read.",
            terms.asset, terms.pay_to, terms.price
        ),
    });
    (StatusCode::PAYMENT_REQUIRED, Json(body)).into_response()
}

async fn paid_feed(
    State(state): State<Arc<AppState>>,
    Path(index): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let terms = state.terms().ok_or_else(|| {
        ApiError::Unavailable("the paid feed is off: set USDC_ID and FEED_PAYEE".into())
    })?;
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .map(String::from)
    };
    let (Some(tx_hash), Some(payer)) = (header("x-payment"), header("x-payer")) else {
        return Ok(payment_required(&terms, &state.config.network, None));
    };
    if !feed::valid_tx_hash(&tx_hash) {
        return Ok(payment_required(
            &terms,
            &state.config.network,
            Some("X-Payment is not a transaction hash".into()),
        ));
    }
    if !service::valid_account(&payer) {
        return Ok(payment_required(
            &terms,
            &state.config.network,
            Some("X-Payer is not a NEAR account id".into()),
        ));
    }

    let network = state.config.network.clone();
    blocking(move || {
        let (chain, contracts) = state.chain.as_ref().ok_or(ApiError::NotConfigured)?;
        // Read first: if the chain cannot answer, the payment is not spent.
        let fixing = service::latest_fixing(chain.as_ref(), contracts, &index)?;
        let code = service::code_ref(chain.as_ref(), contracts).ok();

        let refused = |why: String| Ok(payment_required(&terms, &network, Some(why)));
        let tx = match chain.transaction(&tx_hash, &payer) {
            Ok(Some(tx)) => tx,
            Ok(None) => {
                return refused(format!(
                    "payment not found: no transaction {tx_hash} from {payer}"
                ))
            }
            Err(e) => return Err(ApiError::Upstream(e)),
        };
        let paid = match feed::verify(&tx, &payer, &terms) {
            Ok(paid) => paid,
            Err(ApiError::BadRequest(why)) => return refused(why),
            Err(e) => return Err(e),
        };
        if !state.ledger.spend(&tx_hash) {
            return refused("payment not accepted: this transaction was already used".into());
        }
        let body = json!({
            "fixing": fixing,
            "oracle": contracts.fixing_oracle,
            "code_hash": code.as_ref().map(|c| &c.wasm_hash),
            "wasm_url": code.as_ref().map(|c| &c.wasm_url),
            "paid": { "tx": tx_hash, "payer": paid.payer, "amount": decimal(paid.amount) },
        });
        Ok(Json(body).into_response())
    })
    .await
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
