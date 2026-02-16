use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Standard prismgate home directory (~/.prismgate).
/// Falls back to `.prismgate` in the current directory if home cannot be resolved.
pub fn prismgate_home() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".prismgate"))
        .unwrap_or_else(|| PathBuf::from(".prismgate"))
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
