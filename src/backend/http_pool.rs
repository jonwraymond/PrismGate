//! Shared reqwest client builder with bounded connection pooling.
//!
//! HTTP backends still get their own `Client` when they carry custom headers
//! (Authorization rotation, API keys). The pool settings themselves are the
//! win: idle sockets are reused instead of re-handshaking TLS on every call.

use std::time::Duration;

use anyhow::{Context, Result};

const POOL_MAX_IDLE_PER_HOST: usize = 16;
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);

/// Build a pooled HTTP client, optionally with per-backend default headers.
pub fn pooled_client(default_headers: reqwest::header::HeaderMap) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .default_headers(default_headers)
        .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .tcp_keepalive(TCP_KEEPALIVE)
        .build()
        .context("failed to build pooled HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooled_client_builds() {
        assert!(pooled_client(reqwest::header::HeaderMap::new()).is_ok());
    }
}
