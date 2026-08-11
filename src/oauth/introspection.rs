//! OAuth 2.0 Token Introspection (RFC 7662)
//!
//! Allows checking if a token is active and retrieving metadata.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Token introspection response.
#[derive(Debug, Deserialize, Serialize)]
pub struct IntrospectionResponse {
    /// REQUIRED. Boolean indicator of whether or not the presented token is currently active.
    pub active: bool,

    /// OPTIONAL. The scope associated with the token.
    pub scope: Option<String>,

    /// OPTIONAL. Client identifier for the OAuth 2.0 client that requested this token.
    pub client_id: Option<String>,

    /// OPTIONAL. Human-readable identifier for the resource owner who authorized this token.
    pub username: Option<String>,

    /// OPTIONAL. Type of the token (e.g., "Bearer").
    pub token_type: Option<String>,

    /// OPTIONAL. Timestamp indicating when the token will expire (seconds since epoch).
    pub exp: Option<u64>,

    /// OPTIONAL. Timestamp indicating when the token was issued (seconds since epoch).
    pub iat: Option<u64>,

    /// OPTIONAL. Timestamp indicating when the token is not to be used before (seconds since epoch).
    pub nbf: Option<u64>,

    /// OPTIONAL. Subject of the token (usually a machine-readable identifier).
    pub sub: Option<String>,

    /// OPTIONAL. Intended audience for the token.
    pub aud: Option<String>,

    /// OPTIONAL. Issuer of the token.
    pub iss: Option<String>,

    /// OPTIONAL. Token identifier.
    pub jti: Option<String>,
}

/// OAuth 2.0 Token Introspection client.
pub struct IntrospectionClient {
    client: reqwest::Client,
    introspection_endpoint: String,
    client_id: String,
    client_secret: Option<String>,
}

impl IntrospectionClient {
    /// Create a new introspection client.
    pub fn new(
        introspection_endpoint: String,
        client_id: String,
        client_secret: Option<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            introspection_endpoint,
            client_id,
            client_secret,
        }
    }

    /// Introspect a token to check if it's active and get metadata.
    pub async fn introspect(&self, token: &str) -> Result<IntrospectionResponse> {
        let mut params = std::collections::HashMap::new();
        params.insert("token", token);
        params.insert("client_id", &self.client_id);

        if let Some(secret) = &self.client_secret {
            params.insert("client_secret", secret);
        }

        let response = self
            .client
            .post(&self.introspection_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(serde_urlencoded::to_string(&params).context("failed to encode params")?)
            .send()
            .await
            .context("failed to introspect token")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("token introspection failed: {} - {}", status, body);
        }

        let introspection_response: IntrospectionResponse = response
            .json()
            .await
            .context("failed to parse introspection response")?;

        Ok(introspection_response)
    }

    /// Check if a token is active.
    pub async fn is_active(&self, token: &str) -> Result<bool> {
        let response = self.introspect(token).await?;
        Ok(response.active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_introspection_response_deserialize() {
        let json = r#"{
            "active": true,
            "scope": "read write",
            "client_id": "test-client",
            "username": "user@example.com",
            "token_type": "Bearer",
            "exp": 1735689600,
            "iat": 1735603200,
            "sub": "user-123",
            "aud": "api.example.com",
            "iss": "https://auth.example.com"
        }"#;

        let response: IntrospectionResponse = serde_json::from_str(json).unwrap();
        assert!(response.active);
        assert_eq!(response.scope, Some("read write".to_string()));
        assert_eq!(response.client_id, Some("test-client".to_string()));
    }

    #[test]
    fn test_inactive_token_response() {
        let json = r#"{"active": false}"#;
        let response: IntrospectionResponse = serde_json::from_str(json).unwrap();
        assert!(!response.active);
    }
}
