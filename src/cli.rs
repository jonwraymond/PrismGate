//! Command-line interface and standard platform path helpers.

use clap::{Parser, Subcommand};
use clap_complete::{Generator, shells};
use std::path::PathBuf;
use std::time::Duration;

/// Standard prismgate config directory.
/// Defaults to `~/.prismgate`. Falls back to platform config dirs if home
/// cannot be resolved.
pub fn prismgate_home() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".prismgate"))
        .or_else(dirs::config_dir)
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from(".prismgate"))
}

/// Standard cache root for downloaded assets and generated caches.
/// Uses the platform cache directory (Linux/macOS: ~/.cache, Windows: %LOCALAPPDATA%)
/// with fallback to config/data home.
pub fn prismgate_cache_home() -> PathBuf {
    dirs::cache_dir()
        .or_else(dirs::config_dir)
        .or_else(dirs::data_dir)
        .or_else(|| dirs::home_dir().map(|h| h.join(".gatemini_cache")))
        .unwrap_or_else(|| PathBuf::from(".gatemini_cache"))
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
    #[arg(short, long, default_value_os_t = prismgate_home().join("gatemini.yaml"))]
    pub config: PathBuf,

    /// Run in legacy direct stdio mode (1:1, no daemon).
    #[arg(long)]
    pub direct: bool,

    /// Validate the config file and exit without starting backends or the daemon.
    /// Useful for testing config changes before deploying them.
    #[arg(long)]
    pub dry_run: bool,

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
        /// Internal: promote this staged daemon to the public socket after initialization.
        #[arg(long, hide = true)]
        promote_to: Option<PathBuf>,
        /// Internal: PID of the daemon generation that should enter drain mode.
        #[arg(long, hide = true)]
        old_pid: Option<i32>,
    },
    /// Show the status of a running daemon.
    Status,
    /// Stop a running daemon.
    Stop,
    /// Restart a running daemon (stop + let proxies auto-spawn new).
    Restart,
    /// Hot-upgrade the daemon without breaking existing MCP client connections.
    Upgrade {
        /// Timeout for staging and promoting the new daemon generation.
        #[arg(long, default_value = "60s", value_parser = parse_duration)]
        timeout: Duration,
    },
    /// Diagnose local proxy/daemon/runtime state without starting backends.
    Doctor,
    /// Generate shell completion scripts for bash, zsh, and fish.
    Completion {
        /// Shell to generate completions for: bash, elvish, fish, powershell, zsh.
        #[arg(value_name = "shell", default_value = "bash")]
        shell: String,
    },
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    let value = value.trim();
    if let Some(seconds) = value.strip_suffix('s') {
        seconds
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|e| format!("invalid duration '{value}': {e}"))
    } else if let Some(minutes) = value.strip_suffix('m') {
        minutes
            .parse::<u64>()
            .map(|m| Duration::from_secs(m * 60))
            .map_err(|e| format!("invalid duration '{value}': {e}"))
    } else if let Some(hours) = value.strip_suffix('h') {
        hours
            .parse::<u64>()
            .map(|h| Duration::from_secs(h * 3600))
            .map_err(|e| format!("invalid duration '{value}': {e}"))
    } else {
        value
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| format!("invalid duration '{value}': expected 30s, 5m, or 1h"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_accepts_doctor_command() {
        let cli = Cli::try_parse_from(["gatemini", "doctor"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Doctor)));
    }

    #[test]
    fn cli_accepts_upgrade_command_with_timeout() {
        let cli = Cli::try_parse_from(["gatemini", "upgrade", "--timeout", "90s"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Upgrade { timeout }) if timeout == std::time::Duration::from_secs(90)
        ));
    }
}
