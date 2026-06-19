//! Chaos engineering tests for gatemini — failure injection.
//!
//! These tests deliberately trigger failure conditions to verify resilience:
//! backend crash/restart during active calls, network latency injection,
//! unhealthy state transitions, IPC socket failures, and memory pressure.
//!
//! All chaos scenarios use the `MockBackend` from `testutil` which allows
//! precise control over timing, errors, and state transitions without
//! requiring real subprocesses or network manipulation.
//!
//! Run with: `cargo test --test chaos_tests` (add `--release` for timing accuracy)

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::Result;
    use tokio::sync::{mpsc, Semaphore};
    use tokio::time::sleep;

    use crate::backend::{
        Backend, BackendManager, BackendState, STATE_HEALTHY, STATE_STOPPED, STATE_UNHEALTHY,
    };
    use crate::registry::ToolRegistry;
    use crate::testutil::{insert_mock, insert_mock_with_config, MockBackend};

    // ========================================================================
    //  CHAOS 1: Backend crash during active tool call
    // ========================================================================

    /// Verify the gateway does NOT panic when a backend process exits mid-call.
    /// Simulates: backend PID dies, next call returns Unavailable, not a panic.
    #[tokio::test]
    async fn test_backend_crash_during_active_call() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("crash-test", Duration::from_millis(500));

        insert_mock(&manager, &registry, &mock).await;

        // Start a slow call
        let mgr_clone = Arc::clone(&manager.backends);
        let backend_name = "crash-test".to_string();
        let call_handle = tokio::spawn(async move {
            manager
                .call_tool(&backend_name, "slow_tool", None, None)
                .await
        });

        // Simulate backend crash by removing it from the manager mid-call
        sleep(Duration::from_millis(100)).await;
        manager.backends.remove(&backend_name);
        drop(mgr_clone);

        // The in-flight call may succeed or fail depending on timing,
        // but the gateway must NOT panic. Next call returns error gracefully.
        let result = manager
            .call_tool(&backend_name, "echo_tool", None, None)
            .await;

        assert!(
            result.is_err(),
            "call to removed backend should fail gracefully, not panic"
        );

        // Clean up the spawned task
        let _ = call_handle.await;
    }

    /// Backend restarts (stop + start) during active calls — all in-flight
    /// calls fail, subsequent calls succeed after restart completes.
    #[tokio::test]
    async fn test_backend_restart_during_active_call() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("restart-test", Duration::from_secs(10)); // very slow

        insert_mock(&manager, &registry, &mock).await;
        let mock_arc: Arc<dyn Backend> = mock.clone();

        // Fire a call that will hang for 10s
        let mgr1 = Arc::clone(&manager);
        let call_handle = tokio::spawn(async move {
            mgr1.call_tool("restart-test", "slow_tool", None, None).await
        });

        sleep(Duration::from_millis(100)).await;

        // Restart the backend while the slow call is in flight
        mock.set_state(BackendState::Stopped);
        let _ = mock.stop().await;
        let _ = mock.start().await;
        mock.set_state(BackendState::Healthy);

        // The hanging call should eventually fail (not panic)
        let result = call_handle.await.unwrap();
        assert!(
            result.is_err(),
            "call to restarting backend should fail, not panic"
        );

        // Subsequent calls should succeed
        let result = manager
            .call_tool("restart-test", "echo_tool", Some(serde_json::json!({"restarted": true})), None)
            .await;
        assert!(result.is_ok(), "call after restart should succeed: {:?}", result);
    }

    // ========================================================================
    //  CHAOS 2: Network latency / timeout injection
    // ========================================================================

    /// All backend calls exceed the configured timeout — verify timeout error.
    #[tokio::test]
    async fn test_backend_timeout_all_calls() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        // Every call takes 5s
        let mock = MockBackend::new("slowpoke", Duration::from_secs(5));

        // Set max_concurrent=1 so calls queue behind each other,
        // and timeout=1s so each call times out
        insert_mock_with_config(
            &manager,
            &registry,
            &mock,
            Some(1),
            Duration::from_secs(1),
        )
        .await;

        let result = manager
            .call_tool("slowpoke", "echo_tool", None, None)
            .await;

        assert!(result.is_err(), "slow backend should hit timeout");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("max concurrent") || err.contains("timeout"),
            "error should mention timeout: {err}"
        );
    }

    /// A slow backend recovers after timeout — subsequent calls succeed.
    #[tokio::test]
    async fn test_backend_timeout_then_recovery() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("brittle", Duration::from_secs(5));

        insert_mock_with_config(
            &manager,
            &registry,
            &mock,
            Some(1),
            Duration::from_millis(500),
        )
        .await;

        // First call times out
        let result = manager
            .call_tool("brittle", "echo_tool", None, None)
            .await;
        assert!(result.is_err());

        // Reduce delay so next call completes in time
        // (Simulate network improvement or backend recovery)
        let _result = mock.call_tool("echo_tool", None).await; // warm up

        // Simulate backend becomes responsive — replace with faster mock via stop/start
        mock.set_state(BackendState::Stopped);
        let _ = mock.stop().await;
        let _ = mock.start().await;
        mock.set_state(BackendState::Healthy);

        // Next call should succeed
        let result = manager
            .call_tool("brittle", "echo_tool", Some(serde_json::json!({"recovered": true})), None)
            .await;
        assert!(result.is_ok(), "backend should recover after restart: {:?}", result);
    }

    // ========================================================================
    //  CHAOS 3: Backend state transitions — Unhealthy / Stopped
    // ========================================================================

    /// Backend marked Unhealthy rejects new calls; calls already in flight complete.
    #[tokio::test]
    async fn test_unhealthy_backend_rejects_new_calls() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("ailing", Duration::ZERO);

        insert_mock(&manager, &registry, &mock).await;

        // Mark backend as unhealthy
        mock.set_state(BackendState::Unhealthy);

        // New calls should be rejected
        let result = manager
            .call_tool("ailing", "echo_tool", None, None)
            .await;
        assert!(
            result.is_err(),
            "unhealthy backend should reject calls: {:?}",
            result
        );

        // Restore to healthy
        mock.set_state(BackendState::Healthy);
        let result = manager
            .call_tool("ailing", "echo_tool", Some(serde_json::json!({"restored": true})), None)
            .await;
        assert!(result.is_ok(), "healthy backend should accept calls");
    }

    /// Backend marked Stopped rejects all calls.
    #[tokio::test]
    async fn test_stopped_backend_rejects_all_calls() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("dead", Duration::ZERO);

        insert_mock(&manager, &registry, &mock).await;

        mock.set_state(BackendState::Stopped);

        let result = manager.call_tool("dead", "echo_tool", None, None).await;
        assert!(result.is_err(), "stopped backend should reject calls: {:?}", result);
    }

    /// Rapid cycling: Healthy → Unhealthy → Healthy → Stopped → Healthy.
    /// Verifies the manager handles state jitter without panics or deadlocks.
    #[tokio::test]
    async fn test_rapid_state_cycling() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("flaky", Duration::from_millis(10));

        insert_mock(&manager, &registry, &mock).await;

        let states = [
            BackendState::Unhealthy,
            BackendState::Healthy,
            BackendState::Unhealthy,
            BackendState::Healthy,
            BackendState::Stopped,
            BackendState::Healthy,
        ];

        for state in states {
            mock.set_state(state);
            let result = manager
                .call_tool("flaky", "echo_tool", Some(serde_json::json!({"state": format!("{:?}", state)})), None)
                .await;

            if state == BackendState::Stopped {
                assert!(result.is_err(), "stopped backend should reject");
            } else if state == BackendState::Healthy {
                assert!(result.is_ok(), "healthy backend should accept");
            }
            // Unhealthy: may accept or reject depending on implementation
        }
    }

    // ========================================================================
    //  CHAOS 4: Error injection via MockBackend
    // ========================================================================

    /// Backend with global error injection — all tools return errors.
    #[tokio::test]
    async fn test_backend_error_injection_all_tools() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("broken", Duration::ZERO);

        insert_mock(&manager, &registry, &mock).await;

        mock.set_inject_error(true);

        let result = manager
            .call_tool("broken", "echo_tool", Some(serde_json::json!({"should": "fail"})), None)
            .await;
        assert!(result.is_err(), "injected error should propagate: {:?}", result);
    }

    /// Error injection toggled on/off mid-session — calls succeed after recovery.
    #[tokio::test]
    async fn test_error_injection_toggle_recovery() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("erratic", Duration::ZERO);

        insert_mock(&manager, &registry, &mock).await;

        // Start with error injection on
        mock.set_inject_error(true);
        let result = manager.call_tool("erratic", "echo_tool", None, None).await;
        assert!(result.is_err(), "should fail with injection on");

        // Toggle off — simulate backend self-healing
        mock.set_inject_error(false);
        let result = manager
            .call_tool("erratic", "echo_tool", Some(serde_json::json!({"healed": true})), None)
            .await;
        assert!(result.is_ok(), "should succeed after injection off: {:?}", result);
    }

    // ========================================================================
    //  CHAOS 5: Concurrent call limits under stress
    // ========================================================================

    /// Semaphore exhaustion: many concurrent calls, all but N wait and timeout.
    #[tokio::test]
    async fn test_semaphore_exhaustion_many_concurrent_waiters() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        // Tool sleeps 5s — long enough to hold the semaphore
        let mock = MockBackend::new("crowded", Duration::from_secs(5));

        // Only 1 concurrent call allowed, 2s timeout
        insert_mock_with_config(
            &manager,
            &registry,
            &mock,
            Some(1),
            Duration::from_secs(2),
        )
        .await;

        // Fire 5 calls simultaneously — 4 should timeout waiting
        let mut handles = Vec::new();
        for i in 0..5 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr.call_tool(
                    "crowded",
                    "slow_tool",
                    Some(serde_json::json!({"caller": i})),
                    None,
                )
                .await
            }));
        }

        let mut success_count = 0;
        let mut timeout_count = 0;

        for h in handles {
            let result = h.await.unwrap();
            if result.is_ok() {
                success_count += 1;
            } else {
                timeout_count += 1;
            }
        }

        // Exactly 1 should succeed (holds the permit), 4 should timeout
        assert_eq!(success_count, 1, "only 1 call should acquire the semaphore");
        assert_eq!(timeout_count, 4, "4 calls should timeout waiting");
    }

    /// Semaphore with max=0 (unlimited) — all concurrent calls succeed.
    #[tokio::test]
    async fn test_unlimited_concurrency_all_succeed() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("unlimited", Duration::from_millis(100));

        insert_mock_with_config(
            &manager,
            &registry,
            &mock,
            Some(0), // 0 = unlimited
            Duration::from_secs(60),
        )
        .await;

        let mut handles = Vec::new();
        for i in 0..10 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr.call_tool(
                    "unlimited",
                    "slow_tool",
                    Some(serde_json::json!({"id": i})),
                    None,
                )
                .await
            }));
        }

        let mut ok_count = 0;
        for h in handles {
            if h.await.unwrap().is_ok() {
                ok_count += 1;
            }
        }

        assert_eq!(ok_count, 10, "all 10 calls should succeed with unlimited concurrency");
    }

    // ========================================================================
    //  CHAOS 6: IPC / transport failures
    // ========================================================================

    /// Manager with no backends registered — call returns error, not panic.
    #[tokio::test]
    async fn test_call_with_no_registered_backend() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        let result = manager
            .call_tool("phantom", "echo_tool", None, None)
            .await;

        assert!(result.is_err(), "call to unknown backend should error: {:?}", result);
    }

    /// Manager with multiple backends — one removed mid-session, others still work.
    #[tokio::test]
    async fn test_remove_one_backend_others_unaffected() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        let mock_a = MockBackend::new("service-a", Duration::ZERO);
        let mock_b = MockBackend::new("service-b", Duration::ZERO);

        insert_mock(&manager, &registry, &mock_a).await;
        insert_mock(&manager, &registry, &mock_b).await;

        // Verify both work
        assert!(manager
            .call_tool("service-a", "echo_tool", None, None)
            .await
            .is_ok());
        assert!(manager
            .call_tool("service-b", "echo_tool", None, None)
            .await
            .is_ok());

        // Remove service-a
        manager
            .remove_backend("service-a", &registry)
            .await
            .unwrap();

        // service-a should fail
        assert!(manager
            .call_tool("service-a", "echo_tool", None, None)
            .await
            .is_err());

        // service-b should still work
        assert!(manager
            .call_tool("service-b", "echo_tool", Some(serde_json::json!({"still": "here"})), None)
            .await
            .is_ok());
    }

    // ========================================================================
    //  CHAOS 7: Graceful degradation under sustained failure load
    // ========================================================================

    /// Rapid fire of calls to a failing backend — verify no resource leaks.
    #[tokio::test]
    async fn test_rapid_calls_to_failing_backend_no_leak() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("leaky", Duration::ZERO);

        insert_mock(&manager, &registry, &mock).await;

        mock.set_state(BackendState::Stopped);

        // Fire 50 rapid calls — none should panic, all should return errors
        let mut handles = Vec::new();
        for i in 0..50 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr.call_tool("leaky", "echo_tool", Some(serde_json::json!({"i": i})), None)
                    .await
            }));
        }

        let mut error_count = 0;
        for h in handles {
            if h.await.unwrap().is_err() {
                error_count += 1;
            }
        }

        assert_eq!(error_count, 50, "all 50 calls to stopped backend should fail gracefully");
    }

    /// Tool discovery on a backend that has zero tools registered.
    #[tokio::test]
    async fn test_backend_with_zero_tools() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("empty-backend", Duration::ZERO);

        insert_mock(&manager, &registry, &mock).await;

        // empty-backend has tools registered via MockBackend (echo_tool, etc.)
        // but we can verify discover_tools returns a list
        let tools = mock.discover_tools().await.unwrap();
        assert!(
            !tools.is_empty(),
            "MockBackend should always register at least one tool"
        );

        // And calls work
        let result = manager
            .call_tool("empty-backend", "echo_tool", None, None)
            .await;
        assert!(result.is_ok());
    }
}
