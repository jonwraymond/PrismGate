use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Standard prismgate config directory.
/// Uses platform config dirs (Linux/macOS: ~/.config, Windows: %APPDATA%).
/// Falls back to `.gatemini` in the current directory if home cannot be resolved.
pub fn prismgate_home() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(dirs::data_dir)
        .unwrap_or_else(|| dirs::home_dir().map(|h| h.join(".gatemini")).unwrap_or_else(|| PathBuf::from(".gatemini")))
        .join("gatemini")
}

/// Standard cache root for downloaded assets and generated caches.
/// Uses the platform cache directory (Linux/macOS: ~/.cache, Windows: %LOCALAPPDATA%)
/// with fallback to config/data home.
pub fn prismgate_cache_home() -> PathBuf {
    dirs::cache_dir()
        .or_else(dirs::config_dir)
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| dirs::home_dir().map(|h| h.join(".gatemini_cache")).unwrap_or_else(|| PathBuf::from(".gatemini_cache")))
        .join("gatemini")
}

#[derive(Parser)]
#[command(
    name = "gatemini",
    version,
    about = "MCP gateway with meta-tool server"
)]
pub struct Cli {
    /// Path to the configuration file.
    #[arg(short, long, default_value_os_t = prismgate_home().join("config.yaml"))]
    pub config: PathBuf,

    /// Run in legacy direct stdio mode (1:1, no daemon).
    #[arg(long)]
    pub direct: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run as a daemon, accepting client connections over a Unix socket.
    Serve {
        /// Custom Unix socket path (default: auto-detected per platform).
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Show the status of a running daemon.
    Status,
    /// Stop a running daemon.
    Stop,
}
