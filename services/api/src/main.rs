//! Starts the read API. Configuration comes from the environment:
//!
//! * `PORT` (default 8080)
//! * `NEAR_RPC_URL` (default NEAR testnet)
//! * `CLAIM_MARKET_ID` and `FIXING_ORACLE_ID`: the deployed contracts. Without
//!   both, chain routes answer 503 and only the live index is served.
//! * `USDC_ID`: the collateral token contract (shown to the app, and the token
//!   feed payments must be made in).
//! * `FEED_PAYEE`: who is paid for feed reads; without it the paid feed is off.
//! * `FEED_PRICE_MICRO`: price of one read in micro-USDC (default 10000 = 0.01).
//! * `FEED_LEDGER`: file of spent payment hashes (default `logs/feed-payments.txt`).
//! * `ONECLICK_JWT`: optional 1Click key (without it 1Click charges 0.2%).
//! * `APP_DIR`: the web app to serve at `/` (default `app` when it exists).
//! * `NETWORK` and `EXPLORER`: shown to the app (default testnet, nearblocks).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use quill_api::chain::{Chain, Rpc};
use quill_api::feed::Ledger;
use quill_api::intents::HttpOneClick;
use quill_api::service::Contracts;
use quill_api::{app, AppState, Config};
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
                Arc::new(Rpc::new(rpc.clone())) as Arc<dyn Chain>,
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

    let defaults = Config::default();
    let config = Config {
        network: env("NETWORK").unwrap_or(defaults.network),
        rpc,
        usdc: env("USDC_ID"),
        usdc_symbol: env("USDC_SYMBOL").unwrap_or(defaults.usdc_symbol),
        feed_payee: env("FEED_PAYEE"),
        feed_price: env("FEED_PRICE_MICRO")
            .map(|p| p.parse())
            .transpose()?
            .unwrap_or(defaults.feed_price),
        explorer: env("EXPLORER").unwrap_or(defaults.explorer),
    };
    match (&config.usdc, &config.feed_payee) {
        (Some(usdc), Some(payee)) => eprintln!(
            "feed: {} {usdc} per read, paid to {payee}",
            config.feed_price
        ),
        _ => eprintln!("feed: off (set USDC_ID and FEED_PAYEE)"),
    }
    let ledger = Ledger::open(PathBuf::from(
        env("FEED_LEDGER").unwrap_or_else(|| "logs/feed-payments.txt".into()),
    ));
    let app_dir = env("APP_DIR")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("app")).filter(|d| d.join("index.html").exists()));
    match &app_dir {
        Some(dir) => eprintln!("app: serving {} at /", dir.display()),
        None => eprintln!("app: none (set APP_DIR)"),
    }

    let oneclick = Arc::new(HttpOneClick::new(env("ONECLICK_JWT")));
    let app = app(
        AppState::with(chain, Arc::new(Http), config, oneclick, ledger),
        app_dir,
    );
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
    eprintln!("quill-api listening on http://localhost:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
