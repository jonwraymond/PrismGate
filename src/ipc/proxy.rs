use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info, warn};

#[cfg(unix)]
use tokio::net::UnixStream;

use crate::ipc::mcp_framing;
use crate::ipc::socket;

// ── Reconnection parameters ────────────────────────────────────────────────

const MAX_RECONNECTS: u32 = 10;
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(5);
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(60);

// ── Bridge buffer size ─────────────────────────────────────────────────────

const BUF_SIZE: usize = 8192;

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
#[cfg(test)]
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
#[cfg(test)]
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

    loop {
        match bridge_phase(&mut stream, &mut stdin, &mut stdout).await {
            BridgeOutcome::StdinClosed => {
                debug!("stdin closed, proxy exiting cleanly");
                return Ok(());
            }
            BridgeOutcome::DaemonDisconnected => {
                // Reset counter if the bridge ran for a reasonable time.
                if bridge_start.elapsed() > Duration::from_secs(30) {
                    reconnect_count = 0;
                }

                reconnect_count += 1;
                if reconnect_count > MAX_RECONNECTS {
                    bail!(
                        "daemon disconnected {} times in rapid succession, giving up (max {})",
                        reconnect_count,
                        MAX_RECONNECTS
                    );
                }

                warn!(
                    attempt = reconnect_count,
                    max = MAX_RECONNECTS,
                    "daemon disconnected, attempting reconnect (test mode)"
                );

                // Exponential backoff
                let backoff =
                    INITIAL_BACKOFF * 2u32.saturating_pow(reconnect_count.saturating_sub(1));
                let backoff = backoff.min(MAX_BACKOFF);
                tokio::time::sleep(backoff).await;

                // Try to connect (no spawning) with timeout
                let connect_result = tokio::time::timeout(
                    RECONNECT_TIMEOUT,
                    wait_for_socket(socket_path, Duration::from_secs(30)),
                )
                .await;

                match connect_result {
                    Ok(Ok(new_stream)) => match replay_handshake(new_stream, cache).await {
                        Ok(replayed) => {
                            stream = replayed;
                            bridge_start = std::time::Instant::now();
                            info!(
                                attempt = reconnect_count,
                                "reconnected to daemon (test mode)"
                            );
                        }
                        Err(e) => {
                            bail!("handshake replay failed: {}", e);
                        }
                    },
                    Ok(Err(e)) => bail!("reconnect failed: {}", e),
                    Err(_) => bail!("reconnect timed out"),
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

    loop {
        match bridge_phase(&mut stream, &mut stdin, &mut stdout).await {
            BridgeOutcome::StdinClosed => {
                debug!("stdin closed, proxy exiting cleanly");
                return Ok(());
            }
            BridgeOutcome::DaemonDisconnected => {
                // Reset counter if the bridge ran for a reasonable time (stable connection).
                if bridge_start.elapsed() > Duration::from_secs(30) {
                    reconnect_count = 0;
                }

                reconnect_count += 1;
                if reconnect_count > MAX_RECONNECTS {
                    bail!(
                        "daemon disconnected {} times in rapid succession, giving up (max {})",
                        reconnect_count,
                        MAX_RECONNECTS
                    );
                }

                warn!(
                    attempt = reconnect_count,
                    max = MAX_RECONNECTS,
                    "daemon disconnected, attempting reconnect"
                );

                match reconnect_phase(cache, socket_path, config_path, reconnect_count).await {
                    Ok(new_stream) => {
                        stream = new_stream;
                        bridge_start = std::time::Instant::now();
                        info!(attempt = reconnect_count, "reconnected to daemon");
                    }
                    Err(e) => {
                        bail!("reconnect failed after {} attempts: {}", reconnect_count, e);
                    }
                }
            }
        }
    }
}

// ── Bridge phase (bidirectional copy) ──────────────────────────────────────

/// Bidirectional byte bridge: stdin → socket, socket → stdout.
///
/// Uses explicit read/write_all with 8KB buffers (no io::copy) to avoid
/// lost bytes from internal BufReader buffering on cancel.
///
/// Returns when either stdin closes (StdinClosed) or the daemon socket
/// closes/errors (DaemonDisconnected).
#[cfg(unix)]
async fn bridge_phase<R, W>(stream: &mut UnixStream, stdin: &mut R, stdout: &mut W) -> BridgeOutcome
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut sock_read, mut sock_write) = stream.split();
    let mut stdin_buf = [0u8; BUF_SIZE];
    let mut sock_buf = [0u8; BUF_SIZE];

    loop {
        tokio::select! {
            // Bias toward draining daemon responses first (avoids backpressure)
            biased;

            result = sock_read.read(&mut sock_buf) => {
                match result {
                    Ok(0) => return BridgeOutcome::DaemonDisconnected,
                    Ok(n) => {
                        if stdout.write_all(&sock_buf[..n]).await.is_err() {
                            // stdout broken (Claude Code crashed) — treat as stdin close
                            return BridgeOutcome::StdinClosed;
                        }
                    }
                    Err(_) => return BridgeOutcome::DaemonDisconnected,
                }
            }

            result = stdin.read(&mut stdin_buf) => {
                match result {
                    Ok(0) => return BridgeOutcome::StdinClosed,
                    Ok(n) => {
                        if sock_write.write_all(&stdin_buf[..n]).await.is_err() {
                            return BridgeOutcome::DaemonDisconnected;
                        }
                    }
                    Err(_) => return BridgeOutcome::StdinClosed,
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
        .or_else(|| dirs::home_dir())
        .unwrap_or_else(|| std::path::PathBuf::from("/"));

    // Spawn detached: stdin/stdout null so the daemon doesn't hold our stdio.
    let _child = std::process::Command::new(exe)
        .arg("-c")
        .arg(config_str)
        .arg("serve")
        .current_dir(&daemon_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit()) // Daemon logs to stderr via tracing
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
