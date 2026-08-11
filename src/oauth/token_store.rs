//! Secure token storage and retrieval.
//!
//! Tokens are encrypted at rest using AES-256-GCM with keys stored in the system keyring.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use super::encryption::EncryptionKeyManager;

/// An OAuth access token with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    /// The access token
    pub access_token: String,

    /// Optional refresh token
    pub refresh_token: Option<String>,

    /// When the token expires
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,

    /// Scopes granted with this token
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl OAuthToken {
    /// Check if the token is expired (with 60s buffer).
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = chrono::Utc::now();
            let buffer = chrono::Duration::seconds(60);
            now >= expires_at - buffer
        } else {
            false // No expiry means never expired
        }
    }

    /// Check if this token has a refresh token.
    pub fn has_refresh_token(&self) -> bool {
        self.refresh_token.is_some()
    }

    /// Get seconds until expiry.
    pub fn seconds_until_expiry(&self) -> i64 {
        if let Some(expires_at) = self.expires_at {
            let now = chrono::Utc::now();
            (expires_at - now).num_seconds()
        } else {
            i64::MAX // No expiry
        }
    }
}

/// Persistent token storage.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TokenCache {
    /// Encrypted tokens (base64-encoded ciphertext)
    tokens: HashMap<String, String>,
}

/// Token store for OAuth tokens.
pub struct TokenStore {
    cache_path: PathBuf,
    encryption: EncryptionKeyManager,
}

impl TokenStore {
    /// Create a new token store using the default cache location.
    pub fn new() -> Result<Self> {
        let cache_path = crate::cli::prismgate_cache_home().join("oauth_tokens.json");

        // Ensure parent directory exists
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).context("failed to create oauth cache directory")?;
        }

        let encryption = EncryptionKeyManager::new()?;

        Ok(Self {
            cache_path,
            encryption,
        })
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
        
        if let Some(encrypted_b64) = cache.tokens.get(backend_name) {
            let encrypted = BASE64
                .decode(encrypted_b64)
                .context("failed to decode encrypted token")?;
            
            let decrypted = self.encryption.decrypt(&encrypted)?;
            let token: OAuthToken = serde_json::from_slice(&decrypted)
                .context("failed to deserialize token")?;
            
            Ok(Some(token))
        } else {
            Ok(None)
        }
    }

    /// Store a token for a backend.
    pub fn store(&self, backend_name: &str, token: &OAuthToken) -> Result<()> {
        let mut cache = self.load_cache()?;
        
        let serialized = serde_json::to_vec(token)
            .context("failed to serialize token")?;
        
        let encrypted = self.encryption.encrypt(&serialized)?;
        let encrypted_b64 = BASE64.encode(&encrypted);
        
        cache.tokens.insert(backend_name.to_string(), encrypted_b64);
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
