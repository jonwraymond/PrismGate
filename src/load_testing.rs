//! Load testing utilities for PrismGate / gatemini.
//!
//! This module provides reusable helpers for building and running load tests
//! against `BackendManager`.  It is compiled for all targets (not gated on
//! `#[cfg(test)]`) so that:
//! - Criterion benches can import types/functions from here without
//!   `#[cfg(bench)]` / `#[cfg(test)]` gating friction.
//! - Integration tests and external load-test harnesses can also call these
//!   utilities directly.
//!
//! Key types:
//! - [`LoadTestConfig`] — configure number of calls, concurrency level, per-call
//!   delay, and whether errors should be injected.
//! - [`run_load_test`] — execute the load test and return [`LoadTestResult`].
//! - [`LoadTestResult`] — aggregate stats: total calls, success/failure counts,
//!   elapsed time, throughput (calls/s), and per-backend max-seen concurrency.
//!
//! Usage in a criterion bench:
//! ```ignore
//! use gatemini::load_testing::{LoadTestConfig, run_load_test};
//! use std::time::Duration;
//!
//! # async fn example(manager: Arc<BackendManager>) {
//! let result = run_load_test(manager, LoadTestConfig {
//!     num_calls: 200,
//!     max_concurrent: 50,
//!     call_delay: Duration::from_millis(5),
//!     ..Default::default()
//! }).await;
//! println!("throughput: {:.1} calls/s", result.throughput());
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::backend::{BackendManager, BackendState, STATE_HEALTHY};
use crate::config::{BackendConfig, InstanceMode, PoolConfig, RetryConfig, Transport};
use crate::registry::ToolRegistry;

// ─── Load test configuration ──────────────────────────────────────────

/// Configuration for a single load-test scenario.
#[derive(Clone, Debug)]
pub struct LoadTestConfig {
    /// Total number of tool calls to fire.
    pub num_calls: usize,
    /// Maximum number of calls to have in-flight simultaneously.
    pub max_concurrent: usize,
    /// Delay that each mock backend introduces per `call_tool`.
    pub call_delay: Duration,
    /// If true, calls to `error_tool` will fail, exercising the error path.
    pub inject_errors: bool,
    /// Maximum calls any single backend will accept concurrently
    /// (sets the semaphore permit count per backend).
    pub per_backend_limit: u32,
    /// Number of distinct mock backends to register.
    pub num_backends: usize,
}

impl Default for LoadTestConfig {
    fn default() -> Self {
        Self {
            num_calls: 100,
            max_concurrent: 20,
            call_delay: Duration::from_millis(5),
            inject_errors: false,
            per_backend_limit: 0, // unlimited
            num_backends: 1,
        }
    }
}

// ─── Results ───────────────────────────────────────────────────────────

/// Summary of a completed load test run.
#[derive(Clone, Debug)]
pub struct LoadTestResult {
    /// Total wall-clock time from first call to last completion.
    pub elapsed: Duration,
    /// Number of successful calls.
    pub successes: usize,
    /// Number of failed calls (backend returned error or timed out).
    pub failures: usize,
    /// Per-backend max-seen-concurrent snapshot at end of run.
    pub per_backend_max_conc: HashMap<String, usize>,
    /// Backend name -> total calls routed to that backend.
    pub backend_call_counts: HashMap<String, usize>,
}

impl LoadTestResult {
    /// Total calls (successes + failures).
    pub fn total_calls(&self) -> usize {
        self.successes + self.failures
    }

    /// Calls per second over the full elapsed wall time.
    pub fn throughput(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs == 0.0 {
            0.0
        } else {
            self.total_calls() as f64 / secs
        }
    }

    /// Success rate as a fraction (0.0 – 1.0).
    pub fn success_rate(&self) -> f64 {
        let total = self.total_calls();
        if total == 0 {
            1.0
        } else {
            self.successes as f64 / total as f64
        }
    }
}

// ─── Mock backends (mirrors testutil::MockBackend but public for benches) ──

/// Mock backend that supports load-test scenarios.
///
/// Tracks concurrent call count and max-seen concurrency, honours a semaphore
/// (set externally by `register_mock_backend`), and optionally injects errors.
pub struct LoadTestMockBackend {
    pub name: String,
    pub delay: Duration,
    pub max_permit: usize,
    pub inject_error: bool,
    pub total_calls: Arc<AtomicUsize>,
    pub in_flight: Arc<AtomicUsize>,
    pub max_seen_concurrent: Arc<AtomicUsize>,
}

impl LoadTestMockBackend {
    pub fn new(
        name: impl Into<String>,
        delay: Duration,
        max_permit: usize,
        inject_error: bool,
    ) -> Self {
        Self {
            name: name.into(),
            delay,
            max_permit,
            inject_error,
            total_calls: Arc::new(AtomicUsize::new(0)),
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_seen_concurrent: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Record a call; update max-seen-concurrent.
    fn record_call_start(&self) -> usize {
        let n = self.total_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let inflight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        loop {
            let current = self.max_seen_concurrent.load(Ordering::SeqCst);
            if inflight <= current {
                break;
            }
            if self
                .max_seen_concurrent
                .compare_exchange(current, inflight, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
        inflight
    }

    fn record_call_end(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Boxed trait object used for BackendManager registration.
type LoadTestBackendDyn = Arc<dyn gatemini::backend::Backend>;

// ─── Implementation of `Backend` for `LoadTestMockBackend` ─────────────

// SAFETY: `&self` is `Send + Sync` because all fields are `Arc` or plain data.
unsafe impl Send for LoadTestMockBackend {}
unsafe impl Sync for LoadTestMockBackend {}

impl gatemini::backend::Backend for LoadTestMockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn call_tool(
        &self,
        _tool_name: &str,
        _arguments: Option<Value>,
    ) -> anyhow::Result<Value> {
        let _concurrent = self.record_call_start();

        if self.inject_error {
            self.record_call_end();
            return Err(anyhow::anyhow!("injected error"));
        }

        tokio::time::sleep(self.delay).await;
        self.record_call_end();

        Ok(Value::String(format!(
            "ok:{}:{}",
            self.name,
            self.total_calls.load(Ordering::SeqCst)
        )))
    }

    async fn discover_tools(&self) -> anyhow::Result<Vec<crate::registry::ToolEntry>> {
        Ok(vec![])
    }

    fn is_available(&self) -> bool {
        true
    }

    fn state(&self) -> BackendState {
        STATE_HEALTHY
    }

    fn set_state(&self, _state: BackendState) {}
}

// ─── Builder helper ────────────────────────────────────────────────────

/// Register `num_backends` mock backends into `manager` with the given config.
/// Returns the vector of mock handles (one per backend) so callers can inspect
/// counters after the run.
fn register_mock_backends(
    manager: &Arc<BackendManager>,
    cfg: &LoadTestConfig,
) -> Vec<Arc<LoadTestMockBackend>> {
    let mut mocks = Vec::new();
    for i in 0..cfg.num_backends {
        let name = format!("load-backend-{}", i);
        let mock = Arc::new(LoadTestMockBackend::new(
            &name,
            cfg.call_delay,
            cfg.per_backend_limit,
            cfg.inject_errors,
        ));
        mocks.push(Arc::clone(&mock));

        // Register as virtual backend (bypasses child-process management)
        manager.register_virtual_backend(&name, Arc::clone(&mock) as LoadTestBackendDyn);

        // Set config + semaphore so BackendManager::call_tool sees them
        let mut configs = manager
            .configs
            .try_write_for(Duration::from_secs(1))
            .unwrap();
        configs.insert(
            name.clone(),
            BackendConfig {
                transport: Transport::Stdio,
                namespace: None,
                command: None,
                args: Vec::new(),
                env: std::collections::HashMap::new(),
                cwd: None,
                url: None,
                headers: std::collections::HashMap::new(),
                timeout: Duration::from_secs(10),
                required_keys: Vec::new(),
                max_concurrent_calls: if cfg.per_backend_limit > 0 {
                    Some(cfg.per_backend_limit)
                } else {
                    None
                },
                semaphore_timeout: Duration::from_secs(30),
                retry: RetryConfig {
                    max_retries: 1,
                    initial_delay: Duration::from_millis(10),
                    backoff_multiplier: 1.5,
                    max_delay: Duration::from_millis(50),
                },
                prerequisite: None,
                rate_limit: None,
                tags: Vec::new(),
                fallback_chain: Vec::new(),
                tools: None,
                adapter_file: None,
                health_check: None,
                instance_mode: InstanceMode::Shared,
                pool: PoolConfig::default(),
                shutdown_grace_period: Duration::from_secs(5),
                max_memory_mb: None,
            },
        );
        drop(configs);

        if cfg.per_backend_limit > 0 {
            manager.call_semaphores.insert(
                name,
                Arc::new(Semaphore::new(cfg.per_backend_limit as usize)),
            );
        }
    }
    mocks
}

// ─── Core runner ───────────────────────────────────────────────────────

/// Run a load test against a `BackendManager`.
///
/// Registers mock backends, fires `cfg.num_calls` concurrent calls (limited by
/// `cfg.max_concurrent` via an outer semaphore), and collects stats.
pub async fn run_load_test(manager: Arc<BackendManager>, cfg: LoadTestConfig) -> LoadTestResult {
    let _registry = Arc::new(ToolRegistry::new());
    let mocks = register_mock_backends(&manager, &cfg);

    let start = Instant::now();
    let total_ok = Arc::new(AtomicUsize::new(0));
    let total_err = Arc::new(AtomicUsize::new(0));
    let outer_sem = Arc::new(Semaphore::new(cfg.max_concurrent));

    let mut js = JoinSet::new();
    for i in 0..cfg.num_calls {
        let mgr = Arc::clone(&manager);
        let sem = Arc::clone(&outer_sem);
        let ok_counter = Arc::clone(&total_ok);
        let err_counter = Arc::clone(&total_err);
        let backend_idx = i % cfg.num_backends;
        let backend_name = mocks[backend_idx].name.clone();

        js.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            let result = mgr
                .call_tool(&backend_name, "echo_tool", Some(Value::Null), None)
                .await;
            match result {
                Ok(_) => {
                    ok_counter.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => {
                    err_counter.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
    }

    while let Some(res) = js.join_next().await {
        let _ = res;
    }

    let elapsed = start.elapsed();

    let mut per_backend_max_conc = HashMap::new();
    let mut backend_call_counts = HashMap::new();
    for mock in &mocks {
        per_backend_max_conc.insert(
            mock.name.clone(),
            mock.max_seen_concurrent.load(Ordering::SeqCst),
        );
        backend_call_counts.insert(mock.name.clone(), mock.total_calls.load(Ordering::SeqCst));
    }

    LoadTestResult {
        elapsed,
        successes: total_ok.load(Ordering::SeqCst),
        failures: total_err.load(Ordering::SeqCst),
        per_backend_max_conc,
        backend_call_counts,
    }
}

// ─── Convenience: build a BackendManager for standalone use ────────────

/// Create a bare `BackendManager` with no backends registered.
pub fn new_manager() -> Arc<BackendManager> {
    Arc::new(BackendManager::new())
}
