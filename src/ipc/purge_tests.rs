//! End-to-end purge over a real Unix socket, without starting backend processes.
#[cfg(unix)]
#[tokio::test]
async fn cli_purge_resets_live_tracker_without_replacing_runtime() {
    use rmcp::ServiceExt;
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("purge.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let tracker = Arc::new(crate::tracker::CallTracker::new());
    tracker.record("old", "backend", std::time::Duration::from_millis(1), true);
    tracker.record_bytes("old", 100, 500);
    let manager = crate::backend::BackendManager::new();
    let registry = crate::registry::ToolRegistry::new();
    let mock = crate::testutil::MockBackend::new("backend", std::time::Duration::ZERO);
    crate::testutil::insert_mock(&manager, &registry, &mock).await;
    let before = registry.get_all_names();
    let server = crate::server::PrismGateServer::new(
        Arc::clone(&registry),
        Arc::clone(&manager),
        Arc::clone(&tracker),
        dir.path().join("cache.json"),
        false,
        0,
        Arc::new(tokio::sync::Semaphore::new(1)),
        Some(99),
        Default::default(),
        Default::default(),
    );
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = tokio::io::split(stream);
        server
            .serve((read, write))
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    super::purge::run(Some(path)).await.unwrap();
    task.await.unwrap();
    assert!(tracker.recent_calls(10).is_empty());
    assert_eq!(tracker.session_stats().total_bytes_processed, 0);
    assert_eq!(registry.get_all_names(), before);
    manager
        .call_tool(
            "backend",
            "echo_tool",
            Some(serde_json::json!({"fresh": true})),
            None,
        )
        .await
        .expect("backend must remain callable after purge");
    let cache: serde_json::Value = serde_json::from_slice(
        &tokio::fs::read(dir.path().join("cache.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(cache["usage_stats"], serde_json::json!({}));
}

#[cfg(unix)]
#[tokio::test]
async fn cli_purge_does_not_start_missing_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.sock");
    assert!(super::purge::run(Some(path.clone())).await.is_err());
    assert!(!path.exists());
}
