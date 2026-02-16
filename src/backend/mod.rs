pub mod health;
pub mod http;
pub mod lenient_client;
pub mod prerequisite;
pub mod stdio;

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use rmcp::model::{CallToolResult, RawContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::config::{BackendConfig, Config, Transport};
use crate::registry::{ToolEntry, ToolRegistry};

// Shared state constants used by both stdio and http backends.
pub(crate) const STATE_STARTING: u8 = 0;
pub(crate) const STATE_HEALTHY: u8 = 1;
pub(crate) const STATE_UNHEALTHY: u8 = 3;
pub(crate) const STATE_STOPPED: u8 = 7;

/// Map a CallToolResult to a JSON Value.
pub(crate) fn map_call_tool_result(result: CallToolResult) -> Value {
    let contents: Vec<Value> = result
        .content
        .into_iter()
        .map(|c| match c.raw {
            RawContent::Text(t) => Value::String(t.text),
            _ => Value::String("[non-text content]".to_string()),
        })
        .collect();

    if contents.len() == 1 {
        contents.into_iter().next().unwrap()
    } else {
        Value::Array(contents)
    }
}

/// Map rmcp Tool list to ToolEntry vec.
pub(crate) fn map_tools_to_entries(tools: Vec<rmcp::model::Tool>, backend_name: &str) -> Vec<ToolEntry> {
    tools
        .into_iter()
        .map(|t| ToolEntry {
            name: t.name.to_string(),
            description: t.description.unwrap_or_default().to_string(),
            backend_name: backend_name.to_string(),
            input_schema: serde_json::to_value(&t.input_schema)
                .unwrap_or(Value::Object(Default::default())),
        })
        .collect()
}

/// Read BackendState from an AtomicU8.
pub(crate) fn state_from_atomic(state: &AtomicU8) -> BackendState {
    match state.load(Ordering::Acquire) {
        STATE_STARTING => BackendState::Starting,
        STATE_HEALTHY => BackendState::Healthy,
        STATE_UNHEALTHY => BackendState::Unhealthy,
        STATE_STOPPED => BackendState::Stopped,
        _ => BackendState::Unhealthy,
    }
}

/// Check if backend is available from an AtomicU8.
pub(crate) fn is_available_from_atomic(state: &AtomicU8) -> bool {
    state.load(Ordering::Acquire) == STATE_HEALTHY
}

/// Store a BackendState into an AtomicU8.
pub(crate) fn store_state(atomic: &AtomicU8, state: BackendState) {
    let val = match state {
        BackendState::Starting => STATE_STARTING,
        BackendState::Healthy => STATE_HEALTHY,
        BackendState::Unhealthy => STATE_UNHEALTHY,
        BackendState::Stopped => STATE_STOPPED,
    };
    atomic.store(val, Ordering::Release);
}

/// Backend state (circuit-open is tracked internally by the health checker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendState {
    Starting,
    Healthy,
    Unhealthy,
    Stopped,
}

/// Trait for MCP backend implementations (stdio or HTTP).
#[async_trait]
pub trait Backend: Send + Sync {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn call_tool(&self, tool_name: &str, arguments: Option<Value>) -> Result<Value>;
    async fn discover_tools(&self) -> Result<Vec<ToolEntry>>;
    fn is_available(&self) -> bool;
    fn state(&self) -> BackendState;
    fn set_state(&self, state: BackendState);

    /// Wait for the backend process to exit. Returns the exit status when
    /// the child process terminates. Returns `None` immediately for HTTP
    /// backends (no child process to monitor).
    async fn wait_for_exit(&self) -> Option<std::process::ExitStatus> {
        None
    }
}

/// RAII guard that tracks in-flight calls for graceful drain on shutdown.
struct CallGuard(Arc<AtomicUsize>);

impl CallGuard {
    fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(counter))
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Retry delays for transient backend unavailability (Starting state).
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

/// Manages all backends: startup, shutdown, tool forwarding.
pub struct BackendManager {
    backends: DashMap<String, Arc<dyn Backend>>,
    configs: RwLock<std::collections::HashMap<String, BackendConfig>>,
    in_flight_calls: Arc<AtomicUsize>,
    /// Backends registered at runtime via register_manual (not from config file).
    dynamic_backends: RwLock<HashSet<String>>,
    /// PIDs of managed prerequisite processes (stopped on daemon shutdown).
    prerequisite_pids: DashMap<String, u32>,
}

impl BackendManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            backends: DashMap::new(),
            configs: RwLock::new(std::collections::HashMap::new()),
            in_flight_calls: Arc::new(AtomicUsize::new(0)),
            dynamic_backends: RwLock::new(HashSet::new()),
            prerequisite_pids: DashMap::new(),
        })
    }

    /// Start all backends from config, discover tools, register in registry.
    pub async fn start_all(
        self: &Arc<Self>,
        config: &Config,
        registry: &Arc<ToolRegistry>,
    ) -> Result<()> {
        let mut configs = self.configs.write().await;
        configs.clone_from(&config.backends);
        drop(configs);

        let mut join_set = tokio::task::JoinSet::new();

        for (name, backend_config) in &config.backends {
            let name = name.clone();
            let backend_config = backend_config.clone();
            let manager = Arc::clone(self);
            let registry = Arc::clone(registry);

            join_set.spawn(async move {
                match manager
                    .start_backend(&name, &backend_config, &registry)
                    .await
                {
                    Ok(tool_count) => {
                        info!(backend = %name, tools = tool_count, "backend started");
                    }
                    Err(e) => {
                        error!(backend = %name, error = %e, "failed to start backend");
                    }
                }
            });
        }

        // Wait for all backends to start
        while join_set.join_next().await.is_some() {}

        info!(
            backends = self.backends.len(),
            "all backends started"
        );

        Ok(())
    }

    /// Start a single backend, discover its tools, register them.
    /// For stdio backends, also spawns a reaper task that detects unexpected
    /// child process exits and marks the backend as Stopped immediately
    /// (rather than waiting for the next health check ping).
    async fn start_backend(
        &self,
        name: &str,
        config: &BackendConfig,
        registry: &Arc<ToolRegistry>,
    ) -> Result<usize> {
        // Ensure prerequisite process is running before starting backend
        if let Some(prereq) = &config.prerequisite {
            match prerequisite::ensure_prerequisite(name, prereq).await {
                Ok(Some(pid)) => {
                    if prereq.managed {
                        self.prerequisite_pids.insert(name.to_string(), pid);
                    }
                }
                Ok(None) => {} // Already running
                Err(e) => {
                    anyhow::bail!("prerequisite failed for backend '{name}': {e}");
                }
            }
        }

        let is_stdio = config.transport == Transport::Stdio;

        let backend: Arc<dyn Backend> = match config.transport {
            Transport::Stdio => {
                let b = stdio::StdioBackend::new(name.to_string(), config.clone());
                b.start().await?;
                Arc::new(b)
            }
            Transport::StreamableHttp => {
                let b = http::HttpBackend::new(name.to_string(), config.clone());
                b.start().await?;
                Arc::new(b)
            }
        };

        // Discover tools
        let tools = backend.discover_tools().await?;
        let tool_count = tools.len();

        // Register in registry
        registry.register_backend_tools(name, tools);

        // Store backend
        self.backends.insert(name.to_string(), Arc::clone(&backend));

        // Spawn reaper task for stdio backends — monitors child process and
        // marks backend as Stopped immediately on unexpected exit.
        // The health checker will then auto-restart it with backoff.
        if is_stdio {
            let reaper_name = name.to_string();
            tokio::spawn(async move {
                if let Some(status) = backend.wait_for_exit().await
                    && backend.state() != BackendState::Stopped
                {
                    warn!(
                        backend = %reaper_name,
                        exit_code = ?status.code(),
                        "backend process exited unexpectedly"
                    );
                    backend.set_state(BackendState::Stopped);
                }
            });
        }

        Ok(tool_count)
    }

    /// Add a backend at runtime (from register_manual or hot-reload).
    ///
    /// If a backend with the same name already exists, it is stopped first
    /// to prevent orphaned child processes.
    pub async fn add_backend(
        self: &Arc<Self>,
        name: &str,
        config: BackendConfig,
        registry: &Arc<ToolRegistry>,
    ) -> Result<usize> {
        // Stop existing backend if present (prevent orphaned processes)
        if let Some((_, old_backend)) = self.backends.remove(name) {
            warn!(backend = %name, "stopping existing backend before re-registration");
            if let Err(e) = old_backend.stop().await {
                warn!(backend = %name, error = %e, "error stopping existing backend");
            }
            registry.remove_backend_tools(name);
        }

        // Store config
        let mut configs = self.configs.write().await;
        configs.insert(name.to_string(), config.clone());
        drop(configs);

        self.start_backend(name, &config, registry).await
    }

    /// Remove a backend (from deregister_manual or hot-reload).
    ///
    /// Also cleans up the dynamic backend tracking set to prevent stale entries
    /// from blocking future registrations or allowing static backend deregistration.
    pub async fn remove_backend(&self, name: &str, registry: &ToolRegistry) -> Result<()> {
        // Stop the backend
        if let Some((_, backend)) = self.backends.remove(name)
            && let Err(e) = backend.stop().await
        {
            warn!(backend = %name, error = %e, "error stopping backend");
        }

        // Remove tools from registry
        registry.remove_backend_tools(name);

        // Remove config and dynamic tracking
        let mut configs = self.configs.write().await;
        configs.remove(name);
        drop(configs);

        self.dynamic_backends.write().await.remove(name);

        // Stop managed prerequisite if tracked
        if let Some((_, pid)) = self.prerequisite_pids.remove(name) {
            prerequisite::stop_prerequisite(name, pid).await;
        }

        info!(backend = %name, "backend removed");
        Ok(())
    }

    /// Forward a tool call to the correct backend.
    ///
    /// If the backend is in `Starting` state (not yet connected), retries up to 3 times
    /// with exponential backoff (500ms, 1s, 2s). Fails immediately for `Unhealthy`/`Stopped`.
    pub async fn call_tool(
        &self,
        backend_name: &str,
        tool_name: &str,
        arguments: Option<Value>,
    ) -> Result<Value> {
        let _guard = CallGuard::new(&self.in_flight_calls);

        for (attempt, delay) in RETRY_DELAYS.iter().enumerate() {
            // Check if backend exists in the DashMap
            let backend = self.backends.get(backend_name).map(|r| Arc::clone(r.value()));

            match backend {
                Some(b) => {
                    let state = b.state();
                    match state {
                        BackendState::Healthy => {
                            return b.call_tool(tool_name, arguments).await;
                        }
                        BackendState::Starting => {
                            debug!(
                                backend = %backend_name,
                                tool = %tool_name,
                                attempt = attempt + 1,
                                delay_ms = delay.as_millis() as u64,
                                "backend starting, retrying"
                            );
                            tokio::time::sleep(*delay).await;
                        }
                        // Unhealthy or Stopped — fail immediately, no point retrying
                        _ => {
                            anyhow::bail!(
                                "backend '{}' is not available (state: {:?})",
                                backend_name,
                                state
                            );
                        }
                    }
                }
                None => {
                    // Backend not in DashMap yet — may still be starting up from cache
                    debug!(
                        backend = %backend_name,
                        tool = %tool_name,
                        attempt = attempt + 1,
                        delay_ms = delay.as_millis() as u64,
                        "backend not found, retrying"
                    );
                    tokio::time::sleep(*delay).await;
                }
            }
        }

        // All retries exhausted — produce a descriptive error
        match self.backends.get(backend_name).map(|r| r.value().state()) {
            Some(BackendState::Starting) => {
                anyhow::bail!(
                    "backend '{}' is still starting (retried {} times over ~3.5s). \
                     The tool '{}' is cached but the backend hasn't connected yet.",
                    backend_name,
                    RETRY_DELAYS.len(),
                    tool_name
                )
            }
            Some(state) => {
                anyhow::bail!(
                    "backend '{}' is not available (state: {:?})",
                    backend_name,
                    state
                )
            }
            None => {
                anyhow::bail!(
                    "backend '{}' not found after {} retries. \
                     It may not be configured or failed to start.",
                    backend_name,
                    RETRY_DELAYS.len()
                )
            }
        }
    }

    /// Check if a backend is ready to accept tool calls.
    #[allow(dead_code)]
    pub fn is_backend_ready(&self, name: &str) -> bool {
        self.backends
            .get(name)
            .is_some_and(|r| r.value().is_available())
    }

    /// Get the current state of a backend.
    #[allow(dead_code)]
    pub fn get_backend_state(&self, name: &str) -> Option<BackendState> {
        self.backends.get(name).map(|r| r.value().state())
    }

    /// Get the config for a backend (used by get_required_keys_for_tool).
    pub async fn get_backend_config(&self, name: &str) -> Option<BackendConfig> {
        let configs = self.configs.read().await;
        configs.get(name).cloned()
    }

    /// Stop all backends gracefully (in parallel), draining in-flight calls first.
    pub async fn stop_all(&self) {
        let backends: Vec<(String, Arc<dyn Backend>)> = self
            .backends
            .iter()
            .map(|r| (r.key().clone(), Arc::clone(r.value())))
            .collect();

        // Clear the map first so no new calls can be dispatched
        self.backends.clear();

        // Wait for in-flight calls to drain (max 10s)
        let drain_start = std::time::Instant::now();
        let in_flight = self.in_flight_calls.load(Ordering::SeqCst);
        if in_flight > 0 {
            info!(in_flight, "draining in-flight calls before shutdown");
            loop {
                let remaining = self.in_flight_calls.load(Ordering::SeqCst);
                if remaining == 0 {
                    info!(
                        elapsed_ms = drain_start.elapsed().as_millis() as u64,
                        "all in-flight calls drained"
                    );
                    break;
                }
                if drain_start.elapsed() > Duration::from_secs(10) {
                    warn!(
                        in_flight = remaining,
                        "drain timeout after 10s, forcing shutdown"
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        let mut join_set = tokio::task::JoinSet::new();
        for (name, backend) in backends {
            join_set.spawn(async move {
                if let Err(e) = backend.stop().await {
                    warn!(backend = %name, error = %e, "error stopping backend");
                }
            });
        }

        while join_set.join_next().await.is_some() {}

        // Stop managed prerequisite processes
        for entry in self.prerequisite_pids.iter() {
            prerequisite::stop_prerequisite(entry.key(), *entry.value()).await;
        }
        self.prerequisite_pids.clear();

        info!("all backends stopped");
    }

    /// Ping a backend to check if it's responsive (used by health checker).
    /// For stdio backends, this calls tools/list as a lightweight probe.
    pub async fn ping_backend(&self, name: &str) -> Result<()> {
        let backend = self
            .backends
            .get(name)
            .map(|r| Arc::clone(r.value()))
            .ok_or_else(|| anyhow::anyhow!("backend '{name}' not found"))?;

        // Use discover_tools as a ping — it calls tools/list over the MCP connection.
        // This is lightweight and verifies the connection is alive.
        backend
            .discover_tools()
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("ping failed for '{name}': {e}"))
    }

    /// Restart a backend: stop it, re-read config, start fresh, re-discover tools.
    pub async fn restart_backend(
        &self,
        name: &str,
        registry: &Arc<ToolRegistry>,
    ) -> Result<usize> {
        // Stop old backend
        if let Some((_, backend)) = self.backends.remove(name)
            && let Err(e) = backend.stop().await
        {
            warn!(backend = %name, error = %e, "error stopping backend for restart");
        }

        // Remove old tools
        registry.remove_backend_tools(name);

        // Get config
        let configs = self.configs.read().await;
        let config = configs
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no config for backend '{name}'"))?;
        drop(configs);

        // Start fresh
        self.start_backend(name, &config, registry).await
    }

    /// Return the names of all configured backends (including those that failed
    /// initial handshake and never entered the DashMap).
    pub async fn get_configured_names(&self) -> Vec<String> {
        let configs = self.configs.read().await;
        configs.keys().cloned().collect()
    }

    /// Try to start a backend from its stored config. Unlike `restart_backend()`,
    /// this does not attempt to stop/remove an existing backend first — it's
    /// designed for backends that failed initial handshake and never entered the
    /// DashMap.
    pub async fn try_start_from_config(
        &self,
        name: &str,
        registry: &Arc<ToolRegistry>,
    ) -> Result<usize> {
        let configs = self.configs.read().await;
        let config = configs
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no config for backend '{name}'"))?;
        drop(configs);

        self.start_backend(name, &config, registry).await
    }

    /// Set the state of a backend (used by health checker for circuit breaker).
    pub fn set_backend_state(&self, name: &str, state: BackendState) {
        if let Some(backend) = self.backends.get(name) {
            backend.set_state(state);
        }
    }

    /// Get status of all backends.
    pub fn get_all_status(&self) -> Vec<BackendStatus> {
        self.backends
            .iter()
            .map(|r| BackendStatus {
                name: r.key().clone(),
                state: r.value().state(),
                available: r.value().is_available(),
            })
            .collect()
    }

    /// Mark a backend as dynamically registered (via register_manual).
    pub async fn mark_dynamic(&self, name: &str) {
        self.dynamic_backends.write().await.insert(name.to_string());
    }

    /// Check if a backend was dynamically registered (safe to deregister).
    pub async fn is_dynamic(&self, name: &str) -> bool {
        self.dynamic_backends.read().await.contains(name)
    }

    /// Remove a backend from the dynamic tracking set.
    pub async fn unmark_dynamic(&self, name: &str) {
        self.dynamic_backends.write().await.remove(name);
    }

    /// Number of dynamically registered backends.
    pub async fn dynamic_count(&self) -> usize {
        self.dynamic_backends.read().await.len()
    }
}

/// Status summary for a backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendStatus {
    pub name: String,
    pub state: BackendState,
    pub available: bool,
}
