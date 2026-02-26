#[cfg(unix)]
use anyhow::{Result, bail};
#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

use crate::ipc::socket;

/// Restart a running gatemini daemon by sending SIGTERM and waiting for it to exit.
///
/// The daemon's client drain timeout (default 30s) plus margin gives us a 45s wait.
/// Connected proxies will detect the disconnect and auto-reconnect, spawning a new
/// daemon with the updated binary.
#[cfg(unix)]
pub fn run() -> Result<()> {
    let socket_path = socket::default_socket_path();

    let Some(pid) = socket::read_pid(&socket_path) else {
        bail!("no daemon PID file found (is the daemon running?)");
    };

    if !socket::is_daemon_alive(&socket_path) {
        println!(
            "Daemon (PID {}) is not running. Cleaning up stale files.",
            pid
        );
        socket::cleanup_files(&socket_path);
        return Ok(());
    }

    println!("Sending SIGTERM to daemon (PID {}) for restart", pid);
    signal::kill(Pid::from_raw(pid), Signal::SIGTERM)?;

    // Wait for daemon to exit. Use 45s timeout to account for client drain (30s default)
    // plus backend shutdown time.
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(45);
    while start.elapsed() < timeout {
        if !socket::is_daemon_alive(&socket_path) {
            // Clean up in case the daemon didn't get to it.
            socket::cleanup_files(&socket_path);
            println!("Daemon stopped. Proxies will auto-reconnect and spawn a new daemon.");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    println!(
        "Daemon did not stop within 45s. You may need to kill PID {} manually.",
        pid
    );
    Ok(())
}

#[cfg(not(unix))]
pub fn run() -> anyhow::Result<()> {
    println!(
        "`gatemini restart` is not supported on Windows because the daemon mode uses Unix sockets."
    );
    Ok(())
}
