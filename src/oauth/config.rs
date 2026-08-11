//! OAuth configuration types.

use serde::{Deserialize, Serialize};

/// OAuth 2.0 configuration for a backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuthConfig {
    /// Auto-discover OAuth endpoints from /.well-known/oauth-authorization-server
    #[serde(default)]
    pub discover: bool,

    /// Manual authorization endpoint URL (used if discover=false)
    pub authorization_url: Option<String>,

    /// Manual token endpoint URL (used if discover=false)
    pub token_url: Option<String>,

    /// OAuth client ID
    pub client_id: String,

    /// OAuth client secret (optional for PKCE-only flows)
    pub client_secret: Option<String>,

    /// OAuth scopes to request
    #[serde(default)]
    pub scopes: Vec<String>,

    /// Use PKCE (Proof Key for Code Exchange) - RFC 7636
    #[serde(default = "default_true")]
    pub use_pkce: bool,

    /// Redirect URI (defaults to http://localhost:8080/callback)
    #[serde(default = "default_redirect_uri")]
    pub redirect_uri: String,

    /// Local callback server port (defaults to 8080)
    #[serde(default = "default_callback_port")]
    pub callback_port: u16,
}

fn default_true() -> bool {
    true
}

fn default_redirect_uri() -> String {
    "http://localhost:8080/callback".to_string()
}

fn default_callback_port() -> u16 {
    8080
}

impl OAuthConfig {
    /// Validate the OAuth configuration.
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.discover {
            if self.authorization_url.is_none() {
                anyhow::bail!("authorization_url required when discover=false");
            }
            if self.token_url.is_none() {
                anyhow::bail!("token_url required when discover=false");
            }
        }

        if self.client_id.is_empty() {
            anyhow::bail!("client_id is required");
        }

        Ok(())
    }
}
