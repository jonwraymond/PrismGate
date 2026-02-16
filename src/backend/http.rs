use anyhow::{Context, Result};
use rmcp::{
    ServiceExt,
    model::*,
    service::RunningService,
    transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use super::lenient_client::LenientClient;
use super::{Backend, BackendState, STATE_HEALTHY, STATE_STARTING, STATE_STOPPED};
use super::{
    is_available_from_atomic, map_call_tool_result, map_tools_to_entries, state_from_atomic,
    store_state,
};
use crate::config::BackendConfig;
use crate::registry::ToolEntry;

/// A streamable-HTTP MCP backend using rmcp's reqwest-based transport.
pub struct HttpBackend {
    name: String,
    config: BackendConfig,
    service: RwLock<Option<RunningService<rmcp::RoleClient, ()>>>,
    state: AtomicU8,
}

impl HttpBackend {
    pub fn new(name: String, config: BackendConfig) -> Self {
        Self {
            name,
            config,
            service: RwLock::new(None),
            state: AtomicU8::new(STATE_STARTING),
        }
    }
}

#[async_trait::async_trait]
impl Backend for HttpBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<()> {
        self.state.store(STATE_STARTING, Ordering::Release);

        let url = self
            .config
            .url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("HTTP backend '{}' missing url", self.name))?;

        // Build transport config
        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url);

        // Add auth header if present (look for Authorization header in config)
        if let Some(auth) = self.config.headers.get("Authorization") {
            // Strip "Bearer " prefix if present — rmcp adds it back
            let token = auth.strip_prefix("Bearer ").unwrap_or(auth);
            transport_config = transport_config.auth_header(token);
        }

        // Build custom reqwest client with all non-Authorization headers as defaults.
        // This ensures headers like x-ref-api-key, CONTEXT7_API_KEY, etc. are sent
        // on every request — from_config() only forwarded Authorization via auth_header().
        let mut default_headers = reqwest::header::HeaderMap::new();
        for (key, value) in &self.config.headers {
            if key.eq_ignore_ascii_case("Authorization") {
                continue;
            }
            match (
                key.parse::<reqwest::header::HeaderName>(),
                value.parse::<reqwest::header::HeaderValue>(),
            ) {
                (Ok(name), Ok(val)) => {
                    default_headers.insert(name, val);
                }
                _ => {
                    warn!(
                        backend = %self.name,
                        header = %key,
                        "skipping unparseable custom header"
                    );
                }
            }
        }

        let reqwest_client = reqwest::Client::builder()
            .default_headers(default_headers)
            .build()
            .context("failed to build HTTP client")?;

        // Wrap in LenientClient to tolerate missing Content-Type on responses
        // (e.g., z.ai servers return 200 with no Content-Type for initialized notification)
        let client = LenientClient::new(reqwest_client);

        let transport = StreamableHttpClientTransport::with_client(client, transport_config);

        // Connect rmcp client — performs MCP initialize handshake
        let service = ().serve(transport).await.with_context(|| {
            format!(
                "failed MCP handshake with HTTP backend '{}' at {}",
                self.name, url
            )
        })?;

        if let Some(peer) = service.peer_info() {
            info!(
                backend = %self.name,
                url = %url,
                server_name = %peer.server_info.name,
                server_version = %peer.server_info.version,
                "HTTP MCP handshake complete"
            );
        } else {
            info!(backend = %self.name, url = %url, "HTTP MCP handshake complete (no peer info)");
        }

        *self.service.write().await = Some(service);
        self.state.store(STATE_HEALTHY, Ordering::Release);

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.state.store(STATE_STOPPED, Ordering::Release);

        let mut guard = self.service.write().await;
        if let Some(service) = guard.take()
            && let Err(e) = service.cancel().await
        {
            error!(backend = %self.name, error = %e, "error cancelling HTTP service");
        }

        info!(backend = %self.name, "HTTP backend stopped");
        Ok(())
    }

    async fn call_tool(&self, tool_name: &str, arguments: Option<Value>) -> Result<Value> {
        let guard = self.service.read().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("HTTP backend '{}' not started", self.name))?;

        let params = CallToolRequestParams {
            meta: None,
            name: tool_name.to_string().into(),
            arguments: arguments.and_then(|v| v.as_object().cloned()),
            task: None,
        };

        debug!(backend = %self.name, tool = %tool_name, "calling tool via HTTP");

        let result = tokio::time::timeout(self.config.timeout, service.call_tool(params))
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "tool call '{}' on HTTP backend '{}' timed out after {:?}",
                    tool_name,
                    self.name,
                    self.config.timeout
                )
            })?
            .map_err(|e| {
                anyhow::anyhow!(
                    "tool call '{}' on HTTP backend '{}' failed: {}",
                    tool_name,
                    self.name,
                    e
                )
            })?;

        Ok(map_call_tool_result(result))
    }

    async fn discover_tools(&self) -> Result<Vec<ToolEntry>> {
        let guard = self.service.read().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("HTTP backend '{}' not started", self.name))?;

        let tools = service.list_all_tools().await.map_err(|e| {
            anyhow::anyhow!(
                "tool discovery on HTTP backend '{}' failed: {}",
                self.name,
                e
            )
        })?;

        let entries = map_tools_to_entries(tools, &self.name);
        info!(backend = %self.name, tools = entries.len(), "discovered HTTP tools");
        Ok(entries)
    }

    fn is_available(&self) -> bool {
        is_available_from_atomic(&self.state)
    }

    fn state(&self) -> BackendState {
        state_from_atomic(&self.state)
    }

    fn set_state(&self, state: BackendState) {
        store_state(&self.state, state);
    }
}
