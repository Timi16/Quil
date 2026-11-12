//! One small parser per price source.
//!
//! Each takes the raw response body and returns prices in micro-USD. Nothing
//! here does I/O, so every parser is tested against a saved real response.
//! A source that changes shape yields an error or no prices; it never panics.

use serde_json::Value;

/// How each source names one GPU model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gpu {
    /// Quill's index id.
    pub index: &'static str,
    pub vast_name: &'static str,
    pub akash_model: &'static str,
    pub runpod_id: &'static str,
    pub shadeform_type: &'static str,
}

pub const H100: Gpu = Gpu {
    index: "H100",
    vast_name: "H100 SXM",
    akash_model: "h100",
    runpod_id: "NVIDIA H100 80GB HBM3",
    shadeform_type: "H100",
};

pub const B200: Gpu = Gpu {
    index: "B200",
    vast_name: "B200",
    akash_model: "b200",
    runpod_id: "NVIDIA B200",
    shadeform_type: "B200",
};

/// The GPU behind an index id, if it is a GPU index.
pub fn gpu_for(index: &str) -> Option<&'static Gpu> {
    [&H100, &B200].into_iter().find(|gpu| gpu.index == index)
}

pub const AKASH_URL: &str = "https://console-api.akash.network/v1/gpu-prices";
pub const RUNPOD_URL: &str = "https://api.runpod.io/graphql";
pub const RUNPOD_QUERY: &str =
    r#"{"query":"query { gpuTypes { id displayName securePrice communityPrice } }"}"#;
pub const SHADEFORM_URL: &str = "https://api.shadeform.ai/v1/instances/types";
pub const NEAR_AI_URL: &str = "https://cloud-api.near.ai/v1/model/list";

/// Highest price accepted from any source, in USD. Anything above is a typo or a unit mix-up.
const MAX_USD: f64 = 1_000_000.0;

/// Dollars to micro-dollars. `None` for anything that is not a sane positive price.
pub fn usd_to_micro(usd: f64) -> Option<u128> {
    if !usd.is_finite() || usd <= 0.0 || usd > MAX_USD {
        return None;
    }
    Some((usd * 1e6).round() as u128)
}

fn parse(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))
}

fn array<'a>(root: &'a Value, path: &[&str]) -> Result<&'a Vec<Value>, String> {
    let mut node = root;
    for key in path {
        node = node.get(key).ok_or_else(|| format!("missing `{key}`"))?;
    }
    node.as_array()
        .ok_or_else(|| format!("`{}` is not a list", path.join(".")))
}

/// Vast.ai's search URL for on-demand, rentable offers of one GPU model.
pub fn vast_url(gpu: &Gpu) -> String {
    let query = format!(
        r#"{{"gpu_name":{{"eq":"{}"}},"rentable":{{"eq":true}},"type":"on-demand"}}"#,
        gpu.vast_name
    );
    format!(
        "https://console.vast.ai/api/v0/bundles/?q={}",
        percent_encode(&query)
    )
}

/// Vast.ai: one price per offer, divided by its GPU count.
pub fn parse_vast(body: &str, gpu: &Gpu) -> Result<Vec<u128>, String> {
    let root = parse(body)?;
    Ok(array(&root, &["offers"])?
        .iter()
        .filter(|offer| {
            offer["gpu_name"] == gpu.vast_name && offer["rentable"] != Value::Bool(false)
        })
        .filter_map(|offer| {
            let gpus = offer["num_gpus"].as_u64().filter(|n| *n > 0)?;
            usd_to_micro(offer["dph_total"].as_f64()? / gpus as f64)
        })
        .collect())
}

/// Akash publishes one median per model. Returns it with the number of GPUs behind it.
///
/// If a model appears more than once (different memory or interface), the
/// entry with the most GPUs is used.
pub fn parse_akash(body: &str, gpu: &Gpu) -> Result<Option<(u128, usize)>, String> {
    let root = parse(body)?;
    Ok(array(&root, &["models"])?
        .iter()
        .filter(|m| {
            m["model"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(gpu.akash_model))
        })
        .filter_map(|m| {
            let median = usd_to_micro(m["price"]["med"].as_f64()?)?;
            let total = m["availability"]["total"].as_u64().unwrap_or(0) as usize;
            Some((median, total))
        })
        .max_by_key(|(_, total)| *total))
}

/// RunPod: its two list prices for the model (secure and community cloud).
pub fn parse_runpod(body: &str, gpu: &Gpu) -> Result<Vec<u128>, String> {
    let root = parse(body)?;
    Ok(array(&root, &["data", "gpuTypes"])?
        .iter()
        .filter(|g| g["id"] == gpu.runpod_id)
        .flat_map(|g| [g["securePrice"].as_f64(), g["communityPrice"].as_f64()])
        .filter_map(|price| usd_to_micro(price?))
        .collect())
}

/// Shadeform: one price per instance type across its clouds. `hourly_price` is in cents.
pub fn parse_shadeform(body: &str, gpu: &Gpu) -> Result<Vec<u128>, String> {
    let root = parse(body)?;
    Ok(array(&root, &["instance_types"])?
        .iter()
        .filter(|t| t["gpu_type"] == gpu.shadeform_type)
        .filter_map(|t| {
            let gpus = u128::from(t["num_gpus"].as_u64().filter(|n| *n > 0)?);
            let cents = u128::from(t["hourly_price"].as_u64().filter(|c| *c > 0)?);
            Some(cents * 10_000 / gpus)
        })
        .collect())
}

/// NEAR AI Cloud: the output price of `model`, in micro-USD per one million tokens.
///
/// Prices arrive as `{ amount, scale, currency }` meaning `amount / 10^scale` dollars per token.
pub fn parse_near_ai(body: &str, model: &str) -> Result<u128, String> {
    let root = parse(body)?;
    let entry = array(&root, &["models"])?
        .iter()
        .find(|m| m["modelId"] == model)
        .ok_or_else(|| format!("model `{model}` is not listed"))?;
    let price = &entry["outputCostPerToken"];
    if price["currency"] != "USD" {
        return Err("price is not in USD".into());
    }
    let amount = u128::from(price["amount"].as_u64().ok_or("missing amount")?);
    let scale = u32::try_from(price["scale"].as_u64().ok_or("missing scale")?)
        .map_err(|_| "scale too large")?;
    // micro-USD per 1M tokens = amount / 10^scale * 10^6 * 10^6
    let value = if scale <= 12 {
        amount
            .checked_mul(10u128.pow(12 - scale))
            .ok_or("price overflows")?
    } else {
        amount / 10u128.checked_pow(scale - 12).ok_or("scale too large")?
    };
    if value == 0 {
        return Err("price is zero".into());
    }
    Ok(value)
}

/// Percent-encode everything except unreserved characters.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
