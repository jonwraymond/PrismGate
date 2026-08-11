//! Secure token storage and retrieval.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// An OAuth access token with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    /// The access token
    pub access_token: String,

    /// Optional refresh token
    pub refresh_token: Option<String>,

    /// Token type (usually "Bearer")
    #[serde(default = "default_token_type")]
    pub token_type: String,

    /// Unix timestamp when the token expires
    pub expires_at: u64,

    /// Scopes granted with this token
    #[serde(default)]
    pub scopes: Vec<String>,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

impl OAuthToken {
    /// Check if the token is expired (with 60s buffer).
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Consider expired if within 60 seconds of expiry
        now >= self.expires_at.saturating_sub(60)
    }

    /// Get seconds until expiry.
    pub fn seconds_until_expiry(&self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.expires_at as i64 - now as i64
    }
}

/// Persistent token storage.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TokenCache {
    tokens: HashMap<String, OAuthToken>,
}

/// Token store for OAuth tokens.
pub struct TokenStore {
    cache_path: PathBuf,
}

impl TokenStore {
    /// Create a new token store using the default cache location.
    pub fn new() -> Result<Self> {
        let cache_path = crate::cli::prismgate_cache_home().join("oauth_tokens.json");

        // Ensure parent directory exists
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).context("failed to create oauth cache directory")?;
        }

        Ok(Self { cache_path })
    }

    /// Load the token cache from disk.
    fn load_cache(&self) -> Result<TokenCache> {
        if !self.cache_path.exists() {
            return Ok(TokenCache::default());
        }

        let contents =
            fs::read_to_string(&self.cache_path).context("failed to read token cache")?;

        let cache: TokenCache =
            serde_json::from_str(&contents).context("failed to parse token cache")?;

        Ok(cache)
    }

    /// Save the token cache to disk.
    fn save_cache(&self, cache: &TokenCache) -> Result<()> {
        let contents =
            serde_json::to_string_pretty(cache).context("failed to serialize token cache")?;

        fs::write(&self.cache_path, contents).context("failed to write token cache")?;

        // Set restrictive permissions (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&self.cache_path)?.permissions();
            perms.set_mode(0o600); // rw-------
            fs::set_permissions(&self.cache_path, perms)?;
        }

        Ok(())
    }

    /// Get a token for a backend.
    pub fn get(&self, backend_name: &str) -> Result<Option<OAuthToken>> {
        let cache = self.load_cache()?;
        Ok(cache.tokens.get(backend_name).cloned())
    }

    /// Store a token for a backend.
    pub fn store(&self, backend_name: &str, token: &OAuthToken) -> Result<()> {
        let mut cache = self.load_cache()?;
        cache.tokens.insert(backend_name.to_string(), token.clone());
        self.save_cache(&cache)?;
        Ok(())
    }

    /// Remove a token for a backend.
    pub fn remove(&self, backend_name: &str) -> Result<()> {
        let mut cache = self.load_cache()?;
        cache.tokens.remove(backend_name);
        self.save_cache(&cache)?;
        Ok(())
    }

    /// List all stored backend names.
    pub fn list_backends(&self) -> Result<Vec<String>> {
        let cache = self.load_cache()?;
        Ok(cache.tokens.keys().cloned().collect())
    }
}
