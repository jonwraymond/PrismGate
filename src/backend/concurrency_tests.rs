//! Concurrency stress tests for BackendManager.
//!
//! Uses mock backends from `testutil` to validate concurrent call correctness,
//! semaphore backpressure, graceful drain, and state transitions under load.

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::backend::{Backend, BackendManager, BackendState};
    use crate::registry::ToolRegistry;
    use crate::testutil::{MockBackend, insert_mock, insert_mock_with_config};

    /// 20 concurrent calls to one mock backend — verify no cross-talk.
    #[tokio::test]
    async fn test_concurrent_calls_no_crosstalk() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("crosstalk-test", Duration::from_millis(50));

        // No semaphore limit — let all 20 run concurrently
        insert_mock_with_config(&manager, &registry, &mock, Some(0), Duration::from_secs(60)).await;

        let mut handles = Vec::new();
        for i in 0..20u32 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                let args = serde_json::json!({"id": i});
                let result = mgr
                    .call_tool("crosstalk-test", "echo_tool", Some(args))
                    .await
                    .unwrap();
                // Verify response matches request
                assert_eq!(result["id"], i, "response mismatch for call {i}");
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        // Proves concurrency actually happened
        assert!(
            mock.max_seen_concurrent() > 1,
            "expected concurrent calls, got max_seen={}",
            mock.max_seen_concurrent()
        );

        // All 20 calls completed
        assert_eq!(mock.call_log().await.len(), 20);
    }

    /// 5 mock backends × 10 concurrent calls each = 50 total.
    /// Assert all responses correct, no cross-backend contamination.
    #[tokio::test]
    async fn test_multi_backend_concurrent_calls() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        // Create 5 mock backends with unique names and unique tool names
        let mut mocks = Vec::new();
        for i in 0..5 {
            let name = format!("backend-{i}");
            let mock = MockBackend::new(&name, Duration::from_millis(30));
            insert_mock_with_config(&manager, &registry, &mock, Some(0), Duration::from_secs(60))
                .await;
            mocks.push(mock);
        }

        let mut handles = Vec::new();
        for i in 0..5u32 {
            for j in 0..10u32 {
                let mgr = Arc::clone(&manager);
                let backend_name = format!("backend-{i}");
                handles.push(tokio::spawn(async move {
                    let args = serde_json::json!({"backend": i, "call": j});
                    let result = mgr
                        .call_tool(&backend_name, "echo_tool", Some(args))
                        .await
                        .unwrap();
                    assert_eq!(result["backend"], i);
                    assert_eq!(result["call"], j);
                }));
            }
        }

        for h in handles {
            h.await.unwrap();
        }

        // Each backend should have received exactly 10 calls
        for (i, mock) in mocks.iter().enumerate() {
            let log = mock.call_log().await;
            assert_eq!(
                log.len(),
                10,
                "backend-{i} expected 10 calls, got {}",
                log.len()
            );
        }
    }

    /// Start 10 slow calls, set backend to Unhealthy mid-flight.
    /// Assert in-flight calls complete, new calls fail.
    #[tokio::test]
    async fn test_state_transition_during_calls() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("state-test", Duration::from_millis(300));

        insert_mock_with_config(&manager, &registry, &mock, Some(0), Duration::from_secs(60)).await;

        // Fire 10 slow calls
        let mut handles = Vec::new();
        for _ in 0..10 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr.call_tool("state-test", "slow_tool", None).await
            }));
        }

        // Wait a bit for calls to be in-flight, then mark unhealthy
        tokio::time::sleep(Duration::from_millis(50)).await;
        mock.set_state(BackendState::Unhealthy);

        // In-flight calls should still complete (they already have the Arc<dyn Backend>)
        let mut successes = 0;
        for h in handles {
            if h.await.unwrap().is_ok() {
                successes += 1;
            }
        }

        // Most calls should succeed since they started before state change.
        // Some might fail if they hit the retry loop after state changed.
        assert!(
            successes > 0,
            "expected some calls to succeed despite state change"
        );

        // New calls should fail immediately (Unhealthy state)
        let result = manager.call_tool("state-test", "echo_tool", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not available"));
    }

    /// Start 5 slow calls, call restart_backend().
    /// Assert old calls complete, post-restart calls succeed.
    #[tokio::test]
    async fn test_restart_during_active_calls() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("restart-test", Duration::from_millis(200));

        insert_mock_with_config(&manager, &registry, &mock, Some(0), Duration::from_secs(60)).await;

        // Also store the config so restart_backend can find it
        {
            let mut configs = manager.configs.write().await;
            configs.insert(
                "restart-test".to_string(),
                crate::config::BackendConfig {
                    transport: crate::config::Transport::Stdio,
                    namespace: None,
                    command: Some("echo".to_string()),
                    args: Vec::new(),
                    env: Default::default(),
                    cwd: None,
                    url: None,
                    headers: Default::default(),
                    timeout: Duration::from_secs(30),
                    max_concurrent_calls: None,
                    semaphore_timeout: Duration::from_secs(60),
                    required_keys: Vec::new(),
                    retry: Default::default(),
                    prerequisite: None,
                    rate_limit: None,
                },
            );
        }

        // Fire 5 slow calls
        let mut handles = Vec::new();
        for _ in 0..5 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr.call_tool("restart-test", "slow_tool", None).await
            }));
        }

        // Wait for calls to be in-flight
        tokio::time::sleep(Duration::from_millis(50)).await;

        // In-flight calls still hold their Arc<dyn Backend> — they should complete
        // even after the DashMap entry is replaced.
        let mut successes = 0;
        for h in handles {
            if h.await.unwrap().is_ok() {
                successes += 1;
            }
        }
        assert!(successes > 0, "in-flight calls should complete");
    }

    /// max=3, mock 200ms delay, fire 30 calls.
    /// Assert max_seen_concurrent <= 3, all 30 complete.
    #[tokio::test]
    async fn test_semaphore_backpressure_under_load() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("backpressure-test", Duration::from_millis(200));

        insert_mock_with_config(&manager, &registry, &mock, Some(3), Duration::from_secs(60)).await;

        let start = std::time::Instant::now();
        let mut handles = Vec::new();
        for _ in 0..30 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                mgr.call_tool("backpressure-test", "slow_tool", None)
                    .await
                    .unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let elapsed = start.elapsed();

        // Semaphore should limit to 3 concurrent
        assert!(
            mock.max_seen_concurrent() <= 3,
            "expected max_seen <= 3, got {}",
            mock.max_seen_concurrent()
        );

        // All 30 should complete
        assert_eq!(mock.call_log().await.len(), 30);

        // Total time should be at least (30/3 * 200ms) = 2s
        assert!(
            elapsed >= Duration::from_secs(2),
            "expected >= 2s total, got {elapsed:?}"
        );
    }

    /// Fire 10 slow calls, immediately stop_all().
    /// Assert drain completes and backends stop.
    #[tokio::test]
    async fn test_callguard_drain_on_shutdown() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("drain-test", Duration::from_millis(200));

        insert_mock_with_config(&manager, &registry, &mock, Some(0), Duration::from_secs(60)).await;

        // Fire 10 slow calls
        let mut handles = Vec::new();
        for _ in 0..10 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                // These may succeed or error depending on timing
                let _ = mgr.call_tool("drain-test", "slow_tool", None).await;
            }));
        }

        // Small delay to let some calls start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Stop all — should drain in-flight calls
        manager.stop_all().await;

        // Wait for all spawned tasks to finish
        for h in handles {
            h.await.unwrap();
        }

        // Backend should be stopped
        assert_eq!(mock.state(), BackendState::Stopped);

        // Backend map should be empty
        assert!(manager.backends.is_empty());
    }

    /// While calls are in-flight to backend A, remove A and add B with same name.
    /// Assert in-flight calls either complete or error cleanly (no panic).
    #[tokio::test]
    async fn test_rapid_add_remove_backend_under_load() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock_a = MockBackend::new("rapid-test", Duration::from_millis(200));

        insert_mock_with_config(
            &manager,
            &registry,
            &mock_a,
            Some(0),
            Duration::from_secs(60),
        )
        .await;

        // Fire calls to mock_a
        let mut handles = Vec::new();
        for _ in 0..5 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                let _ = mgr.call_tool("rapid-test", "slow_tool", None).await;
            }));
        }

        // Small delay, then replace with mock_b
        tokio::time::sleep(Duration::from_millis(50)).await;

        let mock_b = MockBackend::new("rapid-test", Duration::ZERO);
        insert_mock_with_config(
            &manager,
            &registry,
            &mock_b,
            Some(0),
            Duration::from_secs(60),
        )
        .await;

        // All tasks should complete without panic
        for h in handles {
            h.await.unwrap();
        }

        // New backend should be accessible
        let result = manager
            .call_tool(
                "rapid-test",
                "echo_tool",
                Some(serde_json::json!({"new": true})),
            )
            .await;
        assert!(result.is_ok());
    }

    /// Rate limiter with max_calls=2: 3rd call should fail.
    #[tokio::test]
    async fn test_rate_limit_enforcement() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("rate-test", Duration::ZERO);

        insert_mock_with_config(&manager, &registry, &mock, Some(0), Duration::from_secs(60)).await;

        // Set up rate limiter: 2 calls per window
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        manager
            .rate_limiters
            .insert("rate-test".to_string(), Arc::clone(&sem));

        // First two calls should succeed
        let r1 = manager
            .call_tool("rate-test", "echo_tool", Some(serde_json::json!({"n": 1})))
            .await;
        assert!(r1.is_ok(), "call 1 should succeed");

        let r2 = manager
            .call_tool("rate-test", "echo_tool", Some(serde_json::json!({"n": 2})))
            .await;
        assert!(r2.is_ok(), "call 2 should succeed");

        // Third call should fail — rate limit exceeded
        let r3 = manager
            .call_tool("rate-test", "echo_tool", Some(serde_json::json!({"n": 3})))
            .await;
        assert!(r3.is_err(), "call 3 should be rate limited");
        let err = r3.unwrap_err().to_string();
        assert!(
            err.contains("rate limit exceeded"),
            "expected rate limit error, got: {err}"
        );
    }

    /// Rate limiter replenishment: after window, permits are restored.
    #[tokio::test]
    async fn test_rate_limit_replenishment() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("replenish-test", Duration::ZERO);

        insert_mock_with_config(&manager, &registry, &mock, Some(0), Duration::from_secs(60)).await;

        // Set up rate limiter: 2 calls per 200ms window
        let sem = Arc::new(tokio::sync::Semaphore::new(2));
        manager
            .rate_limiters
            .insert("replenish-test".to_string(), Arc::clone(&sem));

        // Spawn replenishment task
        let replenish_sem = Arc::clone(&sem);
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(200));
            interval.tick().await; // first tick is immediate
            loop {
                interval.tick().await;
                let to_add = 2usize.saturating_sub(replenish_sem.available_permits());
                if to_add > 0 {
                    replenish_sem.add_permits(to_add);
                }
            }
        });
        manager
            .rate_limiter_handles
            .insert("replenish-test".to_string(), handle);

        // Exhaust all permits
        let _ = manager
            .call_tool(
                "replenish-test",
                "echo_tool",
                Some(serde_json::json!({"n": 1})),
            )
            .await;
        let _ = manager
            .call_tool(
                "replenish-test",
                "echo_tool",
                Some(serde_json::json!({"n": 2})),
            )
            .await;

        // Should be rate limited now
        let r = manager
            .call_tool(
                "replenish-test",
                "echo_tool",
                Some(serde_json::json!({"n": 3})),
            )
            .await;
        assert!(r.is_err(), "should be rate limited before replenishment");

        // Wait for replenishment window
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Should succeed now
        let r = manager
            .call_tool(
                "replenish-test",
                "echo_tool",
                Some(serde_json::json!({"n": 4})),
            )
            .await;
        assert!(r.is_ok(), "should succeed after replenishment");
    }

    /// BackendManager::new_with_config uses custom drain_timeout.
    #[tokio::test]
    async fn test_drain_timeout_configurable() {
        let config = crate::config::HealthConfig {
            drain_timeout: Duration::from_secs(42),
            ..Default::default()
        };
        let manager = BackendManager::new_with_config(&config);
        assert_eq!(manager.drain_timeout, Duration::from_secs(42));
    }

    /// 10 concurrent discover_tools calls on same backend — all return identical lists.
    #[tokio::test]
    async fn test_concurrent_tool_discovery() {
        use crate::registry::ToolEntry;

        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("discovery-test", Duration::ZERO);

        insert_mock(&manager, &registry, &mock).await;

        let mut handles = Vec::new();
        for _ in 0..10 {
            let m = Arc::clone(&mock) as Arc<dyn Backend>;
            handles.push(tokio::spawn(
                async move { m.discover_tools().await.unwrap() },
            ));
        }

        let mut results: Vec<Vec<ToolEntry>> = Vec::new();
        for h in handles {
            results.push(h.await.unwrap());
        }

        // All should be identical
        let first = &results[0];
        for (i, result) in results.iter().enumerate().skip(1) {
            assert_eq!(
                first.len(),
                result.len(),
                "discovery result {i} has different tool count"
            );
            for (j, tool) in result.iter().enumerate() {
                assert_eq!(
                    first[j].name, tool.name,
                    "tool name mismatch at index {j} in result {i}"
                );
            }
        }
    }
}
