//! Fetch-and-index: server-side URL fetching with SSRF guard.
//!
//! Instead of dumping raw HTML into the conversation, this tool:
//! 1. Validates the URL (http/only, no private IPs — SSRF guard)
//! 2. Fetches server-side via reqwest
//! 3. Extracts readable text (strips HTML tags, scripts, styles)
//! 4. Stores the text behind a result handle
//! 5. Indexes it into the session FTS5 store for later search
//!
//! The model gets a reference, never the raw HTML.

use anyhow::{Context as _, Result};
use rmcp::schemars;
use serde::Deserialize;
use std::sync::Arc;

use crate::registry::ToolRegistry;
use crate::result_store::ResultStore;
use crate::session::SessionEventStore;
use crate::session::overflow;

#[allow(dead_code)]
pub const MAX_FETCH_BYTES: usize = 2_000_000; // 2MB cap

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[allow(dead_code)]
pub struct FetchAndIndexParams {
    /// URL to fetch. Must be http or https. Private IPs are blocked.
    pub url: String,
    /// Optional source label for search indexing (e.g. "docs", "api", "blog").
    #[serde(default)]
    pub source: Option<String>,
}

/// SSRF guard: validates the URL is safe to fetch server-side.
#[allow(dead_code)]
fn validate_url(url_str: &str) -> Result<url::Url> {
    let url = url::Url::parse(url_str).map_err(|e| anyhow::anyhow!("Invalid URL: {e}"))?;

    if url.scheme() != "http" && url.scheme() != "https" {
        anyhow::bail!(
            "URL scheme '{}' is not allowed. Only http and https are permitted.",
            url.scheme()
        );
    }

    let host = url.host_str().unwrap_or("");

    // Block localhost and loopback
    if host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" || host == "::1" {
        anyhow::bail!("URL host '{host}' is blocked (loopback)");
    }

    // Block private IPv4 ranges by octets
    let octets: Vec<&str> = host.split('.').collect();
    if octets.len() == 4
        && let (Ok(a), Ok(b), Ok(_c), Ok(_d)) = (
            octets[0].parse::<u8>(),
            octets[1].parse::<u8>(),
            octets[2].parse::<u8>(),
            octets[3].parse::<u8>(),
        )
    {
        // 10.0.0.0/8
        if a == 10 {
            anyhow::bail!("URL host '{host}' is blocked (private network)");
        }
        // 172.16.0.0/12
        if a == 172 && (16..=31).contains(&b) {
            anyhow::bail!("URL host '{host}' is blocked (private network)");
        }
        // 192.168.0.0/16
        if a == 192 && b == 168 {
            anyhow::bail!("URL host '{host}' is blocked (private network)");
        }
        // 169.254.0.0/16 (link-local)
        if a == 169 && b == 254 {
            anyhow::bail!("URL host '{host}' is blocked (link-local)");
        }
        // 127.0.0.0/8 (loopback, defensive)
        if a == 127 {
            anyhow::bail!("URL host '{host}' is blocked (loopback)");
        }
    }

    // Block IPv6 private ranges (simplified)
    if host.starts_with("fd") || host.starts_with("fc") || host.starts_with("fe8") {
        anyhow::bail!("URL host '{host}' is blocked (private IPv6)");
    }

    Ok(url)
}

/// Extract readable text from HTML.
/// Strips script/style tags, removes HTML tags, collapses whitespace.
#[allow(dead_code)]
fn extract_text(html: &str) -> String {
    // Remove script tags and content
    let text = regex::Regex::new(r"<script[^>]*>.*?</script>")
        .unwrap()
        .replace_all(html, "");
    let text = regex::Regex::new(r"<style[^>]*>.*?</style>")
        .unwrap()
        .replace_all(&text, "");
    // Remove HTML comments
    let text = regex::Regex::new(r"<!--.*?-->")
        .unwrap()
        .replace_all(&text, "");
    // Remove all HTML tags
    let text = regex::Regex::new(r"<[^>]+>")
        .unwrap()
        .replace_all(&text, " ");
    // Replace entities (common ones)
    let text = text.replace("&nbsp;", " ");
    let text = text.replace("&lt;", "<");
    let text = text.replace("&gt;", ">");
    let text = text.replace("&amp;", "&");
    let text = text.replace("&quot;", "\"");
    // Collapse whitespace
    let text = regex::Regex::new(r"\s+").unwrap().replace_all(&text, " ");
    text.trim().to_string()
}

/// Handle fetch_and_index: fetch URL server-side, extract text, index, return reference.
#[allow(dead_code)]
pub async fn handle_fetch_and_index(
    params: &FetchAndIndexParams,
    _registry: &Arc<ToolRegistry>,
    result_store: &Arc<ResultStore>,
    session_store: Option<&SessionEventStore>,
    session_key: Option<&str>,
) -> Result<String> {
    let url = validate_url(&params.url)?;

    // Fetch server-side
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    let response = client
        .get(url.clone())
        .header("User-Agent", "PrismGate/1.0")
        .send()
        .await
        .context("Fetch failed")?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("Fetch returned status {status}");
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    if bytes.len() > MAX_FETCH_BYTES {
        anyhow::bail!(
            "Response too large ({} bytes, max {}). Use a more specific URL or search.",
            bytes.len(),
            MAX_FETCH_BYTES
        );
    }

    let raw = String::from_utf8_lossy(&bytes).to_string();

    // Extract readable text
    let text = if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        extract_text(&raw)
    } else {
        raw.trim().to_string()
    };

    if text.is_empty() {
        anyhow::bail!("No readable text found at {url}");
    }

    // Store behind handle
    let handle = result_store.insert(text.clone()).ok_or_else(|| {
        anyhow::anyhow!("Failed to retain fetch result: result store full or disabled")
    })?;

    // Index into session FTS5 if available
    if let (Some(store), Some(key)) = (session_store, session_key) {
        let handle_for_index = handle.clone();
        let source_for_index = params.source.clone().unwrap_or_else(|| "fetch".to_string());
        let text_for_index = text.clone();
        let _ = overflow::index_overflow(
            store,
            key,
            &handle_for_index,
            &text_for_index,
            &source_for_index,
            None,
        )
        .await;
    }

    // Build reference
    let source = params.source.clone().unwrap_or_else(|| "fetch".to_string());
    let result = serde_json::json!({
        "handle": handle,
        "url": params.url,
        "source": source,
        "content_type": content_type,
        "bytes": text.len(),
        "total_bytes": raw.len(),
        "lookup": "read_result(handle)",
        "search": "session_search(query, source=\"{source}\")",
        "note": "Raw HTML was never returned. Use read_result for content, or session_search to find relevant sections."
    });

    Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_url_blocks_private_ips() {
        assert!(validate_url("http://10.0.0.1/test").is_err());
        assert!(validate_url("http://192.168.1.1/test").is_err());
        assert!(validate_url("http://172.16.0.1/test").is_err());
        assert!(validate_url("http://169.254.1.1/test").is_err());
        assert!(validate_url("http://127.0.0.1/test").is_err());
        assert!(validate_url("http://localhost/test").is_err());
    }

    #[test]
    fn validate_url_allows_public() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("https://www.rust-lang.org").is_ok());
    }

    #[test]
    fn validate_url_blocks_non_http() {
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn extract_text_strips_html() {
        let html =
            r#"<html><body><script>alert(1)</script><h1>Hello</h1><p>World</p></body></html>"#;
        let text = extract_text(html);
        assert!(!text.contains("script"));
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }
}
