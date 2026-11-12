//! TEST ONLY. A stand-in for OutLayer inside a local NEAR sandbox.
//!
//! `request_execution` answers at once with the oracle program's output shape
//! (`{slot, v, n, inputs}`) for the hour the caller asked about, taking the
//! next value from a script. The real OutLayer runs the pinned program in a
//! TEE and answers later; the fixing-oracle cannot tell the difference, which
//! is the point: everything after the answer is the real contract.

use near_sdk::json_types::U64;
use near_sdk::serde_json::{self, json, Value};
use near_sdk::{env, near};

#[near(contract_state)]
#[derive(Default)]
pub struct MockOutlayer {
    /// H100 values in micro-USD, used in order and repeated.
    script: Vec<u64>,
    calls: u64,
}

#[near]
impl MockOutlayer {
    /// The values the next requests answer with.
    pub fn set_script(&mut self, values: Vec<U64>) {
        self.script = values.into_iter().map(|v| v.0).collect();
        self.calls = 0;
    }

    /// Same arguments as OutLayer's; only `input_data` is read.
    #[payable]
    pub fn request_execution(&mut self, input_data: String) -> Value {
        let input: Value = serde_json::from_str(&input_data)
            .unwrap_or_else(|_| env::panic_str("input_data is not JSON"));
        let slot = input["slot"]
            .as_u64()
            .unwrap_or_else(|| env::panic_str("input_data has no slot"));
        if self.script.is_empty() {
            env::panic_str("no script set");
        }
        let value = self.script[(self.calls % self.script.len() as u64) as usize];
        self.calls += 1;
        output(slot, value, self.calls)
    }

    pub fn calls(&self) -> U64 {
        U64(self.calls)
    }
}

/// The oracle program's output for one H100 value from four sources.
pub fn output(slot: u64, value: u64, run: u64) -> Value {
    json!({ "slot": slot, "v": { "H100": value }, "n": { "H100": 4 }, "inputs": format!("sha256:sandbox-run-{run}") })
}
