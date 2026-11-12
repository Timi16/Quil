//! Quill's keeper: decides what to poke each tick.
//!
//! The keeper has no privileges. `request_fixing` and `settle` are public
//! methods anyone can call; the keeper only makes sure someone does. If it
//! stops, the protocol waits; if two run, the contracts reject the duplicate.
//!
//! This module is the decision, as a pure function, so it is tested without a
//! chain. `main.rs` reads the chain, calls [`plan`], and signs what it returns.

use std::collections::BTreeMap;

/// Seconds in one hourly fixing slot.
pub const SLOT_SECS: u64 = 3600;
/// How long to wait before retrying `settle` on a market whose value was not ready.
pub const SETTLE_RETRY_SECS: u64 = 600;
/// How long to wait before re-requesting a fixing for the same hour after a failed attempt.
pub const FIXING_RETRY_SECS: u64 = 300;

/// What the keeper needs to know about one market.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketInfo {
    pub id: u32,
    pub expiry_slot: u64,
    /// Settled on a price or voided. Either way there is nothing left to do.
    pub finished: bool,
}

/// One transaction to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    RequestFixing { slot: u64 },
    Settle { market: u32 },
}

/// What the keeper remembers between ticks. Losing it is harmless: the
/// contracts reject duplicates, so a restart costs at most one wasted call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Memory {
    /// The slot last requested, and when.
    pub fixing_attempt: Option<(u64, u64)>,
    /// Market id to the time `settle` was last sent for it.
    pub settle_attempts: BTreeMap<u32, u64>,
}

/// What the keeper sees on-chain this tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Unix seconds now.
    pub now: u64,
    /// Whether the oracle already holds a fixing for the current hour.
    pub hour_is_fixed: bool,
    pub markets: Vec<MarketInfo>,
}

/// The transactions to send this tick, in order, and the memory to keep.
pub fn plan(seen: &Observation, memory: &Memory) -> (Vec<Action>, Memory) {
    let slot = seen.now / SLOT_SECS;
    let mut actions = Vec::new();
    let mut next = memory.clone();

    let retry_due = match memory.fixing_attempt {
        Some((last_slot, at)) if last_slot == slot => seen.now >= at + FIXING_RETRY_SECS,
        _ => true,
    };
    if !seen.hour_is_fixed && retry_due {
        actions.push(Action::RequestFixing { slot });
        next.fixing_attempt = Some((slot, seen.now));
    }

    for market in &seen.markets {
        if market.finished {
            next.settle_attempts.remove(&market.id);
            continue;
        }
        if slot < market.expiry_slot {
            continue;
        }
        let due = memory
            .settle_attempts
            .get(&market.id)
            .is_none_or(|at| seen.now >= at + SETTLE_RETRY_SECS);
        if due {
            actions.push(Action::Settle { market: market.id });
            next.settle_attempts.insert(market.id, seen.now);
        }
    }
    (actions, next)
}

/// One readable line for a failed transaction, from the client's debug output.
pub fn explain(error: &str) -> String {
    if let Some(start) = error.find("Smart contract panicked: ") {
        let rest = &error[start + "Smart contract panicked: ".len()..];
        return format!(
            "refused: {}",
            rest.split(['"', '\\']).next().unwrap_or(rest)
        );
    }
    if error.contains("AccountDoesNotExist") || error.contains("UnknownAccount") {
        return "the contract account does not exist yet".into();
    }
    if error.contains("CodeDoesNotExist") {
        return "the account has no contract deployed".into();
    }
    if error.contains("NotEnoughBalance") || error.contains("LackBalanceForState") {
        return "the keeper account is out of NEAR".into();
    }
    error.chars().take(160).collect()
}
