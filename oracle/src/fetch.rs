//! The real network, for running outside tests.
//!
//! Native builds use plain HTTPS. The enclave build (`wasm32-wasip2`) goes
//! through the WASI Preview 2 HTTP host interface, which is what OutLayer
//! provides. The enclave path compiles but has not yet run inside OutLayer.

use std::time::Duration;

/// Fetches over HTTPS with a timeout. A non-2xx status is an error.
pub struct Http;

const TIMEOUT: Duration = Duration::from_secs(20);
const USER_AGENT: &str = "quill-oracle/0.1";

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{Http, TIMEOUT, USER_AGENT};
    use crate::Fetch;

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout(TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
    }

    fn read(response: Result<ureq::Response, ureq::Error>) -> Result<String, String> {
        response
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())
    }

    impl Fetch for Http {
        fn get(&self, url: &str) -> Result<String, String> {
            read(agent().get(url).set("accept", "application/json").call())
        }

        fn post_json(&self, url: &str, body: &str) -> Result<String, String> {
            read(
                agent()
                    .post(url)
                    .set("content-type", "application/json")
                    .send_string(body),
            )
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod enclave {
    use wasi_http_client::{Client, Response};

    use super::{Http, TIMEOUT, USER_AGENT};
    use crate::Fetch;

    fn read(response: Response) -> Result<String, String> {
        let status = response.status();
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}"));
        }
        let bytes = response.body().map_err(|e| e.to_string())?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    }

    impl Fetch for Http {
        fn get(&self, url: &str) -> Result<String, String> {
            let response = Client::new()
                .get(url)
                .header("accept", "application/json")
                .header("user-agent", USER_AGENT)
                .connect_timeout(TIMEOUT)
                .send()
                .map_err(|e| e.to_string())?;
            read(response)
        }

        fn post_json(&self, url: &str, body: &str) -> Result<String, String> {
            let response = Client::new()
                .post(url)
                .header("content-type", "application/json")
                .header("user-agent", USER_AGENT)
                .body(body.as_bytes())
                .connect_timeout(TIMEOUT)
                .send()
                .map_err(|e| e.to_string())?;
            read(response)
        }
    }
}
