//! OAuth 2.0 support for MCP backends.
//!
//! This module provides:
//! - RFC 8414 OAuth discovery
//! - Authorization code flow with PKCE (RFC 7636)
//! - Device authorization flow (RFC 8628)
//! - Token introspection (RFC 7662)
//! - JWT bearer grant (RFC 7523)
//! - Token storage and automatic refresh
//! - Token encryption at rest
//! - Browser-based user consent flow
//! - OAuth 2.1 compliance (mandatory PKCE, refresh token rotation)

#![allow(unused_imports, dead_code)]

pub mod callback_server;
pub mod config;
pub mod device_flow;
pub mod discovery;
pub mod encryption;
pub mod flow;
pub mod introspection;
pub mod jwt_bearer;
pub mod pkce;
pub mod token_store;

pub use callback_server::CallbackServer;
pub use config::OAuthConfig;
pub use device_flow::DeviceFlowClient;
pub use discovery::discover_oauth_endpoints;
pub use encryption::EncryptionKeyManager;
pub use flow::OAuthFlow;
pub use introspection::IntrospectionClient;
pub use jwt_bearer::JwtBearerClient;
pub use pkce::PkceChallenge;
pub use token_store::{OAuthToken, TokenStore};

use anyhow::Result;

/// Authenticate a backend using OAuth 2.0.
///
/// This is the main entry point for OAuth authentication.
/// It will:
/// 1. Discover OAuth endpoints (if configured)
/// 2. Generate PKCE challenge
/// 3. Open browser for user consent
/// 4. Start local callback server
/// 5. Exchange authorization code for tokens
/// 6. Store tokens securely
pub async fn authenticate(
    backend_name: &str,
    base_url: &str,
    config: &OAuthConfig,
) -> Result<OAuthToken> {
    let flow = OAuthFlow::new(base_url, config).await?;
    let token = flow.authorize().await?;

    let store = TokenStore::new()?;
    store.store(backend_name, &token)?;

    Ok(token)
}

/// Refresh an OAuth token.
pub async fn refresh_token(
    backend_name: &str,
    base_url: &str,
    config: &OAuthConfig,
    refresh_token: &str,
) -> Result<OAuthToken> {
    let flow = OAuthFlow::new(base_url, config).await?;
    let token = flow.refresh(refresh_token).await?;

    let store = TokenStore::new()?;
    store.store(backend_name, &token)?;

    Ok(token)
}
