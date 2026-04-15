use anyhow::Result;

use crate::ipc::socket;

#[cfg(unix)]
pub fn run() -> Result<()> {
    let socket_path = socket::default_socket_path();
    let pid = socket::read_pid(&socket_path);
    let alive = socket::is_daemon_alive(&socket_path);

    println!("gatemini doctor");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("socket: {}", socket_path.display());
    println!("pid_file: {}", socket::pid_path(&socket_path).display());
    println!(
        "daemon_pid: {}",
        pid.map(|p| p.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!("daemon_alive: {alive}");
    println!("socket_exists: {}", socket_path.exists());
    if let Some(info) = socket::read_generation_info(&socket_path) {
        println!("daemon_version: {}", info.version);
        println!("daemon_role: {:?}", info.role);
        if info.version != env!("CARGO_PKG_VERSION") {
            println!(
                "warning: installed gatemini version {} differs from daemon version {}",
                env!("CARGO_PKG_VERSION"),
                info.version
            );
        }
    } else {
        println!("daemon_version: unknown");
    }

    if !alive && socket_path.exists() {
        println!(
            "warning: socket exists but daemon is not alive; run `gatemini status` or `gatemini stop` to clean stale files"
        );
    }

    let drains = socket::discover_drain_generations(&socket_path);
    println!("draining_generations: {}", drains.len());
    for drain in drains {
        let version = std::fs::read(&drain.info_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<socket::GenerationInfo>(&bytes).ok())
            .map(|info| info.version)
            .unwrap_or_else(|| "unknown".to_string());
        println!(
            "draining_pid: {} version={} alive={} socket={}",
            drain.pid,
            version,
            drain.alive,
            drain.socket_path.display()
        );
    }

    let current_exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("unknown ({e})"));
    println!("current_exe: {current_exe}");

    Ok(())
}

#[cfg(not(unix))]
pub fn run() -> Result<()> {
    println!("gatemini doctor");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("daemon_mode: unsupported on this platform");
    Ok(())
}
