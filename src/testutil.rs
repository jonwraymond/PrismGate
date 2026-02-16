//! Test utilities for gatemini — mock backends, helpers, and test fixtures.
//!
//! This module is only compiled under `#[cfg(test)]` and provides a controllable
//! mock MCP backend that implements the `Backend` trait directly. This enables
//! testing `BackendManager`, semaphores, and concurrency without real child
//! processes or network connections.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::backend::{
    Backend, BackendState, STATE_HEALTHY, is_available_from_atomic, state_from_atomic, store_state,
};
use crate::registry::ToolEntry;

/// A controllable mock MCP backend for testing.
///
/// Tracks concurrent call count, records call parameters, supports configurable
/// delays and error injection. Implements `Backend` trait directly — no rmcp
/// protocol overhead, ideal for unit testing `BackendManager` and semaphores.
///
/// ## Tools registered:
/// - `echo_tool`: returns args as JSON (response verification)
/// - `slow_tool`: sleeps `call_delay`, returns (concurrency testing)
/// - `error_tool`: always returns error
/// - `counter_tool`: returns current concurrent call count
pub struct MockBackend {
    name: String,
    state: AtomicU8,
    /// Current number of concurrent calls in flight.
    concurrent_calls: AtomicUsize,
    /// Peak concurrent calls observed.
    max_seen_concurrent: AtomicUsize,
    /// Per-call delay for concurrency testing.
    call_delay: Duration,
    /// Whether to inject errors on all calls.
    inject_error: AtomicBool,
    /// Record of all call parameters: (tool_name, arguments).
    call_log: Mutex<Vec<(String, Option<Value>)>>,
    /// Tools this mock provides.
    tools: Vec<ToolEntry>,
}

impl MockBackend {
    /// Create a new mock backend with the given name and per-call delay.
    pub fn new(name: &str, call_delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            state: AtomicU8::new(STATE_HEALTHY),
            concurrent_calls: AtomicUsize::new(0),
            max_seen_concurrent: AtomicUsize::new(0),
            call_delay,
            inject_error: AtomicBool::new(false),
            call_log: Mutex::new(Vec::new()),
            tools: vec![
                ToolEntry {
                    name: "echo_tool".to_string(),
                    original_name: "echo_tool".to_string(),
                    description: "Returns args as JSON".to_string(),
                    backend_name: name.to_string(),
                    input_schema: serde_json::json!({"type": "object", "properties": {}}),
                    tags: Vec::new(),
                },
                ToolEntry {
                    name: "slow_tool".to_string(),
                    original_name: "slow_tool".to_string(),
                    description: "Sleeps call_delay then returns".to_string(),
                    backend_name: name.to_string(),
                    input_schema: serde_json::json!({"type": "object", "properties": {}}),
                    tags: Vec::new(),
                },
                ToolEntry {
                    name: "error_tool".to_string(),
                    original_name: "error_tool".to_string(),
                    description: "Always returns an error".to_string(),
                    backend_name: name.to_string(),
                    input_schema: serde_json::json!({"type": "object", "properties": {}}),
                    tags: Vec::new(),
                },
                ToolEntry {
                    name: "counter_tool".to_string(),
                    original_name: "counter_tool".to_string(),
                    description: "Returns current concurrent call count".to_string(),
                    backend_name: name.to_string(),
                    input_schema: serde_json::json!({"type": "object", "properties": {}}),
                    tags: Vec::new(),
                },
            ],
        })
    }

    /// Enable or disable error injection for all calls.
    pub fn set_inject_error(&self, inject: bool) {
        self.inject_error.store(inject, Ordering::SeqCst);
    }

    /// Get the peak concurrent call count observed.
    pub fn max_seen_concurrent(&self) -> usize {
        self.max_seen_concurrent.load(Ordering::SeqCst)
    }

    /// Get the current number of in-flight calls.
    #[allow(dead_code)]
    pub fn current_concurrent(&self) -> usize {
        self.concurrent_calls.load(Ordering::SeqCst)
    }

    /// Get a snapshot of all recorded calls.
    pub async fn call_log(&self) -> Vec<(String, Option<Value>)> {
        self.call_log.lock().await.clone()
    }
}

/// RAII guard that decrements an `AtomicUsize` counter on drop.
/// Ensures the concurrent call counter stays accurate even if the
/// future is cancelled (e.g., via `tokio::select!` or `abort()`).
struct ConcurrencyGuard<'a>(&'a AtomicUsize);

impl Drop for ConcurrencyGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<()> {
        store_state(&self.state, BackendState::Healthy);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        store_state(&self.state, BackendState::Stopped);
        Ok(())
    }

    async fn call_tool(&self, tool_name: &str, arguments: Option<Value>) -> Result<Value> {
        // Track concurrent calls with RAII guard for cancellation safety.
        // If this future is dropped (e.g., via abort or select!), the guard's
        // Drop impl ensures the counter is decremented.
        let current = self.concurrent_calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_seen_concurrent
            .fetch_max(current, Ordering::SeqCst);
        let _guard = ConcurrencyGuard(&self.concurrent_calls);

        // Record the call
        self.call_log
            .lock()
            .await
            .push((tool_name.to_string(), arguments.clone()));

        // Check error injection (global or error_tool)
        if self.inject_error.load(Ordering::SeqCst) || tool_name == "error_tool" {
            anyhow::bail!("injected error for tool '{}'", tool_name);
        }

        // Simulate delay
        if !self.call_delay.is_zero() {
            tokio::time::sleep(self.call_delay).await;
        }

        let result = match tool_name {
            "echo_tool" => arguments.unwrap_or(Value::Null),
            "slow_tool" => {
                serde_json::json!({
                    "status": "completed",
                    "delay_ms": self.call_delay.as_millis() as u64
                })
            }
            "counter_tool" => serde_json::json!({"concurrent": current}),
            _ => serde_json::json!({"tool": tool_name, "status": "ok"}),
        };

        Ok(result)
    }

    async fn discover_tools(&self) -> Result<Vec<ToolEntry>> {
        Ok(self.tools.clone())
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

/// Helper: insert a mock backend into a `BackendManager` and register its tools.
/// Uses no concurrency limit (unlimited) with a 60s semaphore timeout.
pub async fn insert_mock(
    manager: &crate::backend::BackendManager,
    registry: &Arc<crate::registry::ToolRegistry>,
    mock: &Arc<MockBackend>,
) {
    insert_mock_with_config(manager, registry, mock, None, Duration::from_secs(60)).await;
}

/// Helper: insert a mock backend with explicit concurrency config.
/// `max_concurrent`: None = no semaphore (unlimited), Some(0) = no semaphore, Some(n) = limit to n.
pub async fn insert_mock_with_config(
    manager: &crate::backend::BackendManager,
    registry: &Arc<crate::registry::ToolRegistry>,
    mock: &Arc<MockBackend>,
    max_concurrent: Option<u32>,
    semaphore_timeout: Duration,
) {
    let tools = mock.discover_tools().await.unwrap();
    registry.register_backend_tools_namespaced(mock.name(), mock.name(), tools);
    manager.backends.insert(
        mock.name().to_string(),
        Arc::clone(mock) as Arc<dyn Backend>,
    );

    // Create semaphore if configured
    if let Some(max) = max_concurrent
        && max > 0
    {
        manager.call_semaphores.insert(
            mock.name().to_string(),
            Arc::new(tokio::sync::Semaphore::new(max as usize)),
        );
    }
    manager
        .semaphore_timeouts
        .insert(mock.name().to_string(), semaphore_timeout);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_server_echo() {
        let mock = MockBackend::new("test", Duration::ZERO);
        let args = serde_json::json!({"message": "hello", "count": 42});
        let result = mock
            .call_tool("echo_tool", Some(args.clone()))
            .await
            .unwrap();
        assert_eq!(result, args);
    }

    #[tokio::test]
    async fn test_mock_server_delay() {
        let mock = MockBackend::new("test", Duration::from_millis(100));
        let start = std::time::Instant::now();
        let result = mock.call_tool("slow_tool", None).await.unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(90),
            "delay too short: {elapsed:?}"
        );
        assert_eq!(result["status"], "completed");
    }

    #[tokio::test]
    async fn test_mock_server_error() {
        let mock = MockBackend::new("test", Duration::ZERO);

        // error_tool always errors
        let result = mock.call_tool("error_tool", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("injected error"));

        // inject_error makes all tools error
        mock.set_inject_error(true);
        let result = mock.call_tool("echo_tool", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_server_concurrent_tracking() {
        let mock = MockBackend::new("test", Duration::from_millis(200));
        let mock = Arc::clone(&mock);

        let mut handles = Vec::new();
        for _ in 0..5 {
            let m = Arc::clone(&mock);
            handles.push(tokio::spawn(async move {
                m.call_tool("slow_tool", None).await.unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(
            mock.max_seen_concurrent(),
            5,
            "all 5 calls should run concurrently"
        );
    }

    #[tokio::test]
    async fn test_mock_backend_integrates_with_manager() {
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        let mock = MockBackend::new("mock-backend", Duration::ZERO);

        insert_mock(&manager, &registry, &mock).await;

        // Verify tools registered
        assert!(registry.get_by_name("echo_tool").is_some());

        // Call via manager
        let args = serde_json::json!({"key": "value"});
        let result = manager
            .call_tool("mock-backend", "echo_tool", Some(args.clone()))
            .await
            .unwrap();
        assert_eq!(result, args);

        // Verify call was logged
        let log = mock.call_log().await;
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "echo_tool");
    }

    // --- Phase 1: Semaphore tests ---

    #[tokio::test]
    async fn test_semaphore_limits_concurrent_calls() {
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        let mock = MockBackend::new("sem-test", Duration::from_millis(500));

        // max=2: only 2 concurrent calls allowed
        insert_mock_with_config(&manager, &registry, &mock, Some(2), Duration::from_secs(60)).await;

        let mut handles = Vec::new();
        for i in 0..5u32 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr.call_tool("sem-test", "slow_tool", Some(serde_json::json!({"id": i})))
                    .await
                    .unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        // The mock's max_seen_concurrent should be <=2 because the semaphore limits it
        assert!(
            mock.max_seen_concurrent() <= 2,
            "expected max_seen_concurrent <= 2, got {}",
            mock.max_seen_concurrent()
        );
        // All 5 calls should have completed
        assert_eq!(mock.call_log().await.len(), 5);
    }

    #[tokio::test]
    async fn test_semaphore_timeout_on_exhaustion() {
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        // Very long delay — the first call holds the permit for 10s
        let mock = MockBackend::new("timeout-test", Duration::from_secs(10));

        // max=1, timeout=500ms: 2nd call should timeout
        insert_mock_with_config(
            &manager,
            &registry,
            &mock,
            Some(1),
            Duration::from_millis(500),
        )
        .await;

        // Fire first call (holds the permit)
        let mgr1 = Arc::clone(&manager);
        let _first = tokio::spawn(async move {
            mgr1.call_tool("timeout-test", "slow_tool", None)
                .await
                .unwrap();
        });

        // Small delay to ensure first call acquires permit
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Second call should timeout waiting for permit
        let result = manager.call_tool("timeout-test", "echo_tool", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("max concurrent calls"),
            "expected timeout error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_semaphore_default_values() {
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        let mock = MockBackend::new("defaults-test", Duration::ZERO);

        // Use default insert (no explicit semaphore config)
        insert_mock(&manager, &registry, &mock).await;

        // No semaphore should be created by insert_mock (it doesn't set one by default)
        // Calls should proceed without semaphore
        let result = manager
            .call_tool(
                "defaults-test",
                "echo_tool",
                Some(serde_json::json!({"x": 1})),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_semaphore_cleanup_on_remove() {
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        let mock = MockBackend::new("cleanup-test", Duration::ZERO);

        insert_mock_with_config(&manager, &registry, &mock, Some(5), Duration::from_secs(60)).await;

        // Verify semaphore exists
        assert!(manager.call_semaphores.contains_key("cleanup-test"));

        // Remove backend
        manager
            .remove_backend("cleanup-test", &registry)
            .await
            .unwrap();

        // Semaphore should be cleaned up
        assert!(!manager.call_semaphores.contains_key("cleanup-test"));
        assert!(!manager.semaphore_timeouts.contains_key("cleanup-test"));
    }

    #[tokio::test]
    async fn test_semaphore_zero_means_unlimited() {
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        let mock = MockBackend::new("unlimited-test", Duration::from_millis(100));

        // max=0: no semaphore created
        insert_mock_with_config(&manager, &registry, &mock, Some(0), Duration::from_secs(60)).await;

        // No semaphore should exist
        assert!(!manager.call_semaphores.contains_key("unlimited-test"));

        // Fire 10 concurrent calls — all should run without semaphore blocking
        let mut handles = Vec::new();
        for _ in 0..10 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr.call_tool("unlimited-test", "slow_tool", None)
                    .await
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // All 10 should have run concurrently
        assert_eq!(mock.max_seen_concurrent(), 10);
    }

    #[tokio::test]
    async fn test_semaphore_permit_released_on_error() {
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        let mock = MockBackend::new("error-release-test", Duration::ZERO);

        // max=1: only 1 concurrent call
        insert_mock_with_config(&manager, &registry, &mock, Some(1), Duration::from_secs(60)).await;

        // Call that errors — should still release the permit
        let result = manager
            .call_tool("error-release-test", "error_tool", None)
            .await;
        assert!(result.is_err());

        // Next call should succeed (permit was released)
        let result = manager
            .call_tool(
                "error-release-test",
                "echo_tool",
                Some(serde_json::json!({"ok": true})),
            )
            .await;
        assert!(result.is_ok());
    }

    // --- Phase 2: Sandbox semaphore tests ---

    #[tokio::test]
    async fn test_sandbox_semaphore_limits_concurrent_v8() {
        // Verify semaphore limits concurrent acquires
        let semaphore = Arc::new(tokio::sync::Semaphore::new(2));

        let mut handles = Vec::new();
        let max_concurrent = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));

        for _ in 0..5 {
            let sem = Arc::clone(&semaphore);
            let max_c = Arc::clone(&max_concurrent);
            let cur = Arc::clone(&current);
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                let c = cur.fetch_add(1, Ordering::SeqCst) + 1;
                max_c.fetch_max(c, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(100)).await;
                cur.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert!(
            max_concurrent.load(Ordering::SeqCst) <= 2,
            "expected max concurrent <= 2, got {}",
            max_concurrent.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn test_direct_call_bypasses_sandbox_semaphore() {
        // Direct tool calls should NOT acquire the sandbox semaphore.
        // Create a semaphore with 0 permits — if sandbox path is hit, it would block forever.
        let semaphore = Arc::new(tokio::sync::Semaphore::new(0));
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        let mock = MockBackend::new("test-backend", Duration::ZERO);
        insert_mock(&manager, &registry, &mock).await;

        // This is a simple direct tool call pattern — should bypass sandbox.
        // Use the actual backend name (with hyphen) since namespaced keys are "test-backend.echo_tool"
        let result = crate::tools::sandbox::handle_call_tool_chain(
            &registry,
            &manager,
            r#"test-backend.echo_tool({"hello": "world"})"#,
            None,
            None,
            &semaphore,
        )
        .await;
        assert!(
            result.is_ok(),
            "direct call should bypass sandbox: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_sandbox_semaphore_timeout_message() {
        // Semaphore with 0 permits — any sandbox call should timeout
        let semaphore = Arc::new(tokio::sync::Semaphore::new(0));
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();

        // Code that can't be parsed as direct call → hits sandbox path.
        // Use a short timeout (500ms) so the test doesn't wait 30s.
        let result = crate::tools::sandbox::handle_call_tool_chain(
            &registry,
            &manager,
            "const x = 1 + 2; return x;",
            Some(500), // 500ms timeout
            None,
            &semaphore,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("concurrency limit") || err.contains("sandbox"),
            "expected sandbox limit error, got: {err}"
        );
    }

    #[test]
    fn test_sandbox_config_defaults_via_struct() {
        let config = crate::config::SandboxConfig::default();
        assert_eq!(config.max_concurrent_sandboxes, 8);
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.max_output_size, 200_000);
    }

    #[tokio::test]
    async fn test_semaphore_permit_released_on_backend_timeout() {
        let manager = crate::backend::BackendManager::new();
        let registry = crate::registry::ToolRegistry::new();
        let mock = MockBackend::new("timeout-release-test", Duration::ZERO);

        // max=1, timeout=500ms
        insert_mock_with_config(
            &manager,
            &registry,
            &mock,
            Some(1),
            Duration::from_millis(500),
        )
        .await;

        // Normal call — should succeed and release permit
        let result = manager
            .call_tool(
                "timeout-release-test",
                "echo_tool",
                Some(serde_json::json!({"first": true})),
            )
            .await;
        assert!(result.is_ok());

        // Second call should also succeed (first permit released)
        let result = manager
            .call_tool(
                "timeout-release-test",
                "echo_tool",
                Some(serde_json::json!({"second": true})),
            )
            .await;
        assert!(result.is_ok());
    }
}
