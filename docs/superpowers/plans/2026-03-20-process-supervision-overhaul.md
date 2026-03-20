# Process Supervision Overhaul Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix memory/swap issues caused by improper process cleanup — configurable kill grace periods, stderr capture, memory tracking, and pool replenish delay.

**Architecture:** Extend existing BackendConfig/PoolConfig/HealthConfig with new supervision fields. Modify StdioBackend kill_child() to poll-wait instead of sleep. Add stderr ring buffer per backend. Add periodic RSS sampling via `ps` subprocess. Expose via new `gatemini://health` resource.

**Tech Stack:** Rust, tokio, nix (Unix signals), std::process (Windows taskkill)

**Spec:** `docs/superpowers/specs/2026-03-20-process-supervision-overhaul-design.md`

---

## Chunk 1: Config + Graceful Kill

### Task 1: Add new config fields

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add `shutdown_grace_period` to BackendConfig**

After the `health_check` field (~line 270), add:

```rust
/// Time to wait after SIGTERM before SIGKILL. Default: 5s.
#[serde(default = "default_shutdown_grace_period", with = "humantime_duration")]
pub shutdown_grace_period: Duration,
```

Add default function after other defaults (~line 656):

```rust
fn default_shutdown_grace_period() -> Duration {
    Duration::from_secs(5)
}
```

- [ ] **Step 2: Add `max_memory_mb` to BackendConfig**

After `shutdown_grace_period`:

```rust
/// Auto-restart backend if RSS exceeds this (MB). None = no limit.
#[serde(default)]
pub max_memory_mb: Option<u64>,
```

- [ ] **Step 3: Add `replenish_delay` to PoolConfig**

In `PoolConfig` struct (~line 366), after `acquire_timeout`:

```rust
/// Delay after stopping an instance before spawning replacement. Default: 2s.
#[serde(default = "default_replenish_delay", with = "humantime_duration")]
pub replenish_delay: Duration,
```

Add default and update `Default` impl:

```rust
fn default_replenish_delay() -> Duration {
    Duration::from_secs(2)
}
```

- [ ] **Step 4: Add memory fields to HealthConfig**

In `HealthConfig` struct (~line 421), after `drain_timeout`:

```rust
/// How often to sample child process RSS. Default: 30s.
#[serde(default = "default_memory_check_interval", with = "humantime_duration")]
pub memory_check_interval: Duration,

/// Min time between memory-triggered restarts per backend. Default: 60s.
#[serde(default = "default_memory_restart_cooldown", with = "humantime_duration")]
pub memory_restart_cooldown: Duration,
```

Add defaults and update `Default` impl.

- [ ] **Step 5: Update register.rs BackendConfig construction**

In `src/tools/register.rs` (~line 161), add:

```rust
shutdown_grace_period: default_shutdown_grace_period(),
max_memory_mb: None,
```

- [ ] **Step 6: Update cli_adapter.rs BackendConfig construction**

In `src/backend/cli_adapter.rs` test (~line 767), add the new fields.

- [ ] **Step 7: Update concurrency_tests.rs BackendConfig construction**

In `src/backend/concurrency_tests.rs` (~line 163), add the new fields.

- [ ] **Step 8: Verify compilation**

Run: `cargo check --all-features`
Expected: compiles with no errors

- [ ] **Step 9: Commit**

```bash
git add src/config.rs src/tools/register.rs src/backend/cli_adapter.rs src/backend/concurrency_tests.rs
git commit -m "feat: add supervision config fields (shutdown_grace_period, max_memory_mb, replenish_delay, memory_check)"
```

---

### Task 2: Rewrite kill_child() with configurable grace period

**Files:**
- Modify: `src/backend/stdio.rs`

- [ ] **Step 1: Write test for graceful shutdown**

Add to the test module in `src/backend/stdio.rs` (or `src/testutil.rs`):

```rust
#[test]
fn test_shutdown_grace_period_config_default() {
    let config = BackendConfig { ..default_test_config() };
    assert_eq!(config.shutdown_grace_period, Duration::from_secs(5));
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test test_shutdown_grace_period`
Expected: FAIL (field doesn't exist yet or default not wired)

- [ ] **Step 3: Rewrite `kill_child` (lines 64–81)**

Replace the current implementation:

```rust
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
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo test`
Expected: all 273+ tests pass, clippy clean

- [ ] **Step 5: Commit**

```bash
git add src/backend/stdio.rs
git commit -m "feat: rewrite kill_child with configurable grace period and Windows support"
```

---

## Chunk 2: Stderr Ring Buffer

### Task 3: Add stderr capture to StdioBackend

**Files:**
- Modify: `src/backend/stdio.rs`

- [ ] **Step 1: Add stderr ring buffer field to StdioBackend struct**

```rust
/// Ring buffer of recent stderr lines (last 200 lines per backend).
stderr_buffer: Arc<Mutex<VecDeque<String>>>,
```

Add `const STDERR_BUFFER_SIZE: usize = 200;` at the top of the file.

Update `StdioBackend::new()` to initialize it:

```rust
stderr_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_BUFFER_SIZE))),
```

- [ ] **Step 2: Change stderr from null to piped in `start()`**

Line 96: change `.stderr(Stdio::null())` to `.stderr(Stdio::piped())`

- [ ] **Step 3: Spawn stderr reader task in `start()`**

After taking stdout (line 109), add stderr reader:

```rust
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
```

- [ ] **Step 4: Add `recent_stderr` method**

```rust
/// Get the last N lines from the stderr ring buffer.
pub fn recent_stderr(&self, limit: usize) -> Vec<String> {
    let buffer = self.stderr_buffer.lock().unwrap_or_else(|e| e.into_inner());
    buffer.iter().rev().take(limit).rev().cloned().collect()
}
```

- [ ] **Step 5: Log stderr on unexpected exit (reaper task)**

In `src/backend/mod.rs`, in the reaper task (~line 418), after logging the unexpected exit, add:

```rust
// Log last stderr lines for debugging
if let Some(stdio) = backend.as_any().downcast_ref::<stdio::StdioBackend>() {
    let stderr = stdio.recent_stderr(20);
    if !stderr.is_empty() {
        warn!(backend = %reaper_name, "last stderr lines:\n{}", stderr.join("\n"));
    }
}
```

Note: This requires adding a `fn as_any(&self) -> &dyn std::any::Any` method to the `Backend` trait, or alternatively making `recent_stderr` available via a new trait method. The simpler approach is to add `fn recent_stderr(&self, _limit: usize) -> Vec<String> { Vec::new() }` as a default method on `Backend`.

- [ ] **Step 6: Add `recent_stderr` to Backend trait**

In `src/backend/mod.rs`, add to the `Backend` trait:

```rust
/// Recent stderr lines (last N). Only available for stdio backends.
fn recent_stderr(&self, _limit: usize) -> Vec<String> {
    Vec::new()
}
```

Override in `StdioBackend`:

```rust
fn recent_stderr(&self, limit: usize) -> Vec<String> {
    let buffer = self.stderr_buffer.lock().unwrap_or_else(|e| e.into_inner());
    buffer.iter().rev().take(limit).rev().cloned().collect()
}
```

- [ ] **Step 7: Verify compilation and tests**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo test`
Expected: all tests pass, clippy clean

- [ ] **Step 8: Commit**

```bash
git add src/backend/stdio.rs src/backend/mod.rs
git commit -m "feat: capture backend stderr in ring buffer (200 lines per backend)"
```

---

### Task 4: Expose stderr in backend resource

**Files:**
- Modify: `src/resources.rs`

- [ ] **Step 1: Add `recent_stderr` to BackendDetail struct**

In `src/resources.rs`, add to `BackendDetail`:

```rust
#[serde(skip_serializing_if = "Vec::is_empty")]
recent_stderr: Vec<String>,
```

- [ ] **Step 2: Populate stderr in read_resource backend handler**

In the `gatemini://backend/{name}` match arm, after building `BackendDetail`, add:

```rust
// Get stderr from the backend if available
let recent_stderr = backend_manager
    .get_backend_stderr(backend_name, 50)
    .unwrap_or_default();
```

Add `get_backend_stderr` method to `BackendManager`:

```rust
pub fn get_backend_stderr(&self, name: &str, limit: usize) -> Option<Vec<String>> {
    self.backends.get(name).map(|b| b.value().recent_stderr(limit))
}
```

- [ ] **Step 3: Verify and commit**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo test`

```bash
git add src/resources.rs src/backend/mod.rs
git commit -m "feat: expose backend stderr in gatemini://backend/{name} resource"
```

---

## Chunk 3: Pool Replenish Delay

### Task 5: Add replenish delay to pool release

**Files:**
- Modify: `src/backend/pool.rs`

- [ ] **Step 1: Write test for replenish delay**

```rust
#[test]
fn test_replenish_delay_config() {
    let config = PoolConfig {
        replenish_delay: Duration::from_secs(5),
        ..PoolConfig::default()
    };
    assert_eq!(config.replenish_delay, Duration::from_secs(5));
}
```

- [ ] **Step 2: Add `replenish_delay` field to InstancePool**

```rust
replenish_delay: Duration,
```

Wire it in `InstancePool::new()` from `config.pool.replenish_delay`.

- [ ] **Step 3: Add delay in `release()` before replenish check**

After `self.capacity.add_permits(1);` (line ~292), before the min_idle check:

```rust
// Wait for OS to reclaim memory from stopped process before spawning replacement
if !self.replenish_delay.is_zero() {
    tokio::time::sleep(self.replenish_delay).await;
}
```

- [ ] **Step 4: Verify and commit**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo test`

```bash
git add src/backend/pool.rs src/config.rs
git commit -m "feat: add configurable replenish_delay to pool release"
```

---

## Chunk 4: Memory Tracking

### Task 6: Add MemoryStats and RSS sampling

**Files:**
- Create: `src/backend/memory.rs`
- Modify: `src/backend/mod.rs`

- [ ] **Step 1: Create `src/backend/memory.rs`**

```rust
//! Per-backend memory tracking via periodic RSS sampling.

use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;
use dashmap::DashMap;
use serde::Serialize;
use tracing::{debug, warn};

/// Memory statistics for a single backend process.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryStats {
    pub pid: u32,
    pub rss_kb: u64,
    pub peak_rss_kb: u64,
    pub last_sampled: String, // ISO timestamp for serialization
    #[serde(skip)]
    pub last_sampled_instant: Instant,
}

/// Sample RSS for a list of PIDs using `ps` (Unix) or `tasklist` (Windows).
/// Returns a map of PID → RSS in KB.
pub async fn sample_rss(pids: &[u32]) -> Result<HashMap<u32, u64>> {
    if pids.is_empty() {
        return Ok(HashMap::new());
    }

    #[cfg(unix)]
    {
        let pid_args: Vec<String> = pids.iter().map(|p| p.to_string()).collect();
        let output = tokio::process::Command::new("ps")
            .arg("-o")
            .arg("pid=,rss=")
            .arg("-p")
            .arg(pid_args.join(","))
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = HashMap::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let (Ok(pid), Ok(rss)) = (parts[0].parse::<u32>(), parts[1].parse::<u64>()) {
                    result.insert(pid, rss);
                }
            }
        }
        Ok(result)
    }

    #[cfg(windows)]
    {
        let mut result = HashMap::new();
        for pid in pids {
            let output = tokio::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Parse CSV: "name","pid","session","session#","mem usage"
            for line in stdout.lines() {
                let fields: Vec<&str> = line.split(',').collect();
                if fields.len() >= 5 {
                    let mem_str = fields[4].trim_matches('"').replace(" K", "").replace(",", "");
                    if let Ok(rss) = mem_str.parse::<u64>() {
                        result.insert(*pid, rss);
                    }
                }
            }
        }
        Ok(result)
    }

    #[cfg(not(any(unix, windows)))]
    {
        Ok(HashMap::new())
    }
}
```

- [ ] **Step 2: Register module in mod.rs**

Add `pub mod memory;` to `src/backend/mod.rs`.

- [ ] **Step 3: Add memory tracking to BackendManager**

Add field:

```rust
memory_stats: DashMap<String, memory::MemoryStats>,
```

Initialize in both constructors. Add methods:

```rust
pub fn update_memory_stats(&self, name: &str, stats: memory::MemoryStats) {
    self.memory_stats.insert(name.to_string(), stats);
}

pub fn get_memory_stats(&self, name: &str) -> Option<memory::MemoryStats> {
    self.memory_stats.get(name).map(|r| r.value().clone())
}

pub fn get_all_memory_stats(&self) -> Vec<(String, memory::MemoryStats)> {
    self.memory_stats.iter().map(|r| (r.key().clone(), r.value().clone())).collect()
}
```

- [ ] **Step 4: Add PID accessor to Backend trait**

```rust
fn pid(&self) -> Option<u32> { None }
```

Override in `StdioBackend`:

```rust
fn pid(&self) -> Option<u32> {
    // Read from the child handle
    tokio::task::block_in_place(|| {
        self.child.blocking_read().as_ref().and_then(|c| c.id())
    })
}
```

Actually, since `child.id()` is available after spawn and doesn't change, store it as a field:

```rust
pid: std::sync::atomic::AtomicU32,
```

Set it during `start()`, read it in `pid()`:

```rust
fn pid(&self) -> Option<u32> {
    let p = self.pid.load(Ordering::Relaxed);
    if p > 0 { Some(p) } else { None }
}
```

- [ ] **Step 5: Verify and commit**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo test`

```bash
git add src/backend/memory.rs src/backend/mod.rs src/backend/stdio.rs
git commit -m "feat: add per-backend memory tracking via RSS sampling"
```

---

### Task 7: Wire memory checks into health checker

**Files:**
- Modify: `src/backend/health.rs`

- [ ] **Step 1: Add memory check state**

Add to `BackendHealth`:

```rust
last_memory_restart: Option<Instant>,
```

- [ ] **Step 2: Add memory check phase after Phase 3**

After the existing Phase 3 (pending backend retry), add Phase 4:

```rust
// Phase 4: Memory check — sample RSS and auto-restart if over limit
if !config.memory_check_interval.is_zero() {
    let pids: Vec<(String, u32)> = statuses.iter()
        .filter_map(|s| {
            manager.backends.get(&s.name)
                .and_then(|b| b.value().pid().map(|p| (s.name.clone(), p)))
        })
        .collect();

    if !pids.is_empty() {
        let pid_list: Vec<u32> = pids.iter().map(|(_, p)| *p).collect();
        if let Ok(rss_map) = crate::backend::memory::sample_rss(&pid_list).await {
            for (name, pid) in &pids {
                if let Some(&rss_kb) = rss_map.get(pid) {
                    // Update stats
                    let existing = manager.get_memory_stats(name);
                    let peak = existing.as_ref().map(|s| s.peak_rss_kb.max(rss_kb)).unwrap_or(rss_kb);
                    manager.update_memory_stats(name, crate::backend::memory::MemoryStats {
                        pid: *pid,
                        rss_kb,
                        peak_rss_kb: peak,
                        last_sampled: chrono::Utc::now().to_rfc3339(), // or use a simpler timestamp
                        last_sampled_instant: Instant::now(),
                    });

                    // Check memory limit
                    if let Some(backend_config) = manager.get_backend_config(name).await {
                        if let Some(limit_mb) = backend_config.max_memory_mb {
                            let rss_mb = rss_kb / 1024;
                            if rss_mb > limit_mb {
                                let health = health_map.get_mut(name).unwrap();
                                let can_restart = health.last_memory_restart
                                    .map(|t| t.elapsed() > config.memory_restart_cooldown)
                                    .unwrap_or(true);
                                if can_restart {
                                    warn!(backend = %name, rss_mb, limit_mb, "RSS exceeds memory limit, restarting");
                                    // Use same restart path as health checker
                                    let _ = manager.restart_backend(name, &registry).await;
                                    health.last_memory_restart = Some(Instant::now());
                                }
                            } else if rss_mb > limit_mb * 80 / 100 {
                                warn!(backend = %name, rss_mb, limit_mb, "RSS at 80% of memory limit");
                            }
                        }
                    }
                }
            }
        }
    }
}
```

Note: We don't need the `chrono` crate — use a simple epoch string instead. Replace `chrono::Utc::now().to_rfc3339()` with a humantime format or just store the Instant.

- [ ] **Step 3: Verify and commit**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo test`

```bash
git add src/backend/health.rs
git commit -m "feat: add memory check phase to health checker with auto-restart"
```

---

## Chunk 5: Health Resource + Stop Fix + Cleanup Guard

### Task 8: Add gatemini://health resource

**Files:**
- Modify: `src/resources.rs`

- [ ] **Step 1: Add to `list_static_resources()`**

Add a new `Resource` entry for `gatemini://health`:

```rust
Annotated::new(
    RawResource::new("gatemini://health", "health")
        .with_title("Backend Health & Memory")
        .with_description("Per-backend PID, RSS, peak RSS, memory limit, and stderr")
        .with_mime_type("application/json"),
    None,
),
```

- [ ] **Step 2: Add match arm in `read_resource()`**

```rust
"health" => {
    let statuses = backend_manager.get_all_status();
    let health: Vec<serde_json::Value> = statuses.iter().map(|s| {
        let mem = backend_manager.get_memory_stats(&s.name);
        let stderr = backend_manager.get_backend_stderr(&s.name, 10).unwrap_or_default();
        serde_json::json!({
            "name": s.name,
            "state": format!("{:?}", s.state),
            "available": s.available,
            "pid": backend_manager.backends.get(&s.name).and_then(|b| b.value().pid()),
            "memory": mem.map(|m| serde_json::json!({
                "rss_kb": m.rss_kb,
                "rss_mb": m.rss_kb / 1024,
                "peak_rss_kb": m.peak_rss_kb,
                "peak_rss_mb": m.peak_rss_kb / 1024,
            })),
            "recent_stderr": stderr,
        })
    }).collect();
    let json = serde_json::to_string_pretty(&health)
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    Ok(text_resource(uri, &json))
}
```

- [ ] **Step 3: Verify and commit**

```bash
git add src/resources.rs
git commit -m "feat: add gatemini://health resource with memory stats and stderr"
```

---

### Task 9: Fix stop_all() drain and gatemini stop timeout

**Files:**
- Modify: `src/backend/mod.rs`
- Modify: `src/ipc/stop.rs`
- Modify: `src/ipc/daemon.rs`

- [ ] **Step 1: Add per-backend timeout to stop_all()**

In `stop_all()`, replace the bare `backend.stop().await` join set with timeouts:

```rust
for (name, backend) in backends {
    let grace = self.configs.read().await
        .get(&name)
        .map(|c| c.shutdown_grace_period)
        .unwrap_or(Duration::from_secs(5));
    join_set.spawn(async move {
        match tokio::time::timeout(grace + Duration::from_secs(2), backend.stop()).await {
            Ok(Ok(())) => info!(backend = %name, "backend stopped"),
            Ok(Err(e)) => warn!(backend = %name, error = %e, "error stopping backend"),
            Err(_) => warn!(backend = %name, grace_secs = grace.as_secs(), "backend stop timed out, force killing"),
        }
    });
}
```

- [ ] **Step 2: Fix stop command timeout**

In `src/ipc/stop.rs`, replace the hardcoded 5s:

```rust
// Read config to determine appropriate timeout
let config_path = socket::config_path_from_socket(&socket_path);
let timeout_secs = if let Ok(config) = crate::config::Config::load(&config_path) {
    config.daemon.client_drain_timeout.as_secs() + config.health.drain_timeout.as_secs() + 5
} else {
    35 // fallback: 30s drain + 5s buffer
};
let timeout = Duration::from_secs(timeout_secs);
```

If `config_path_from_socket` doesn't exist, add a simpler approach: just use 35s as a reasonable default that matches the typical daemon shutdown time.

- [ ] **Step 3: Add CleanupGuard to daemon shutdown**

In `src/ipc/daemon.rs`, before the shutdown sequence (~line 237):

```rust
struct CleanupGuard<'a> {
    path: &'a std::path::Path,
}
impl Drop for CleanupGuard<'_> {
    fn drop(&mut self) {
        socket::cleanup_files(self.path);
    }
}
let _cleanup = CleanupGuard { path: &socket_path };
```

Remove the existing `socket::cleanup_files(&socket_path)` call since the guard handles it.

- [ ] **Step 4: Verify and commit**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo test`

```bash
git add src/backend/mod.rs src/ipc/stop.rs src/ipc/daemon.rs
git commit -m "fix: enforce drain timeout in stop_all, fix stop command timeout, add cleanup guard"
```

---

### Task 10: Fix prerequisite cleanup with grace period

**Files:**
- Modify: `src/backend/prerequisite.rs`

- [ ] **Step 1: Add grace period and SIGKILL to stop_prerequisite()**

Replace the current fire-and-forget SIGTERM:

```rust
pub async fn stop_prerequisite(backend_name: &str, pid: u32) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        let pgid = Pid::from_raw(-(pid as i32));
        match signal::kill(pgid, Signal::SIGTERM) {
            Ok(()) => {
                info!(backend = %backend_name, pid, "sent SIGTERM to prerequisite process group");

                // Wait up to 5s for exit
                for _ in 0..50 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if signal::kill(pgid, None).is_err() {
                        info!(backend = %backend_name, pid, "prerequisite exited gracefully");
                        return;
                    }
                }

                // Force kill
                warn!(backend = %backend_name, pid, "prerequisite didn't exit within 5s, sending SIGKILL");
                let _ = signal::kill(pgid, Signal::SIGKILL);
            }
            Err(e) => {
                warn!(backend = %backend_name, pid, error = %e, "failed to send SIGTERM to prerequisite");
            }
        }
    }

    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/PID", &pid.to_string()])
            .status();
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }

    #[cfg(not(any(unix, windows)))]
    {
        warn!(backend = %backend_name, pid, "prerequisite stop not supported on this platform");
    }
}
```

- [ ] **Step 2: Verify and commit**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo test`

```bash
git add src/backend/prerequisite.rs
git commit -m "fix: prerequisite cleanup waits for exit with SIGKILL fallback"
```

---

### Task 11: Update CLAUDE.md and docs

**Files:**
- Modify: `CLAUDE.md`
- Modify: `docs/backend-management.md`

- [ ] **Step 1: Add supervision section to CLAUDE.md**

Under `## Important implementation notes`:

```markdown
## Process supervision

- `shutdown_grace_period` (default 5s) controls SIGTERM → SIGKILL window per backend
- backend stderr captured in ring buffer (200 lines), exposed via `gatemini://backend/{name}`
- `gatemini://health` shows per-backend PID, RSS, peak RSS, memory limit
- `max_memory_mb` auto-restarts backends exceeding RSS limit (with 60s cooldown)
- pool `replenish_delay` (default 2s) prevents memory spike when recycling instances
- prerequisite cleanup sends SIGTERM, waits 5s, then SIGKILL
- daemon socket cleanup guaranteed via Drop guard even on panic
```

- [ ] **Step 2: Update backend-management.md**

Add a `## Process supervision` section documenting the new behaviors.

- [ ] **Step 3: Verify and commit**

```bash
git add CLAUDE.md docs/backend-management.md
git commit -m "docs: document process supervision features"
```

---

### Task 12: Final verification

- [ ] **Step 1: Full test + clippy**

Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
Expected: all tests pass, clippy clean, fmt clean

- [ ] **Step 2: Create PR**

```bash
git checkout -b feat/process-supervision-overhaul
git push -u origin feat/process-supervision-overhaul
gh pr create --title "feat: process supervision overhaul" --body "..."
```

- [ ] **Step 3: Wait for CI, merge, release**
