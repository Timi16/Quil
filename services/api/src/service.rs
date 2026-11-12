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
    /// A feature that needs configuration the server does not have.
    Unavailable(String),
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

/// Most hours of history one request may ask for (the oracle's own bound).
pub const MAX_HISTORY_HOURS: u64 = 168;

fn check_index(index: &str) -> Result<(), ApiError> {
    if index.is_empty()
        || index.len() > 32
        || !index
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(ApiError::BadRequest(
            "index id must be 1 to 32 letters, digits or underscores".into(),
        ));
    }
    Ok(())
}

fn to_fixing(raw: RawFixing) -> Result<Fixing, ApiError> {
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

pub fn latest_fixing(
    chain: &dyn Chain,
    contracts: &Contracts,
    index: &str,
) -> Result<Fixing, ApiError> {
    check_index(index)?;
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
    to_fixing(parse(raw, "fixing")?)
}

/// Every fixing (accepted and flagged) in the `hours` up to and including `to_slot`, newest first.
pub fn fixing_history(
    chain: &dyn Chain,
    contracts: &Contracts,
    index: &str,
    to_slot: u64,
    hours: u64,
) -> Result<Vec<Fixing>, ApiError> {
    check_index(index)?;
    if hours == 0 || hours > MAX_HISTORY_HOURS {
        return Err(ApiError::BadRequest(format!(
            "hours must be between 1 and {MAX_HISTORY_HOURS}"
        )));
    }
    let raw = chain
        .view(
            &contracts.fixing_oracle,
            "fixings_between",
            json!({ "index": index, "to_slot": to_slot, "hours": hours }),
        )
        .map_err(ApiError::Upstream)?;
    let raw: Vec<RawFixing> = parse(raw, "fixing history")?;
    raw.into_iter().map(to_fixing).collect()
}

/// The oracle program every fixing came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeRef {
    pub wasm_url: String,
    pub wasm_hash: String,
    pub build_target: String,
}

pub fn code_ref(chain: &dyn Chain, contracts: &Contracts) -> Result<CodeRef, ApiError> {
    let raw = chain
        .view(&contracts.fixing_oracle, "code_ref", json!({}))
        .map_err(ApiError::Upstream)?;
    parse(raw, "code ref")
}

pub fn market(
    chain: &dyn Chain,
    contracts: &Contracts,
    id: u32,
    now_slot: u64,
) -> Result<Market, ApiError> {
    let raw = get_market(chain, contracts, id)?;
    let (market, _) = describe(chain, contracts, raw, now_slot, &mut Prices::default())?;
    Ok(market)
}

pub fn markets(
    chain: &dyn Chain,
    contracts: &Contracts,
    now_slot: u64,
) -> Result<Vec<Market>, ApiError> {
    let mut prices = Prices::default();
    // Newest first; a bounded page keeps one request from walking an unbounded list.
    page(chain, contracts)?
        .into_iter()
        .map(|id| {
            let raw = get_market(chain, contracts, id)?;
            Ok(describe(chain, contracts, raw, now_slot, &mut prices)?.0)
        })
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
    let ids = page(chain, contracts)?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // Every LONG and SHORT balance in one NEP-245 call instead of two calls per market.
    let token_ids: Vec<String> = ids
        .iter()
        .flat_map(|id| [format!("{id}:L"), format!("{id}:S")])
        .collect();
    let raw = chain
        .view(
            &contracts.claim_market,
            "mt_batch_balance_of",
            json!({ "account_id": account, "token_ids": token_ids }),
        )
        .map_err(ApiError::Upstream)?;
    let balances: Vec<String> = parse(raw, "balances")?;
    if balances.len() != token_ids.len() {
        return Err(ApiError::Upstream(format!(
            "asked for {} balances, got {}",
            token_ids.len(),
            balances.len()
        )));
    }

    let mut prices = Prices::default();
    let mut positions = Vec::new();
    let (pairs, _) = balances.as_chunks::<2>();
    for (id, [long, short]) in ids.into_iter().zip(pairs) {
        let (long, short) = (amount(long, "balance")?, amount(short, "balance")?);
        if long == 0 && short == 0 {
            continue;
        }
        let raw = get_market(chain, contracts, id)?;
        let (market, pricing) = describe(chain, contracts, raw, now_slot, &mut prices)?;
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

/// Most markets one request reads.
pub const PAGE: u32 = 100;

/// The newest [`PAGE`] market ids, newest first.
fn page(chain: &dyn Chain, contracts: &Contracts) -> Result<Vec<u32>, ApiError> {
    let count = chain
        .view(&contracts.claim_market, "market_count", json!({}))
        .map_err(ApiError::Upstream)?;
    let count = count
        .as_u64()
        .and_then(|c| u32::try_from(c).ok())
        .ok_or_else(|| ApiError::Upstream("market_count is not a number".into()))?;
    Ok((0..count).rev().take(PAGE as usize).collect())
}

fn get_market(chain: &dyn Chain, contracts: &Contracts, id: u32) -> Result<RawMarket, ApiError> {
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
    parse(raw, "market")
}

/// The latest fixing of each index, read at most once per request.
#[derive(Default)]
struct Prices(std::collections::HashMap<String, Option<u128>>);

impl Prices {
    fn latest(&mut self, chain: &dyn Chain, contracts: &Contracts, index: &str) -> Option<u128> {
        *self.0.entry(index.to_string()).or_insert_with(|| {
            latest_fixing(chain, contracts, index)
                .ok()
                .map(|fixing| amount_of_decimal(&fixing.value))
        })
    }
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
    prices: &mut Prices,
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
        let latest = prices
            .latest(chain, contracts, &raw.index)
            .map(Outcome::Settled);
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
/// NEAR named accounts and 64-hex implicit accounts.
pub fn valid_account(account: &str) -> bool {
    (2..=64).contains(&account.len())
        && account.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        })
        && !account.starts_with(['.', '-', '_'])
        && !account.ends_with(['.', '-', '_'])
}
