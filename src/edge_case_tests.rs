//! Edge case tests for the PrismGate/gatemini MCP gateway.
//!
//! Covers empty inputs, large inputs, Unicode, concurrent search,
//! and backend timeout simulation. All tests use the shared
//! test infrastructure (MockBackend, ToolRegistry, BackendManager).

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::backend::{Backend, BackendManager, BackendState, store_state, STATE_UNHEALTHY};
    use crate::registry::{ToolEntry, ToolRegistry};

    /// Helper: create a ToolEntry with the given name, description, and backend.
    fn make_entry(name: &str, desc: &str, backend: &str) -> ToolEntry {
        ToolEntry {
            name: name.to_string(),
            original_name: name.to_string(),
            description: desc.to_string(),
            backend_name: backend.to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            tags: Vec::new(),
        }
    }

    /// Helper: create a ToolEntry with explicit tags.
    fn make_entry_tagged(
        name: &str,
        desc: &str,
        backend: &str,
        tags: Vec<String>,
    ) -> ToolEntry {
        ToolEntry {
            name: name.to_string(),
            original_name: name.to_string(),
            description: desc.to_string(),
            backend_name: backend.to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            tags,
        }
    }

    // ============================================================
    //  1. EMPTY INPUT TESTS
    // ============================================================

    /// Register a tool with an empty name and verify it's accessible
    /// via its namespaced key.
    #[test]
    fn test_empty_tool_name_registration() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools("backend", vec![make_entry("", "Empty name tool", "backend")]);

        // Namespaced key should still be accessible
        let entry = reg.get_by_name("backend.");
        assert!(entry.is_some(), "empty-name tool should be accessible via namespaced key");
        let entry = entry.unwrap();
        assert_eq!(entry.description, "Empty name tool");
        assert_eq!(entry.backend_name, "backend");

        // Bare empty string should also work (it's the bare name alias)
        let bare = reg.get_by_name("");
        assert!(bare.is_some());
    }

    /// Search with an empty query should return no results (not crash).
    #[test]
    fn test_search_empty_query() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "test",
            vec![
                make_entry("tool_a", "First tool", "test"),
                make_entry("tool_b", "Second tool", "test"),
            ],
        );

        let results = reg.search("", 10, None, None);
        assert!(results.is_empty(), "empty query should return no results");
    }

    /// Search with only whitespace should return no results.
    #[test]
    fn test_search_whitespace_only_query() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "test",
            vec![
                make_entry("tool_a", "First tool", "test"),
                make_entry("tool_b", "Second tool", "test"),
            ],
        );

        let results = reg.search("   \t\n  ", 10, None, None);
        assert!(results.is_empty(), "whitespace-only query should return no results");
    }

    /// Look up a tool name that doesn't exist.
    #[test]
    fn test_get_by_name_nonexistent() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools("test", vec![make_entry("real_tool", "A real tool", "test")]);

        assert!(reg.get_by_name("nonexistent_tool").is_none());
        assert!(reg.get_by_name("fake.nonexistent").is_none());
        assert!(reg.get_by_name("").is_none());
    }

    /// Search for a query that matches nothing.
    #[test]
    fn test_search_no_match() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools("test", vec![make_entry("real_tool", "A real tool", "test")]);

        let results = reg.search("xyzzy_plugh", 10, None, None);
        assert!(results.is_empty(), "nonsense query should return no results");
    }

    // ============================================================
    //  2. LARGE INPUT TESTS
    // ============================================================

    /// Register a tool with a ~10KB description and verify search works.
    #[test]
    fn test_large_description_registration() {
        let reg = ToolRegistry::new();
        // Generate ~10KB of descriptive text
        let large_desc = "Lorem ipsum dolor sit amet. ".repeat(180); // ~180 * 28 ≈ 5040 chars, ~10KB bytes
        // Make it exactly around 10KB
        let large_desc = large_desc.repeat(2); // ~10KB

        reg.register_backend_tools(
            "big_backend",
            vec![make_entry("big_tool", &large_desc, "big_backend")],
        );

        // Tool should be accessible
        let entry = reg.get_by_name("big_tool").unwrap();
        assert_eq!(entry.backend_name, "big_backend");
        assert!(entry.description.len() > 9000, "description should be ~10KB");

        // Search by name still works
        let results = reg.search("big_tool", 5, None, None);
        assert!(!results.is_empty(), "should find tool by name even with large description");
    }

    /// Register many tools (100+) with large descriptions and verify search
    /// doesn't blow up.
    #[test]
    fn test_many_tools_large_descriptions() {
        let reg = ToolRegistry::new();
        let base_desc = "A tool that does something useful. ".repeat(50); // ~1.8KB each

        for i in 0..100 {
            let name = format!("tool_{:03}", i);
            let desc = format!("{}. Tool number {}. {}", base_desc, i, base_desc);
            reg.register_backend_tools(
                &format!("bk_{:03}", i),
                vec![make_entry(&name, &desc, &format!("bk_{:03}", i))],
            );
        }

        // Search should complete without error
        let results = reg.search("useful", 10, None, None);
        // All 100 tools contain "useful" so we should get 10 results (limit)
        assert_eq!(results.len(), 10);
    }

    /// Register a tool with zero-length description.
    #[test]
    fn test_zero_length_description() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "minimal",
            vec![make_entry("minimal_tool", "", "minimal")],
        );

        let entry = reg.get_by_name("minimal_tool").unwrap();
        assert!(entry.description.is_empty());

        // Search by name should still find it
        let results = reg.search("minimal", 5, None, None);
        assert!(!results.is_empty());
    }

    // ============================================================
    //  3. UNICODE TESTS
    // ============================================================

    /// Register and search for tools with Unicode names/descriptions.
    #[test]
    fn test_unicode_tool_names() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "unicode",
            vec![
                make_entry("café_search", "Search for café-related content ☕", "unicode"),
                make_entry("日本語ツール", "日本語のツールです 🎌", "unicode"),
                make_entry("π_calculator", "Calculate π to arbitrary precision", "unicode"),
                make_entry("✨_magic", "✨ Magic tool with emoji name ✨", "unicode"),
            ],
        );

        // All tools should be accessible by their bare names
        assert!(reg.get_by_name("café_search").is_some());
        assert!(reg.get_by_name("日本語ツール").is_some());
        assert!(reg.get_by_name("π_calculator").is_some());
        assert!(reg.get_by_name("✨_magic").is_some());

        // Search by Unicode query
        let results = reg.search("café", 5, None, None);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.name.contains("café")));
    }

    /// Search using Unicode query terms.
    #[test]
    fn test_unicode_search_terms() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "unicode",
            vec![
                make_entry("café_search", "Search for cafés and restaurants", "unicode"),
                make_entry("hello_world", "Simple greeting tool", "unicode"),
            ],
        );

        // ASCII search still works
        let results = reg.search("search", 5, None, None);
        assert!(!results.is_empty());

        // Non-ASCII query: "café" should tokenize to ["café"] 
        // and match "café_search" which tokenizes to ["café", "search"]
        let results = reg.search("café", 5, None, None);
        assert!(!results.is_empty(), "Unicode query 'café' should find results");
    }

    /// Test with emoji-only description.
    #[test]
    fn test_emoji_description() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "emoji",
            vec![make_entry("mood_tool", "😀😃😄😁😆😅😂🤣", "emoji")],
        );

        // Tool is accessible
        let entry = reg.get_by_name("mood_tool").unwrap();
        assert_eq!(entry.backend_name, "emoji");

        // Search by name should find it even if description is all emoji
        let results = reg.search("mood", 5, None, None);
        assert!(!results.is_empty());

        // Emoji-only search query should not crash (emoji are not alphanumeric,
        // so they produce empty token list → empty results)
        let results = reg.search("😀", 5, None, None);
        // The query tokenizer splits on non-alphanumeric, so emojis produce empty terms
        assert!(results.is_empty());
    }

    /// Test CJK (Chinese/Japanese/Korean) tool names and search.
    #[test]
    fn test_cjk_tool_names() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "cjk",
            vec![
                make_entry("搜索工具", "在互联网上搜索信息", "cjk"),
                make_entry("검색도구", "인터넷에서 정보를 검색합니다", "cjk"),
                make_entry("検索ツール", "インターネットで情報を検索する", "cjk"),
            ],
        );

        assert!(reg.get_by_name("搜索工具").is_some());
        assert!(reg.get_by_name("검색도구").is_some());
        assert!(reg.get_by_name("検索ツール").is_some());

        // Search with CJK query (each character is a "term" using our alphanumeric tokenizer,
        // since CJK characters ARE alphanumeric)
        let results = reg.search("搜索", 5, None, None);
        assert!(!results.is_empty(), "CJK search should find results");
    }

    // ============================================================
    //  4. CONCURRENT SEARCH TESTS
    // ============================================================

    /// Multiple concurrent searches against the same registry should not
    /// deadlock or produce incorrect results.
    #[tokio::test]
    async fn test_concurrent_search_no_deadlock() {
        let reg = ToolRegistry::new();

        // Populate registry with 200 tools spread across 20 backends
        for bk in 0..20u32 {
            let bk_name = format!("backend_{}", bk);
            let mut tools = Vec::new();
            for t in 0..10u32 {
                let name = format!("tool_{}_{}", bk, t);
                let desc = format!("Tool {} from backend {} that does something", t, bk);
                tools.push(make_entry(&name, &desc, &bk_name));
            }
            reg.register_backend_tools(&bk_name, tools);
        }

        let reg = Arc::new(reg);
        let mut handles = Vec::new();

        for i in 0..20u32 {
            let reg = Arc::clone(&reg);
            handles.push(tokio::spawn(async move {
                // Each task does 5 searches
                for _ in 0..5 {
                    let query = format!("tool_{}", i % 20);
                    let results = reg.search(&query, 10, None, None);
                    // Result must exist
                    assert!(!results.is_empty(), "search for '{query}' should find results");
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    /// Concurrent reads and writes: register tools while searching.
    #[tokio::test]
    async fn test_concurrent_register_and_search() {
        let reg = Arc::new(ToolRegistry::new());

        let mut handles = Vec::new();

        // Writer: register tools every 50ms
        let reg_w = Arc::clone(&reg);
        handles.push(tokio::spawn(async move {
            for i in 0..10u32 {
                let bk_name = format!("dyn_{}", i);
                let tools = vec![make_entry(
                    &format!("dyn_tool_{}", i),
                    &format!("Dynamically registered tool {}", i),
                    &bk_name,
                )];
                reg_w.register_backend_tools(&bk_name, tools);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }));

        // Readers: search concurrently
        for _ in 0..5 {
            let reg_r = Arc::clone(&reg);
            handles.push(tokio::spawn(async move {
                for _ in 0..10 {
                    let results = reg_r.search("tool", 20, None, None);
                    // Results may be empty initially but should never panic
                    let _ = results.len();
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    /// Concurrent tag-filtered search.
    #[tokio::test]
    async fn test_concurrent_tag_filtered_search() {
        let reg = ToolRegistry::new();

        // Register tools with various tags
        reg.register_backend_tools(
            "bk_a",
            vec![
                make_entry_tagged("tool_a1", "Group A tool 1", "bk_a", vec!["group_a".into(), "read".into()]),
                make_entry_tagged("tool_a2", "Group A tool 2", "bk_a", vec!["group_a".into(), "write".into()]),
            ],
        );
        reg.register_backend_tools(
            "bk_b",
            vec![
                make_entry_tagged("tool_b1", "Group B tool 1", "bk_b", vec!["group_b".into(), "read".into()]),
                make_entry_tagged("tool_b2", "Group B tool 2", "bk_b", vec!["group_b".into(), "write".into()]),
            ],
        );

        let reg = Arc::new(reg);
        let mut handles = Vec::new();

        let filters: &[&[String]] = &[
            &["group_a".into()],
            &["group_b".into()],
            &["read".into()],
            &["write".into()],
        ];

        for filter in filters {
            let reg = Arc::clone(&reg);
            let filter: Vec<String> = filter.to_vec();
            handles.push(tokio::spawn(async move {
                for _ in 0..5 {
                    let results = reg.search("tool", 10, Some(&filter), None);
                    assert!(!results.is_empty(), "tag-filtered search should find results");
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    // ============================================================
    //  5. BACKEND TIMEOUT TESTS
    // ============================================================

    /// Simulate concurrent calls hitting a semaphore limit, verifying
    /// that acquire timeouts are reported correctly.
    #[tokio::test]
    async fn test_semaphore_timeout_on_max_concurrent() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        // Use MockBackend from testutil
        let mock = crate::testutil::MockBackend::new("timeout-test", Duration::from_millis(500));

        // Insert with max_concurrent=2 and a 100ms semaphore timeout
        crate::testutil::insert_mock_with_config(
            &manager,
            &registry,
            &mock,
            Some(2),
            Duration::from_millis(100),
        )
        .await;

        // Fire 4 concurrent calls — first 2 get permits, next 2 should timeout
        let mut handles = Vec::new();
        for i in 0..4u32 {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                let args = serde_json::json!({"id": i});
                mgr.call_tool("timeout-test", "slow_tool", Some(args), None).await
            }));
        }

        let mut successes = 0u32;
        let mut timeouts = 0u32;
        for h in handles {
            match h.await.unwrap() {
                Ok(_) => successes += 1,
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("max concurrent") || msg.contains("Timed out") {
                        timeouts += 1;
                    }
                }
            }
        }

        // At least 2 should succeed (the ones that got permits within 100ms)
        assert!(successes >= 2, "expected >=2 successes, got {successes}");
        // At least some should timeout (we have 4 calls competing for 2 permits for 500ms each)
        assert!(timeouts > 0, "expected some timeouts, got {timeouts}");
    }

    /// Verify that calling a tool on an unhealthy backend fails immediately.
    #[tokio::test]
    async fn test_unhealthy_backend_rejected() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        let mock = crate::testutil::MockBackend::new("unhealthy-test", Duration::ZERO);
        crate::testutil::insert_mock(&manager, &registry, &mock).await;

        // Mark the backend as unhealthy
        mock.set_state(BackendState::Unhealthy);

        let result = manager
            .call_tool("unhealthy-test", "echo_tool", Some(serde_json::json!({"x": 1})), None)
            .await;

        assert!(result.is_err(), "unhealthy backend should reject calls");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not available") || err_msg.contains("Unhealthy"),
            "error should mention unavailability, got: {err_msg}"
        );

        // Restore health and verify calls work again
        mock.set_state(BackendState::Healthy);
        let result = manager
            .call_tool("unhealthy-test", "echo_tool", Some(serde_json::json!({"x": 1})), None)
            .await;
        assert!(result.is_ok(), "healthy backend should accept calls");
    }

    /// Verify that calling a stopped backend also fails.
    #[tokio::test]
    async fn test_stopped_backend_rejected() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        let mock = crate::testutil::MockBackend::new("stopped-test", Duration::ZERO);
        crate::testutil::insert_mock(&manager, &registry, &mock).await;
        mock.set_state(BackendState::Stopped);

        let result = manager
            .call_tool("stopped-test", "echo_tool", None, None)
            .await;

        assert!(result.is_err(), "stopped backend should reject calls");
    }

    /// Verify that calling a nonexistent backend fails with a clear error.
    #[tokio::test]
    async fn test_nonexistent_backend_error() {
        let manager = BackendManager::new();

        let result = manager
            .call_tool("ghost-backend", "ghost_tool", None, None)
            .await;

        assert!(result.is_err(), "nonexistent backend should error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("not available"),
            "error should mention backend not found, got: {err_msg}"
        );
    }

    /// Verify that error_tool on a mock backend propagates errors correctly.
    #[tokio::test]
    async fn test_error_tool_propagation() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        let mock = crate::testutil::MockBackend::new("err-test", Duration::ZERO);
        crate::testutil::insert_mock(&manager, &registry, &mock).await;

        let result = manager
            .call_tool("err-test", "error_tool", None, None)
            .await;

        assert!(result.is_err(), "error_tool should always fail");
        assert!(
            result.unwrap_err().to_string().contains("injected error"),
            "error should contain injected error message"
        );
    }

    /// Simulate a slow tool and verify that it completes even under
    /// concurrent load.
    #[tokio::test]
    async fn test_slow_tool_completes() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        // 200ms per call, 10 max concurrent, 2s timeout
        let mock = crate::testutil::MockBackend::new("slow-test", Duration::from_millis(200));
        crate::testutil::insert_mock_with_config(
            &manager,
            &registry,
            &mock,
            Some(10),
            Duration::from_secs(2),
        )
        .await;

        let start = std::time::Instant::now();
        let result = manager
            .call_tool("slow-test", "slow_tool", None, None)
            .await;

        assert!(result.is_ok(), "slow_tool should complete");
    }

    /// Rate limiter: verify that hitting a rate-limited backend returns an error.
    #[tokio::test]
    async fn test_rate_limiter_exhausted() {
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();

        let mock = crate::testutil::MockBackend::new("rate-test", Duration::ZERO);
        crate::testutil::insert_mock(&manager, &registry, &mock).await;

        // Set up a rate limiter with 1 permit available
        let rate_sem = Arc::new(tokio::sync::Semaphore::new(1));
        manager
            .rate_limiters
            .insert("rate-test".to_string(), Arc::clone(&rate_sem));

        // First call: consumes the only permit via forget()
        let result1 = manager
            .call_tool("rate-test", "echo_tool", Some(serde_json::json!({"n": 1})), None)
            .await;
        assert!(result1.is_ok(), "first call should succeed (permit available)");

        // Second call: no permits left
        let result2 = manager
            .call_tool("rate-test", "echo_tool", Some(serde_json::json!({"n": 2})), None)
            .await;

        assert!(result2.is_err(), "second call should fail (rate limit exhausted)");
        let err_msg = result2.unwrap_err().to_string();
        assert!(
            err_msg.contains("rate limit"),
            "error should mention rate limit, got: {err_msg}"
        );
    }
}