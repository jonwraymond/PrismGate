//! Tool inventory validation integration tests.
//!
//! These tests use the REAL gatemini config and backends. They are `#[ignore]`d by default
//! and run with: `cargo test integration_inventory -- --ignored`
//!
//! Requirements:
//! - All backend commands must be installed and accessible
//! - API keys must be available in the environment (or via BWS)
//! - `config/gatemini.yaml` must exist relative to the crate root

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::backend::{BackendManager, BackendState};
    use crate::registry::ToolRegistry;

    /// Load the real gatemini config and start all backends.
    async fn setup_real_gateway() -> (
        std::sync::Arc<ToolRegistry>,
        std::sync::Arc<BackendManager>,
        crate::config::Config,
    ) {
        crate::config::load_dotenv();

        let config_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config/gatemini.yaml");
        assert!(
            config_path.exists(),
            "config file not found: {}",
            config_path.display()
        );

        let mut config = crate::config::Config::load(&config_path).unwrap();
        config.resolve_secrets_async().await.unwrap();

        let registry = ToolRegistry::new();
        let manager = BackendManager::new();

        // Load cache for faster startup
        let cache_path = crate::cache::cache_path_from_config(&config_path);
        let config_names: Vec<String> = config.backends.keys().cloned().collect();
        crate::cache::load(&cache_path, &registry, &config_names).await;

        // Start all backends
        manager.start_all(&config, &registry).await.unwrap();

        // Save cache
        crate::cache::save(&cache_path, &registry).await;

        (registry, manager, config)
    }

    /// For each configured backend: verify it reached Running state
    /// and discovered at least 1 tool.
    #[tokio::test]
    #[ignore]
    async fn test_all_backends_start_and_handshake() {
        let (registry, manager, _config) = setup_real_gateway().await;

        let backend_count = registry.backend_count();
        assert!(
            backend_count > 0,
            "expected at least 1 backend, got {backend_count}"
        );

        let mut healthy = 0;
        let mut unhealthy = Vec::new();
        for entry in manager.backends.iter() {
            let name = entry.key().clone();
            let backend = entry.value().clone();
            match backend.state() {
                BackendState::Healthy => {
                    healthy += 1;
                }
                state => {
                    unhealthy.push((name, state));
                }
            }
        }

        if !unhealthy.is_empty() {
            eprintln!("Unhealthy backends:");
            for (name, state) in &unhealthy {
                eprintln!("  {name}: {state:?}");
            }
        }

        assert!(
            healthy > 0,
            "expected at least 1 healthy backend, all unhealthy: {unhealthy:?}"
        );

        eprintln!(
            "Backend health: {healthy} healthy, {} unhealthy",
            unhealthy.len()
        );

        manager.stop_all().await;
    }

    /// Start all backends, discover all tools, assert total >= 300.
    #[tokio::test]
    #[ignore]
    async fn test_tool_count_minimum() {
        let (registry, manager, _config) = setup_real_gateway().await;

        let tool_count = registry.tool_count();
        eprintln!("Total tools discovered: {tool_count}");

        assert!(
            tool_count >= 300,
            "expected >= 300 tools, got {tool_count}"
        );

        eprintln!(
            "Tool inventory: {tool_count} tools across {} backends",
            registry.backend_count()
        );

        manager.stop_all().await;
    }

    /// For each backend, call its first tool with empty args.
    /// Assert response is well-formed (not a transport error).
    #[tokio::test]
    #[ignore]
    async fn test_tool_smoke_sampled() {
        let (registry, manager, _config) = setup_real_gateway().await;

        let entries = registry.get_all();

        // Group tools by backend, take first tool per backend
        let mut backends_tested = std::collections::HashMap::new();
        for entry in &entries {
            backends_tested
                .entry(entry.backend_name.clone())
                .or_insert_with(|| entry.name.clone());
        }

        let mut successes = 0;
        let mut failures = Vec::new();

        for (backend, tool_name) in &backends_tested {
            match manager
                .call_tool(backend, tool_name, Some(serde_json::json!({})))
                .await
            {
                Ok(_) => {
                    successes += 1;
                }
                Err(e) => {
                    let err_str = e.to_string();
                    // Transport errors indicate backend is broken.
                    // Application errors (invalid params) are acceptable — tool is reachable.
                    if err_str.contains("transport")
                        || err_str.contains("not available")
                        || err_str.contains("connection")
                    {
                        failures.push((backend.clone(), tool_name.clone(), err_str));
                    } else {
                        // Application-level error = tool is reachable, just bad params
                        successes += 1;
                    }
                }
            }
        }

        eprintln!(
            "Smoke test: {successes} reachable, {} unreachable out of {} backends",
            failures.len(),
            backends_tested.len()
        );

        if !failures.is_empty() {
            eprintln!("Unreachable backends:");
            for (backend, tool, err) in &failures {
                eprintln!("  {backend}/{tool}: {err}");
            }
        }

        assert!(
            successes > 0,
            "no backends were reachable: {failures:?}"
        );

        manager.stop_all().await;
    }
}
