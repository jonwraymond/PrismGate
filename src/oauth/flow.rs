//! OAuth 2.0 authorization code flow implementation.
//!
//! Implements OAuth 2.1 compliance:
//! - Mandatory PKCE for all flows
//! - Refresh token rotation

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{
    callback_server::CallbackServer, config::OAuthConfig, discovery::discover_oauth_endpoints,
    pkce::PkceChallenge, token_store::OAuthToken,
};

/// OAuth 2.0 authorization flow.
pub struct OAuthFlow {
    config: OAuthConfig,
    authorization_url: String,
    token_url: String,
    /// OAuth 2.1 mode: enforces PKCE and refresh token rotation
    oauth21_mode: bool,
}

impl OAuthFlow {
    /// Create a new OAuth flow.
    ///
    /// If `config.discover` is true, this will fetch OAuth metadata
    /// from the authorization server.
    pub async fn new(base_url: &str, config: &OAuthConfig) -> Result<Self> {
        config.validate()?;

        let (authorization_url, token_url) = if config.discover {
            let metadata = discover_oauth_endpoints(base_url).await?;
            (metadata.authorization_endpoint, metadata.token_endpoint)
        } else {
            (
                config.authorization_url.clone().unwrap(),
                config.token_url.clone().unwrap(),
            )
        };

        Ok(Self {
            config: config.clone(),
            authorization_url,
            token_url,
            oauth21_mode: true, // Default to OAuth 2.1 compliance
        })
    }

    /// Enable or disable OAuth 2.1 mode.
    pub fn with_oauth21_mode(mut self, enabled: bool) -> Self {
        self.oauth21_mode = enabled;
        self
    }

    /// Start the authorization flow.
    ///
    /// This will:
    /// 1. Generate PKCE challenge (if enabled)
    /// 2. Build authorization URL
    /// 3. Open browser for user consent
    /// 4. Start local callback server
    /// 5. Exchange authorization code for tokens
    pub async fn authorize(&self) -> Result<OAuthToken> {
        // Generate PKCE challenge
        // OAuth 2.1: PKCE is mandatory
        let pkce = if self.oauth21_mode {
            Some(PkceChallenge::new()?)
        } else {
            None
        };

        // Build authorization URL
        let auth_url = self.build_authorization_url(pkce.as_ref())?;

        // Start callback server
        let server = CallbackServer::new(self.config.callback_port);

        // Open browser.
        // Long authorize URLs (e.g. Workfront DCR client_ids) can break macOS `open`
        // when passed directly. Prefer a tiny local HTML redirect file.
        eprintln!("Opening browser for authorization...");
        eprintln!("URL: {}", auth_url);

        if let Err(e) = open_authorization_url(&auth_url) {
            eprintln!("Failed to open browser: {}", e);
            eprintln!("Please open this URL manually: {}", auth_url);
        }

        // Wait for callback
        let callback_result = server.wait_for_callback().await?;

        // Exchange code for token
        self.exchange_code(&callback_result.code, pkce.as_ref())
            .await
    }

    /// Refresh an access token using a refresh token.
    ///
    /// OAuth 2.1: Returns a new refresh token (rotation).
    pub async fn refresh(&self, refresh_token: &str) -> Result<OAuthToken> {
        let client = reqwest::Client::new();

        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token");
        params.insert("refresh_token", refresh_token);
        params.insert("client_id", &self.config.client_id);

        if let Some(ref secret) = self.config.client_secret {
            params.insert("client_secret", secret);
        }

        // RFC 6749 §4.1.3 / §6: token endpoint expects application/x-www-form-urlencoded
        let response = client
            .post(&self.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(serde_urlencoded::to_string(&params).context("failed to encode params")?)
            .send()
            .await
            .context("failed to refresh token")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("token refresh failed: HTTP {} - {}", status, body);
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("failed to parse token response")?;

        let mut token = self.token_response_to_oauth_token(token_response);

        // OAuth 2.1: If no new refresh token, keep the old one (but warn)
        if self.oauth21_mode && token.refresh_token.is_none() {
            eprintln!("Warning: OAuth 2.1 mode enabled but server didn't rotate refresh token");
            token.refresh_token = Some(refresh_token.to_string());
        }

        Ok(token)
    }

    /// Build the authorization URL.
    fn build_authorization_url(&self, pkce: Option<&PkceChallenge>) -> Result<String> {
        let mut url =
            url::Url::parse(&self.authorization_url).context("invalid authorization URL")?;

        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &self.config.redirect_uri);

        if !self.config.scopes.is_empty() {
            url.query_pairs_mut()
                .append_pair("scope", &self.config.scopes.join(" "));
        }

        if let Some(ref resource) = self.config.resource {
            url.query_pairs_mut().append_pair("resource", resource);
        }

        if let Some(pkce) = pkce {
            url.query_pairs_mut()
                .append_pair("code_challenge", &pkce.challenge)
                .append_pair("code_challenge_method", &pkce.method);
        }

        Ok(url.to_string())
    }

    /// Exchange authorization code for access token.
    async fn exchange_code(&self, code: &str, pkce: Option<&PkceChallenge>) -> Result<OAuthToken> {
        let client = reqwest::Client::new();

        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("code", code);
        params.insert("redirect_uri", &self.config.redirect_uri);
        params.insert("client_id", &self.config.client_id);

        if let Some(ref secret) = self.config.client_secret {
            params.insert("client_secret", secret);
        }

        if let Some(ref resource) = self.config.resource {
            params.insert("resource", resource);
        }

        if let Some(pkce) = pkce {
            params.insert("code_verifier", &pkce.verifier);
        }

        // RFC 6749 §4.1.3: token endpoint expects application/x-www-form-urlencoded
        let response = client
            .post(&self.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(serde_urlencoded::to_string(&params).context("failed to encode params")?)
            .send()
            .await
            .context("failed to exchange authorization code")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("token exchange failed: HTTP {} - {}", status, body);
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("failed to parse token response")?;

        Ok(self.token_response_to_oauth_token(token_response))
    }

    /// Convert a token response to an OAuthToken.
    fn token_response_to_oauth_token(&self, response: TokenResponse) -> OAuthToken {
        let expires_at = response
            .expires_in
            .map(|secs| chrono::Utc::now() + chrono::Duration::seconds(secs as i64));

        OAuthToken {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            expires_at,
            scopes: response
                .scope
                .map(|s| s.split_whitespace().map(String::from).collect())
                .unwrap_or_else(|| self.config.scopes.clone()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: Option<String>,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

/// Open an authorization URL in the system browser.
///
/// Always open via a temporary HTML redirect file. Passing long authorize
/// URLs directly to macOS `open` can mangle the query string (especially with
/// Workfront DCR client ids).
fn open_authorization_url(auth_url: &str) -> Result<()> {
    let escaped = auth_url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let js_url = serde_json::to_string(auth_url).context("failed to encode auth URL as JSON")?;
    let html = format!(
        r#"<!doctype html>
<html><head>
<meta charset="utf-8">
<meta http-equiv="refresh" content="0;url={url}">
<title>Gatemini OAuth</title>
</head><body>
<p>Redirecting to authorization server…</p>
<p><a href="{url}">Continue</a></p>
<script>window.location.replace({js_url});</script>
</body></html>"#,
        url = escaped,
        js_url = js_url,
    );

    let path = std::env::temp_dir().join(format!(
        "gatemini-oauth-{}-{}.html",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::write(&path, html).with_context(|| format!("failed to write {}", path.display()))?;
    open::that(&path).with_context(|| format!("failed to open {}", path.display()))?;
    Ok(())
}
