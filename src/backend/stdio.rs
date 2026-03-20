use anyhow::{Context, Result};
use rmcp::{ServiceExt, model::*, service::RunningService};
use serde_json::Value;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

const STDERR_BUFFER_SIZE: usize = 200;

use super::{Backend, BackendState, STATE_HEALTHY, STATE_STARTING, STATE_STOPPED};
use super::{
    is_available_from_atomic, map_call_tool_result, map_tools_to_entries, state_from_atomic,
    store_state,
};
use crate::config::BackendConfig;
use crate::registry::ToolEntry;

/// A stdio child-process MCP backend using rmcp.
///
/// Spawns the child process directly to retain the `Child` handle for:
/// - Instant crash detection via `wait_for_exit()`
/// - Process group isolation for clean kill-group cleanup
/// - PID tracking for the SIGTERM safety net
pub struct StdioBackend {
    name: String,
    config: BackendConfig,
    service: RwLock<Option<RunningService<rmcp::RoleClient, ()>>>,
    state: AtomicU8,
    child: RwLock<Option<tokio::process::Child>>,
    stderr_buffer: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    pid: std::sync::atomic::AtomicU32,
}

impl StdioBackend {
    pub fn new(name: String, config: BackendConfig) -> Self {
        Self {
            name,
            config,
            service: RwLock::new(None),
            state: AtomicU8::new(STATE_STARTING),
            child: RwLock::new(None),
            stderr_buffer: Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::with_capacity(STDERR_BUFFER_SIZE),
            )),
            pid: std::sync::atomic::AtomicU32::new(0),
        }
    }

    fn build_command(&self) -> Command {
        let cmd_str = self.config.command.as_deref().unwrap_or("echo");
        let mut cmd = Command::new(cmd_str);

        if !self.config.args.is_empty() {
            cmd.args(&self.config.args);
        }

        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        if let Some(cwd) = &self.config.cwd {
            cmd.current_dir(cwd);
        }

        cmd
    }

    /// Kill the child's entire process group with a configurable grace period.
    ///
    /// Phase 1: Request graceful shutdown (SIGTERM on unix, taskkill /T on windows).
    /// Phase 2: Poll for exit up to `shutdown_grace_period`.
    /// Phase 3: Force kill if the child hasn't exited.
    async fn kill_child(&self, child: &mut tokio::process::Child) {
        let grace = self.config.shutdown_grace_period;

        // Phase 1: Request graceful shutdown
        #[cfg(unix)]
        if let Some(pid) = child.id() {
            let ret = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
            if ret == 0 {
                debug!(backend = %self.name, pid, grace_secs = grace.as_secs(), "sent SIGTERM to process group");
            } else {
                warn!(backend = %self.name, pid, "failed to signal process group");
            }
        }

        #[cfg(windows)]
        if let Some(pid) = child.id() {
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/PID", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            debug!(backend = %self.name, pid, "sent taskkill /T");
        }

        // Phase 2: Poll for exit up to grace period
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    info!(backend = %self.name, ?status, "child exited gracefully");
                    return;
                }
                Ok(None) => {
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(e) => {
                    warn!(backend = %self.name, error = %e, "error checking child status");
                    break;
                }
            }
        }

        // Phase 3: Force kill
        warn!(backend = %self.name, grace_secs = grace.as_secs(), "child didn't exit within grace period, force killing");

        #[cfg(unix)]
        if let Some(pid) = child.id() {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }

        #[cfg(windows)]
        if let Some(pid) = child.id() {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }

        let _ = child.kill().await;
    }
}

#[async_trait::async_trait]
impl Backend for StdioBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<()> {
        self.state.store(STATE_STARTING, Ordering::Release);

        let mut cmd = self.build_command();
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Each child in its own process group for clean kill-group cleanup
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn backend '{}'", self.name))?;

        let pid = child.id();
        if let Some(p) = pid {
            self.pid.store(p, Ordering::Release);
        }
        debug!(backend = %self.name, pid = ?pid, "spawned child process");

        let stdout = child.stdout.take().ok_or_else(|| {
            anyhow::anyhow!("failed to capture stdout from backend '{}'", self.name)
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            anyhow::anyhow!("failed to capture stdin from backend '{}'", self.name)
        })?;

        if let Some(stderr) = child.stderr.take() {
            let buf = Arc::clone(&self.stderr_buffer);
            let name = self.name.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut buffer = buf.lock().unwrap_or_else(|e| e.into_inner());
                    if buffer.len() >= STDERR_BUFFER_SIZE {
                        buffer.pop_front();
                    }
                    buffer.push_back(line);
                }
                debug!(backend = %name, "stderr reader finished");
            });
        }

        // rmcp accepts (AsyncRead, AsyncWrite) tuples as IntoTransport
        let service = ()
            .serve((stdout, stdin))
            .await
            .with_context(|| format!("failed MCP handshake with backend '{}'", self.name))?;

        if let Some(peer) = service.peer_info() {
            info!(
                backend = %self.name,
                pid = ?pid,
                server_name = %peer.server_info.name,
                server_version = %peer.server_info.version,
                "MCP handshake complete"
            );
        } else {
            info!(backend = %self.name, pid = ?pid, "MCP handshake complete (no peer info)");
        }

        *self.service.write().await = Some(service);
        *self.child.write().await = Some(child);
        self.state.store(STATE_HEALTHY, Ordering::Release);

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.state.store(STATE_STOPPED, Ordering::Release);

        // Cancel rmcp service first (closes transport gracefully)
        if let Some(service) = self.service.write().await.take()
            && let Err(e) = service.cancel().await
        {
            error!(backend = %self.name, error = %e, "error cancelling service");
        }

        // Kill child and its process group
        if let Some(mut child) = self.child.write().await.take() {
            self.kill_child(&mut child).await;
        }

        info!(backend = %self.name, "backend stopped");
        Ok(())
    }

    async fn call_tool(&self, tool_name: &str, arguments: Option<Value>) -> Result<Value> {
        let guard = self.service.read().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("backend '{}' not started", self.name))?;

        let mut params = CallToolRequestParams::new(tool_name.to_string());
        if let Some(args) = arguments.and_then(|v| v.as_object().cloned()) {
            params = params.with_arguments(args);
        }

        debug!(backend = %self.name, tool = %tool_name, "calling tool");

        let result = tokio::time::timeout(self.config.timeout, service.call_tool(params))
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "tool call '{}' on backend '{}' timed out after {:?}",
                    tool_name,
                    self.name,
                    self.config.timeout
                )
            })?
            .map_err(|e| {
                anyhow::anyhow!(
                    "tool call '{}' on backend '{}' failed: {}",
                    tool_name,
                    self.name,
                    e
                )
            })?;

        Ok(map_call_tool_result(result))
    }

    async fn discover_tools(&self) -> Result<Vec<ToolEntry>> {
        let guard = self.service.read().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("backend '{}' not started", self.name))?;

        let tools = service.list_all_tools().await.map_err(|e| {
            anyhow::anyhow!("tool discovery on backend '{}' failed: {}", self.name, e)
        })?;

        let entries = map_tools_to_entries(tools, &self.name);
        info!(backend = %self.name, tools = entries.len(), "discovered tools");
        Ok(entries)
    }

    fn is_available(&self) -> bool {
        is_available_from_atomic(&self.state)
    }

    fn state(&self) -> BackendState {
        state_from_atomic(&self.state)
    }

    fn set_state(&self, state: BackendState) {
        store_state(&self.state, state);
    }

    async fn wait_for_exit(&self) -> Option<std::process::ExitStatus> {
        let mut guard = self.child.write().await;
        if let Some(child) = guard.as_mut() {
            child.wait().await.ok()
        } else {
            None
        }
    }

    fn recent_stderr(&self, limit: usize) -> Vec<String> {
        let buffer = self.stderr_buffer.lock().unwrap_or_else(|e| e.into_inner());
        buffer.iter().rev().take(limit).rev().cloned().collect()
    }

    fn pid(&self) -> Option<u32> {
        let p = self.pid.load(Ordering::Acquire);
        if p > 0 { Some(p) } else { None }
    }
}
