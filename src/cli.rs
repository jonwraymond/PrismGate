//! Command-line interface and standard platform path helpers.

use clap::{Parser, Subcommand, ValueEnum};
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
        .or_else(|| dirs::home_dir().map(|h| h.join(".prismgate_cache")))
        .unwrap_or_else(|| PathBuf::from(".prismgate_cache"))
        .join("prismgate")
}

#[derive(Parser)]
#[command(
    name = "prismgate",
    version,
    about = "MCP gateway with meta-tool server"
)]
pub struct Cli {
    /// Path to the configuration file.
    #[arg(short, long, default_value_os_t = prismgate_home().join("prismgate.yaml"))]
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
    /// Clear shared daemon history/statistics without restarting backends.
    Purge {
        /// Confirm clearing shared state for ALL connected clients.
        #[arg(long, required = true)]
        yes: bool,
        /// Custom daemon socket (does not start a daemon).
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Hot-upgrade the daemon without breaking existing MCP client connections.
    Upgrade {
        /// Timeout for staging and promoting the new daemon generation.
        #[arg(long, default_value = "60s", value_parser = parse_duration)]
        timeout: Duration,
    },
    /// Diagnose local proxy/daemon/runtime state without starting backends.
    Doctor,
    /// Authenticate with an OAuth 2.0 provider.
    Auth {
        /// Backend name to authenticate.
        backend: String,
        /// OAuth provider base URL.
        #[arg(long)]
        url: String,
        /// Client ID.
        #[arg(long)]
        client_id: String,
        /// OAuth scopes (comma-separated).
        #[arg(long)]
        scopes: Option<String>,
        /// Optional OAuth resource indicator (RFC 8707).
        #[arg(long)]
        resource: Option<String>,
    },
    /// Show read-only backend invocation profile from the running daemon.
    Profile {
        /// Output format: `table` (default) or `json`.
        #[arg(long, value_enum, default_value = "table")]
        format: ProfileFormat,
        /// Exact backend name to filter to.
        #[arg(long)]
        backend: Option<String>,
    },
    /// Print the compaction card for this session (open handles, decisions, constraints).
    Compact {
        /// Output format: `table` (default) or `json`.
        #[arg(long, value_enum, default_value = "table")]
        format: CompactFormat,
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

/// Output format for `prismgate profile`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum ProfileFormat {
    Table,
    Json,
}

/// Output format for `prismgate compact`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum CompactFormat {
    Table,
    Json,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_purge_requires_explicit_confirmation() {
        assert!(Cli::try_parse_from(["prismgate", "purge"]).is_err());
        assert!(Cli::try_parse_from(["prismgate", "purge", "--yes"]).is_ok());
    }

    #[test]
    fn cli_compact_accepts_table_and_json_formats() {
        assert!(Cli::try_parse_from(["prismgate", "compact"]).is_ok());
        assert!(Cli::try_parse_from(["prismgate", "compact", "--format", "json"]).is_ok());
        assert!(Cli::try_parse_from(["prismgate", "compact", "--format", "invalid"]).is_err());
    }

    #[test]
    fn cli_accepts_doctor_command() {
        let cli = Cli::try_parse_from(["prismgate", "doctor"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Doctor)));
    }

    #[test]
    fn cli_accepts_upgrade_command_with_timeout() {
        let cli = Cli::try_parse_from(["prismgate", "upgrade", "--timeout", "90s"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Upgrade { timeout }) if timeout == std::time::Duration::from_secs(90)
        ));
    }
}
