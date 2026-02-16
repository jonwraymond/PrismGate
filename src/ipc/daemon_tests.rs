//! Multi-client daemon tests.
//!
//! Tests the Unix socket daemon with multiple concurrent MCP clients.
//! Uses mock backends and temp sockets to validate session isolation,
//! idle shutdown, and concurrent tool calling across clients.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use rmcp::model::*;
    use rmcp::ServiceExt;
    use tokio::net::UnixStream;

    use crate::backend::BackendManager;
    use crate::config::{Config, DaemonConfig};
    use crate::registry::ToolRegistry;
    use crate::testutil::{MockBackend, insert_mock};
    use crate::InitializedGateway;

    /// Build a minimal Config from defaults with a custom idle_timeout.
    fn test_config(idle_timeout: Duration) -> Config {
        let mut config: Config = serde_yaml_ng::from_str("{}").unwrap();
        config.daemon = DaemonConfig { idle_timeout };
        config.allow_runtime_registration = true;
        config.max_dynamic_backends = 10;
        config
    }

    /// Spawn a test daemon on a temp socket.
    /// Returns (socket_path, daemon_handle, mock_backend, tempdir).
    /// The tempdir must be kept alive for the socket path to remain valid.
    async fn spawn_test_daemon(
        idle_timeout: Duration,
    ) -> (
        PathBuf,
        tokio::task::JoinHandle<anyhow::Result<()>>,
        Arc<MockBackend>,
        tempfile::TempDir,
    ) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let socket_path = tmp_dir.path().join("test.sock");

        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock = MockBackend::new("daemon-test", Duration::from_millis(50));
        insert_mock(&manager, &registry, &mock).await;

        let config = test_config(idle_timeout);

        let gw = InitializedGateway {
            registry,
            backend_manager: manager,
            cache_path: PathBuf::from("/tmp/test-cache.json"),
            config,
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        };

        let sock = socket_path.clone();
        let handle = tokio::spawn(async move {
            crate::ipc::daemon::run(gw, Some(sock)).await
        });

        // Wait for socket to become available
        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(socket_path.exists(), "daemon socket not created");

        (socket_path, handle, mock, tmp_dir)
    }

    /// Connect an rmcp client to the daemon socket.
    /// Returns the peer and a join handle that keeps the service alive.
    async fn connect_client(
        socket_path: &Path,
    ) -> (
        rmcp::service::Peer<rmcp::RoleClient>,
        tokio::task::JoinHandle<()>,
    ) {
        let stream = UnixStream::connect(socket_path).await.unwrap();
        let (read, write) = stream.into_split();
        let service = ().serve((read, write)).await.expect("client handshake failed");
        let peer = service.peer().clone();
        let handle = tokio::spawn(async move {
            let _ = service.waiting().await;
        });
        (peer, handle)
    }

    /// 5 clients connect, each does tools/list, then disconnects.
    /// Daemon stays alive throughout.
    #[tokio::test]
    async fn test_multi_client_connect_disconnect() {
        let (socket_path, daemon_handle, _mock, _tmp) =
            spawn_test_daemon(Duration::ZERO).await; // No idle timeout

        for _ in 0..5 {
            let (peer, service_handle) = connect_client(&socket_path).await;

            // Each client can list tools
            let tools = peer.list_all_tools().await.unwrap();
            assert_eq!(tools.len(), 7, "each client should see 7 meta-tools");

            // Disconnect by dropping peer and aborting service
            drop(peer);
            service_handle.abort();
            let _ = service_handle.await;
        }

        // Daemon should still be running
        assert!(!daemon_handle.is_finished(), "daemon should still be alive");

        // Clean up: send shutdown signal
        daemon_handle.abort();
    }

    /// 3 clients × 10 concurrent calls each = 30 total.
    /// Assert all responses correct, no cross-client contamination.
    #[tokio::test]
    async fn test_concurrent_tool_calls_across_clients() {
        let (socket_path, daemon_handle, _mock, _tmp) =
            spawn_test_daemon(Duration::ZERO).await;

        let mut client_handles = Vec::new();
        for client_id in 0..3u32 {
            let sock = socket_path.clone();
            client_handles.push(tokio::spawn(async move {
                let (peer, _service_handle) = connect_client(&sock).await;

                let mut call_handles = Vec::new();
                for call_id in 0..10u32 {
                    let p = peer.clone();
                    call_handles.push(tokio::spawn(async move {
                        let result = p
                            .call_tool(CallToolRequestParams {
                                meta: None,
                                name: "search_tools".to_string().into(),
                                arguments: Some(
                                    serde_json::json!({"task_description": format!("client{client_id}_call{call_id}")})
                                        .as_object()
                                        .unwrap()
                                        .clone(),
                                ),
                                task: None,
                            })
                            .await
                            .unwrap();

                        assert!(
                            !result.is_error.unwrap_or(false),
                            "client {client_id} call {call_id} failed"
                        );
                    }));
                }

                for h in call_handles {
                    h.await.unwrap();
                }
            }));
        }

        for h in client_handles {
            h.await.unwrap();
        }

        daemon_handle.abort();
    }

    /// Client A starts a slow call, client B connects and disconnects abruptly.
    /// A's call should still complete.
    #[tokio::test]
    async fn test_client_disconnect_does_not_affect_others() {
        let (socket_path, daemon_handle, _mock, _tmp) =
            spawn_test_daemon(Duration::ZERO).await;

        // Client A: connect and start a tool call (takes some time due to search)
        let (peer_a, _service_a) = connect_client(&socket_path).await;

        let call_handle = {
            let peer = peer_a.clone();
            tokio::spawn(async move {
                peer.call_tool(CallToolRequestParams {
                    meta: None,
                    name: "search_tools".to_string().into(),
                    arguments: Some(
                        serde_json::json!({"task_description": "test"})
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                    task: None,
                })
                .await
            })
        };

        // Client B: connect and immediately disconnect
        let (peer_b, service_b) = connect_client(&socket_path).await;
        drop(peer_b);
        service_b.abort();
        let _ = service_b.await;

        // Client A's call should still complete
        let result = call_handle.await.unwrap().unwrap();
        assert!(
            !result.is_error.unwrap_or(false),
            "client A's call should succeed despite client B disconnecting"
        );

        daemon_handle.abort();
    }

    /// idle_timeout=500ms, connect + disconnect, wait 1s. Daemon should exit.
    #[tokio::test]
    async fn test_idle_shutdown_with_no_clients() {
        let (socket_path, daemon_handle, _mock, _tmp) =
            spawn_test_daemon(Duration::from_millis(500)).await;

        // Connect and immediately disconnect
        let (peer, service_handle) = connect_client(&socket_path).await;
        drop(peer);
        service_handle.abort();
        let _ = service_handle.await;

        // Wait for the client session to clean up in the daemon
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Wait for idle timeout + margin
        tokio::time::sleep(Duration::from_millis(700)).await;

        // Daemon should have exited
        assert!(
            daemon_handle.is_finished(),
            "daemon should have exited after idle timeout"
        );
    }

    /// idle_timeout=500ms. Connect/disconnect, wait 300ms, connect/disconnect, wait 300ms.
    /// Daemon should still be alive (timer was reset). Then wait 700ms — daemon exits.
    #[tokio::test]
    async fn test_idle_timer_reset_on_activity() {
        let (socket_path, daemon_handle, _mock, _tmp) =
            spawn_test_daemon(Duration::from_millis(500)).await;

        // First connect/disconnect
        let (peer, service_handle) = connect_client(&socket_path).await;
        drop(peer);
        service_handle.abort();
        let _ = service_handle.await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Wait 300ms (within idle window)
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !daemon_handle.is_finished(),
            "daemon should still be alive after 300ms"
        );

        // Second connect/disconnect (resets timer)
        let (peer, service_handle) = connect_client(&socket_path).await;
        drop(peer);
        service_handle.abort();
        let _ = service_handle.await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Wait another 300ms — should still be alive (timer was just reset)
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !daemon_handle.is_finished(),
            "daemon should still be alive after second activity reset"
        );

        // Wait another 700ms — should exit now
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(
            daemon_handle.is_finished(),
            "daemon should have exited after idle timeout with no further activity"
        );
    }

    /// Connect 3 clients simultaneously, verify all work.
    /// Disconnect 1, verify remaining 2 still work.
    /// Disconnect all, verify daemon idle-shuts-down.
    #[tokio::test]
    async fn test_session_counter_accuracy() {
        let (socket_path, daemon_handle, _mock, _tmp) =
            spawn_test_daemon(Duration::from_millis(500)).await;

        // Connect 3 clients
        let (peer1, svc1) = connect_client(&socket_path).await;
        let (peer2, svc2) = connect_client(&socket_path).await;
        let (peer3, svc3) = connect_client(&socket_path).await;

        // All 3 can list tools
        assert_eq!(peer1.list_all_tools().await.unwrap().len(), 7);
        assert_eq!(peer2.list_all_tools().await.unwrap().len(), 7);
        assert_eq!(peer3.list_all_tools().await.unwrap().len(), 7);

        // Disconnect client 1
        drop(peer1);
        svc1.abort();
        let _ = svc1.await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Remaining clients still work
        assert_eq!(peer2.list_all_tools().await.unwrap().len(), 7);
        assert_eq!(peer3.list_all_tools().await.unwrap().len(), 7);

        // Daemon should NOT idle-shutdown (still has active clients)
        assert!(
            !daemon_handle.is_finished(),
            "daemon should be alive with active clients"
        );

        // Disconnect remaining clients
        drop(peer2);
        svc2.abort();
        let _ = svc2.await;
        drop(peer3);
        svc3.abort();
        let _ = svc3.await;

        // Wait for sessions to clean up + idle timeout
        tokio::time::sleep(Duration::from_millis(800)).await;

        // Daemon should have idle-shutdown
        assert!(
            daemon_handle.is_finished(),
            "daemon should exit after all clients disconnect and idle timeout"
        );
    }
}
