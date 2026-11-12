//! `fixing-oracle`: hourly prices from a pinned program running in OutLayer's TEE.
//!
//! Anyone may call `request_fixing` once an hour. The contract builds the
//! OutLayer request itself from a WASM URL and SHA-256 hash that are set once
//! at deployment, so no caller can run different code. The result comes back
//! in `on_fixing`, is checked, and is stored for good.
//!
//! The contract holds no user funds. It has no owner and nothing can be changed
//! after deployment: a new oracle version is a new deployment.
//!
//! Trust, plainly: the root of trust on-chain is the OutLayer contract (and
//! Intel TDX behind it). This contract does not verify a TDX quote itself.

use std::collections::BTreeMap;

use near_sdk::json_types::U128;
use near_sdk::serde_json::{self, json, Value};
use near_sdk::store::LookupMap;
use near_sdk::{env, near, require, AccountId, Gas, PanicOnDefault, Promise};
use quill_core::settlement::{self, Settlement, Streak, Verdict};
use quill_core::{median, slot_of};

/// Accepted fixings remembered per index for the plausibility check.
pub const HISTORY_LEN: usize = 6;
/// Longest `inputs` hash string stored.
pub const MAX_INPUTS_LEN: usize = 128;
/// Largest execution result read from OutLayer. Its own limit is about 900 bytes.
pub const MAX_RESULT_LEN: usize = 2048;

const GAS_OUTLAYER: Gas = Gas::from_tgas(150);
const GAS_CALLBACK: Gas = Gas::from_tgas(40);

/// An index this oracle publishes, and how many sources it needs.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexConfig {
    pub id: String,
    /// 3 for a GPU-hour index; 1 for a single first-party feed such as NEAR AI's price list.
    pub min_sources: u32,
}

/// The program OutLayer runs. Fixed at deployment.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeRef {
    pub wasm_url: String,
    /// SHA-256 of the WASM. OutLayer refuses to run anything else from the URL.
    pub wasm_hash: String,
    pub build_target: String,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
struct Fixing {
    value: u128,
    sources: u32,
    inputs: String,
    accepted: bool,
}

/// Per-index memory for the plausibility check.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, Default)]
struct Track {
    /// Most recent accepted values, oldest first, at most [`HISTORY_LEN`].
    recent: Vec<u128>,
    /// 0 none, 1 above, 2 below.
    streak_dir: u8,
    streak_len: u8,
    latest_accepted_slot: Option<u64>,
}

impl Track {
    fn streak(&self) -> Streak {
        match self.streak_dir {
            1 => Streak::Above(self.streak_len),
            2 => Streak::Below(self.streak_len),
            _ => Streak::None,
        }
    }

    fn set_streak(&mut self, streak: Streak) {
        (self.streak_dir, self.streak_len) = match streak {
            Streak::None => (0, 0),
            Streak::Above(n) => (1, n),
            Streak::Below(n) => (2, n),
        };
    }
}

/// A fixing as returned to callers.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixingView {
    pub index: String,
    pub slot: u64,
    /// Micro-USD per unit.
    pub value: U128,
    pub sources: u32,
    /// Hash of the readings the value was computed from.
    pub inputs: String,
    /// False if the value was flagged as implausible and left out of settlement.
    pub accepted: bool,
}

/// What a market settles on. Mirrors `quill_core::settlement::Settlement`.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettlementView {
    Waiting,
    Final(U128),
    Fallback(U128),
    Void,
}

impl From<Settlement> for SettlementView {
    fn from(s: Settlement) -> Self {
        match s {
            Settlement::Waiting => Self::Waiting,
            Settlement::Final(v) => Self::Final(U128(v)),
            Settlement::Fallback(v) => Self::Fallback(U128(v)),
            Settlement::Void => Self::Void,
        }
    }
}

/// What the enclave program prints: values in micro-USD, source counts, and the inputs hash.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
struct OracleOutput {
    slot: u64,
    v: BTreeMap<String, u64>,
    #[serde(default)]
    n: BTreeMap<String, u32>,
    #[serde(default)]
    inputs: String,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct FixingOracle {
    outlayer: AccountId,
    code: CodeRef,
    indexes: Vec<IndexConfig>,
    /// `"<index>:<slot>"` to the fixing for that hour.
    fixings: LookupMap<String, Fixing>,
    tracks: LookupMap<String, Track>,
    /// The hour a request is in flight for, if any.
    pending_slot: Option<u64>,
}

#[near]
impl FixingOracle {
    #[init]
    pub fn new(outlayer: AccountId, code: CodeRef, indexes: Vec<IndexConfig>) -> Self {
        require!(!indexes.is_empty(), "At least one index is required");
        require!(
            code.wasm_url.starts_with("https://"),
            "wasm_url must be https"
        );
        require!(
            code.wasm_hash.len() == 64 && code.wasm_hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "wasm_hash must be a 64-character hex SHA-256"
        );
        for (i, index) in indexes.iter().enumerate() {
            require!(
                !index.id.is_empty() && !index.id.contains(':'),
                "Index ids must be non-empty and contain no ':'"
            );
            require!(index.min_sources >= 1, "min_sources must be at least 1");
            require!(
                indexes[..i].iter().all(|other| other.id != index.id),
                "Duplicate index id"
            );
        }
        Self {
            outlayer,
            code,
            indexes,
            fixings: LookupMap::new(b"f".to_vec()),
            tracks: LookupMap::new(b"t".to_vec()),
            pending_slot: None,
        }
    }

    /// Ask OutLayer for this hour's fixing. Anyone may call; the attached NEAR
    /// pays the execution and what is left is refunded to the caller by OutLayer.
    #[payable]
    pub fn request_fixing(&mut self) -> Promise {
        let slot = slot_of(env::block_timestamp());
        require!(
            self.pending_slot != Some(slot),
            "A request for this hour is already in flight"
        );
        require!(!self.is_fixed(slot), "This hour already has a fixing");
        self.pending_slot = Some(slot);

        // The program needs the last accepted value of each index for its outlier filter.
        let previous: BTreeMap<&str, u64> = self
            .indexes
            .iter()
            .filter_map(|index| {
                let last = self.tracks.get(&index.id)?.recent.last().copied()?;
                Some((index.id.as_str(), u64::try_from(last).ok()?))
            })
            .collect();
        let ids: Vec<&str> = self.indexes.iter().map(|i| i.id.as_str()).collect();
        let input = json!({ "slot": slot, "indexes": ids, "previous": previous }).to_string();

        let args = json!({
            "source": { "WasmUrl": {
                "url": self.code.wasm_url,
                "hash": self.code.wasm_hash,
                "build_target": self.code.build_target,
            }},
            "resource_limits": { "max_instructions": 50_000_000_000u64, "max_memory_mb": 256, "max_execution_seconds": 120 },
            "input_data": input,
            "response_format": "Json",
            "payer_account_id": env::predecessor_account_id(),
        });
        emit("fixing_requested", json!({ "slot": slot }));

        Promise::new(self.outlayer.clone())
            .function_call(
                "request_execution".to_string(),
                args.to_string().into_bytes(),
                env::attached_deposit(),
                GAS_OUTLAYER,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_CALLBACK)
                    .on_fixing(slot),
            )
    }

    /// OutLayer's answer. Returns how many index values were stored.
    #[private]
    pub fn on_fixing(&mut self, slot: u64) -> u32 {
        // Bounded read: an oversized result is treated as a failure instead of burning all gas.
        let result = env::promise_result_checked(0, MAX_RESULT_LEN).map_err(|_| ());
        self.apply_result(slot, result)
    }

    // ----- views -----

    /// The settlement value for a market on `index` expiring at `expiry_slot`, as of now.
    pub fn settlement_value(&self, index: String, expiry_slot: u64) -> SettlementView {
        let now = slot_of(env::block_timestamp());
        settlement::settlement_value(expiry_slot, now, |slot| {
            self.fixings
                .get(&fixing_key(&index, slot))
                .filter(|f| f.accepted)
                .map(|f| f.value)
        })
        .into()
    }

    /// The most recent accepted fixing for `index`.
    pub fn latest(&self, index: String) -> Option<FixingView> {
        let slot = self.tracks.get(&index)?.latest_accepted_slot?;
        self.fixing_at(index, slot)
    }

    pub fn fixing_at(&self, index: String, slot: u64) -> Option<FixingView> {
        self.fixings
            .get(&fixing_key(&index, slot))
            .map(|f| FixingView {
                index,
                slot,
                value: U128(f.value),
                sources: f.sources,
                inputs: f.inputs.clone(),
                accepted: f.accepted,
            })
    }

    /// The program this oracle runs. It cannot be changed.
    pub fn code_ref(&self) -> CodeRef {
        self.code.clone()
    }

    pub fn indexes(&self) -> Vec<IndexConfig> {
        self.indexes.clone()
    }

    pub fn outlayer(&self) -> AccountId {
        self.outlayer.clone()
    }
}

// ----- internals -----

impl FixingOracle {
    fn is_fixed(&self, slot: u64) -> bool {
        self.indexes
            .iter()
            .any(|index| self.fixings.contains_key(&fixing_key(&index.id, slot)))
    }

    /// Validate and store one execution result. Never panics on bad input:
    /// a bad result is logged and the hour can be requested again.
    fn apply_result(&mut self, slot: u64, result: Result<Vec<u8>, ()>) -> u32 {
        if self.pending_slot == Some(slot) {
            self.pending_slot = None;
        }
        let Ok(bytes) = result else {
            emit(
                "fixing_failed",
                json!({ "slot": slot, "reason": "execution failed" }),
            );
            return 0;
        };
        let Some(output) = parse_output(&bytes) else {
            emit(
                "fixing_failed",
                json!({ "slot": slot, "reason": "unreadable output" }),
            );
            return 0;
        };
        if output.slot != slot {
            emit(
                "fixing_failed",
                json!({ "slot": slot, "reason": "output is for a different hour" }),
            );
            return 0;
        }
        if output.inputs.len() > MAX_INPUTS_LEN {
            emit(
                "fixing_failed",
                json!({ "slot": slot, "reason": "inputs hash too long" }),
            );
            return 0;
        }

        let mut stored = 0;
        for index in self.indexes.clone() {
            let Some(&value) = output.v.get(&index.id) else {
                continue;
            };
            let sources = output.n.get(&index.id).copied().unwrap_or(0);
            let key = fixing_key(&index.id, slot);
            if value == 0 || sources < index.min_sources || self.fixings.contains_key(&key) {
                emit(
                    "fixing_rejected",
                    json!({ "index": index.id, "slot": slot, "value": U128(value.into()), "sources": sources }),
                );
                continue;
            }
            let value = u128::from(value);
            let mut track = self.tracks.get(&index.id).cloned().unwrap_or_default();
            let reference = median(&mut track.recent.clone());
            let (verdict, streak) = settlement::judge(value, reference, track.streak());
            track.set_streak(streak);

            let accepted = verdict == Verdict::Accept;
            if accepted {
                track.recent.push(value);
                if track.recent.len() > HISTORY_LEN {
                    track.recent.remove(0);
                }
                if track
                    .latest_accepted_slot
                    .is_none_or(|latest| slot > latest)
                {
                    track.latest_accepted_slot = Some(slot);
                }
            }
            self.tracks.insert(index.id.clone(), track);
            self.fixings.insert(
                key,
                Fixing {
                    value,
                    sources,
                    inputs: output.inputs.clone(),
                    accepted,
                },
            );
            emit(
                if accepted {
                    "fixing_stored"
                } else {
                    "fixing_flagged"
                },
                json!({ "index": index.id, "slot": slot, "value": U128(value), "sources": sources, "inputs": output.inputs }),
            );
            stored += 1;
        }
        stored
    }
}

fn fixing_key(index: &str, slot: u64) -> String {
    format!("{index}:{slot}")
}

/// OutLayer returns the program's stdout as JSON. Tolerate it arriving as a
/// JSON string containing JSON, or wrapped in `null`-able form.
fn parse_output(bytes: &[u8]) -> Option<OracleOutput> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let value = match value {
        Value::String(inner) => serde_json::from_str(&inner).ok()?,
        other => other,
    };
    serde_json::from_value(value).ok()
}

/// NEP-297 event.
fn emit(event: &str, data: Value) {
    env::log_str(&format!(
        "EVENT_JSON:{}",
        json!({ "standard": "quill", "version": "1.0.0", "event": event, "data": [data] })
    ));
}
