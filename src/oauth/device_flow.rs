//! OAuth 2.0 Device Authorization Grant (RFC 8628)
//!
//! For headless/browserless environments like SSH sessions, CI/CD, or servers.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Device authorization response from the authorization server.
#[derive(Debug, Deserialize)]
pub struct DeviceAuthorizationResponse {
    /// The device verification code.
    pub device_code: String,

    /// The end-user verification code.
    pub user_code: String,

    /// The end-user verification URI on the authorization server.
    pub verification_uri: String,

    /// Optional verification URI with user_code embedded.
    pub verification_uri_complete: Option<String>,

    /// The lifetime in seconds of the device_code and user_code.
    pub expires_in: u64,

    /// The minimum amount of time in seconds that the client should wait between polling requests.
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// Token response from device flow polling.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DeviceTokenResponse {
    Success {
        access_token: String,
        token_type: String,
        expires_in: Option<u64>,
        refresh_token: Option<String>,
        scope: Option<String>,
    },
    Pending {
        error: String,
    },
}

/// OAuth 2.0 Device Flow implementation.
pub struct DeviceFlowClient {
    client: reqwest::Client,
    device_authorization_endpoint: String,
    token_endpoint: String,
    client_id: String,
    scopes: Vec<String>,
}

impl DeviceFlowClient {
    /// Create a new device flow.
    pub fn new(
        device_authorization_endpoint: String,
        token_endpoint: String,
        client_id: String,
        scopes: Vec<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            device_authorization_endpoint,
            token_endpoint,
            client_id,
            scopes,
        }
    }

    /// Initiate device authorization.
    pub async fn authorize(&self) -> Result<DeviceAuthorizationResponse> {
        let scope = self.scopes.join(" ");

        let params = [
            ("client_id", self.client_id.as_str()),
            ("scope", scope.as_str()),
        ];

        let response = self
            .client
            .post(&self.device_authorization_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(serde_urlencoded::to_string(params).context("failed to encode params")?)
            .send()
            .await
            .context("failed to request device authorization")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("device authorization failed: {} - {}", status, body);
        }

        let auth_response: DeviceAuthorizationResponse = response
            .json()
            .await
            .context("failed to parse device authorization response")?;

        Ok(auth_response)
    }

    /// Poll for token using device code.
    pub async fn poll_for_token(
        &self,
        device_code: &str,
        interval: Duration,
        expires_at: Instant,
    ) -> Result<crate::oauth::OAuthToken> {
        let mut poll_interval = interval;

        loop {
            if Instant::now() >= expires_at {
                anyhow::bail!("device code expired");
            }

            tokio::time::sleep(poll_interval).await;

            let params = [
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
                ("client_id", &self.client_id),
            ];

            let response = self
                .client
                .post(&self.token_endpoint)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(serde_urlencoded::to_string(params).context("failed to encode params")?)
                .send()
                .await
                .context("failed to poll for token")?;

            if response.status().is_success() {
                let token_response: DeviceTokenResponse = response
                    .json()
                    .await
                    .context("failed to parse token response")?;

                match token_response {
                    DeviceTokenResponse::Success {
                        access_token,
                        expires_in,
                        refresh_token,
                        ..
                    } => {
                        let expires_at = expires_in.map(|secs| {
                            chrono::Utc::now() + chrono::Duration::seconds(secs as i64)
                        });

                        return Ok(crate::oauth::OAuthToken {
                            access_token,
                            refresh_token,
                            expires_at,
                            scopes: self.scopes.clone(),
                        });
                    }
                    DeviceTokenResponse::Pending { error } => {
                        match error.as_str() {
                            "authorization_pending" => {
                                // Continue polling
                                continue;
                            }
                            "slow_down" => {
                                // Increase polling interval by 5 seconds
                                poll_interval += Duration::from_secs(5);
                                continue;
                            }
                            "access_denied" => {
                                anyhow::bail!("user denied authorization");
                            }
                            "expired_token" => {
                                anyhow::bail!("device code expired");
                            }
                            _ => {
                                anyhow::bail!("authorization error: {}", error);
                            }
                        }
                    }
                }
            } else {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("token request failed: {} - {}", status, body);
            }
        }
    }

    /// Run the complete device flow.
    pub async fn run(&self) -> Result<crate::oauth::OAuthToken> {
        // Step 1: Request device and user codes
        let auth_response = self.authorize().await?;

        // Step 2: Display user instructions
        println!("\n🔐 Device Authorization Required");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!();
        println!("Please visit: {}", auth_response.verification_uri);
        println!("And enter code: {}", auth_response.user_code);
        println!();

        if let Some(complete_uri) = &auth_response.verification_uri_complete {
            println!("Or visit this URL directly:");
            println!("{}", complete_uri);
            println!();
        }

        println!("Waiting for authorization...");
        println!();

        // Step 3: Poll for token
        let interval = Duration::from_secs(auth_response.interval);
        let expires_at = Instant::now() + Duration::from_secs(auth_response.expires_in);

        self.poll_for_token(&auth_response.device_code, interval, expires_at)
            .await
    }
}
