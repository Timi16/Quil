//! Starts the read API. Configuration comes from the environment:
//!
//! * `PORT` (default 8080)
//! * `NEAR_RPC_URL` (default NEAR testnet)
//! * `CLAIM_MARKET_ID` and `FIXING_ORACLE_ID`: the deployed contracts. Without
//!   both, chain routes answer 503 and only the live index is served.

use std::net::SocketAddr;
use std::sync::Arc;

use quill_api::chain::{Chain, Rpc};
use quill_api::service::Contracts;
use quill_api::{router, AppState};
use quill_oracle::fetch::Http;

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = env("PORT").map(|p| p.parse()).transpose()?.unwrap_or(8080);
    let rpc = env("NEAR_RPC_URL").unwrap_or_else(|| "https://rpc.testnet.near.org".into());

    let chain = match (env("CLAIM_MARKET_ID"), env("FIXING_ORACLE_ID")) {
        (Some(claim_market), Some(fixing_oracle)) => {
            eprintln!("chain: {claim_market} and {fixing_oracle} via {rpc}");
            Some((
                Arc::new(Rpc::new(rpc)) as Arc<dyn Chain>,
                Contracts {
                    claim_market,
                    fixing_oracle,
                },
            ))
        }
        _ => {
            eprintln!("chain: not configured (set CLAIM_MARKET_ID and FIXING_ORACLE_ID); serving the live index only");
            None
        }
    };

    let app = router(AppState::new(chain, Arc::new(Http)));
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
    eprintln!("quill-api listening on http://localhost:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
