use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{self, AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info, warn};

#[cfg(unix)]
use tokio::net::UnixStream;

use crate::ipc::mcp_framing;
use crate::ipc::socket;

// ── Reconnection parameters ────────────────────────────────────────────────

const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(5);
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(60);

/// How often to check if the parent process is still alive.
const ORPHAN_CHECK_INTERVAL: Duration = Duration::from_secs(5);

// ── Handshake cache ────────────────────────────────────────────────────────

/// Cached MCP handshake messages for session replay on reconnect.
#[derive(Debug, Clone)]
struct HandshakeCache {
    /// Raw bytes of the `initialize` request (including trailing \n)
    initialize_request: Vec<u8>,
    /// Raw bytes of the `notifications/initialized` notification (including trailing \n)
    initialized_notification: Vec<u8>,
}

// ── Bridge outcome ─────────────────────────────────────────────────────────

/// Outcome of the bidirectional bridge phase.
enum BridgeOutcome {
    /// stdin closed (Claude Code exited) — clean shutdown
    StdinClosed,
    /// Daemon socket closed or errored — attempt reconnect
    DaemonDisconnected,
    /// Parent process died (orphaned proxy) — exit immediately
    ParentDied,
}

/// Tracks JSON-RPC request/response IDs across daemon reconnects.
///
/// The MCP stdio transport has no session-resume primitive if the server
/// process exits, so the proxy protects the client connection by accounting for
/// requests that were in flight when the daemon side disappeared.
#[derive(Debug, Default)]
struct JsonRpcTracker {
    /// Client -> daemon requests waiting for daemon responses.
    client_pending: HashMap<String, Value>,
    /// Daemon -> client requests waiting for client responses.
    server_pending: HashSet<String>,
}

impl JsonRpcTracker {
    fn observe_client_message(&mut self, line: &[u8]) -> ClientMessageAction {
        let Some(obj) = parse_json_object(line) else {
            return ClientMessageAction::Forward;
        };

        if obj.get("method").is_some() {
            if let Some(id) = obj.get("id")
                && let Some(key) = request_id_key(id)
            {
                self.client_pending.insert(key, id.clone());
            }
            return ClientMessageAction::Forward;
        }

        if (obj.contains_key("result") || obj.contains_key("error"))
            && let Some(id) = obj.get("id")
            && let Some(key) = request_id_key(id)
            && !self.server_pending.remove(&key)
        {
            debug!(
                id = %id,
                "dropping client response for request from a previous daemon generation"
            );
            return ClientMessageAction::Drop;
        }

        ClientMessageAction::Forward
    }

    fn observe_daemon_message(&mut self, line: &[u8]) {
        let Some(obj) = parse_json_object(line) else {
            return;
        };

        if obj.get("method").is_some() {
            if let Some(id) = obj.get("id")
                && let Some(key) = request_id_key(id)
            {
                self.server_pending.insert(key);
            }
            return;
        }

        if (obj.contains_key("result") || obj.contains_key("error"))
            && let Some(id) = obj.get("id")
            && let Some(key) = request_id_key(id)
        {
            self.client_pending.remove(&key);
        }
    }

    async fn fail_pending_client_requests<W>(&mut self, stdout: &mut W) -> Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        let pending: Vec<Value> = self.client_pending.drain().map(|(_, id)| id).collect();
        self.server_pending.clear();

        if pending.is_empty() {
            return Ok(());
        }

        warn!(
            count = pending.len(),
            "daemon disconnected with in-flight client requests; reporting retryable errors"
        );

        for id in pending {
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32000,
                    "message": "Gatemini daemon disconnected before this request completed; retry the request on the still-open client connection."
                }
            });
            let mut bytes = serde_json::to_vec(&response)
                .context("failed to serialize retryable request error")?;
            bytes.push(b'\n');
            stdout
                .write_all(&bytes)
                .await
                .context("failed to write retryable request error to client stdout")?;
        }

        stdout
            .flush()
            .await
            .context("failed to flush retryable request errors to client stdout")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMessageAction {
    Forward,
    Drop,
}

#[cfg(unix)]
enum ReconnectOutcome {
    Reconnected(UnixStream),
    ClientClosed,
    ParentDied,
    Failed(anyhow::Error),
}

#[cfg(unix)]
struct ReconnectClientState<'a, R>
where
    R: AsyncRead + Unpin,
{
    stdin: &'a mut R,
    tracker: &'a mut JsonRpcTracker,
    queued_client_messages: &'a mut Vec<Vec<u8>>,
    original_ppid: u32,
}

fn parse_json_object(line: &[u8]) -> Option<serde_json::Map<String, Value>> {
    serde_json::from_slice::<Value>(line)
        .ok()?
        .as_object()
        .cloned()
}

fn request_id_key(id: &Value) -> Option<String> {
    match id {
        Value::String(s) => Some(format!("s:{s}")),
        Value::Number(n) => Some(format!("n:{n}")),
        Value::Null => Some("null".to_string()),
        _ => None,
    }
}

// ── Public entry point ─────────────────────────────────────────────────────

/// Run as a resilient proxy: bridge Claude Code's stdio to the daemon's Unix socket.
///
/// If no daemon is running, auto-start one. On daemon disconnection, reconnect
/// transparently by replaying the cached MCP handshake to a new daemon instance.
#[cfg(unix)]
pub async fn run(config_path: &Path) -> Result<()> {
    run_inner(io::stdin(), io::stdout(), config_path).await
}

#[cfg(not(unix))]
pub async fn run(_config_path: &Path) -> Result<()> {
    bail!("daemon proxy mode is not supported on Windows. Use --direct mode.");
}

// ── Core implementation (generic for testability) ──────────────────────────

#[cfg(unix)]
async fn run_inner<R, W>(stdin: R, stdout: W, config_path: &Path) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let socket_path = socket::default_socket_path();

    // Clean up stale socket from a crashed daemon before anything else.
    cleanup_stale_socket(&socket_path);

    // Connect to existing daemon or spawn a new one.
    let stream = connect_or_spawn(&socket_path, config_path).await?;

    // Intercept and cache the MCP handshake.
    let (stream, cache, stdin, stdout) = handshake_phase(stream, stdin, stdout).await?;

    // Enter the reconnecting bridge loop.
    bridge_loop(stream, &cache, &socket_path, config_path, stdin, stdout).await
}

/// Test-only entry point: connect to a pre-existing daemon at the given socket path.
///
/// Unlike `run_inner`, this does NOT spawn daemons or use flock coordination.
/// Reconnection attempts connect directly to the same socket path (waiting for
/// the test infrastructure to restart the daemon).
#[cfg(all(test, unix))]
pub(crate) async fn run_on_socket<R, W>(stdin: R, stdout: W, socket_path: &Path) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let stream = try_connect(socket_path).await?;
    let (stream, cache, stdin, stdout) = handshake_phase(stream, stdin, stdout).await?;
    bridge_loop_connect_only(stream, &cache, socket_path, stdin, stdout).await
}

/// Bridge loop variant that reconnects by connecting only (no daemon spawning).
/// Used in tests where the daemon is managed externally.
#[cfg(all(test, unix))]
async fn bridge_loop_connect_only<R, W>(
    mut stream: UnixStream,
    cache: &HandshakeCache,
    socket_path: &Path,
    mut stdin: R,
    mut stdout: W,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reconnect_count: u32 = 0;
    let mut bridge_start = std::time::Instant::now();
    let original_ppid = std::os::unix::process::parent_id();
    let mut tracker = JsonRpcTracker::default();
    let mut queued_client_messages: Vec<Vec<u8>> = Vec::new();

    loop {
        match bridge_phase(
            &mut stream,
            &mut stdin,
            &mut stdout,
            &mut tracker,
            original_ppid,
        )
        .await
        {
            BridgeOutcome::StdinClosed | BridgeOutcome::ParentDied => {
                debug!("proxy exiting (stdin closed or parent died)");
                return Ok(());
            }
            BridgeOutcome::DaemonDisconnected => {
                if let Err(e) = tracker.fail_pending_client_requests(&mut stdout).await {
                    warn!(error = %e, "client stdout closed while reporting daemon disconnect");
                    return Ok(());
                }

                // Reset counter if the bridge ran for a reasonable time.
                if bridge_start.elapsed() > Duration::from_secs(30) {
                    reconnect_count = 0;
                }

                reconnect_count += 1;
                warn!(
                    attempt = reconnect_count,
                    "daemon disconnected, attempting reconnect (test mode)"
                );

                match reconnect_connect_only_monitoring_client(
                    cache,
                    socket_path,
                    reconnect_count,
                    ReconnectClientState {
                        stdin: &mut stdin,
                        tracker: &mut tracker,
                        queued_client_messages: &mut queued_client_messages,
                        original_ppid,
                    },
                )
                .await
                {
                    ReconnectOutcome::Reconnected(mut new_stream) => {
                        if let Err(e) =
                            flush_queued_client_messages(&mut new_stream, &queued_client_messages)
                                .await
                        {
                            warn!(error = %e, "failed to flush queued client messages after reconnect");
                            continue;
                        }
                        queued_client_messages.clear();
                        stream = new_stream;
                        bridge_start = std::time::Instant::now();
                        info!(
                            attempt = reconnect_count,
                            "reconnected to daemon (test mode)"
                        );
                    }
                    ReconnectOutcome::ClientClosed => return Ok(()),
                    ReconnectOutcome::ParentDied => return Ok(()),
                    ReconnectOutcome::Failed(e) => {
                        warn!(error = %e, "reconnect failed; continuing reconnect loop");
                    }
                }
            }
        }
    }
}

// ── Handshake phase ────────────────────────────────────────────────────────

/// Intercept the 3-message MCP handshake, forwarding each message while caching
/// the client's messages for replay on reconnect.
///
/// Returns the stream (ready for bridge), the cached handshake, and the
/// stdin/stdout handles (moved through for ownership).
#[cfg(unix)]
async fn handshake_phase<R, W>(
    stream: UnixStream,
    mut stdin: R,
    mut stdout: W,
) -> Result<(UnixStream, HandshakeCache, R, W)>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut sock_read, mut sock_write) = stream.into_split();

    // 1. Read initialize request from stdin → cache → forward to socket
    let initialize_request = mcp_framing::read_line(&mut stdin)
        .await
        .context("failed to read initialize request from stdin")?;
    if initialize_request.is_empty() {
        bail!("stdin closed before sending initialize request");
    }
    debug_assert_eq!(
        mcp_framing::classify(&initialize_request),
        mcp_framing::McpMessage::InitializeRequest,
        "first message should be initialize request"
    );
    sock_write
        .write_all(&initialize_request)
        .await
        .context("failed to forward initialize request to daemon")?;

    // 2. Read initialize response from socket → forward to stdout
    let initialize_response = mcp_framing::read_line(&mut sock_read)
        .await
        .context("failed to read initialize response from daemon")?;
    if initialize_response.is_empty() {
        bail!("daemon closed before sending initialize response");
    }
    stdout
        .write_all(&initialize_response)
        .await
        .context("failed to forward initialize response to stdout")?;
    stdout
        .flush()
        .await
        .context("failed to flush initialize response to stdout")?;

    // 3. Read initialized notification from stdin → cache → forward to socket
    let initialized_notification = mcp_framing::read_line(&mut stdin)
        .await
        .context("failed to read initialized notification from stdin")?;
    if initialized_notification.is_empty() {
        bail!("stdin closed before sending initialized notification");
    }
    debug_assert_eq!(
        mcp_framing::classify(&initialized_notification),
        mcp_framing::McpMessage::InitializedNotification,
        "third message should be initialized notification"
    );
    sock_write
        .write_all(&initialized_notification)
        .await
        .context("failed to forward initialized notification to daemon")?;

    let cache = HandshakeCache {
        initialize_request,
        initialized_notification,
    };

    // Reunite the split halves back into a whole UnixStream for the bridge phase.
    let stream = sock_read
        .reunite(sock_write)
        .context("failed to reunite socket halves after handshake")?;

    info!("MCP handshake complete, entering bridge mode");
    Ok((stream, cache, stdin, stdout))
}

// ── Bridge loop (reconnecting outer loop) ──────────────────────────────────

/// Outer loop: runs the bridge, detects daemon disconnection, reconnects.
#[cfg(unix)]
async fn bridge_loop<R, W>(
    mut stream: UnixStream,
    cache: &HandshakeCache,
    socket_path: &Path,
    config_path: &Path,
    mut stdin: R,
    mut stdout: W,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reconnect_count: u32 = 0;
    let mut bridge_start = std::time::Instant::now();
    let original_ppid = std::os::unix::process::parent_id();
    let mut tracker = JsonRpcTracker::default();
    let mut queued_client_messages: Vec<Vec<u8>> = Vec::new();

    loop {
        match bridge_phase(
            &mut stream,
            &mut stdin,
            &mut stdout,
            &mut tracker,
            original_ppid,
        )
        .await
        {
            BridgeOutcome::StdinClosed | BridgeOutcome::ParentDied => {
                debug!("proxy exiting (stdin closed or parent died)");
                return Ok(());
            }
            BridgeOutcome::DaemonDisconnected => {
                if let Err(e) = tracker.fail_pending_client_requests(&mut stdout).await {
                    warn!(error = %e, "client stdout closed while reporting daemon disconnect");
                    return Ok(());
                }

                // Reset counter if the bridge ran for a reasonable time (stable connection).
                if bridge_start.elapsed() > Duration::from_secs(30) {
                    reconnect_count = 0;
                }

                reconnect_count += 1;
                warn!(
                    attempt = reconnect_count,
                    "daemon disconnected, attempting reconnect"
                );

                match reconnect_monitoring_client(
                    cache,
                    socket_path,
                    config_path,
                    reconnect_count,
                    ReconnectClientState {
                        stdin: &mut stdin,
                        tracker: &mut tracker,
                        queued_client_messages: &mut queued_client_messages,
                        original_ppid,
                    },
                )
                .await
                {
                    ReconnectOutcome::Reconnected(mut new_stream) => {
                        if let Err(e) =
                            flush_queued_client_messages(&mut new_stream, &queued_client_messages)
                                .await
                        {
                            warn!(error = %e, "failed to flush queued client messages after reconnect");
                            continue;
                        }
                        queued_client_messages.clear();
                        stream = new_stream;
                        bridge_start = std::time::Instant::now();
                        info!(attempt = reconnect_count, "reconnected to daemon");
                    }
                    ReconnectOutcome::ClientClosed => return Ok(()),
                    ReconnectOutcome::ParentDied => return Ok(()),
                    ReconnectOutcome::Failed(e) => {
                        warn!(
                            attempt = reconnect_count,
                            error = %e,
                            "reconnect failed; keeping client transport open"
                        );
                    }
                }
            }
        }
    }
}

// ── Bridge phase (bidirectional copy) ──────────────────────────────────────

/// Bidirectional MCP line bridge: stdin → socket, socket → stdout.
///
/// Tracks JSON-RPC request IDs while forwarding newline-delimited MCP messages
/// so daemon reconnects can report lost in-flight requests without closing the
/// client stdio transport.
///
/// Returns when stdin closes (StdinClosed), the daemon socket closes/errors
/// (DaemonDisconnected), or the parent process dies (ParentDied).
#[cfg(unix)]
async fn bridge_phase<R, W>(
    stream: &mut UnixStream,
    stdin: &mut R,
    stdout: &mut W,
    tracker: &mut JsonRpcTracker,
    original_ppid: u32,
) -> BridgeOutcome
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut sock_read, mut sock_write) = stream.split();

    // If the parent PID changes (reparented to init/launchd), the client
    // process has exited or crashed and this proxy is orphaned.
    let mut orphan_check = tokio::time::interval(ORPHAN_CHECK_INTERVAL);
    orphan_check.tick().await; // consume immediate first tick

    loop {
        tokio::select! {
            // Bias toward draining daemon responses first (avoids backpressure)
            biased;

            result = mcp_framing::read_line(&mut sock_read) => {
                match result {
                    Ok(line) if line.is_empty() => return BridgeOutcome::DaemonDisconnected,
                    Ok(line) => {
                        tracker.observe_daemon_message(&line);
                        if stdout.write_all(&line).await.is_err() {
                            // stdout broken (Claude Code crashed) — treat as stdin close
                            return BridgeOutcome::StdinClosed;
                        }
                        if stdout.flush().await.is_err() {
                            return BridgeOutcome::StdinClosed;
                        }
                    }
                    Err(_) => return BridgeOutcome::DaemonDisconnected,
                }
            }

            result = mcp_framing::read_line(stdin) => {
                match result {
                    Ok(line) if line.is_empty() => return BridgeOutcome::StdinClosed,
                    Ok(line) => {
                        if tracker.observe_client_message(&line) == ClientMessageAction::Drop {
                            continue;
                        }
                        if sock_write.write_all(&line).await.is_err() {
                            return BridgeOutcome::DaemonDisconnected;
                        }
                    }
                    Err(_) => return BridgeOutcome::StdinClosed,
                }
            }

            _ = orphan_check.tick() => {
                let current_ppid = std::os::unix::process::parent_id();
                if current_ppid != original_ppid {
                    warn!(
                        original_ppid,
                        current_ppid,
                        "parent process died, proxy is orphaned — exiting"
                    );
                    return BridgeOutcome::ParentDied;
                }
            }
        }
    }
}

// ── Reconnect phase ────────────────────────────────────────────────────────

/// Reconnect to the daemon (spawning if needed) and replay the cached handshake.
#[cfg(unix)]
async fn reconnect_phase(
    cache: &HandshakeCache,
    socket_path: &Path,
    config_path: &Path,
    attempt: u32,
) -> Result<UnixStream> {
    // Exponential backoff before attempting
    let backoff = INITIAL_BACKOFF * 2u32.saturating_pow(attempt.saturating_sub(1));
    let backoff = backoff.min(MAX_BACKOFF);
    debug!(
        backoff_ms = backoff.as_millis(),
        attempt, "backoff before reconnect"
    );
    tokio::time::sleep(backoff).await;

    // Clean stale socket if daemon crashed
    cleanup_stale_socket(socket_path);

    // Connect or spawn with overall timeout
    let stream = tokio::time::timeout(
        RECONNECT_TIMEOUT,
        connect_or_spawn(socket_path, config_path),
    )
    .await
    .context("reconnect timed out")?
    .context("connect_or_spawn failed during reconnect")?;

    // Replay the handshake to the new daemon
    replay_handshake(stream, cache).await
}

#[cfg(all(test, unix))]
async fn reconnect_phase_connect_only(
    cache: &HandshakeCache,
    socket_path: &Path,
    attempt: u32,
) -> Result<UnixStream> {
    let backoff = INITIAL_BACKOFF * 2u32.saturating_pow(attempt.saturating_sub(1));
    let backoff = backoff.min(MAX_BACKOFF);
    tokio::time::sleep(backoff).await;

    let stream = tokio::time::timeout(
        RECONNECT_TIMEOUT,
        wait_for_socket(socket_path, Duration::from_secs(30)),
    )
    .await
    .context("reconnect timed out")?
    .context("connect failed during reconnect")?;

    replay_handshake(stream, cache).await
}

#[cfg(all(test, unix))]
async fn reconnect_connect_only_monitoring_client<R>(
    cache: &HandshakeCache,
    socket_path: &Path,
    attempt: u32,
    client: ReconnectClientState<'_, R>,
) -> ReconnectOutcome
where
    R: AsyncRead + Unpin,
{
    let reconnect = reconnect_phase_connect_only(cache, socket_path, attempt);
    monitor_client_during_reconnect(reconnect, client).await
}

#[cfg(unix)]
async fn reconnect_monitoring_client<R>(
    cache: &HandshakeCache,
    socket_path: &Path,
    config_path: &Path,
    attempt: u32,
    client: ReconnectClientState<'_, R>,
) -> ReconnectOutcome
where
    R: AsyncRead + Unpin,
{
    let reconnect = reconnect_phase(cache, socket_path, config_path, attempt);
    monitor_client_during_reconnect(reconnect, client).await
}

#[cfg(unix)]
async fn monitor_client_during_reconnect<R, F>(
    reconnect: F,
    client: ReconnectClientState<'_, R>,
) -> ReconnectOutcome
where
    R: AsyncRead + Unpin,
    F: std::future::Future<Output = Result<UnixStream>>,
{
    let mut orphan_check = tokio::time::interval(ORPHAN_CHECK_INTERVAL);
    orphan_check.tick().await;
    tokio::pin!(reconnect);

    loop {
        tokio::select! {
            result = &mut reconnect => {
                return match result {
                    Ok(stream) => ReconnectOutcome::Reconnected(stream),
                    Err(e) => ReconnectOutcome::Failed(e),
                };
            }
            line = mcp_framing::read_line(client.stdin) => {
                match line {
                    Ok(line) if line.is_empty() => {
                        info!("client stdin closed during daemon reconnect");
                        return ReconnectOutcome::ClientClosed;
                    }
                    Ok(line) => {
                        if client.tracker.observe_client_message(&line) == ClientMessageAction::Forward {
                            client.queued_client_messages.push(line);
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "error reading client stdin during daemon reconnect");
                        return ReconnectOutcome::ClientClosed;
                    }
                }
            }
            _ = orphan_check.tick() => {
                if std::os::unix::process::parent_id() != client.original_ppid {
                    warn!("parent process died during reconnect, proxy is orphaned — exiting");
                    return ReconnectOutcome::ParentDied;
                }
            }
        }
    }
}

#[cfg(unix)]
async fn flush_queued_client_messages(
    stream: &mut UnixStream,
    queued_client_messages: &[Vec<u8>],
) -> Result<()> {
    if queued_client_messages.is_empty() {
        return Ok(());
    }

    info!(
        count = queued_client_messages.len(),
        "flushing queued client messages after daemon reconnect"
    );

    for line in queued_client_messages {
        stream
            .write_all(line)
            .await
            .context("failed to write queued client message to reconnected daemon")?;
    }

    stream
        .flush()
        .await
        .context("failed to flush queued client messages to reconnected daemon")
}

/// Send cached handshake messages to the new daemon and discard the server's response.
///
/// The original initialize response was already forwarded to Claude Code during
/// the initial handshake. Since gatemini always advertises identical capabilities,
/// the new response is safe to discard.
#[cfg(unix)]
async fn replay_handshake(stream: UnixStream, cache: &HandshakeCache) -> Result<UnixStream> {
    let (mut sock_read, mut sock_write) = stream.into_split();

    // Send cached initialize request
    sock_write
        .write_all(&cache.initialize_request)
        .await
        .context("failed to replay initialize request")?;

    // Read (and discard) the server's initialize response
    let response = mcp_framing::read_line(&mut sock_read)
        .await
        .context("failed to read initialize response during replay")?;
    if response.is_empty() {
        bail!("daemon closed during handshake replay");
    }
    debug!("discarded replayed initialize response");

    // Send cached initialized notification
    sock_write
        .write_all(&cache.initialized_notification)
        .await
        .context("failed to replay initialized notification")?;

    // Reunite for bridge phase
    let stream = sock_read
        .reunite(sock_write)
        .context("failed to reunite socket halves after replay")?;

    info!("handshake replay complete");
    Ok(stream)
}

// ── Connection helpers (mostly preserved from original) ────────────────────

/// Connect to an existing daemon or spawn a new one.
///
/// Uses the flock + double-check pattern to prevent duplicate daemon spawning.
#[cfg(unix)]
async fn connect_or_spawn(socket_path: &Path, config_path: &Path) -> Result<UnixStream> {
    // Fast path: connect to an existing daemon.
    if let Ok(stream) = try_connect(socket_path).await {
        return Ok(stream);
    }

    // No daemon. Try to become the spawner via exclusive flock.
    match socket::try_acquire_lock(socket_path) {
        Ok(lock_guard) => {
            // Won the lock. Double-check: another proxy may have just finished
            // spawning a daemon and released its lock between our try_connect above
            // and our lock acquisition.
            if let Ok(stream) = try_connect(socket_path).await {
                drop(lock_guard);
                return Ok(stream);
            }

            // Definitely no daemon running. Spawn one.
            spawn_daemon(config_path)?;

            // Wait for daemon socket, holding lock the entire time to prevent
            // other proxies from also spawning.
            let stream = wait_for_socket(socket_path, Duration::from_secs(30)).await?;
            drop(lock_guard);
            Ok(stream)
        }
        Err(_) => {
            // Lock held by another proxy that's already spawning the daemon.
            // Just wait for the socket to appear.
            let stream = wait_for_socket(socket_path, Duration::from_secs(30)).await?;
            Ok(stream)
        }
    }
}

/// Remove stale socket/pid files if the daemon is dead.
/// Deliberately does NOT remove the lock file — that's the flock coordination mechanism.
#[cfg(unix)]
fn cleanup_stale_socket(socket_path: &Path) {
    if socket_path.exists() && !socket::is_daemon_alive(socket_path) {
        let _ = std::fs::remove_file(socket_path);
        let _ = std::fs::remove_file(socket::pid_path(socket_path));
    }
}

/// Try connecting to the daemon socket with a short timeout.
/// Pure connectivity check — no side effects (no file deletion).
#[cfg(unix)]
async fn try_connect(socket_path: &Path) -> Result<UnixStream> {
    let stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(socket_path))
        .await
        .context("connect timeout")?
        .context("connect failed")?;

    Ok(stream)
}

/// Spawn the daemon as a detached child process.
/// Called only after acquiring the exclusive flock and double-checking no daemon exists.
#[cfg(unix)]
fn spawn_daemon(config_path: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("could not determine own executable path")?;
    let config_str = config_path
        .to_str()
        .context("config path is not valid UTF-8")?;

    // Daemon cwd: use config file's parent directory (where sibling .env files live),
    // falling back to user home or /. This prevents the daemon from inheriting
    // whatever random directory the proxy happened to be in, which would leak into
    // V8 sandbox source URLs and child process working directories.
    let daemon_cwd = config_path
        .parent()
        .filter(|p| p.is_dir())
        .map(|p| p.to_path_buf())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("/"));

    let stderr = socket::open_daemon_log()
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    // Spawn detached: stdio points away from the proxy so the daemon can
    // outlive short-lived clients without logging to a closed pipe.
    let _child = std::process::Command::new(exe)
        .arg("-c")
        .arg(config_str)
        .arg("serve")
        .current_dir(&daemon_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .context("failed to spawn daemon process")?;

    Ok(())
}

/// Poll the socket path with exponential backoff until it's connectable.
#[cfg(unix)]
async fn wait_for_socket(socket_path: &Path, timeout: Duration) -> Result<UnixStream> {
    let start = std::time::Instant::now();
    let mut delay = Duration::from_millis(50);

    loop {
        if start.elapsed() > timeout {
            bail!(
                "timed out waiting for daemon socket at {}",
                socket_path.display()
            );
        }

        match UnixStream::connect(socket_path).await {
            Ok(stream) => return Ok(stream),
            Err(_) => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(1));
            }
        }
    }
}
