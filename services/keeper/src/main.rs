//! Runs the keeper. Configuration comes from the environment:
//!
//! * `KEEPER_ACCOUNT`: the account that signs and pays (required).
//! * `KEEPER_KEY_FILE`: its key (default `~/.near-credentials/testnet/<account>.json`).
//! * `CLAIM_MARKET_ID`, `FIXING_ORACLE_ID`: the deployed contracts (required).
//! * `FIXING_DEPOSIT_MILLINEAR`: NEAR attached to each fixing request, in thousandths
//!   (default 20 = 0.02 NEAR; OutLayer refunds what it does not use to the keeper).
//! * `TICK_SECS`: seconds between ticks (default 60).
//! * `NEAR_RPC_URL`: comma-separated RPC endpoints with failover (default FastNEAR,
//!   then NEAR's own testnet RPC).
//! * `ONCE=1`: run a single tick and exit.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use near_api::types::{AccountId, Data, NearGas, NearToken};
use near_api::{Contract, NetworkConfig, Signer};
use quill_keeper::{explain, plan, Action, MarketInfo, Memory, Observation, SLOT_SECS};
use serde::Deserialize;
use serde_json::json;

struct Config {
    account: AccountId,
    signer: Arc<Signer>,
    market: Contract,
    oracle: Contract,
    deposit: NearToken,
    tick: Duration,
    once: bool,
    network: NetworkConfig,
}

#[derive(Debug, Deserialize)]
struct RawMarket {
    id: u32,
    expiry_slot: u64,
    settled_at: Option<String>,
    void: bool,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn required(name: &str) -> Result<String, String> {
    env(name).ok_or_else(|| format!("{name} is not set"))
}

fn config() -> Result<Config, String> {
    let account: AccountId = required("KEEPER_ACCOUNT")?
        .parse()
        .map_err(|e| format!("KEEPER_ACCOUNT: {e}"))?;
    let key_file = env("KEEPER_KEY_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(format!("{home}/.near-credentials/testnet/{account}.json"))
        });
    let signer = Signer::from_access_keyfile(key_file.clone())
        .map_err(|e| format!("{}: {e}", key_file.display()))?;
    let contract = |name: &str| -> Result<Contract, String> {
        let id: AccountId = required(name)?
            .parse()
            .map_err(|e| format!("{name}: {e}"))?;
        Ok(Contract(id))
    };
    let millinear: u128 = env("FIXING_DEPOSIT_MILLINEAR")
        .map(|v| v.parse())
        .transpose()
        .map_err(|e| format!("FIXING_DEPOSIT_MILLINEAR: {e}"))?
        .unwrap_or(20);
    let tick: u64 = env("TICK_SECS")
        .map(|v| v.parse())
        .transpose()
        .map_err(|e| format!("TICK_SECS: {e}"))?
        .unwrap_or(60);
    Ok(Config {
        account,
        signer,
        market: contract("CLAIM_MARKET_ID")?,
        oracle: contract("FIXING_ORACLE_ID")?,
        deposit: NearToken::from_millinear(millinear),
        tick: Duration::from_secs(tick.max(10)),
        once: env("ONCE").as_deref() == Some("1"),
        network: network(env("NEAR_RPC_URL"))?,
    })
}

/// Testnet with failover: NEAR_RPC_URL (comma-separated) or FastNEAR then NEAR's own RPC.
/// A missed hour can never be fixed later, so one flaky endpoint must not cost one.
fn network(urls: Option<String>) -> Result<NetworkConfig, String> {
    let urls =
        urls.unwrap_or_else(|| "https://test.rpc.fastnear.com,https://rpc.testnet.near.org".into());
    let mut net = NetworkConfig::testnet();
    net.rpc_endpoints = urls
        .split(',')
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(|u| {
            u.parse()
                .map(near_api::RPCEndpoint::new)
                .map_err(|e| format!("NEAR_RPC_URL `{u}`: {e}"))
        })
        .collect::<Result<_, _>>()?;
    if net.rpc_endpoints.is_empty() {
        return Err("NEAR_RPC_URL names no endpoint".into());
    }
    Ok(net)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read what the keeper needs. A contract that cannot be read is reported and treated as empty.
async fn observe(cfg: &Config) -> Observation {
    let now = now();
    let slot = now / SLOT_SECS;

    let indexes: Vec<String> = match cfg
        .oracle
        .call_function("indexes", json!({}))
        .read_only::<Vec<serde_json::Value>>()
        .fetch_from(&cfg.network)
        .await
    {
        Ok(Data { data, .. }) => data
            .iter()
            .filter_map(|i| i["id"].as_str().map(String::from))
            .collect(),
        Err(e) => {
            eprintln!(
                "oracle unreadable ({}); will still try to request a fixing",
                explain(&e.to_string())
            );
            Vec::new()
        }
    };
    let mut hour_is_fixed = false;
    for index in &indexes {
        let found = cfg
            .oracle
            .call_function("fixing_at", json!({ "index": index, "slot": slot }))
            .read_only::<Option<serde_json::Value>>()
            .fetch_from(&cfg.network)
            .await;
        if matches!(found, Ok(Data { data: Some(_), .. })) {
            hour_is_fixed = true;
            break;
        }
    }

    let mut markets = Vec::new();
    match cfg
        .market
        .call_function("market_count", json!({}))
        .read_only::<u32>()
        .fetch_from(&cfg.network)
        .await
    {
        Ok(Data { data: count, .. }) => {
            for id in 0..count {
                match cfg
                    .market
                    .call_function("get_market", json!({ "market_id": id }))
                    .read_only::<Option<RawMarket>>()
                    .fetch_from(&cfg.network)
                    .await
                {
                    Ok(Data { data: Some(m), .. }) => markets.push(MarketInfo {
                        id: m.id,
                        expiry_slot: m.expiry_slot,
                        finished: m.void || m.settled_at.is_some(),
                    }),
                    Ok(_) => {}
                    Err(e) => eprintln!("market {id} unreadable: {e}"),
                }
            }
        }
        Err(e) => eprintln!("claim-market unreadable: {e}"),
    }
    Observation {
        now,
        hour_is_fixed,
        markets,
    }
}

/// Send one action and report the contract's own events.
async fn act(cfg: &Config, action: Action) {
    let (contract, method, args, deposit, label) = match action {
        Action::RequestFixing { slot } => (
            &cfg.oracle,
            "request_fixing",
            json!({}),
            cfg.deposit,
            format!("request fixing for slot {slot}"),
        ),
        Action::Settle { market } => (
            &cfg.market,
            "settle",
            json!({ "market_id": market }),
            NearToken::from_yoctonear(0),
            format!("settle market {market}"),
        ),
    };
    let sent = contract
        .call_function(method, args)
        .transaction()
        .gas(NearGas::from_tgas(300))
        .deposit(deposit)
        .with_signer(cfg.account.clone(), cfg.signer.clone())
        .send_to(&cfg.network)
        .await;
    match sent
        .map_err(|e| e.to_string())
        .and_then(|r| r.into_result().map_err(|e| format!("{e:?}")))
    {
        Ok(success) => {
            println!("ok    {label}");
            for log in success.logs() {
                if let Some(event) = log.strip_prefix("EVENT_JSON:") {
                    println!("      {event}");
                } else if log.contains("Execution failed") || log.contains("Refunded") {
                    println!("      outlayer: {log}");
                }
            }
        }
        Err(e) => println!("fail  {label}: {}", explain(&e)),
    }
}

#[tokio::main]
async fn main() {
    let cfg = match config() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("keeper not started: {e}");
            std::process::exit(2);
        }
    };
    println!(
        "keeper {} watching {} and {}",
        cfg.account, cfg.market.0, cfg.oracle.0
    );
    let mut memory = Memory::default();
    loop {
        let seen = observe(&cfg).await;
        let (actions, next) = plan(&seen, &memory);
        memory = next;
        if actions.is_empty() {
            println!(
                "idle  slot {}: hour fixed = {}, {} market(s)",
                seen.now / SLOT_SECS,
                seen.hour_is_fixed,
                seen.markets.len()
            );
        }
        for action in actions {
            act(&cfg, action).await;
        }
        if cfg.once {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(cfg.tick) => {}
            _ = tokio::signal::ctrl_c() => break,
        }
    }
}
