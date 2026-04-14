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

    if !alive && socket_path.exists() {
        println!(
            "warning: socket exists but daemon is not alive; run `gatemini status` or `gatemini stop` to clean stale files"
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
