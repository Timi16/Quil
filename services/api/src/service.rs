//! What the API knows, as plain functions over a [`Chain`].
//!
//! The HTTP layer in `lib.rs` only routes to these, so every answer the API
//! gives is tested here without sockets. All amounts are exact decimal strings
//! with six places; nothing is ever a float.

use quill_core::payout::{Band, Outcome, Side};
use quill_core::UNIT;
use quill_oracle::{Fetch, Input, Run};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chain::Chain;

/// The two deployed contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contracts {
    pub claim_market: String,
    pub fixing_oracle: String,
}

/// Why a request could not be answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The contract ids are not configured yet.
    NotConfigured,
    NotFound(String),
    BadRequest(String),
    /// NEAR or a price source failed.
    Upstream(String),
}

/// Micro-units as an exact decimal string: `3060000` is `"3.060000"`.
pub fn decimal(micro: u128) -> String {
    format!("{}.{:06}", micro / UNIT, micro % UNIT)
}

fn amount(text: &str, field: &str) -> Result<u128, ApiError> {
    text.parse()
        .map_err(|_| ApiError::Upstream(format!("contract returned a bad `{field}`: {text}")))
}

// ----- shapes the contracts return -----

#[derive(Debug, Deserialize)]
struct RawMarket {
    id: u32,
    index: String,
    floor: String,
    cap: String,
    expiry_slot: u64,
    long_supply: String,
    short_supply: String,
    locked: String,
    settled_at: Option<String>,
    void: bool,
}

#[derive(Debug, Deserialize)]
struct RawFixing {
    index: String,
    slot: u64,
    value: String,
    sources: u32,
    inputs: String,
    accepted: bool,
}

// ----- shapes the API returns -----

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fixing {
    pub index: String,
    pub slot: u64,
    /// Unix seconds at the start of the fixing's hour.
    pub at: u64,
    pub value: String,
    pub sources: u32,
    pub inputs: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Market {
    pub id: u32,
    pub index: String,
    pub floor: String,
    pub cap: String,
    /// What one LONG plus one SHORT is always worth.
    pub width: String,
    pub expiry_slot: u64,
    /// Unix seconds.
    pub expires_at: u64,
    /// `open`, `expired` (waiting to be settled), `settled` or `void`.
    pub status: &'static str,
    pub collateral_locked: String,
    pub long_supply: String,
    pub short_supply: String,
    /// The latest accepted fixing, or the settlement value once settled.
    pub index_value: Option<String>,
    /// What one LONG would pay at `index_value`.
    pub long_value: Option<String>,
    /// What one SHORT would pay at `index_value`.
    pub short_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Position {
    pub market: u32,
    pub index: String,
    pub status: &'static str,
    pub long: String,
    pub short: String,
    /// What the position would pay at the market's current `index_value`.
    pub worth_now: Option<String>,
    /// The most it can ever pay.
    pub max_payout: String,
    /// True when the market is settled or void and there is something to claim.
    pub claimable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveValue {
    pub index: String,
    pub value: String,
    pub sources: u32,
}

/// A fixing computed right now from the public sources. Not on-chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveIndex {
    pub slot: u64,
    pub values: Vec<LiveValue>,
    /// `index|provider|median|listings`, one line per source that counted.
    pub evidence: Vec<String>,
    /// Sources that failed or were unusable this time.
    pub problems: Vec<String>,
    pub inputs: String,
}

// ----- reads -----

/// Compute the indexes from the live sources, exactly as the enclave program would.
pub fn live_index(fetch: &dyn Fetch, slot: u64, indexes: &[&str]) -> LiveIndex {
    let input = Input {
        slot,
        indexes: indexes.iter().map(|s| s.to_string()).collect(),
        previous: Default::default(),
    };
    let Run {
        output,
        evidence,
        problems,
    } = quill_oracle::run(&input, fetch);
    let values = output
        .v
        .iter()
        .map(|(index, value)| LiveValue {
            index: index.clone(),
            value: decimal(u128::from(*value)),
            sources: output.n.get(index).copied().unwrap_or(0),
        })
        .collect();
    LiveIndex {
        slot,
        values,
        evidence,
        problems,
        inputs: output.inputs,
    }
}

pub fn latest_fixing(
    chain: &dyn Chain,
    contracts: &Contracts,
    index: &str,
) -> Result<Fixing, ApiError> {
    if index.is_empty() || index.len() > 32 {
        return Err(ApiError::BadRequest(
            "index id must be 1 to 32 characters".into(),
        ));
    }
    let raw = chain
        .view(
            &contracts.fixing_oracle,
            "latest",
            json!({ "index": index }),
        )
        .map_err(ApiError::Upstream)?;
    if raw.is_null() {
        return Err(ApiError::NotFound(format!(
            "no accepted fixing for {index} yet"
        )));
    }
    let raw: RawFixing = parse(raw, "fixing")?;
    Ok(Fixing {
        at: raw.slot * 3600,
        value: decimal(amount(&raw.value, "value")?),
        index: raw.index,
        slot: raw.slot,
        sources: raw.sources,
        inputs: raw.inputs,
        accepted: raw.accepted,
    })
}

pub fn market(
    chain: &dyn Chain,
    contracts: &Contracts,
    id: u32,
    now_slot: u64,
) -> Result<Market, ApiError> {
    let raw = chain
        .view(
            &contracts.claim_market,
            "get_market",
            json!({ "market_id": id }),
        )
        .map_err(ApiError::Upstream)?;
    if raw.is_null() {
        return Err(ApiError::NotFound(format!("no market {id}")));
    }
    let (market, _) = describe(chain, contracts, parse(raw, "market")?, now_slot)?;
    Ok(market)
}

pub fn markets(
    chain: &dyn Chain,
    contracts: &Contracts,
    now_slot: u64,
) -> Result<Vec<Market>, ApiError> {
    let count = chain
        .view(&contracts.claim_market, "market_count", json!({}))
        .map_err(ApiError::Upstream)?;
    let count = count
        .as_u64()
        .ok_or_else(|| ApiError::Upstream("market_count is not a number".into()))?;
    // Newest first; a bounded page keeps one request from walking an unbounded list.
    (0..count as u32)
        .rev()
        .take(100)
        .map(|id| market(chain, contracts, id, now_slot))
        .collect()
}

pub fn portfolio(
    chain: &dyn Chain,
    contracts: &Contracts,
    account: &str,
    now_slot: u64,
) -> Result<Vec<Position>, ApiError> {
    if !valid_account(account) {
        return Err(ApiError::BadRequest(format!(
            "`{account}` is not a NEAR account id"
        )));
    }
    let count = chain
        .view(&contracts.claim_market, "market_count", json!({}))
        .map_err(ApiError::Upstream)?;
    let count = count
        .as_u64()
        .ok_or_else(|| ApiError::Upstream("market_count is not a number".into()))?;

    let mut positions = Vec::new();
    for id in (0..count as u32).rev().take(100) {
        let balance = |side: &str| -> Result<u128, ApiError> {
            let args = json!({ "market_id": id, "side": side, "account_id": account });
            let raw = chain
                .view(&contracts.claim_market, "balance_of", args)
                .map_err(ApiError::Upstream)?;
            amount(raw.as_str().unwrap_or(""), "balance")
        };
        let (long, short) = (balance("Long")?, balance("Short")?);
        if long == 0 && short == 0 {
            continue;
        }
        let raw = chain
            .view(
                &contracts.claim_market,
                "get_market",
                json!({ "market_id": id }),
            )
            .map_err(ApiError::Upstream)?;
        let (market, pricing) = describe(chain, contracts, parse(raw, "market")?, now_slot)?;
        let worth_now = match pricing {
            Some((band, outcome)) => Some(decimal(
                pay(&band, Side::Long, long, outcome)? + pay(&band, Side::Short, short, outcome)?,
            )),
            None => None,
        };
        let width = amount_of_decimal(&market.width);
        positions.push(Position {
            market: id,
            index: market.index,
            status: market.status,
            long: decimal(long),
            short: decimal(short),
            worth_now,
            // Whichever side is larger could be paid the whole band.
            max_payout: decimal(long.max(short).saturating_mul(width) / UNIT),
            claimable: matches!(market.status, "settled" | "void"),
        });
    }
    Ok(positions)
}

// ----- internals -----

fn parse<T: serde::de::DeserializeOwned>(raw: Value, what: &str) -> Result<T, ApiError> {
    serde_json::from_value(raw)
        .map_err(|e| ApiError::Upstream(format!("contract returned an unexpected {what}: {e}")))
}

fn pay(band: &Band, side: Side, units: u128, outcome: Outcome) -> Result<u128, ApiError> {
    band.payout(side, units, outcome)
        .map_err(|_| ApiError::Upstream("amount too large".into()))
}

fn amount_of_decimal(text: &str) -> u128 {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, "0"));
    whole.parse::<u128>().unwrap_or(0) * UNIT + fraction.parse::<u128>().unwrap_or(0)
}

/// Turn a contract market into the API's view, with the price it is currently worth at.
fn describe(
    chain: &dyn Chain,
    contracts: &Contracts,
    raw: RawMarket,
    now_slot: u64,
) -> Result<(Market, Option<(Band, Outcome)>), ApiError> {
    let (floor, cap) = (amount(&raw.floor, "floor")?, amount(&raw.cap, "cap")?);
    let band = Band::new(floor, cap)
        .map_err(|_| ApiError::Upstream("contract returned an empty band".into()))?;

    let (status, outcome) = if raw.void {
        ("void", Some(Outcome::Void))
    } else if let Some(settled) = &raw.settled_at {
        (
            "settled",
            Some(Outcome::Settled(amount(settled, "settled_at")?)),
        )
    } else {
        let status = if now_slot >= raw.expiry_slot {
            "expired"
        } else {
            "open"
        };
        // An open market is priced at the latest fixing, if the oracle has one.
        // The market itself comes from claim-market. If the oracle cannot be read
        // (not deployed yet, or down), show the market without a price rather than not at all.
        let latest = latest_fixing(chain, contracts, &raw.index)
            .ok()
            .map(|fixing| Outcome::Settled(amount_of_decimal(&fixing.value)));
        (status, latest)
    };

    let index_value = match outcome {
        Some(Outcome::Settled(value)) => Some(decimal(value)),
        _ => None,
    };
    let per_unit = |side| {
        outcome
            .map(|o| pay(&band, side, UNIT, o).map(decimal))
            .transpose()
    };
    let market = Market {
        id: raw.id,
        index: raw.index,
        floor: decimal(floor),
        cap: decimal(cap),
        width: decimal(band.width()),
        expiry_slot: raw.expiry_slot,
        expires_at: raw.expiry_slot * 3600,
        status,
        collateral_locked: decimal(amount(&raw.locked, "locked")?),
        long_supply: decimal(amount(&raw.long_supply, "long_supply")?),
        short_supply: decimal(amount(&raw.short_supply, "short_supply")?),
        index_value,
        long_value: per_unit(Side::Long)?,
        short_value: per_unit(Side::Short)?,
    };
    Ok((market, outcome.map(|o| (band, o))))
}

/// NEAR account id rules, enough to reject junk before it reaches the RPC.
fn valid_account(account: &str) -> bool {
    (2..=64).contains(&account.len())
        && account.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        })
}
