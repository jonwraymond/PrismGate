use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::{Instant, timeout};
use tracing::{debug, error, info, warn};

use crate::backend::{BackendManager, BackendState};
use crate::config::HealthConfig;
use crate::registry::ToolRegistry;

/// Per-backend health tracking state.
#[allow(dead_code)]
struct BackendHealth {
    name: String, // kept for debug logging
    consecutive_failures: u32,
    last_check: Option<Instant>,
    last_restart: Option<Instant>,
    restart_count: u32,
    restart_window_start: Option<Instant>,
    circuit_open_since: Option<Instant>,
}

impl BackendHealth {
    fn new(name: String) -> Self {
        Self {
            name,
            consecutive_failures: 0,
            last_check: None,
            last_restart: None,
            restart_count: 0,
            restart_window_start: None,
            circuit_open_since: None,
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.circuit_open_since = None;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
    }

    /// Exponential backoff for restarts using configurable initial/max values.
    fn restart_backoff(&self, config: &HealthConfig) -> Duration {
        let multiplier = 2u64.saturating_pow(self.restart_count.min(5));
        let backoff = config.restart_initial_backoff * multiplier as u32;
        backoff.min(config.restart_max_backoff)
    }

    fn should_restart(&self, config: &HealthConfig) -> bool {
        // Check restart window: reset count if window expired
        if let Some(window_start) = self.restart_window_start
            && window_start.elapsed() > config.restart_window
        {
            return true; // Window expired, allow restart with reset count
        }
        self.restart_count < config.max_restarts
    }
}

/// Runs periodic health checks on all backends.
/// Handles circuit breaking and auto-restart of failed stdio backends.
pub async fn run_health_checker(
    manager: Arc<BackendManager>,
    registry: Arc<ToolRegistry>,
    config: HealthConfig,
    shutdown: Arc<Notify>,
    cache_path: PathBuf,
) {
    let interval = config.interval;

    info!(
        interval_secs = interval.as_secs(),
        failure_threshold = config.failure_threshold,
        max_restarts = config.max_restarts,
        "health checker started"
    );

    // Track per-backend health state
    let mut health_map: std::collections::HashMap<String, BackendHealth> =
        std::collections::HashMap::new();

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {},
            _ = shutdown.notified() => {
                info!("health checker shutting down");
                return;
            }
        }

        // Get current backend list
        let statuses = manager.get_all_status();

        // Ensure all backends have health entries and mark check time
        for status in &statuses {
            health_map
                .entry(status.name.clone())
                .or_insert_with(|| BackendHealth::new(status.name.clone()))
                .last_check = Some(Instant::now());
        }

        // Phase 1: Ping all healthy backends concurrently
        let healthy_names: Vec<String> = statuses
            .iter()
            .filter(|s| s.state == BackendState::Healthy)
            .map(|s| s.name.clone())
            .collect();

        if !healthy_names.is_empty() {
            // Stagger pings across 80% of the interval to avoid thundering herd
            let stagger_delay = if healthy_names.len() > 1 {
                interval.mul_f64(0.8) / healthy_names.len() as u32
            } else {
                Duration::ZERO
            };

            let ping_futures: Vec<_> = healthy_names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let mgr = Arc::clone(&manager);
                    let name = name.clone();
                    let ping_timeout = config.timeout;
                    let delay = stagger_delay * i as u32;
                    async move {
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                        let result = timeout(ping_timeout, mgr.ping_backend(&name)).await;
                        (name, result)
                    }
                })
                .collect();

            let ping_results = futures::future::join_all(ping_futures).await;

            for (name, result) in ping_results {
                let health = health_map.get_mut(&name).unwrap();
                match result {
                    Ok(Ok(())) => {
                        if health.consecutive_failures > 0 {
                            info!(backend = %name, "backend recovered");
                        }
                        health.record_success();
                    }
                    Ok(Err(e)) => {
                        health.record_failure();
                        warn!(
                            backend = %name,
                            failures = health.consecutive_failures,
                            error = %e,
                            "health check failed"
                        );

                        if health.consecutive_failures >= config.failure_threshold {
                            warn!(
                                backend = %name,
                                "circuit breaker tripped after {} consecutive failures",
                                health.consecutive_failures
                            );
                            health.circuit_open_since = Some(Instant::now());
                            manager.set_backend_state(&name, BackendState::Unhealthy);
                        }
                    }
                    Err(_) => {
                        health.record_failure();
                        warn!(
                            backend = %name,
                            failures = health.consecutive_failures,
                            timeout_secs = config.timeout.as_secs(),
                            "health check timed out"
                        );

                        if health.consecutive_failures >= config.failure_threshold {
                            warn!(
                                backend = %name,
                                "circuit breaker tripped after {} consecutive failures",
                                health.consecutive_failures
                            );
                            health.circuit_open_since = Some(Instant::now());
                            manager.set_backend_state(&name, BackendState::Unhealthy);
                        }
                    }
                }
            }
        }

        // Phase 2: Handle stopped/unhealthy backends
        // Restarts stay sequential to avoid thundering herd.
        // Circuit-open backends (tracked via circuit_open_since) get half-open probes.
        for status in &statuses {
            match status.state {
                BackendState::Stopped | BackendState::Unhealthy => {
                    let health = health_map.get_mut(&status.name).unwrap();

                    // If circuit is open, try half-open probe before restarting
                    if let Some(opened) = health.circuit_open_since {
                        let recovery_window = config.interval * config.recovery_multiplier;
                        if opened.elapsed() >= recovery_window {
                            debug!(backend = %status.name, "circuit half-open, probing");
                            match timeout(config.timeout, manager.ping_backend(&status.name)).await
                            {
                                Ok(Ok(())) => {
                                    info!(backend = %status.name, "circuit breaker reset — backend recovered");
                                    health.record_success();
                                    manager.set_backend_state(&status.name, BackendState::Healthy);
                                    continue;
                                }
                                Ok(Err(e)) => {
                                    debug!(
                                        backend = %status.name,
                                        error = %e,
                                        "half-open probe failed, circuit stays open"
                                    );
                                    health.circuit_open_since = Some(Instant::now());
                                    continue;
                                }
                                Err(_) => {
                                    debug!(
                                        backend = %status.name,
                                        "half-open probe timed out, circuit stays open"
                                    );
                                    health.circuit_open_since = Some(Instant::now());
                                    continue;
                                }
                            }
                        } else {
                            // Still within recovery window, skip this backend
                            continue;
                        }
                    }

                    // No circuit open — try auto-restart with backoff
                    if health.should_restart(&config) {
                        let backoff = health.restart_backoff(&config);

                        // Check if enough time has passed since last restart
                        let can_restart = health
                            .last_restart
                            .map(|t| t.elapsed() >= backoff)
                            .unwrap_or(true);

                        if can_restart {
                            // Ensure prerequisite is running before restart
                            if let Some(backend_config) =
                                manager.get_backend_config(&status.name).await
                                && let Some(prereq) = &backend_config.prerequisite
                                && let Err(e) = crate::backend::prerequisite::ensure_prerequisite(
                                    &status.name,
                                    prereq,
                                )
                                .await
                            {
                                error!(backend = %status.name, error = %e, "prerequisite failed before restart");
                                health.restart_count += 1;
                                health.last_restart = Some(Instant::now());
                                continue;
                            }

                            info!(
                                backend = %status.name,
                                attempt = health.restart_count + 1,
                                max = config.max_restarts,
                                "attempting auto-restart"
                            );

                            // Reset restart window if needed
                            if health
                                .restart_window_start
                                .map(|t| t.elapsed() > config.restart_window)
                                .unwrap_or(true)
                            {
                                health.restart_count = 0;
                                health.restart_window_start = Some(Instant::now());
                            }

                            match timeout(
                                config.restart_timeout,
                                manager.restart_backend(&status.name, &registry),
                            )
                            .await
                            {
                                Ok(Ok(tool_count)) => {
                                    info!(
                                        backend = %status.name,
                                        tools = tool_count,
                                        "backend restarted successfully"
                                    );
                                    health.record_success();
                                    health.restart_count += 1;
                                    health.last_restart = Some(Instant::now());

                                    // Persist updated tool registry to cache
                                    let reg = Arc::clone(&registry);
                                    let cp = cache_path.clone();
                                    tokio::spawn(async move {
                                        crate::cache::save(&cp, &reg).await;
                                    });
                                }
                                Ok(Err(e)) => {
                                    error!(
                                        backend = %status.name,
                                        error = %e,
                                        "auto-restart failed"
                                    );
                                    health.restart_count += 1;
                                    health.last_restart = Some(Instant::now());
                                }
                                Err(_) => {
                                    error!(
                                        backend = %status.name,
                                        timeout_secs = config.restart_timeout.as_secs(),
                                        "auto-restart timed out"
                                    );
                                    health.restart_count += 1;
                                    health.last_restart = Some(Instant::now());
                                }
                            }
                        } else {
                            debug!(
                                backend = %status.name,
                                backoff_secs = backoff.as_secs(),
                                "waiting for backoff before restart"
                            );
                        }
                    } else {
                        warn!(
                            backend = %status.name,
                            restarts = health.restart_count,
                            "max restarts exceeded, not restarting"
                        );
                    }
                }

                // Healthy already handled in Phase 1; Starting — just wait
                _ => {}
            }
        }

        // Phase 3: Retry configured backends that failed initial handshake.
        // These backends have configs stored but never entered the DashMap.
        let configured_names = manager.get_configured_names().await;
        let running_names: std::collections::HashSet<&String> =
            statuses.iter().map(|s| &s.name).collect();

        for name in &configured_names {
            if running_names.contains(name) {
                continue; // Already in DashMap, handled by Phase 1/2
            }

            let health = health_map
                .entry(name.clone())
                .or_insert_with(|| BackendHealth::new(name.clone()));

            if !health.should_restart(&config) {
                warn!(
                    backend = %name,
                    restarts = health.restart_count,
                    "pending backend: max restarts exceeded, not retrying"
                );
                continue;
            }

            let backoff = health.restart_backoff(&config);
            let can_retry = health
                .last_restart
                .map(|t| t.elapsed() >= backoff)
                .unwrap_or(true);

            if !can_retry {
                debug!(
                    backend = %name,
                    backoff_secs = backoff.as_secs(),
                    "pending backend: waiting for backoff before retry"
                );
                continue;
            }

            // Ensure prerequisite is running before starting pending backend
            if let Some(backend_config) = manager.get_backend_config(name).await
                && let Some(prereq) = &backend_config.prerequisite
                && let Err(e) =
                    crate::backend::prerequisite::ensure_prerequisite(name, prereq).await
            {
                error!(backend = %name, error = %e, "prerequisite failed before pending backend start");
                health.restart_count += 1;
                health.last_restart = Some(Instant::now());
                continue;
            }

            info!(
                backend = %name,
                attempt = health.restart_count + 1,
                max = config.max_restarts,
                "attempting to start pending backend"
            );

            // Reset restart window if needed
            if health
                .restart_window_start
                .map(|t| t.elapsed() > config.restart_window)
                .unwrap_or(true)
            {
                health.restart_count = 0;
                health.restart_window_start = Some(Instant::now());
            }

            match timeout(
                config.restart_timeout,
                manager.try_start_from_config(name, &registry),
            )
            .await
            {
                Ok(Ok(tool_count)) => {
                    info!(
                        backend = %name,
                        tools = tool_count,
                        "pending backend started successfully"
                    );
                    health.record_success();
                    health.restart_count += 1;
                    health.last_restart = Some(Instant::now());

                    // Persist updated tool registry to cache
                    let reg = Arc::clone(&registry);
                    let cp = cache_path.clone();
                    tokio::spawn(async move {
                        crate::cache::save(&cp, &reg).await;
                    });
                }
                Ok(Err(e)) => {
                    error!(
                        backend = %name,
                        error = %e,
                        "pending backend start failed"
                    );
                    health.restart_count += 1;
                    health.last_restart = Some(Instant::now());
                }
                Err(_) => {
                    error!(
                        backend = %name,
                        timeout_secs = config.restart_timeout.as_secs(),
                        "pending backend start timed out"
                    );
                    health.restart_count += 1;
                    health.last_restart = Some(Instant::now());
                }
            }
        }

        // Clean up health entries for removed backends.
        // Retain entries for backends that are either running (in DashMap) or
        // configured (pending — failed initial handshake but still in config).
        let configured_set: std::collections::HashSet<String> =
            configured_names.into_iter().collect();
        let current_names: std::collections::HashSet<String> =
            statuses.iter().map(|s| s.name.clone()).collect();
        health_map.retain(|name, _| current_names.contains(name) || configured_set.contains(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restart_backoff_default() {
        let config = HealthConfig::default();
        let mut h = BackendHealth::new("test".to_string());
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(1));

        h.restart_count = 1;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(2));

        h.restart_count = 2;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(4));

        h.restart_count = 3;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(8));

        h.restart_count = 4;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(16));

        h.restart_count = 5;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(30)); // capped

        h.restart_count = 10;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(30)); // still capped
    }

    #[test]
    fn test_restart_backoff_custom() {
        let config = HealthConfig {
            restart_initial_backoff: Duration::from_secs(2),
            restart_max_backoff: Duration::from_secs(60),
            ..Default::default()
        };
        let mut h = BackendHealth::new("test".to_string());
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(2));

        h.restart_count = 1;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(4));

        h.restart_count = 2;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(8));

        h.restart_count = 5;
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(60)); // capped at custom max
    }

    #[test]
    fn test_should_restart() {
        let config = HealthConfig::default(); // max_restarts = 5
        let mut h = BackendHealth::new("test".to_string());

        assert!(h.should_restart(&config)); // 0 < 5

        h.restart_count = 4;
        assert!(h.should_restart(&config)); // 4 < 5

        h.restart_count = 5;
        h.restart_window_start = Some(Instant::now());
        assert!(!h.should_restart(&config)); // 5 >= 5, window not expired
    }

    #[test]
    fn test_record_success_resets_failures() {
        let mut h = BackendHealth::new("test".to_string());
        h.consecutive_failures = 5;
        h.circuit_open_since = Some(Instant::now());

        h.record_success();
        assert_eq!(h.consecutive_failures, 0);
        assert!(h.circuit_open_since.is_none());
    }

    #[test]
    fn test_pending_backend_backoff_tracking() {
        // Simulate Phase 3 behavior: a pending backend that fails repeatedly
        // should track restart_count and respect backoff/max_restarts.
        let config = HealthConfig::default(); // max_restarts = 5
        let mut h = BackendHealth::new("pending-backend".to_string());

        // First attempt is always allowed
        assert!(h.should_restart(&config));
        assert!(h.last_restart.is_none());

        // Simulate a failed start attempt
        h.restart_count = 1;
        h.restart_window_start = Some(Instant::now());
        h.last_restart = Some(Instant::now());

        // Backoff should be 2s after first failure
        assert_eq!(h.restart_backoff(&config), Duration::from_secs(2));

        // Can't restart immediately (backoff not elapsed)
        let can_retry = h
            .last_restart
            .map(|t| t.elapsed() >= h.restart_backoff(&config))
            .unwrap_or(true);
        assert!(!can_retry);

        // After enough failures, max_restarts should block further retries
        h.restart_count = 5;
        assert!(!h.should_restart(&config)); // 5 >= 5, window still active
    }

    #[test]
    fn test_pending_backend_success_resets_state() {
        // When a pending backend finally starts, record_success should
        // clear failure tracking so it's treated as a healthy backend.
        let mut h = BackendHealth::new("pending-backend".to_string());
        h.consecutive_failures = 3;
        h.circuit_open_since = Some(Instant::now());

        // Simulate successful start
        h.record_success();
        h.restart_count += 1;
        h.last_restart = Some(Instant::now());

        assert_eq!(h.consecutive_failures, 0);
        assert!(h.circuit_open_since.is_none());
    }
}
