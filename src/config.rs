use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};
use std::time::Duration;

static DOTENV_ONCE: Once = Once::new();

/// Load `~/.env` into the process environment exactly once.
///
/// Must be called early in `main()` before spawning concurrent tasks.
/// Uses `Once` to guarantee single execution — safe to call multiple times
/// but only the first call has any effect. Subsequent calls (e.g., from
/// hot-reload) are no-ops, preventing UB from `set_var` in multi-threaded context.
pub fn load_dotenv() {
    DOTENV_ONCE.call_once(|| {
        let env_path = dirs::home_dir()
            .map(|h| h.join(".env"))
            .filter(|p| p.is_file());
        if let Some(env_file) = env_path
            && let Ok(contents) = std::fs::read_to_string(&env_file)
        {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    // SAFETY: The tokio multi-thread runtime has worker threads
                    // running, but no user tasks have been spawned yet and no
                    // concurrent env var reads occur at this point. `Once` ensures
                    // this runs at most once.
                    unsafe { std::env::set_var(key.trim(), value.trim()) };
                }
            }
        }
    });
}

/// Top-level gatemini configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_log_level")]
    pub log_level: String,

    #[serde(default)]
    pub secrets: SecretsConfig,

    #[serde(default)]
    pub backends: HashMap<String, BackendConfig>,

    #[serde(default)]
    pub health: HealthConfig,

    #[serde(default)]
    pub admin: AdminConfig,

    #[serde(default)]
    pub sandbox: SandboxConfig,

    #[serde(default)]
    pub semantic: Option<SemanticConfig>,

    #[serde(default)]
    pub daemon: DaemonConfig,

    /// Custom cache file location. Default: ~/.prismgate/cache.json
    #[serde(default)]
    pub cache_path: Option<PathBuf>,

    /// Allow register_manual / deregister_manual at runtime.
    /// Set to false to lock down the gateway to only config-file backends.
    #[serde(default = "default_true_config")]
    pub allow_runtime_registration: bool,

    /// Maximum number of dynamically registered backends (via register_manual).
    #[serde(default = "default_max_dynamic_backends")]
    pub max_dynamic_backends: usize,
}

/// Secrets resolution configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretsConfig {
    #[serde(default)]
    pub strict: bool,

    #[serde(default)]
    pub providers: SecretProvidersConfig,
}

/// Secret provider configurations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecretProvidersConfig {
    #[serde(default)]
    pub bws: BwsProviderConfig,
}

/// Bitwarden Secrets Manager provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BwsProviderConfig {
    #[serde(default)]
    pub enabled: bool,

    /// BWS access token. Falls back to BWS_ACCESS_TOKEN env var.
    pub access_token: Option<String>,

    /// Organization UUID. Falls back to BWS_ORG_ID env var.
    pub organization_id: Option<String>,
}

/// Per-backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackendConfig {
    #[serde(default = "default_transport")]
    pub transport: Transport,

    /// Custom namespace prefix for tools from this backend.
    /// Default: the backend's YAML key name. Tools are registered as `namespace.tool_name`.
    #[serde(default)]
    pub namespace: Option<String>,

    /// Command to spawn (stdio backends).
    pub command: Option<String>,

    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables passed to the child process.
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Working directory for the child process.
    pub cwd: Option<String>,

    /// URL for streamable-http backends.
    pub url: Option<String>,

    /// HTTP headers for streamable-http backends.
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Request timeout.
    #[serde(default = "default_timeout", with = "humantime_duration")]
    pub timeout: Duration,

    /// Required environment variable keys (for get_required_keys_for_tool).
    #[serde(default)]
    pub required_keys: Vec<String>,

    /// Max concurrent tool calls to this backend. None = use transport default
    /// (stdio: 10, HTTP: 100). Set to 0 for unlimited.
    #[serde(default)]
    pub max_concurrent_calls: Option<u32>,

    /// Timeout for acquiring a call semaphore permit. Accepts humantime durations
    /// (e.g., "60s", "5m"). Default: 60s.
    #[serde(default = "default_semaphore_timeout", with = "humantime_duration")]
    pub semaphore_timeout: Duration,

    /// Per-backend retry configuration for transient failures.
    #[serde(default)]
    pub retry: RetryConfig,

    /// Optional prerequisite process that must be running before this backend starts.
    #[serde(default)]
    pub prerequisite: Option<PrerequisiteConfig>,

    /// Rate limit: max calls per time window. None = no rate limit.
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
}

/// Per-backend retry configuration for transient failures (Starting state).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryConfig {
    /// Maximum number of retries before giving up. Default: 3.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Initial delay before first retry. Default: 500ms.
    #[serde(default = "default_retry_initial_delay", with = "humantime_duration")]
    pub initial_delay: Duration,
    /// Maximum delay between retries. Default: 2s.
    #[serde(default = "default_retry_max_delay", with = "humantime_duration")]
    pub max_delay: Duration,
    /// Multiplier applied to delay after each retry. Default: 2.0.
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_delay: default_retry_initial_delay(),
            max_delay: default_retry_max_delay(),
            backoff_multiplier: default_backoff_multiplier(),
        }
    }
}

/// Rate limiting configuration: max calls per time window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimitConfig {
    /// Maximum calls allowed per window.
    pub max_calls: u32,
    /// Time window for rate limiting. Default: 60s.
    #[serde(default = "default_rate_window", with = "humantime_duration")]
    pub window: Duration,
}

/// Configuration for a prerequisite process that must be running before a backend starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrerequisiteConfig {
    /// Command to spawn the prerequisite process.
    pub command: String,

    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables passed to the prerequisite process.
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Working directory for the prerequisite process.
    pub cwd: Option<String>,

    /// Substring to match against running process command lines (pgrep -f).
    /// If omitted, prerequisite is spawned every time (with dedup warning).
    pub process_match: Option<String>,

    /// true = stop on daemon shutdown, false = fire-and-forget.
    #[serde(default)]
    pub managed: bool,

    /// Wait after spawning before proceeding with backend start.
    #[serde(default = "default_startup_delay", with = "humantime_duration")]
    pub startup_delay: Duration,
}

/// Transport type for a backend.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    Stdio,
    StreamableHttp,
}

/// Global health check configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    #[serde(default = "default_health_interval", with = "humantime_duration")]
    pub interval: Duration,

    #[serde(default = "default_health_timeout", with = "humantime_duration")]
    pub timeout: Duration,

    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,

    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,

    #[serde(default = "default_restart_window", with = "humantime_duration")]
    pub restart_window: Duration,

    /// Initial backoff duration for restart attempts. Default: 1s.
    #[serde(default = "default_restart_initial_backoff", with = "humantime_duration")]
    pub restart_initial_backoff: Duration,

    /// Maximum backoff duration for restart attempts. Default: 30s.
    #[serde(default = "default_restart_max_backoff", with = "humantime_duration")]
    pub restart_max_backoff: Duration,

    /// Timeout for a single restart operation. Default: 30s.
    #[serde(default = "default_restart_timeout", with = "humantime_duration")]
    pub restart_timeout: Duration,

    /// Circuit breaker recovery multiplier (recovery_window = interval * recovery_multiplier). Default: 3.
    #[serde(default = "default_recovery_multiplier")]
    pub recovery_multiplier: u32,

    /// Maximum time to wait for in-flight calls to drain during shutdown. Default: 10s.
    #[serde(default = "default_drain_timeout", with = "humantime_duration")]
    pub drain_timeout: Duration,
}

/// Admin API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_admin_listen")]
    pub listen: String,

    #[serde(default = "default_allowed_cidrs")]
    pub allowed_cidrs: Vec<String>,
}

/// TypeScript sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default = "default_sandbox_timeout", with = "humantime_duration")]
    pub timeout: Duration,

    #[serde(default = "default_max_output_size")]
    pub max_output_size: usize,

    /// Max concurrent V8 sandbox executions. Default: 8.
    #[serde(default = "default_max_concurrent_sandboxes")]
    pub max_concurrent_sandboxes: u32,
}

/// Daemon lifecycle configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Shut down daemon after this duration with no active clients.
    /// Default: 5m. Set to "0s" to disable.
    #[serde(default = "default_idle_timeout", with = "humantime_duration")]
    pub idle_timeout: Duration,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            idle_timeout: default_idle_timeout(),
        }
    }
}

fn default_idle_timeout() -> Duration {
    Duration::from_secs(300)
}

/// Semantic embedding search configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticConfig {
    /// HuggingFace Hub model ID or local path to a model2vec model.
    #[serde(default = "default_semantic_model")]
    pub model_path: String,

    /// Directory for cached embedding models. Default: ~/.prismgate/models/
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
}

fn default_semantic_model() -> String {
    "minishlab/potion-base-8M".to_string()
}

// --- Defaults ---

fn default_log_level() -> String {
    "info".to_string()
}
fn default_transport() -> Transport {
    Transport::Stdio
}
fn default_timeout() -> Duration {
    Duration::from_secs(30)
}
fn default_health_interval() -> Duration {
    Duration::from_secs(30)
}
fn default_health_timeout() -> Duration {
    Duration::from_secs(5)
}
fn default_failure_threshold() -> u32 {
    3
}
fn default_max_restarts() -> u32 {
    5
}
fn default_restart_window() -> Duration {
    Duration::from_secs(60)
}
fn default_admin_listen() -> String {
    "127.0.0.1:19999".to_string()
}
fn default_allowed_cidrs() -> Vec<String> {
    vec!["127.0.0.1/32".to_string()]
}
fn default_sandbox_timeout() -> Duration {
    Duration::from_secs(30)
}
fn default_max_output_size() -> usize {
    200_000
}
fn default_true_config() -> bool {
    true
}
fn default_max_dynamic_backends() -> usize {
    10
}
fn default_startup_delay() -> Duration {
    Duration::from_secs(2)
}
fn default_semaphore_timeout() -> Duration {
    Duration::from_secs(60)
}
fn default_max_concurrent_sandboxes() -> u32 {
    8
}
fn default_max_retries() -> u32 {
    3
}
fn default_retry_initial_delay() -> Duration {
    Duration::from_millis(500)
}
fn default_retry_max_delay() -> Duration {
    Duration::from_secs(2)
}
fn default_backoff_multiplier() -> f64 {
    2.0
}
fn default_rate_window() -> Duration {
    Duration::from_secs(60)
}
fn default_restart_initial_backoff() -> Duration {
    Duration::from_secs(1)
}
fn default_restart_max_backoff() -> Duration {
    Duration::from_secs(30)
}
fn default_restart_timeout() -> Duration {
    Duration::from_secs(30)
}
fn default_recovery_multiplier() -> u32 {
    3
}
fn default_drain_timeout() -> Duration {
    Duration::from_secs(10)
}

// --- Default impls ---

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            interval: default_health_interval(),
            timeout: default_health_timeout(),
            failure_threshold: default_failure_threshold(),
            max_restarts: default_max_restarts(),
            restart_window: default_restart_window(),
            restart_initial_backoff: default_restart_initial_backoff(),
            restart_max_backoff: default_restart_max_backoff(),
            restart_timeout: default_restart_timeout(),
            recovery_multiplier: default_recovery_multiplier(),
            drain_timeout: default_drain_timeout(),
        }
    }
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: default_admin_listen(),
            allowed_cidrs: default_allowed_cidrs(),
        }
    }
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            timeout: default_sandbox_timeout(),
            max_output_size: default_max_output_size(),
            max_concurrent_sandboxes: default_max_concurrent_sandboxes(),
        }
    }
}

// --- humantime_duration serde helper ---

mod humantime_duration {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = humantime_format(duration);
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        humantime_parse(&s).map_err(serde::de::Error::custom)
    }

    fn humantime_format(d: &Duration) -> String {
        let secs = d.as_secs();
        if secs.is_multiple_of(60) && secs >= 60 {
            format!("{}m", secs / 60)
        } else {
            format!("{}s", secs)
        }
    }

    fn humantime_parse(s: &str) -> Result<Duration, String> {
        let s = s.trim();
        if let Some(n) = s.strip_suffix('s') {
            n.parse::<u64>()
                .map(Duration::from_secs)
                .map_err(|e| format!("invalid duration '{s}': {e}"))
        } else if let Some(n) = s.strip_suffix('m') {
            n.parse::<u64>()
                .map(|m| Duration::from_secs(m * 60))
                .map_err(|e| format!("invalid duration '{s}': {e}"))
        } else if let Some(n) = s.strip_suffix('h') {
            n.parse::<u64>()
                .map(|h| Duration::from_secs(h * 3600))
                .map_err(|e| format!("invalid duration '{s}': {e}"))
        } else {
            // Try parsing as raw seconds
            s.parse::<u64>().map(Duration::from_secs).map_err(|_| {
                format!("invalid duration '{s}': expected format like '30s', '5m', '1h'")
            })
        }
    }
}

// --- Loading ---

impl Config {
    /// Load config from a YAML file, performing environment variable interpolation
    /// and secret resolution.
    ///
    /// Pipeline: read file → shellexpand ${VAR} → deserialize YAML → resolve secretref: → validate
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;

        // Expand ${VAR} references from process environment
        let expanded = shellexpand::env(&raw)
            .map_err(|e| anyhow::anyhow!("env var interpolation failed: {e}"))?;

        let config: Config = serde_yaml_ng::from_str(&expanded)
            .with_context(|| format!("failed to parse config file: {}", path.display()))?;

        config.validate()?;
        Ok(config)
    }

    /// Async counterpart: resolves secrets after the tokio runtime is available.
    /// Call this after `Config::load()` when BWS is enabled.
    pub async fn resolve_secrets_async(&mut self) -> Result<()> {
        if !self.secrets.providers.bws.enabled {
            return Ok(());
        }

        let bws_config = &self.secrets.providers.bws;

        // Resolve access token: config → env var
        let access_token = match &bws_config.access_token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => std::env::var("BWS_ACCESS_TOKEN").context(
                "BWS enabled but access_token not in config and BWS_ACCESS_TOKEN env var not found",
            )?,
        };

        let provider = crate::secrets::bws::BwsSdkProvider::new(
            access_token,
            bws_config.organization_id.clone(),
        )
        .await
        .context("failed to initialize BWS provider")?;

        let mut resolver = crate::secrets::resolver::SecretResolver::new(self.secrets.strict);
        resolver.register(Box::new(provider));

        self.resolve_secrets(&resolver)?;
        Ok(())
    }

    /// Resolve all secretref patterns in backend configs using the given resolver.
    pub fn resolve_secrets(
        &mut self,
        resolver: &crate::secrets::resolver::SecretResolver,
    ) -> Result<()> {
        for (name, backend) in self.backends.iter_mut() {
            resolver
                .resolve_option(&mut backend.command)
                .with_context(|| format!("backend '{name}' command"))?;

            resolver
                .resolve_slice(&mut backend.args)
                .with_context(|| format!("backend '{name}' args"))?;

            resolver
                .resolve_map(&mut backend.env)
                .with_context(|| format!("backend '{name}' env"))?;

            resolver
                .resolve_option(&mut backend.url)
                .with_context(|| format!("backend '{name}' url"))?;

            resolver
                .resolve_map(&mut backend.headers)
                .with_context(|| format!("backend '{name}' headers"))?;

            if let Some(prereq) = &mut backend.prerequisite {
                resolver
                    .resolve_slice(&mut prereq.args)
                    .with_context(|| format!("backend '{name}' prerequisite args"))?;

                resolver
                    .resolve_map(&mut prereq.env)
                    .with_context(|| format!("backend '{name}' prerequisite env"))?;
            }
        }

        Ok(())
    }

    /// Validate the configuration.
    fn validate(&self) -> Result<()> {
        if self.sandbox.max_concurrent_sandboxes == 0 {
            anyhow::bail!(
                "sandbox.max_concurrent_sandboxes must be >= 1 (got 0, which would deadlock)"
            );
        }

        for (name, backend) in &self.backends {
            match backend.transport {
                Transport::Stdio => {
                    if backend.command.is_none() {
                        anyhow::bail!("backend '{name}': stdio transport requires 'command' field");
                    }
                }
                Transport::StreamableHttp => {
                    if backend.url.is_none() {
                        anyhow::bail!(
                            "backend '{name}': streamable-http transport requires 'url' field"
                        );
                    }
                }
            }

            if let Some(max) = backend.max_concurrent_calls
                && max > 10_000
            {
                anyhow::bail!(
                    "backend '{name}': max_concurrent_calls ({max}) exceeds limit of 10,000"
                );
            }

            if let Some(prereq) = &backend.prerequisite
                && prereq.process_match.is_none()
            {
                tracing::warn!(
                    backend = %name,
                    "prerequisite has no process_match — will spawn every time without dedup"
                );
            }
        }
        Ok(())
    }
}

/// Diff between old and new configs.
pub struct ConfigDiff {
    /// Backends that were added (name -> config).
    pub added: Vec<(String, BackendConfig)>,
    /// Backends that were removed.
    pub removed: Vec<String>,
    /// Backends whose config changed (need restart).
    pub changed: Vec<(String, BackendConfig)>,
}

impl Config {
    /// Compute the diff between this config and a new config.
    pub fn diff_backends(&self, new: &Config) -> ConfigDiff {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();

        // Find added and changed backends
        for (name, new_config) in &new.backends {
            match self.backends.get(name) {
                None => added.push((name.clone(), new_config.clone())),
                Some(old_config) if old_config != new_config => {
                    changed.push((name.clone(), new_config.clone()));
                }
                _ => {} // Unchanged
            }
        }

        // Find removed backends
        for name in self.backends.keys() {
            if !new.backends.contains_key(name) {
                removed.push(name.clone());
            }
        }

        ConfigDiff {
            added,
            removed,
            changed,
        }
    }
}

/// Watch a config file for changes and apply diffs to the backend manager.
/// Runs as a background task until the shutdown notify is triggered.
pub async fn watch_config(
    config_path: std::path::PathBuf,
    current_config: Arc<arc_swap::ArcSwap<Config>>,
    manager: Arc<crate::backend::BackendManager>,
    registry: Arc<crate::registry::ToolRegistry>,
    cache_path: std::path::PathBuf,
    shutdown: Arc<tokio::sync::Notify>,
) {
    use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use tracing::{error, info, warn};

    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

    // Set up file watcher
    let watcher_result: std::result::Result<RecommendedWatcher, _> =
        notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
            if let Ok(event) = res
                && matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                )
            {
                let _ = tx.try_send(());
            }
        });

    let mut watcher = match watcher_result {
        Ok(w) => w,
        Err(e) => {
            error!(error = %e, "failed to create config file watcher");
            return;
        }
    };

    if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
        error!(path = %config_path.display(), error = %e, "failed to watch config file");
        return;
    }

    info!(path = %config_path.display(), "config file watcher started");

    // Debounce: wait a bit after a change before reloading
    let debounce = std::time::Duration::from_millis(500);

    loop {
        tokio::select! {
            Some(()) = rx.recv() => {
                // Debounce: drain any rapid-fire events
                tokio::time::sleep(debounce).await;
                while rx.try_recv().is_ok() {}

                info!("config file changed, reloading");

                let mut new_config = match Config::load(&config_path) {
                    Ok(c) => c,
                    Err(e) => {
                        error!(error = %e, "failed to parse new config, keeping current");
                        continue;
                    }
                };

                // Resolve secrets for hot-reloaded config
                if let Err(e) = new_config.resolve_secrets_async().await {
                    error!(error = %e, "failed to resolve secrets in new config, keeping current");
                    continue;
                }

                let old_config = current_config.load();
                let diff = old_config.diff_backends(&new_config);

                let has_changes = !diff.added.is_empty()
                    || !diff.removed.is_empty()
                    || !diff.changed.is_empty();

                if !has_changes {
                    info!("config reloaded, no backend changes detected");
                    current_config.store(Arc::new(new_config));
                    continue;
                }

                info!(
                    added = diff.added.len(),
                    removed = diff.removed.len(),
                    changed = diff.changed.len(),
                    "applying config changes"
                );

                // Remove backends
                for name in &diff.removed {
                    if let Err(e) = manager.remove_backend(name, &registry).await {
                        warn!(backend = %name, error = %e, "error removing backend");
                    }
                }

                // Restart changed backends (remove + add)
                for (name, new_backend_config) in &diff.changed {
                    if let Err(e) = manager.remove_backend(name, &registry).await {
                        warn!(backend = %name, error = %e, "error removing changed backend");
                    }
                    match manager.add_backend(name, new_backend_config.clone(), &registry).await {
                        Ok(tools) => info!(backend = %name, tools, "changed backend restarted"),
                        Err(e) => error!(backend = %name, error = %e, "failed to restart changed backend"),
                    }
                }

                // Add new backends
                for (name, backend_config) in &diff.added {
                    match manager.add_backend(name, backend_config.clone(), &registry).await {
                        Ok(tools) => info!(backend = %name, tools, "new backend added"),
                        Err(e) => error!(backend = %name, error = %e, "failed to add new backend"),
                    }
                }

                // Store new config
                current_config.store(Arc::new(new_config));

                // Save updated tool cache
                crate::cache::save(&cache_path, &registry).await;

                info!(
                    total_tools = registry.tool_count(),
                    total_backends = registry.backend_count(),
                    "config reload complete"
                );
            }
            _ = shutdown.notified() => {
                info!("config watcher shutting down");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let yaml = r#"
log_level: debug
backends:
  test-echo:
    transport: stdio
    command: echo
    args: ["hello"]
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.backends.len(), 1);
        let backend = config.backends.get("test-echo").unwrap();
        assert_eq!(backend.transport, Transport::Stdio);
        assert_eq!(backend.command.as_deref(), Some("echo"));
    }

    #[test]
    fn test_parse_http_backend() {
        let yaml = r#"
backends:
  my-service:
    transport: streamable-http
    url: "http://localhost:8080/mcp"
    headers:
      Authorization: "Bearer token123"
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let backend = config.backends.get("my-service").unwrap();
        assert_eq!(backend.transport, Transport::StreamableHttp);
        assert_eq!(backend.url.as_deref(), Some("http://localhost:8080/mcp"));
        assert_eq!(
            backend.headers.get("Authorization").map(String::as_str),
            Some("Bearer token123")
        );
    }

    #[test]
    fn test_validate_stdio_missing_command() {
        let yaml = r#"
backends:
  broken:
    transport: stdio
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_http_missing_url() {
        let yaml = r#"
backends:
  broken:
    transport: streamable-http
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_defaults() {
        let yaml = "{}";
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.log_level, "info");
        assert!(config.backends.is_empty());
        assert_eq!(config.health.interval, Duration::from_secs(30));
        assert!(!config.admin.enabled);
    }

    #[test]
    fn test_diff_backends() {
        let old_yaml = r#"
backends:
  exa:
    transport: stdio
    command: npx
    args: ["-y", "exa-server"]
  tavily:
    transport: stdio
    command: npx
    args: ["-y", "tavily-server"]
"#;
        let new_yaml = r#"
backends:
  exa:
    transport: stdio
    command: npx
    args: ["-y", "exa-server", "--new-flag"]
  firecrawl:
    transport: stdio
    command: npx
    args: ["-y", "firecrawl-server"]
"#;
        let old: Config = serde_yaml_ng::from_str(old_yaml).unwrap();
        let new: Config = serde_yaml_ng::from_str(new_yaml).unwrap();

        let diff = old.diff_backends(&new);

        // firecrawl is added
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].0, "firecrawl");

        // tavily is removed
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0], "tavily");

        // exa is changed (different args)
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].0, "exa");
    }

    #[test]
    fn test_secrets_config_defaults() {
        let yaml = "{}";
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(!config.secrets.strict);
        assert!(!config.secrets.providers.bws.enabled);
        assert!(config.secrets.providers.bws.access_token.is_none());
    }

    #[test]
    fn test_resolve_secrets_in_backends() {
        use crate::secrets::resolver::{SecretProvider, SecretResolver};

        struct TestProvider;
        impl SecretProvider for TestProvider {
            fn name(&self) -> &str {
                "test"
            }
            fn resolve(&self, reference: &str) -> anyhow::Result<String> {
                match reference {
                    "key/MY_KEY" => Ok("resolved-value".to_string()),
                    "key/MY_TOKEN" => Ok("resolved-token".to_string()),
                    _ => anyhow::bail!("not found: {reference}"),
                }
            }
        }

        let yaml = r#"
backends:
  my-backend:
    transport: stdio
    command: echo
    env:
      API_KEY: "secretref:test:key/MY_KEY"
    args: ["--token", "secretref:test:key/MY_TOKEN"]
  my-http:
    transport: streamable-http
    url: "https://example.com?token=secretref:test:key/MY_TOKEN"
    headers:
      Authorization: "Bearer secretref:test:key/MY_TOKEN"
"#;
        let mut config: Config = serde_yaml_ng::from_str(yaml).unwrap();

        let mut resolver = SecretResolver::new(false);
        resolver.register(Box::new(TestProvider));
        config.resolve_secrets(&resolver).unwrap();

        let backend = config.backends.get("my-backend").unwrap();
        assert_eq!(backend.env.get("API_KEY").unwrap(), "resolved-value");
        assert_eq!(backend.args[1], "resolved-token");

        let http = config.backends.get("my-http").unwrap();
        assert_eq!(
            http.url.as_deref(),
            Some("https://example.com?token=resolved-token")
        );
        assert_eq!(
            http.headers.get("Authorization").unwrap(),
            "Bearer resolved-token"
        );
    }

    #[test]
    fn test_parse_prerequisite_config() {
        let yaml = r#"
backends:
  vibe-kanban:
    transport: stdio
    command: npx
    args: ["-y", "vibe-kanban@latest", "--mcp"]
    prerequisite:
      command: npx
      args: ["-y", "vibe-kanban@latest"]
      process_match: "vibe-kanban"
      managed: false
      startup_delay: 3s
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let backend = config.backends.get("vibe-kanban").unwrap();
        let prereq = backend.prerequisite.as_ref().unwrap();
        assert_eq!(prereq.command, "npx");
        assert_eq!(prereq.args, vec!["-y", "vibe-kanban@latest"]);
        assert_eq!(prereq.process_match.as_deref(), Some("vibe-kanban"));
        assert!(!prereq.managed);
        assert_eq!(prereq.startup_delay, Duration::from_secs(3));
    }

    #[test]
    fn test_prerequisite_defaults() {
        let yaml = r#"
backends:
  test:
    transport: stdio
    command: echo
    prerequisite:
      command: my-app
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let prereq = config
            .backends
            .get("test")
            .unwrap()
            .prerequisite
            .as_ref()
            .unwrap();
        assert!(!prereq.managed);
        assert_eq!(prereq.startup_delay, Duration::from_secs(2));
        assert!(prereq.process_match.is_none());
        assert!(prereq.args.is_empty());
        assert!(prereq.env.is_empty());
        assert!(prereq.cwd.is_none());
    }

    #[test]
    fn test_prerequisite_none_by_default() {
        let yaml = r#"
backends:
  test:
    transport: stdio
    command: echo
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.backends.get("test").unwrap().prerequisite.is_none());
    }

    #[test]
    fn test_diff_no_changes() {
        let yaml = r#"
backends:
  exa:
    transport: stdio
    command: npx
    args: ["-y", "exa-server"]
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let diff = config.diff_backends(&config);

        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.changed.is_empty());
    }

    #[test]
    fn test_parse_max_concurrent_calls_default() {
        let yaml = r#"
backends:
  test:
    transport: stdio
    command: echo
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let backend = config.backends.get("test").unwrap();
        assert!(backend.max_concurrent_calls.is_none());
        assert_eq!(backend.semaphore_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_parse_max_concurrent_calls_explicit() {
        let yaml = r#"
backends:
  test:
    transport: stdio
    command: echo
    max_concurrent_calls: 25
    semaphore_timeout: 30s
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let backend = config.backends.get("test").unwrap();
        assert_eq!(backend.max_concurrent_calls, Some(25));
        assert_eq!(backend.semaphore_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_validate_max_concurrent_calls_too_high() {
        let yaml = r#"
backends:
  test:
    transport: stdio
    command: echo
    max_concurrent_calls: 99999
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_sandbox_config_defaults() {
        let yaml = "{}";
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.max_concurrent_sandboxes, 8);
        assert_eq!(config.sandbox.timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_retry_config_defaults() {
        let retry = RetryConfig::default();
        // Defaults must match the old hardcoded RETRY_DELAYS = [500ms, 1s, 2s]
        assert_eq!(retry.max_retries, 3);
        assert_eq!(retry.initial_delay, Duration::from_millis(500));
        assert_eq!(retry.max_delay, Duration::from_secs(2));
        assert_eq!(retry.backoff_multiplier, 2.0);

        // Verify the sequence: 500ms, 1s, 2s (matches old constants)
        let mut delay = retry.initial_delay;
        assert_eq!(delay, Duration::from_millis(500));
        delay = delay.mul_f64(retry.backoff_multiplier).min(retry.max_delay);
        assert_eq!(delay, Duration::from_secs(1));
        delay = delay.mul_f64(retry.backoff_multiplier).min(retry.max_delay);
        assert_eq!(delay, Duration::from_secs(2));
    }

    #[test]
    fn test_custom_retry_parsing() {
        let yaml = r#"
backends:
  github:
    transport: stdio
    command: echo
    retry:
      max_retries: 5
      initial_delay: 1s
      max_delay: 10s
      backoff_multiplier: 3.0
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let backend = config.backends.get("github").unwrap();
        assert_eq!(backend.retry.max_retries, 5);
        assert_eq!(backend.retry.initial_delay, Duration::from_secs(1));
        assert_eq!(backend.retry.max_delay, Duration::from_secs(10));
        assert_eq!(backend.retry.backoff_multiplier, 3.0);
    }

    #[test]
    fn test_rate_limit_config_parsing() {
        let yaml = r#"
backends:
  github:
    transport: stdio
    command: echo
    rate_limit:
      max_calls: 100
      window: 60s
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let backend = config.backends.get("github").unwrap();
        let rate_limit = backend.rate_limit.as_ref().unwrap();
        assert_eq!(rate_limit.max_calls, 100);
        assert_eq!(rate_limit.window, Duration::from_secs(60));
    }

    #[test]
    fn test_no_rate_limit_default() {
        let yaml = r#"
backends:
  test:
    transport: stdio
    command: echo
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let backend = config.backends.get("test").unwrap();
        assert!(backend.rate_limit.is_none());
    }

    #[test]
    fn test_health_config_extensions_defaults() {
        let config = HealthConfig::default();
        assert_eq!(config.restart_initial_backoff, Duration::from_secs(1));
        assert_eq!(config.restart_max_backoff, Duration::from_secs(30));
        assert_eq!(config.restart_timeout, Duration::from_secs(30));
        assert_eq!(config.recovery_multiplier, 3);
        assert_eq!(config.drain_timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_health_config_custom_parsing() {
        let yaml = r#"
health:
  interval: 30s
  restart_initial_backoff: 2s
  restart_max_backoff: 60s
  restart_timeout: 45s
  recovery_multiplier: 5
  drain_timeout: 15s
backends: {}
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.health.restart_initial_backoff, Duration::from_secs(2));
        assert_eq!(config.health.restart_max_backoff, Duration::from_secs(60));
        assert_eq!(config.health.restart_timeout, Duration::from_secs(45));
        assert_eq!(config.health.recovery_multiplier, 5);
        assert_eq!(config.health.drain_timeout, Duration::from_secs(15));
    }
}
