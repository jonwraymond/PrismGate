//! OAuth 2.0 Authorization Server Metadata discovery - RFC 8414.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// OAuth 2.0 Authorization Server Metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthMetadata {
    /// The authorization server's issuer identifier
    pub issuer: String,

    /// URL of the authorization endpoint
    pub authorization_endpoint: String,

    /// URL of the token endpoint
    pub token_endpoint: String,

    /// Supported response types
    #[serde(default)]
    pub response_types_supported: Vec<String>,

    /// Supported grant types
    #[serde(default)]
    pub grant_types_supported: Vec<String>,

    /// Supported PKCE code challenge methods
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,

    /// Supported scopes
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

/// Discover OAuth endpoints from a base URL.
///
/// Implements RFC 8414: OAuth 2.0 Authorization Server Metadata.
/// Fetches metadata from `{base_url}/.well-known/oauth-authorization-server`.
pub async fn discover_oauth_endpoints(base_url: &str) -> Result<OAuthMetadata> {
    let discovery_url = format!(
        "{}/.well-known/oauth-authorization-server",
        base_url.trim_end_matches('/')
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;

    let response = client
        .get(&discovery_url)
        .send()
        .await
        .context("failed to fetch OAuth metadata")?;

    if !response.status().is_success() {
        anyhow::bail!("OAuth discovery failed: HTTP {}", response.status());
    }

    let metadata: OAuthMetadata = response
        .json()
        .await
        .context("failed to parse OAuth metadata")?;

    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discovery_url_format() {
        // Test URL formatting
        let base_url = "https://example.com";
        let discovery_url = format!(
            "{}/.well-known/oauth-authorization-server",
            base_url.trim_end_matches('/')
        );
        assert_eq!(
            discovery_url,
            "https://example.com/.well-known/oauth-authorization-server"
        );

        // Test with trailing slash
        let base_url = "https://example.com/";
        let discovery_url = format!(
            "{}/.well-known/oauth-authorization-server",
            base_url.trim_end_matches('/')
        );
        assert_eq!(
            discovery_url,
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }
}
