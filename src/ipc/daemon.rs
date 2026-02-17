use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
#[cfg(unix)]
use tokio::net::UnixListener;
use tracing::{error, info, warn};

use crate::InitializedGateway;
use crate::ipc::socket;
use crate::server::GateminiServer;

/// Bound daemon socket, ready to accept connections.
///
/// Created early (before gateway initialization) so the proxy can connect
/// immediately. MCP bytes queue in the kernel socket buffer until `run()`
/// starts accepting.
#[cfg(unix)]
pub struct BoundSocket {
    pub listener: UnixListener,
    pub socket_path: PathBuf,
}

/// Windows does not support Unix socket daemon mode.
#[cfg(not(unix))]
pub struct BoundSocket {
    pub socket_path: PathBuf,
}

/// Bind the daemon socket early, before heavy initialization.
///
/// This lets the proxy connect immediately while `initialize()` resolves
/// secrets, loads embedding models, and starts backends. Connections queue
/// in the kernel's listen backlog until `run()` calls `accept()`.
#[cfg(unix)]
pub fn bind_early(custom_socket: Option<PathBuf>) -> Result<BoundSocket> {
    let socket_path = custom_socket.unwrap_or_else(socket::default_socket_path);

    // Check for an existing daemon by trying to connect rather than stat-checking.
    // This avoids the TOCTOU race between is_daemon_alive() and remove_file().
    if socket_path.exists() {
        match std::os::unix::net::UnixStream::connect(&socket_path) {
            Ok(_) => {
                // Socket is live — another daemon is serving.
                anyhow::bail!("another daemon is already running (socket connectable)");
            }
            Err(_) => {
                // Socket file exists but nobody is listening — stale. Remove it.
                std::fs::remove_file(&socket_path)?;
            }
        }
    }

    // Bind the Unix socket listener.
    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            // Another daemon raced us and bound first — this is fine, just exit.
            anyhow::bail!("failed to bind socket (another daemon likely won the race): {e}");
        }
    };

    // Write PID file.
    let pid = std::process::id();
    std::fs::write(socket::pid_path(&socket_path), pid.to_string())?;

    // Note: tracing may not be initialized yet, so use eprintln
    eprintln!(
        "daemon socket bound: {} (pid {})",
        socket_path.display(),
        pid
    );

    Ok(BoundSocket {
        listener,
        socket_path,
    })
}

/// Non-Unix platforms do not support the daemon socket architecture.
#[cfg(not(unix))]
pub fn bind_early(custom_socket: Option<PathBuf>) -> Result<BoundSocket> {
    let socket_path = custom_socket.unwrap_or_else(socket::default_socket_path);
    Ok(BoundSocket { socket_path })
}

/// Run the daemon accept loop on a pre-bound socket.
///
/// Each connected client gets its own `GateminiServer` instance (cheap: Arc clones + tool_router
/// build). rmcp handles the full MCP protocol per-session. Client disconnect = transport EOF =
/// task ends. Other clients are unaffected.
#[cfg(unix)]
pub async fn run(gw: InitializedGateway, bound: BoundSocket) -> Result<()> {
    let BoundSocket {
        listener,
        socket_path,
    } = bound;

    info!(
        socket = %socket_path.display(),
        pid = std::process::id(),
        "daemon accepting connections"
    );

    // Wrap shared state for cloning into client tasks.
    let registry = Arc::clone(&gw.registry);
    let backend_manager = Arc::clone(&gw.backend_manager);
    let tracker = Arc::clone(&gw.tracker);
    let cache_path = gw.cache_path.clone();
    let allow_runtime_registration = gw.config.allow_runtime_registration;
    let max_dynamic_backends = gw.config.max_dynamic_backends;
    let sandbox_semaphore = Arc::new(tokio::sync::Semaphore::new(
        gw.config.sandbox.max_concurrent_sandboxes as usize,
    ));
    let shutdown_notify = Arc::clone(&gw.shutdown_notify);

    // Track active client tasks for graceful shutdown.
    let client_tracker = tokio_util::task::TaskTracker::new();

    // Session counter for idle shutdown.
    let active_sessions = Arc::new(AtomicUsize::new(0));
    // Notifies the accept loop when a client disconnects so the idle timer can be re-armed.
    let session_change = Arc::new(tokio::sync::Notify::new());
    let idle_timeout = gw.config.daemon.idle_timeout;
    let idle_enabled = !idle_timeout.is_zero();

    if idle_enabled {
        info!(timeout = ?idle_timeout, "idle shutdown enabled");
    }

    // Accept loop with signal handling and idle shutdown.
    let accept_result: Result<(), anyhow::Error> = {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

        // Idle timer — reset whenever session count changes.
        let idle_deadline = tokio::time::Instant::now() + idle_timeout;
        let idle_sleep = tokio::time::sleep_until(idle_deadline);
        tokio::pin!(idle_sleep);

        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _addr)) => {
                            let sessions = Arc::clone(&active_sessions);
                            sessions.fetch_add(1, Ordering::SeqCst);
                            info!(active = sessions.load(Ordering::SeqCst), "client connected");

                            let server = GateminiServer::new(
                                Arc::clone(&registry),
                                Arc::clone(&backend_manager),
                                Arc::clone(&tracker),
                                cache_path.clone(),
                                allow_runtime_registration,
                                max_dynamic_backends,
                                Arc::clone(&sandbox_semaphore),
                            );

                            let notify = Arc::clone(&session_change);
                            client_tracker.spawn(async move {
                                use rmcp::ServiceExt;
                                let (read, write) = tokio::io::split(stream);
                                match server.serve((read, write)).await {
                                    Ok(service) => {
                                        if let Err(e) = service.waiting().await {
                                            warn!(error = %e, "client session ended with error");
                                        }
                                    }
                                    Err(e) => {
                                        error!(error = %e, "failed to start client session");
                                    }
                                }
                                let count = sessions.fetch_sub(1, Ordering::SeqCst) - 1;
                                info!(active = count, "client disconnected");
                                notify.notify_one();
                            });

                            // Push idle timer forward while clients are active.
                            if idle_enabled {
                                idle_sleep.as_mut().reset(tokio::time::Instant::now() + idle_timeout);
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "accept failed");
                        }
                    }
                }
                _ = session_change.notified(), if idle_enabled => {
                    // Client disconnected — re-arm idle timer if no sessions remain.
                    if active_sessions.load(Ordering::SeqCst) == 0 {
                        idle_sleep.as_mut().reset(tokio::time::Instant::now() + idle_timeout);
                    }
                }
                () = &mut idle_sleep, if idle_enabled
                                       && active_sessions.load(Ordering::SeqCst) == 0 => {
                    info!(
                        timeout = ?idle_timeout,
                        "idle timeout reached with no active clients, shutting down"
                    );
                    break;
                }
                _ = sigterm.recv() => {
                    info!("received SIGTERM");
                    break;
                }
                _ = sigint.recv() => {
                    info!("received SIGINT");
                    break;
                }
            }

            // After each iteration: keep timer pushed forward while clients are active.
            if idle_enabled && active_sessions.load(Ordering::SeqCst) > 0 {
                idle_sleep
                    .as_mut()
                    .reset(tokio::time::Instant::now() + idle_timeout);
            }
        }
        Ok(())
    };

    if let Err(e) = accept_result {
        error!(error = %e, "accept loop failed");
    }

    // Graceful shutdown: stop accepting, wait for clients, stop backends, clean up files.
    info!("shutting down daemon");
    client_tracker.close();
    client_tracker.wait().await;
    shutdown_notify.notify_waiters();
    gw.backend_manager.stop_all().await;
    socket::cleanup_files(&socket_path);
    info!("daemon stopped");

    Ok(())
}

/// Non-Unix platforms do not support Unix socket daemon mode.
#[cfg(not(unix))]
pub async fn run(_gw: InitializedGateway, _bound: BoundSocket) -> Result<()> {
    anyhow::bail!("daemon mode is not supported on this platform. Run with --direct.");
}
