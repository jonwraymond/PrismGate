use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "gatemini",
    version,
    about = "MCP gateway with meta-tool server"
)]
pub struct Cli {
    /// Path to the configuration file.
    #[arg(short, long, default_value = "config/gatemini.yaml")]
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
