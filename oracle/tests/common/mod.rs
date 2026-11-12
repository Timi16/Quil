//! Shared test helpers: saved real responses served in place of the network.

// Each test file uses a different subset of these.
#![allow(dead_code)]

use quill_oracle::Fetch;

pub fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Answers each source from its fixture. Sources named in `down` fail instead.
#[derive(Default)]
pub struct Fixtures {
    pub down: Vec<&'static str>,
    /// Replace one source's body (keyed like `down`).
    pub overrides: Vec<(&'static str, String)>,
}

impl Fixtures {
    fn serve(&self, url: &str) -> Result<String, String> {
        let (source, file) = if url.contains("vast.ai") {
            ("vast", "vast_h100.json")
        } else if url.contains("akash") {
            ("akash", "akash_gpu_prices.json")
        } else if url.contains("shadeform") {
            ("shadeform", "shadeform_instance_types.json")
        } else if url.contains("runpod") {
            ("runpod", "runpod_gpu_types.json")
        } else if url.contains("near.ai") {
            ("near-ai", "nearai_model_list.json")
        } else {
            return Err(format!("unexpected url {url}"));
        };
        if self.down.contains(&source) {
            return Err(format!("{source} timed out"));
        }
        if let Some((_, body)) = self.overrides.iter().find(|(s, _)| *s == source) {
            return Ok(body.clone());
        }
        Ok(fixture(file))
    }
}

impl Fetch for Fixtures {
    fn get(&self, url: &str) -> Result<String, String> {
        self.serve(url)
    }
    fn post_json(&self, url: &str, _body: &str) -> Result<String, String> {
        self.serve(url)
    }
}
