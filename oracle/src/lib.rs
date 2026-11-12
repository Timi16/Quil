//! `quill-oracle`: the program Quill runs inside OutLayer's TEE.
//!
//! It reads a small JSON request on stdin, fetches public compute prices,
//! reduces them to one value per index and prints a JSON result that
//! `fixing-oracle` validates on-chain. The result stays far below OutLayer's
//! 900-byte output limit.
//!
//! All arithmetic is in `quill-core`; all response parsing is in [`sources`].
//! This file only wires them together, so the same code runs in the enclave,
//! on a laptop and in tests.

pub mod fetch;
pub mod sources;

use std::collections::BTreeMap;

use quill_core::index::{index_from_medians, plausible, provider_median, ProviderMedian};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use sources::Gpu;

/// Index id of the inference-price market.
pub const INFERENCE_INDEX: &str = "INF_OUT";
/// The NEAR AI Cloud model whose output price the inference index tracks.
/// Changing it changes the program hash, so it is as pinned as the code.
pub const INFERENCE_MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";

/// Providers needed for a GPU index.
pub const GPU_MIN_PROVIDERS: usize = 3;
/// Offers needed from a marketplace that returns raw listings.
pub const MIN_LISTINGS: usize = 5;

/// What `fixing-oracle` sends.
#[derive(Debug, Clone, Deserialize)]
pub struct Input {
    /// The hourly slot this fixing is for. Echoed back so the contract can match it.
    pub slot: u64,
    /// Index ids to compute.
    #[serde(default)]
    pub indexes: Vec<String>,
    /// Last accepted value per index (micro-USD), for the outlier filter.
    #[serde(default)]
    pub previous: BTreeMap<String, u64>,
}

/// What `fixing-oracle` receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Output {
    pub slot: u64,
    /// Index id to value, micro-USD.
    pub v: BTreeMap<String, u64>,
    /// Index id to the number of sources behind the value.
    pub n: BTreeMap<String, u32>,
    /// `sha256:` of the evidence lines, so anyone holding them can check the value.
    pub inputs: String,
}

/// How the program reaches the outside world.
pub trait Fetch {
    fn get(&self, url: &str) -> Result<String, String>;
    fn post_json(&self, url: &str, body: &str) -> Result<String, String>;
}

/// A finished run: the output plus the evidence it was computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub output: Output,
    /// One line per source that counted: `index|provider|median|listings`, sorted.
    pub evidence: Vec<String>,
    /// Sources that failed or were unusable, for logs. Never part of the hash.
    pub problems: Vec<String>,
}

/// Compute every requested index. Unknown ids and indexes without enough sources are left out.
pub fn run(input: &Input, fetch: &dyn Fetch) -> Run {
    let mut v = BTreeMap::new();
    let mut n = BTreeMap::new();
    let mut evidence = Vec::new();
    let mut problems = Vec::new();

    let mut ids: Vec<&str> = input.indexes.iter().map(String::as_str).collect();
    ids.sort_unstable();
    ids.dedup();

    for id in ids {
        let previous = input.previous.get(id).map(|p| u128::from(*p));
        let medians = if let Some(gpu) = sources::gpu_for(id) {
            let medians = gpu_medians(gpu, previous, fetch, &mut problems);
            match index_from_medians(medians, GPU_MIN_PROVIDERS) {
                Ok(result) => Some((result.value, result.providers)),
                Err(e) => {
                    problems.push(format!("{id}: {e:?}"));
                    None
                }
            }
        } else if id == INFERENCE_INDEX {
            match fetch
                .get(sources::NEAR_AI_URL)
                .and_then(|body| sources::parse_near_ai(&body, INFERENCE_MODEL))
            {
                Ok(value) => Some((
                    value,
                    vec![ProviderMedian {
                        provider: "near-ai".into(),
                        median: value,
                        listings: 1,
                    }],
                )),
                Err(e) => {
                    problems.push(format!("{id}: near-ai: {e}"));
                    None
                }
            }
        } else {
            problems.push(format!("{id}: unknown index"));
            None
        };

        let Some((value, providers)) = medians else {
            continue;
        };
        let Ok(value) = u64::try_from(value) else {
            problems.push(format!("{id}: value does not fit"));
            continue;
        };
        for p in &providers {
            evidence.push(format!("{id}|{}|{}|{}", p.provider, p.median, p.listings));
        }
        v.insert(id.to_string(), value);
        n.insert(id.to_string(), providers.len() as u32);
    }

    evidence.sort();
    let inputs = format!(
        "sha256:{}",
        hex(&Sha256::digest(evidence.join("\n").as_bytes()))
    );
    Run {
        output: Output {
            slot: input.slot,
            v,
            n,
            inputs,
        },
        evidence,
        problems,
    }
}

/// One median per provider that answered usefully.
fn gpu_medians(
    gpu: &Gpu,
    previous: Option<u128>,
    fetch: &dyn Fetch,
    problems: &mut Vec<String>,
) -> Vec<ProviderMedian> {
    let mut medians = Vec::new();
    let mut note =
        |provider: &str, why: String| problems.push(format!("{}: {provider}: {why}", gpu.index));

    // Marketplaces that return raw listings: we take the median ourselves.
    let listings = [
        (
            "vast",
            fetch
                .get(&sources::vast_url(gpu))
                .and_then(|b| sources::parse_vast(&b, gpu)),
            MIN_LISTINGS,
        ),
        (
            "shadeform",
            fetch
                .get(sources::SHADEFORM_URL)
                .and_then(|b| sources::parse_shadeform(&b, gpu)),
            MIN_LISTINGS,
        ),
        // RunPod publishes two list prices (secure and community); one is enough.
        (
            "runpod",
            fetch
                .post_json(sources::RUNPOD_URL, sources::RUNPOD_QUERY)
                .and_then(|b| sources::parse_runpod(&b, gpu)),
            1,
        ),
    ];
    for (provider, prices, min) in listings {
        match prices {
            Ok(prices) => match provider_median(provider, &prices, previous, min) {
                Some(m) => medians.push(m),
                None => note(provider, format!("only {} usable prices", prices.len())),
            },
            Err(e) => note(provider, e),
        }
    }

    // Akash already publishes a median per model.
    match fetch
        .get(sources::AKASH_URL)
        .and_then(|b| sources::parse_akash(&b, gpu))
    {
        Ok(Some((median, total))) if plausible(median, previous) => medians.push(ProviderMedian {
            provider: "akash".into(),
            median,
            listings: total.max(1),
        }),
        Ok(Some(_)) => note("akash", "median is far from the previous fixing".into()),
        Ok(None) => note("akash", "model not listed".into()),
        Err(e) => note("akash", e),
    }
    medians
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
