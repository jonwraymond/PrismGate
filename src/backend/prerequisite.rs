use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::config::PrerequisiteConfig;

/// Check if a process matching the given pattern is already running.
/// Uses `pgrep -f <pattern>` — returns true if at least one match found.
pub async fn is_process_running(pattern: &str) -> Result<bool> {
    let output = Command::new("pgrep")
        .args(["-f", pattern])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .context("failed to run pgrep")?;

    Ok(output.success())
}

/// Ensure a prerequisite process is running before a backend starts.
///
/// - If `process_match` is set and a matching process is found, returns `Ok(None)`.
/// - Otherwise, spawns the prerequisite and waits `startup_delay`.
/// - Returns `Ok(Some(pid))` if a new process was spawned.
pub async fn ensure_prerequisite(
    backend_name: &str,
    config: &PrerequisiteConfig,
) -> Result<Option<u32>> {
    // Check if already running
    if let Some(pattern) = &config.process_match {
        match is_process_running(pattern).await {
            Ok(true) => {
                info!(
                    backend = %backend_name,
                    pattern = %pattern,
                    "prerequisite already running"
                );
                return Ok(None);
            }
            Ok(false) => {
                info!(
                    backend = %backend_name,
                    pattern = %pattern,
                    "prerequisite not found, spawning"
                );
            }
            Err(e) => {
                warn!(
                    backend = %backend_name,
                    error = %e,
                    "pgrep failed, attempting to spawn prerequisite anyway"
                );
            }
        }
    } else {
        warn!(
            backend = %backend_name,
            "no process_match set — spawning prerequisite without dedup check"
        );
    }

    // Spawn the prerequisite process
    let mut cmd = Command::new(&config.command);

    if !config.args.is_empty() {
        cmd.args(&config.args);
    }

    for (k, v) in &config.env {
        cmd.env(k, v);
    }

    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }

    // Detached: null stdin/stdout, inherit stderr for debugging
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::inherit());

    // Process group isolation for clean termination
    #[cfg(unix)]
    cmd.process_group(0);

    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn prerequisite for backend '{backend_name}'"))?;

    let pid = child.id().unwrap_or(0);

    info!(
        backend = %backend_name,
        pid,
        command = %config.command,
        "prerequisite process spawned"
    );

    // Wait for the process to initialize
    if !config.startup_delay.is_zero() {
        debug!(
            backend = %backend_name,
            delay_secs = config.startup_delay.as_secs(),
            "waiting for prerequisite startup"
        );
        tokio::time::sleep(config.startup_delay).await;
    }

    Ok(Some(pid))
}

/// Stop a managed prerequisite process. Sends SIGTERM, waits up to 5s, then SIGKILL.
pub async fn stop_prerequisite(backend_name: &str, pid: u32) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        let pgid = Pid::from_raw(-(pid as i32));
        match signal::kill(pgid, Signal::SIGTERM) {
            Ok(()) => {
                info!(
                    backend = %backend_name,
                    pid,
                    "sent SIGTERM to prerequisite process group, waiting up to 5s"
                );

                // Poll for exit up to 5s
                for _ in 0..50 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    // Check if process group is gone (kill with signal 0 = existence check)
                    if signal::kill(pgid, None).is_err() {
                        info!(backend = %backend_name, pid, "prerequisite exited gracefully");
                        return;
                    }
                }

                // Still alive — SIGKILL
                warn!(
                    backend = %backend_name,
                    pid,
                    "prerequisite didn't exit within 5s, sending SIGKILL"
                );
                let _ = signal::kill(pgid, Signal::SIGKILL);
            }
            Err(e) => {
                warn!(
                    backend = %backend_name,
                    pid,
                    error = %e,
                    "failed to send SIGTERM to prerequisite"
                );
            }
        }
    }

    #[cfg(windows)]
    {
        // Graceful tree kill first
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        // Wait up to 5s
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            // Check if process still exists by trying to open it
            let check = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/NH"])
                .output();
            if let Ok(output) = check {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if !stdout.contains(&pid.to_string()) {
                    info!(backend = %backend_name, pid, "prerequisite exited gracefully");
                    return;
                }
            }
        }

        // Force kill
        warn!(backend = %backend_name, pid, "prerequisite didn't exit within 5s, force killing");
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[cfg(not(any(unix, windows)))]
    {
        warn!(
            backend = %backend_name,
            pid,
            "prerequisite stop not supported on this platform"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_is_process_running_nonexistent() {
        let result = is_process_running("nonexistent-process-xyz-12345").await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_ensure_prerequisite_spawns_process() {
        let config = PrerequisiteConfig {
            command: "sleep".to_string(),
            args: vec!["0.1".to_string()],
            env: Default::default(),
            cwd: None,
            process_match: None,
            managed: false,
            startup_delay: Duration::from_millis(50),
        };

        let result = ensure_prerequisite("test-backend", &config).await;
        assert!(result.is_ok());
        let pid = result.unwrap();
        assert!(pid.is_some());
        assert!(pid.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_ensure_prerequisite_skips_when_running() {
        // Use a pattern that matches a process guaranteed to be running.
        // On macOS/Linux, our own test binary has "gatemini" in its path.
        // Fall back to "launchd" or "init" as universal system processes.
        let pattern = if is_process_running("gatemini").await.unwrap_or(false) {
            "gatemini"
        } else {
            "init"
        };

        let config = PrerequisiteConfig {
            command: "echo".to_string(),
            args: vec!["should-not-run".to_string()],
            env: Default::default(),
            cwd: None,
            process_match: Some(pattern.to_string()),
            managed: false,
            startup_delay: Duration::from_millis(10),
        };

        let result = ensure_prerequisite("test-backend", &config).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none()); // Should skip — process is running
    }
}
