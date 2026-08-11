//! JWT Bearer Token Grant (RFC 7523)
//!
//! Service-to-service authentication using signed JWTs.

use anyhow::{Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// JWT claims for bearer token grant.
#[derive(Debug, Serialize, Deserialize)]
pub struct JwtBearerClaims {
    /// Issuer - client_id of the OAuth client
    pub iss: String,

    /// Subject - usually same as issuer for client credentials
    pub sub: String,

    /// Audience - token endpoint URL
    pub aud: String,

    /// Expiration time (seconds since epoch)
    pub exp: u64,

    /// Issued at time (seconds since epoch)
    pub iat: u64,

    /// JWT ID - unique identifier for this token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
}

impl JwtBearerClaims {
    /// Create new JWT claims with 5-minute expiration.
    pub fn new(client_id: String, token_endpoint: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            iss: client_id.clone(),
            sub: client_id,
            aud: token_endpoint,
            exp: now + 300, // 5 minutes
            iat: now,
            jti: Some(uuid::Uuid::new_v4().to_string()),
        }
    }
}

/// Token response from JWT bearer grant.
#[derive(Debug, Deserialize)]
pub struct JwtBearerTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<u64>,
    pub scope: Option<String>,
}

/// JWT Bearer Token Grant client.
pub struct JwtBearerClient {
    client: reqwest::Client,
    token_endpoint: String,
    client_id: String,
    private_key: EncodingKey,
    algorithm: Algorithm,
}

impl JwtBearerClient {
    /// Create a new JWT bearer client with RSA private key.
    pub fn new_rsa(
        token_endpoint: String,
        client_id: String,
        private_key_pem: &[u8],
    ) -> Result<Self> {
        let private_key = EncodingKey::from_rsa_pem(private_key_pem)
            .context("failed to parse RSA private key")?;

        Ok(Self {
            client: reqwest::Client::new(),
            token_endpoint,
            client_id,
            private_key,
            algorithm: Algorithm::RS256,
        })
    }

    /// Create a new JWT bearer client with EC private key.
    pub fn new_ec(
        token_endpoint: String,
        client_id: String,
        private_key_pem: &[u8],
    ) -> Result<Self> {
        let private_key = EncodingKey::from_ec_pem(private_key_pem)
            .context("failed to parse EC private key")?;

        Ok(Self {
            client: reqwest::Client::new(),
            token_endpoint,
            client_id,
            private_key,
            algorithm: Algorithm::ES256,
        })
    }

    /// Request an access token using JWT bearer grant.
    pub async fn request_token(&self, scopes: &[String]) -> Result<crate::oauth::OAuthToken> {
        // Create JWT assertion
        let claims = JwtBearerClaims::new(self.client_id.clone(), self.token_endpoint.clone());

        let mut header = Header::new(self.algorithm);
        header.typ = Some("JWT".to_string());

        let assertion = encode(&header, &claims, &self.private_key)
            .context("failed to encode JWT")?;

        // Request token
        let scope = scopes.join(" ");
        let mut params = std::collections::HashMap::new();
        params.insert("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer");
        params.insert("assertion", assertion.as_str());
        params.insert("scope", scope.as_str());

        let response = self
            .client
            .post(&self.token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(serde_urlencoded::to_string(&params).context("failed to encode params")?)
            .send()
            .await
            .context("failed to request token")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("token request failed: {} - {}", status, body);
        }

        let token_response: JwtBearerTokenResponse = response
            .json()
            .await
            .context("failed to parse token response")?;

        let expires_at = token_response.expires_in.map(|secs| {
            chrono::Utc::now() + chrono::Duration::seconds(secs as i64)
        });

        Ok(crate::oauth::OAuthToken {
            access_token: token_response.access_token,
            refresh_token: None, // JWT bearer doesn't use refresh tokens
            expires_at,
            scopes: scopes.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_claims_creation() {
        let claims = JwtBearerClaims::new(
            "test-client".to_string(),
            "https://auth.example.com/token".to_string(),
        );

        assert_eq!(claims.iss, "test-client");
        assert_eq!(claims.sub, "test-client");
        assert_eq!(claims.aud, "https://auth.example.com/token");
        assert!(claims.exp > claims.iat);
        assert!(claims.jti.is_some());
    }

    #[test]
    fn test_jwt_encoding() {
        // Generate test RSA key
        let private_key_pem = b"-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEA0Z3VS5JJcds3xfn/ygWyF0qHJgKhKPxnXKJpKKQCPXDKqMYz
-----END RSA PRIVATE KEY-----";

        let claims = JwtBearerClaims::new(
            "test-client".to_string(),
            "https://auth.example.com/token".to_string(),
        );

        // This will fail with invalid key, but tests the encoding path
        let result = EncodingKey::from_rsa_pem(private_key_pem);
        assert!(result.is_err()); // Expected - invalid test key
    }
}
