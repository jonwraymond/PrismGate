use anyhow::Result;

use crate::ipc::socket;

/// Show the status of a running gatemini daemon.
pub fn run() -> Result<()> {
    let socket_path = socket::default_socket_path();

    if !socket_path.exists() {
        println!(
            "No daemon running (socket not found at {})",
            socket_path.display()
        );
        return Ok(());
    }

    match socket::read_pid(&socket_path) {
        Some(pid) => {
            if socket::is_daemon_alive(&socket_path) {
                println!(
                    "Daemon running (PID {}, socket {})",
                    pid,
                    socket_path.display()
                );
            } else {
                println!(
                    "Daemon not running (stale PID file for PID {}, socket {})",
                    pid,
                    socket_path.display()
                );
                println!("Run `gatemini stop` to clean up stale files.");
            }
        }
        None => {
            println!(
                "Socket exists at {} but no PID file found",
                socket_path.display()
            );
        }
    }

    Ok(())
}
