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
//! | `GET /v1/portfolio/{account}` | An account's positions |

pub mod chain;
pub mod service;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use chain::Chain;
use quill_oracle::Fetch;
use service::{ApiError, Contracts, LiveIndex};

/// How long a live index is reused before the sources are asked again.
pub const LIVE_TTL: Duration = Duration::from_secs(60);
/// The indexes the live endpoint computes.
pub const LIVE_INDEXES: [&str; 3] = ["H100", "B200", "INF_OUT"];

/// Where the API gets its answers.
pub struct AppState {
    /// NEAR and the contract ids. `None` until the contracts are deployed.
    pub chain: Option<(Arc<dyn Chain>, Contracts)>,
    /// The public price sources.
    pub fetch: Arc<dyn Fetch + Send + Sync>,
    live: Mutex<Option<(Instant, LiveIndex)>>,
}

impl AppState {
    pub fn new(
        chain: Option<(Arc<dyn Chain>, Contracts)>,
        fetch: Arc<dyn Fetch + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            chain,
            fetch,
            live: Mutex::new(None),
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/index/live", get(live))
        .route("/v1/markets", get(markets))
        .route("/v1/markets/{id}", get(market))
        .route("/v1/fixings/{index}/latest", get(latest_fixing))
        .route("/v1/portfolio/{account}", get(portfolio))
        .with_state(state)
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
