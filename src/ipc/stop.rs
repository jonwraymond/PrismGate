use anyhow::{Result, bail};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;

use crate::ipc::socket;

/// Stop a running gatemini daemon by sending SIGTERM.
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

    println!("Sending SIGTERM to daemon (PID {})", pid);
    signal::kill(Pid::from_raw(pid), Signal::SIGTERM)?;

    // Wait briefly for the daemon to exit.
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    while start.elapsed() < timeout {
        if !socket::is_daemon_alive(&socket_path) {
            println!("Daemon stopped.");
            // Clean up in case the daemon didn't get to it.
            socket::cleanup_files(&socket_path);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    println!(
        "Daemon did not stop within 5s. You may need to kill PID {} manually.",
        pid
    );
    Ok(())
}
