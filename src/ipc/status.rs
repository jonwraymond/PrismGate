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
                let version = socket::read_generation_info(&socket_path)
                    .map(|info| info.version)
                    .unwrap_or_else(|| "unknown".to_string());
                println!(
                    "Daemon running (PID {}, version {}, socket {})",
                    pid,
                    version,
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

    let drains = socket::discover_drain_generations(&socket_path);
    if !drains.is_empty() {
        println!("Draining daemon generations:");
        for drain in drains {
            let version = std::fs::read(&drain.info_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<socket::GenerationInfo>(&bytes).ok())
                .map(|info| info.version)
                .unwrap_or_else(|| "unknown".to_string());
            println!(
                "  PID {} (version {}, socket {}, alive {})",
                drain.pid,
                version,
                drain.socket_path.display(),
                drain.alive
            );
        }
    }

    Ok(())
}
