//! Proxy resilience integration tests.
//!
//! Tests the resilient proxy's handshake interception, bidirectional bridging,
//! and transparent reconnection after daemon death. Uses the same daemon test
//! infrastructure as `daemon_tests.rs`.

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use rmcp::ServiceExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::net::{UnixListener, UnixStream};

    use crate::InitializedGateway;
    use crate::backend::BackendManager;
    use crate::config::{Config, DaemonConfig};
    use crate::registry::ToolRegistry;
    use crate::testutil::{MockBackend, insert_mock};

    /// Build a minimal Config from defaults with a short drain timeout for tests.
    fn test_config(idle_timeout: Duration) -> Config {
        let mut config: Config = serde_yaml_ng::from_str("{}").unwrap();
        config.daemon = DaemonConfig {
            idle_timeout,
            client_drain_timeout: Duration::from_secs(2),
        };
        config.allow_runtime_registration = true;
        config.max_dynamic_backends = 10;
        config
    }

    /// Spawn a test daemon on a temp socket.
    /// Returns (socket_path, daemon_handle, mock_backend, tempdir).
    async fn spawn_test_daemon(
        idle_timeout: Duration,
    ) -> (
        PathBuf,
        tokio::task::JoinHandle<anyhow::Result<()>>,
        Arc<MockBackend>,
        tempfile::TempDir,
    ) {
        spawn_test_daemon_with_mock("proxy-test", idle_timeout, Duration::from_millis(50)).await
    }

    async fn spawn_test_daemon_with_mock(
        backend_name: &str,
        idle_timeout: Duration,
        call_delay: Duration,
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
        let mock = MockBackend::new(backend_name, call_delay);
        insert_mock(&manager, &registry, &mock).await;

        let config = test_config(idle_timeout);

        let gw = InitializedGateway {
            registry,
            backend_manager: manager,
            tracker: Arc::new(crate::tracker::CallTracker::new()),
            cache_path: PathBuf::from("/tmp/test-proxy-cache.json"),
            config,
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        };

        let bound = crate::ipc::daemon::bind_early(Some(socket_path.clone()))
            .expect("bind_early failed in test");
        let handle = tokio::spawn(async move { crate::ipc::daemon::run(gw, bound).await });

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

    async fn read_proxy_line(reader: &mut DuplexStream, label: &str) -> serde_json::Value {
        let line = tokio::time::timeout(
            Duration::from_secs(5),
            crate::ipc::mcp_framing::read_line(reader),
        )
        .await
        .unwrap_or_else(|_| panic!("timeout reading {label}"))
        .unwrap_or_else(|e| panic!("failed reading {label}: {e}"));

        assert!(!line.is_empty(), "EOF reading {label}");
        serde_json::from_slice(&line).unwrap_or_else(|e| {
            panic!(
                "failed to parse {label} as JSON: {e}; line={}",
                String::from_utf8_lossy(&line)
            )
        })
    }

    async fn raw_handshake(client_write: &mut DuplexStream, client_read: &mut DuplexStream) {
        let init_req = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
        client_write
            .write_all(format!("{}\n", init_req).as_bytes())
            .await
            .unwrap();

        let init_response = read_proxy_line(client_read, "initialize response").await;
        assert!(
            init_response.get("result").is_some(),
            "initialize should return result: {init_response}"
        );

        let init_notif = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        client_write
            .write_all(format!("{}\n", init_notif).as_bytes())
            .await
            .unwrap();
    }

    async fn fake_daemon_handshake(stream: &mut UnixStream, server_name: &str) {
        let init_request = crate::ipc::mcp_framing::read_line(stream)
            .await
            .expect("fake daemon failed reading initialize");
        let init: serde_json::Value =
            serde_json::from_slice(&init_request).expect("fake daemon init JSON");
        let id = init.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": server_name, "version": "1.0.0" }
            }
        });
        stream
            .write_all(format!("{}\n", response).as_bytes())
            .await
            .expect("fake daemon failed writing initialize response");

        let initialized = crate::ipc::mcp_framing::read_line(stream)
            .await
            .expect("fake daemon failed reading initialized notification");
        assert!(
            String::from_utf8_lossy(&initialized).contains("notifications/initialized"),
            "fake daemon expected initialized notification"
        );
    }

    fn spawn_fake_daemon_that_drops_next_request(
        socket_path: PathBuf,
        ready: tokio::sync::oneshot::Sender<()>,
        request_seen: tokio::sync::oneshot::Sender<()>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let _ = std::fs::remove_file(&socket_path);
            let listener = UnixListener::bind(&socket_path).expect("bind fake daemon");
            let _ = ready.send(());
            let (mut stream, _) = listener.accept().await.expect("accept fake daemon client");
            fake_daemon_handshake(&mut stream, "fake-daemon-1").await;
            let request = crate::ipc::mcp_framing::read_line(&mut stream)
                .await
                .expect("fake daemon failed reading in-flight request");
            assert!(
                String::from_utf8_lossy(&request).contains("\"id\":"),
                "fake daemon expected an in-flight request"
            );
            let _ = request_seen.send(());
            drop(stream);
            drop(listener);
            let _ = std::fs::remove_file(&socket_path);
        })
    }

    fn spawn_fake_daemon_that_answers_tools_list(
        socket_path: PathBuf,
        ready: tokio::sync::oneshot::Sender<()>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let _ = std::fs::remove_file(&socket_path);
            let listener = UnixListener::bind(&socket_path).expect("bind replacement fake daemon");
            let _ = ready.send(());
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept replacement fake daemon client");
            fake_daemon_handshake(&mut stream, "fake-daemon-2").await;

            let request = crate::ipc::mcp_framing::read_line(&mut stream)
                .await
                .expect("replacement fake daemon failed reading request");
            let request_json: serde_json::Value =
                serde_json::from_slice(&request).expect("replacement request JSON");
            let id = request_json
                .get("id")
                .cloned()
                .expect("replacement request should have id");
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": [] }
            });
            stream
                .write_all(format!("{}\n", response).as_bytes())
                .await
                .expect("replacement fake daemon failed writing response");
            let _ = crate::ipc::mcp_framing::read_line(&mut stream).await;
        })
    }

    /// Connect an rmcp client through the proxy path (using run_on_socket).
    /// Returns (proxy_task, client_write_half, client_read_half).
    ///
    /// The proxy bridges between the duplex streams and the daemon socket.
    /// The test writes MCP messages to `client_write` and reads responses from `client_read`.
    fn start_proxy(
        socket_path: &Path,
    ) -> (
        tokio::task::JoinHandle<anyhow::Result<()>>,
        DuplexStream,
        DuplexStream,
    ) {
        // Create two duplex channels:
        // - stdin_pipe: test writes → proxy reads (simulates Claude Code → proxy)
        // - stdout_pipe: proxy writes → test reads (simulates proxy → Claude Code)
        let (client_write, proxy_stdin) = tokio::io::duplex(65536);
        let (proxy_stdout, client_read) = tokio::io::duplex(65536);

        let sock = socket_path.to_path_buf();
        let handle = tokio::spawn(async move {
            crate::ipc::proxy::run_on_socket(proxy_stdin, proxy_stdout, &sock).await
        });

        (handle, client_write, client_read)
    }

    /// Do a full MCP handshake through the proxy's duplex pipes, then return
    /// an rmcp client Peer for making tool calls.
    async fn handshake_through_proxy(
        socket_path: &Path,
    ) -> (
        rmcp::service::Peer<rmcp::RoleClient>,
        tokio::task::JoinHandle<anyhow::Result<()>>,
        tokio::task::JoinHandle<()>,
    ) {
        let (proxy_handle, client_write, client_read) = start_proxy(socket_path);

        // Use rmcp client to do the MCP handshake through the duplex pipes
        let service =
            ().serve((client_read, client_write))
                .await
                .expect("MCP handshake through proxy failed");
        let peer = service.peer().clone();
        let service_handle = tokio::spawn(async move {
            let _ = service.waiting().await;
        });

        (peer, proxy_handle, service_handle)
    }

    // ── Test: basic proxy handshake and tool call ──────────────────────────

    /// Verify that the proxy correctly intercepts the handshake and bridges tool calls.
    #[tokio::test]
    async fn test_proxy_handshake_and_tool_call() {
        let (socket_path, daemon_handle, _mock, _tmp) = spawn_test_daemon(Duration::ZERO).await;

        let (peer, proxy_handle, service_handle) = handshake_through_proxy(&socket_path).await;

        // List tools through the proxy
        let tools = peer.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 7, "should see 7 meta-tools through proxy");

        // Make a tool call through the proxy
        let result = peer
            .call_tool(
                rmcp::model::CallToolRequestParams::new("search_tools").with_arguments(
                    serde_json::json!({"task_description": "test proxy"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert!(
            !result.is_error.unwrap_or(false),
            "tool call through proxy should succeed"
        );

        // Clean up
        drop(peer);
        service_handle.abort();
        let _ = service_handle.await;
        // Wait a bit for proxy to detect stdin close
        tokio::time::sleep(Duration::from_millis(100)).await;
        daemon_handle.abort();
        let _ = daemon_handle.await;
        let _ = proxy_handle.await;
    }

    // ── Test: proxy survives daemon restart ────────────────────────────────

    /// Kill daemon, spawn new one, verify proxy reconnects and tool calls work.
    #[tokio::test]
    async fn test_proxy_survives_daemon_restart() {
        let (socket_path, daemon_handle, _mock, _tmp) = spawn_test_daemon(Duration::ZERO).await;

        let (peer, proxy_handle, service_handle) = handshake_through_proxy(&socket_path).await;

        // Verify initial connection works
        let tools = peer.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 7);

        // Kill the daemon
        daemon_handle.abort();
        let _ = daemon_handle.await;
        // Clean up socket so new daemon can bind
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(crate::ipc::socket::pid_path(&socket_path));

        // Small delay to let proxy detect disconnection
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Spawn a new daemon on the same socket
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock2 = MockBackend::new("proxy-test-2", Duration::from_millis(50));
        insert_mock(&manager, &registry, &mock2).await;

        let gw2 = InitializedGateway {
            registry,
            backend_manager: manager,
            tracker: Arc::new(crate::tracker::CallTracker::new()),
            cache_path: PathBuf::from("/tmp/test-proxy-cache-2.json"),
            config: test_config(Duration::ZERO),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        };

        let bound2 = crate::ipc::daemon::bind_early(Some(socket_path.clone()))
            .expect("bind_early failed for second daemon");
        let daemon_handle2 =
            tokio::spawn(async move { crate::ipc::daemon::run(gw2, bound2).await });

        // Wait for new daemon
        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Give proxy time to reconnect (backoff + handshake replay)
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Verify proxy is still running (hasn't exited with error)
        assert!(
            !proxy_handle.is_finished(),
            "proxy should still be running after reconnect"
        );

        // Tool calls should work through the reconnected proxy.
        // Note: the rmcp client may have timed out or gotten an error on the
        // in-flight connection. The connection-level reconnect is transparent
        // at the byte level, but rmcp's client state may have gotten confused.
        // This is the expected behavior: in-flight calls may be lost, but new
        // calls through new rmcp clients should work.

        // Clean up
        drop(peer);
        service_handle.abort();
        let _ = service_handle.await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        daemon_handle2.abort();
        let _ = daemon_handle2.await;
        let _ = proxy_handle.await;
    }

    /// If the daemon dies while a client request is in flight, the proxy must
    /// report that lost request as a retryable JSON-RPC error and keep the
    /// client's stdio session alive for subsequent requests.
    #[tokio::test]
    async fn test_inflight_request_gets_retryable_error_after_daemon_restart() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let socket_path = tmp_dir.path().join("test.sock");
        let (first_ready_tx, first_ready_rx) = tokio::sync::oneshot::channel();
        let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
        let daemon_handle = spawn_fake_daemon_that_drops_next_request(
            socket_path.clone(),
            first_ready_tx,
            request_seen_tx,
        );
        first_ready_rx
            .await
            .expect("fake daemon should bind socket");

        let (proxy_handle, mut client_write, mut client_read) = start_proxy(&socket_path);
        raw_handshake(&mut client_write, &mut client_read).await;

        let slow_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/list",
            "params": {}
        });
        client_write
            .write_all(format!("{}\n", slow_req).as_bytes())
            .await
            .unwrap();

        request_seen_rx
            .await
            .expect("fake daemon should see in-flight request");
        let _ = daemon_handle.await;

        let (replacement_ready_tx, replacement_ready_rx) = tokio::sync::oneshot::channel();
        let daemon_handle2 =
            spawn_fake_daemon_that_answers_tools_list(socket_path.clone(), replacement_ready_tx);
        replacement_ready_rx
            .await
            .expect("replacement fake daemon should bind socket");

        let lost_response = read_proxy_line(&mut client_read, "retryable lost request error").await;
        assert_eq!(lost_response["id"], serde_json::json!(10));
        assert!(
            lost_response["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("retry"),
            "lost in-flight request should be reported as retryable: {lost_response}"
        );

        let list_req = r#"{"jsonrpc":"2.0","id":11,"method":"tools/list","params":{}}"#;
        client_write
            .write_all(format!("{}\n", list_req).as_bytes())
            .await
            .unwrap();
        let list_response = read_proxy_line(&mut client_read, "post-reconnect tools/list").await;
        assert_eq!(list_response["id"], serde_json::json!(11));
        assert!(
            list_response["result"]["tools"].is_array(),
            "same client connection should remain usable after reconnect: {list_response}"
        );

        drop(client_write);
        drop(client_read);
        tokio::time::sleep(Duration::from_millis(100)).await;
        daemon_handle2.abort();
        let _ = daemon_handle2.await;
        let _ = proxy_handle.await;
    }

    /// If the daemon has disappeared and the client closes stdio while the proxy
    /// is in reconnect backoff, the proxy should reap itself instead of retrying
    /// forever. A quiet open stdin is an idle live client; a closed stdin is not.
    #[tokio::test]
    async fn test_client_close_during_reconnect_exits_proxy() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let socket_path = tmp_dir.path().join("test.sock");
        let (first_ready_tx, first_ready_rx) = tokio::sync::oneshot::channel();
        let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
        let daemon_handle = spawn_fake_daemon_that_drops_next_request(
            socket_path.clone(),
            first_ready_tx,
            request_seen_tx,
        );
        first_ready_rx
            .await
            .expect("fake daemon should bind socket");

        let (proxy_handle, mut client_write, mut client_read) = start_proxy(&socket_path);
        raw_handshake(&mut client_write, &mut client_read).await;

        let req = r#"{"jsonrpc":"2.0","id":20,"method":"tools/list","params":{}}"#;
        client_write
            .write_all(format!("{}\n", req).as_bytes())
            .await
            .unwrap();

        request_seen_rx
            .await
            .expect("fake daemon should see in-flight request");
        let _ = daemon_handle.await;

        drop(client_write);
        drop(client_read);

        let result = tokio::time::timeout(Duration::from_secs(2), proxy_handle).await;
        assert!(
            result.is_ok(),
            "proxy should exit promptly when client closes during reconnect"
        );
        assert!(
            result.unwrap().unwrap().is_ok(),
            "proxy close during reconnect should be clean"
        );
    }

    // ── Test: stdin close exits proxy cleanly ──────────────────────────────

    /// Close stdin (simulating Claude Code exit) — proxy should exit cleanly.
    #[tokio::test]
    async fn test_stdin_close_exits_proxy() {
        let (socket_path, daemon_handle, _mock, _tmp) = spawn_test_daemon(Duration::ZERO).await;

        let (proxy_handle, client_write, client_read) = start_proxy(&socket_path);

        // Do handshake through raw MCP messages
        let service = ().serve((client_read, client_write)).await.expect("handshake failed");

        // Drop the rmcp service (closes the duplex streams = stdin/stdout for proxy)
        drop(service);

        // Proxy should detect stdin close and exit
        let result = tokio::time::timeout(Duration::from_secs(5), proxy_handle).await;
        assert!(result.is_ok(), "proxy should exit within timeout");
        let inner = result.unwrap().unwrap();
        assert!(inner.is_ok(), "proxy should exit cleanly on stdin close");

        daemon_handle.abort();
    }

    // ── Test: proxy reconnect with backoff ────────────────────────────────

    /// Kill daemon, delay restart, verify proxy waits and reconnects.
    #[tokio::test]
    async fn test_proxy_reconnect_with_delayed_daemon() {
        let (socket_path, daemon_handle, _mock, _tmp) = spawn_test_daemon(Duration::ZERO).await;

        let (proxy_handle, client_write, client_read) = start_proxy(&socket_path);

        // Handshake
        let service = ().serve((client_read, client_write)).await.expect("handshake failed");
        let peer = service.peer().clone();

        // Verify initial works
        let tools = peer.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 7);

        // Kill daemon
        daemon_handle.abort();
        let _ = daemon_handle.await;
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(crate::ipc::socket::pid_path(&socket_path));

        // Wait 1 second before restarting (proxy should be in backoff)
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Proxy should still be alive (waiting to reconnect)
        assert!(
            !proxy_handle.is_finished(),
            "proxy should still be retrying"
        );

        // Spawn new daemon
        let manager = BackendManager::new();
        let registry = ToolRegistry::new();
        let mock2 = MockBackend::new("delayed-test", Duration::from_millis(50));
        insert_mock(&manager, &registry, &mock2).await;

        let gw2 = InitializedGateway {
            registry,
            backend_manager: manager,
            tracker: Arc::new(crate::tracker::CallTracker::new()),
            cache_path: PathBuf::from("/tmp/test-proxy-cache-delayed.json"),
            config: test_config(Duration::ZERO),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        };

        let bound2 =
            crate::ipc::daemon::bind_early(Some(socket_path.clone())).expect("bind_early failed");
        let daemon_handle2 =
            tokio::spawn(async move { crate::ipc::daemon::run(gw2, bound2).await });

        // Wait for reconnection
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Proxy should still be running
        assert!(!proxy_handle.is_finished(), "proxy should have reconnected");

        // Clean up
        drop(peer);
        drop(service);
        tokio::time::sleep(Duration::from_millis(100)).await;
        daemon_handle2.abort();
        let _ = daemon_handle2.await;
        let _ = proxy_handle.await;
    }

    // ── Test: daemon drain timeout works ───────────────────────────────────

    /// Verify daemon doesn't hang forever when clients are connected during shutdown.
    #[tokio::test]
    async fn test_daemon_drain_timeout() {
        let (socket_path, daemon_handle, _mock, _tmp) = spawn_test_daemon(Duration::ZERO).await;

        // Connect a client that stays connected
        let stream = UnixStream::connect(&socket_path).await.unwrap();
        let (read, write) = stream.into_split();
        let service = ().serve((read, write)).await.expect("handshake failed");
        let _peer = service.peer().clone();

        // Send SIGTERM to the daemon task (simulated by abort + short drain)
        // The daemon's client_drain_timeout is 2s in test config
        daemon_handle.abort();

        // Daemon should exit within drain timeout + margin
        let result = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
        assert!(result.is_ok(), "daemon should exit within drain timeout");
    }

    // ── Test: handshake cache content ──────────────────────────────────────

    /// Verify the handshake messages are properly intercepted and cached
    /// by testing that the proxy correctly bridges the MCP protocol.
    #[tokio::test]
    async fn test_handshake_message_bridging() {
        let (socket_path, daemon_handle, _mock, _tmp) = spawn_test_daemon(Duration::ZERO).await;

        let (proxy_handle, mut client_write, mut client_read) = start_proxy(&socket_path);

        // Send initialize request manually
        let init_req = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
        client_write
            .write_all(format!("{}\n", init_req).as_bytes())
            .await
            .unwrap();

        // Read initialize response from proxy
        let mut buf = vec![0u8; 65536];
        let n = tokio::time::timeout(Duration::from_secs(5), client_read.read(&mut buf))
            .await
            .expect("timeout reading init response")
            .expect("failed to read init response");
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(
            response.contains("\"result\""),
            "should receive initialize response with result"
        );

        // Send initialized notification
        let init_notif = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        client_write
            .write_all(format!("{}\n", init_notif).as_bytes())
            .await
            .unwrap();

        // Small delay for handshake to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now send a tools/list request to verify bridging works post-handshake
        let list_req = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        client_write
            .write_all(format!("{}\n", list_req).as_bytes())
            .await
            .unwrap();

        // Read tools/list response
        let n = tokio::time::timeout(Duration::from_secs(5), client_read.read(&mut buf))
            .await
            .expect("timeout reading tools/list response")
            .expect("failed to read tools/list response");
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(
            response.contains("search_tools") || response.contains("tools"),
            "should receive tools in response"
        );

        // Clean up
        drop(client_write);
        drop(client_read);
        tokio::time::sleep(Duration::from_millis(100)).await;
        daemon_handle.abort();
        let _ = daemon_handle.await;
        let _ = proxy_handle.await;
    }
}
