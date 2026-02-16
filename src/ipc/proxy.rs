use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io;
use tokio::net::UnixStream;

use crate::ipc::socket;

/// Run as a thin proxy: bridge Claude Code's stdio to the daemon's Unix socket.
///
/// If no daemon is running, auto-start one. The proxy performs no initialization
/// (no config loading, no tracing to stderr, no backend management). It's a pure byte pipe.
pub async fn run(config_path: &Path) -> Result<()> {
    let socket_path = socket::default_socket_path();

    // Clean up stale socket from a crashed daemon before anything else.
    cleanup_stale_socket(&socket_path);

    // Fast path: connect to an existing daemon.
    if let Ok(stream) = try_connect(&socket_path).await {
        return bridge_stdio(stream).await;
    }

    // No daemon. Try to become the spawner via exclusive flock.
    match socket::try_acquire_lock(&socket_path) {
        Ok(lock_guard) => {
            // Won the lock. Double-check: another proxy may have just finished
            // spawning a daemon and released its lock between our try_connect above
            // and our lock acquisition.
            if let Ok(stream) = try_connect(&socket_path).await {
                drop(lock_guard);
                return bridge_stdio(stream).await;
            }

            // Definitely no daemon running. Spawn one.
            spawn_daemon(config_path)?;

            // Wait for daemon socket, holding lock the entire time to prevent
            // other proxies from also spawning.
            let stream = wait_for_socket(&socket_path, Duration::from_secs(30)).await?;
            drop(lock_guard);
            bridge_stdio(stream).await
        }
        Err(_) => {
            // Lock held by another proxy that's already spawning the daemon.
            // Just wait for the socket to appear.
            let stream = wait_for_socket(&socket_path, Duration::from_secs(30)).await?;
            bridge_stdio(stream).await
        }
    }
}

/// Remove stale socket/pid files if the daemon is dead.
/// Deliberately does NOT remove the lock file — that's the flock coordination mechanism.
fn cleanup_stale_socket(socket_path: &Path) {
    if socket_path.exists() && !socket::is_daemon_alive(socket_path) {
        let _ = std::fs::remove_file(socket_path);
        let _ = std::fs::remove_file(socket::pid_path(socket_path));
    }
}

/// Try connecting to the daemon socket with a short timeout.
/// Pure connectivity check — no side effects (no file deletion).
async fn try_connect(socket_path: &Path) -> Result<UnixStream> {
    let stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(socket_path))
        .await
        .context("connect timeout")?
        .context("connect failed")?;

    Ok(stream)
}

/// Spawn the daemon as a detached child process.
/// Called only after acquiring the exclusive flock and double-checking no daemon exists.
fn spawn_daemon(config_path: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("could not determine own executable path")?;
    let config_str = config_path
        .to_str()
        .context("config path is not valid UTF-8")?;

    // Spawn detached: stdin/stdout null so the daemon doesn't hold our stdio.
    let _child = std::process::Command::new(exe)
        .arg("-c")
        .arg(config_str)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit()) // Daemon logs to stderr via tracing
        .spawn()
        .context("failed to spawn daemon process")?;

    Ok(())
}

/// Poll the socket path with exponential backoff until it's connectable.
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

/// Bidirectional byte bridge: stdin↔socket_read, socket_write↔stdout.
/// Exits when either side closes.
async fn bridge_stdio(stream: UnixStream) -> Result<()> {
    let (mut sock_read, mut sock_write) = stream.into_split();

    let mut stdin = io::stdin();
    let mut stdout = io::stdout();

    // stdin → socket (client sends to daemon)
    let to_daemon = io::copy(&mut stdin, &mut sock_write);

    // socket → stdout (daemon sends to client)
    let from_daemon = io::copy(&mut sock_read, &mut stdout);

    tokio::select! {
        r = to_daemon => {
            if let Err(e) = r
                && e.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(e.into());
            }
        }
        r = from_daemon => {
            if let Err(e) = r
                && e.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(e.into());
            }
        }
    }

    Ok(())
}
