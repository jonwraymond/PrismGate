# Gatemini Stability Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the observed stdio lifecycle and transport reliability risks that can make Gatemini or its backend MCP apps unstable.

**Architecture:** Keep the existing proxy/daemon/backend architecture. Tighten lifecycle invariants where they are currently leaky: dedicated pool capacity, backend replacement cleanup, dedicated backend health recovery, CLI adapter child process cleanup, shutdown timeouts, and operator diagnostics. Each task adds a regression test first, then the smallest production change.

**Tech Stack:** Rust 2024, Tokio, RMCP, DashMap, clap, Unix process groups, GitHub Actions release workflow.

---

## File Map

- Modify: `src/backend/pool.rs`
  - Owns dedicated backend instance capacity, assignment, release, shutdown, and pool-level test helpers.
- Modify: `src/backend/mod.rs`
  - Owns backend lifecycle orchestration, restart/re-register cleanup, shutdown drain, backend trait shape, and status data.
- Modify: `src/backend/health.rs`
  - Owns health checker restart decisions and memory-triggered restart paths.
- Modify: `src/backend/cli_adapter.rs`
  - Owns per-call CLI subprocess execution and timeout cleanup.
- Modify: `src/backend/stdio.rs`
  - Owns stdio backend process group termination and backend stop timeout reporting.
- Modify: `src/backend/http.rs`
  - Owns streamable HTTP backend stop timeout reporting.
- Modify: `src/backend/composite.rs`
  - Owns virtual backend trait defaults if the `Backend` trait changes.
- Modify: `src/testutil.rs`
  - Owns mock backend trait implementation for tests if the `Backend` trait changes.
- Modify: `src/cli.rs`
  - Adds the `doctor` command.
- Modify: `src/main.rs`
  - Dispatches the new `doctor` command.
- Create: `src/ipc/doctor.rs`
  - Local process/socket diagnostic command. No daemon initialization; no backend startup.
- Modify: `src/ipc/mod.rs`
  - Exposes the new `doctor` module.
- Modify: `src/resources.rs`
  - Optional: include richer backend process/pool status in Gatemini resources after pool stats exist.

## Current Findings To Address

- `InstancePool::new` prewarms live instances but does not consume capacity permits. The test helper does consume permits, so tests currently hide the production bug.
- `add_backend` and `restart_backend` stop the primary backend but do not stop/remove an existing dedicated pool before replacing a dedicated backend.
- Health checker status is based on `self.backends`, but dedicated restart uses `restart_pool_primary`, creating a mismatch between the status source and restart target.
- `BackendManager::stop_all` hardcodes a `7s` stop timeout even though each backend has `shutdown_grace_period`.
- `CliAdapterBackend` kills only the shell PID on timeout, not the process group.
- `Transport closed` was ambiguous. In the recent failure, one Codex process had no live Gatemini proxy child even though the daemon/backend were healthy. Operators need a quick `doctor` command that separates client/proxy/daemon/backend layers.

---

### Task 1: Fix Dedicated Pool Capacity Accounting

**Files:**
- Modify: `src/backend/pool.rs`

- [ ] **Step 1: Add a test that exposes real prewarm capacity accounting**

Add this test in `mod tests` in `src/backend/pool.rs` near the existing pool capacity tests:

```rust
#[tokio::test]
async fn prewarmed_instances_consume_capacity() {
    let (pool, _mocks) = pool_with_mocks(1, 1, 1).await;

    let _session_one = pool.acquire(1).await.unwrap();
    let result = pool.acquire(2).await;

    let err = result.err().expect("second session should not acquire");
    assert!(
        err.to_string().contains("pool exhausted"),
        "expected pool exhaustion, got {err:#}"
    );
}
```

- [ ] **Step 2: Run the focused test and verify it fails if the helper mirrors production**

Before changing production code, temporarily remove the helper's permit consumption at `src/backend/pool.rs:572-574`, run the test, then restore the helper. This proves the test captures the production bug.

Run:

```bash
cargo test backend::pool::tests::prewarmed_instances_consume_capacity -- --nocapture
```

Expected with helper permit consumption removed:

```text
FAILED
second session should not acquire
```

- [ ] **Step 3: Fix production prewarm to consume capacity permits**

Change `InstancePool::new` so every successfully prewarmed live instance consumes one capacity permit:

```rust
// Pre-warm min_idle instances. Each live idle instance consumes capacity.
for _ in 0..min_idle {
    match pool.capacity.try_acquire() {
        Ok(permit) => {
            permit.forget();
            match pool.spawn_instance().await {
                Ok(instance) => {
                    pool.idle.lock().await.push_back(instance);
                }
                Err(e) => {
                    pool.capacity.add_permits(1);
                    warn!(
                        backend = %name,
                        error = %e,
                        "failed to pre-warm pool instance"
                    );
                }
            }
        }
        Err(_) => {
            warn!(
                backend = %name,
                min_idle,
                max = max_instances,
                "pool min_idle exceeds max_instances; skipping extra prewarm"
            );
            break;
        }
    }
}
```

- [ ] **Step 4: Add capacity introspection for future diagnostics**

Add a small stats type below `InstancePool`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct PoolStats {
    pub backend_name: String,
    pub idle: usize,
    pub assigned: usize,
    pub max_instances: u32,
    pub available_capacity: usize,
}
```

Add this method:

```rust
pub async fn stats(&self) -> PoolStats {
    let idle = self.idle.lock().await.len();
    let assigned = self.assigned.lock().await.len();
    PoolStats {
        backend_name: self.backend_name.clone(),
        idle,
        assigned,
        max_instances: self.max_instances,
        available_capacity: self.capacity.available_permits(),
    }
}
```

- [ ] **Step 5: Verify pool tests**

Run:

```bash
cargo test pool
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit**

```bash
git add src/backend/pool.rs
git commit -m "fix: account for prewarmed dedicated pool capacity"
```

---

### Task 2: Stop Old Dedicated Pools During Backend Replacement

**Files:**
- Modify: `src/backend/mod.rs`

- [ ] **Step 1: Add a test helper backend that records stop calls**

Inside the existing `#[cfg(test)]` tests in `src/backend/mod.rs`, add a mock backend:

```rust
#[derive(Debug)]
struct StopCountingBackend {
    name: String,
    stopped: Arc<AtomicUsize>,
    state: AtomicU8,
}

#[async_trait]
impl Backend for StopCountingBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<()> {
        store_state(&self.state, BackendState::Healthy);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.stopped.fetch_add(1, Ordering::SeqCst);
        store_state(&self.state, BackendState::Stopped);
        Ok(())
    }

    async fn call_tool(&self, _tool_name: &str, _arguments: Option<Value>) -> Result<Value> {
        Ok(serde_json::json!({"ok": true}))
    }

    async fn discover_tools(&self) -> Result<Vec<ToolEntry>> {
        Ok(Vec::new())
    }

    fn is_available(&self) -> bool {
        self.state() == BackendState::Healthy
    }

    fn state(&self) -> BackendState {
        state_from_atomic(&self.state)
    }

    fn set_state(&self, state: BackendState) {
        store_state(&self.state, state);
    }
}
```

- [ ] **Step 2: Add cleanup coverage for backend replacement**

Add a test that verifies replacement removes stale lifecycle components:

```rust
#[tokio::test]
async fn cleanup_backend_components_removes_lifecycle_state() {
    let manager = BackendManager::new();
    let stopped = Arc::new(AtomicUsize::new(0));
    let backend = Arc::new(StopCountingBackend {
        name: "replace-me".to_string(),
        stopped: Arc::clone(&stopped),
        state: AtomicU8::new(STATE_HEALTHY),
    });

    manager.backends.insert("replace-me".to_string(), backend);
    manager
        .call_semaphores
        .insert("replace-me".to_string(), Arc::new(Semaphore::new(1)));
    manager
        .semaphore_timeouts
        .insert("replace-me".to_string(), Duration::from_secs(1));
    manager.retry_configs.insert("replace-me".to_string(), Default::default());

    manager.cleanup_backend_components("replace-me").await;

    assert_eq!(stopped.load(Ordering::SeqCst), 1);
    assert!(!manager.backends.contains_key("replace-me"));
    assert!(!manager.call_semaphores.contains_key("replace-me"));
    assert!(!manager.semaphore_timeouts.contains_key("replace-me"));
    assert!(!manager.retry_configs.contains_key("replace-me"));
}
```

- [ ] **Step 3: Add the cleanup helper**

Add this method on `BackendManager` before `add_backend`:

```rust
async fn cleanup_backend_components(&self, name: &str) {
    if let Some((_, backend)) = self.backends.remove(name)
        && let Err(e) = backend.stop().await
    {
        warn!(backend = %name, error = %e, "error stopping backend");
    }

    if let Some((_, pool)) = self.dedicated_pools.remove(name) {
        pool.stop_all().await;
    }

    self.call_semaphores.remove(name);
    self.semaphore_timeouts.remove(name);
    self.retry_configs.remove(name);
    self.rate_limiters.remove(name);
    if let Some((_, handle)) = self.rate_limiter_handles.remove(name) {
        handle.abort();
    }
    self.memory_stats.remove(name);

    if let Some((_, pid)) = self.prerequisite_pids.remove(name) {
        prerequisite::stop_prerequisite(name, pid).await;
    }
}
```

- [ ] **Step 4: Use cleanup before replacement**

In `add_backend`, replace the existing primary-only cleanup with:

```rust
if self.backends.contains_key(name) || self.dedicated_pools.contains_key(name) {
    warn!(backend = %name, "stopping existing backend before re-registration");
    self.cleanup_backend_components(name).await;
    registry.remove_backend_tools(name);
}
```

In `restart_backend`, replace the primary-only stop block with:

```rust
self.cleanup_backend_components(name).await;
registry.remove_backend_tools(name);
```

In `remove_backend`, use:

```rust
self.cleanup_backend_components(name).await;
registry.remove_backend_tools(name);
```

Keep the config/dynamic removal that is specific to `remove_backend`.

- [ ] **Step 5: Verify lifecycle tests**

Run:

```bash
cargo test cleanup_backend_components restart_backend add_backend remove_backend
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit**

```bash
git add src/backend/mod.rs
git commit -m "fix: stop dedicated pools before backend replacement"
```

---

### Task 3: Align Dedicated Health Recovery With Full Backend Restart

**Files:**
- Modify: `src/backend/health.rs`
- Modify: `src/backend/mod.rs`
- Modify: `src/backend/pool.rs`

- [ ] **Step 1: Add a test for the health restart target decision**

Add a small unit test in `src/backend/health.rs` that locks the intended policy:

```rust
#[test]
fn dedicated_backends_use_full_restart_policy() {
    assert_eq!(restart_policy_for_backend(true), RestartPolicy::FullBackend);
    assert_eq!(restart_policy_for_backend(false), RestartPolicy::FullBackend);
}
```

Define the local enum/function in `health.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestartPolicy {
    FullBackend,
}

fn restart_policy_for_backend(_is_dedicated: bool) -> RestartPolicy {
    RestartPolicy::FullBackend
}
```

- [ ] **Step 2: Replace pool-primary restarts with full backend restarts**

In `src/backend/health.rs`, replace:

```rust
let restart_result = if manager.is_dedicated(&status.name) {
    timeout(
        config.restart_timeout,
        manager.restart_pool_primary(&status.name, &registry),
    )
    .await
} else {
    timeout(
        config.restart_timeout,
        manager.restart_backend(&status.name, &registry),
    )
    .await
};
```

with:

```rust
let restart_result = match restart_policy_for_backend(manager.is_dedicated(&status.name)) {
    RestartPolicy::FullBackend => {
        timeout(
            config.restart_timeout,
            manager.restart_backend(&status.name, &registry),
        )
        .await
    }
};
```

In the memory-triggered restart path, replace the dedicated/single split with:

```rust
if let Err(e) = manager.restart_backend(name, &registry).await {
    error!(backend = %name, error = %e, "memory-triggered restart failed");
}
```

- [ ] **Step 3: Keep `restart_pool_primary` only if diagnostics still use it**

If no callers remain after Step 2, remove:

```rust
pub async fn restart_pool_primary(...)
```

from `src/backend/mod.rs`, and remove:

```rust
pub async fn restart_primary(...)
```

from `src/backend/pool.rs`. If a caller remains, leave them but add `#[allow(dead_code)]` and a comment that full backend restart is the health path.

- [ ] **Step 4: Verify health tests**

Run:

```bash
cargo test health restart
```

Expected:

```text
test result: ok
```

- [ ] **Step 5: Commit**

```bash
git add src/backend/health.rs src/backend/mod.rs src/backend/pool.rs
git commit -m "fix: restart full dedicated backends during health recovery"
```

---

### Task 4: Use Process Groups For CLI Adapter Timeouts

**Files:**
- Modify: `src/backend/cli_adapter.rs`

- [ ] **Step 1: Add a Unix-only regression test for timeout child cleanup**

Add this test in `src/backend/cli_adapter.rs` tests:

```rust
#[cfg(unix)]
#[tokio::test]
async fn timeout_kills_cli_adapter_process_group() {
    use std::fs;

    let marker = tempfile::NamedTempFile::new().unwrap();
    let marker_path = marker.path().to_path_buf();
    let command = format!(
        "sh -c 'trap \"\" TERM; sleep 30 &' && echo child-started > {} && sleep 30",
        marker_path.display()
    );

    let mut tools = HashMap::new();
    tools.insert(
        "hang".to_string(),
        CliToolConfig {
            description: "hangs with child".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            command,
            stdin: None,
            output: CliOutputFormat::Text,
        },
    );

    let backend = CliAdapterBackend {
        name: "cli-timeout".to_string(),
        tools,
        env: HashMap::new(),
        cwd: None,
        timeout: Duration::from_millis(200),
        health_check: None,
        state: AtomicU8::new(STATE_HEALTHY),
    };

    let result = backend.call_tool("hang", None).await;
    assert!(result.is_err());
    assert!(fs::read_to_string(&marker_path).unwrap().contains("child-started"));

    tokio::time::sleep(Duration::from_millis(300)).await;

    let ps = std::process::Command::new("pgrep")
        .args(["-f", marker_path.to_string_lossy().as_ref()])
        .output()
        .unwrap();
    assert!(
        ps.stdout.is_empty(),
        "expected process group to be killed, found: {}",
        String::from_utf8_lossy(&ps.stdout)
    );
}
```

- [ ] **Step 2: Put CLI adapter commands in a process group**

In `build_shell_command`, add before env/cwd:

```rust
#[cfg(unix)]
{
    cmd.process_group(0);
}
```

- [ ] **Step 3: Kill the process group on timeout**

Replace the timeout kill block:

```rust
#[cfg(unix)]
unsafe {
    libc::kill(pid as i32, libc::SIGKILL);
}
```

with:

```rust
#[cfg(unix)]
unsafe {
    libc::kill(-(pid as i32), libc::SIGKILL);
}
```

Keep the Windows path unchanged for now.

- [ ] **Step 4: Verify CLI adapter tests**

Run:

```bash
cargo test cli_adapter
```

Expected:

```text
test result: ok
```

- [ ] **Step 5: Commit**

```bash
git add src/backend/cli_adapter.rs
git commit -m "fix: kill cli adapter process groups on timeout"
```

---

### Task 5: Respect Backend Shutdown Grace Periods

**Files:**
- Modify: `src/backend/mod.rs`
- Modify: `src/backend/stdio.rs`
- Modify: `src/backend/cli_adapter.rs`
- Modify: `src/backend/http.rs`
- Modify: `src/backend/composite.rs`
- Modify: `src/testutil.rs`

- [ ] **Step 1: Extend the backend trait**

In `src/backend/mod.rs`, add this method to the `Backend` trait:

```rust
fn stop_timeout(&self) -> Duration {
    Duration::from_secs(7)
}
```

- [ ] **Step 2: Implement configured stop timeouts**

In `StdioBackend`:

```rust
fn stop_timeout(&self) -> Duration {
    self.config.shutdown_grace_period + Duration::from_secs(2)
}
```

In `CliAdapterBackend`, add a field:

```rust
shutdown_grace_period: Duration,
```

Set it in `new`:

```rust
shutdown_grace_period: config.shutdown_grace_period,
```

Update test struct literals with:

```rust
shutdown_grace_period: Duration::from_secs(5),
```

Implement:

```rust
fn stop_timeout(&self) -> Duration {
    self.shutdown_grace_period + Duration::from_secs(2)
}
```

In `HttpBackend` and `CompositeBackend`, keep the default unless there is a config field already available.

- [ ] **Step 3: Use the backend-provided timeout in stop_all**

Replace the hardcoded timeout block in `BackendManager::stop_all`:

```rust
let timeout = std::time::Duration::from_secs(7);
match tokio::time::timeout(timeout, backend.stop()).await {
```

with:

```rust
let timeout = backend.stop_timeout();
match tokio::time::timeout(timeout, backend.stop()).await {
```

- [ ] **Step 4: Add a unit test for custom shutdown timeout**

Add a test in `src/backend/stdio.rs`:

```rust
#[test]
fn stdio_stop_timeout_uses_configured_grace_period() {
    let mut config = BackendConfig {
        shutdown_grace_period: Duration::from_secs(12),
        ..test_config()
    };
    config.command = Some("sleep".to_string());

    let backend = StdioBackend::new("timeout-check".to_string(), config);
    assert_eq!(backend.stop_timeout(), Duration::from_secs(14));
}
```

- [ ] **Step 5: Verify backend tests**

Run:

```bash
cargo test backend
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit**

```bash
git add src/backend/mod.rs src/backend/stdio.rs src/backend/cli_adapter.rs src/backend/http.rs src/backend/composite.rs src/testutil.rs
git commit -m "fix: respect backend shutdown grace periods"
```

---

### Task 6: Add `gatemini doctor` Diagnostics

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/ipc/mod.rs`
- Create: `src/ipc/doctor.rs`

- [ ] **Step 1: Add the CLI command**

In `src/cli.rs`, add to `enum Command`:

```rust
/// Diagnose local proxy/daemon/runtime state without starting backends.
Doctor,
```

- [ ] **Step 2: Wire command dispatch**

In `src/main.rs`, add to the command match:

```rust
(Some(cli::Command::Doctor), _) => ipc::doctor::run(),
```

- [ ] **Step 3: Expose the module**

In `src/ipc/mod.rs`, add:

```rust
pub mod doctor;
```

- [ ] **Step 4: Implement local diagnostics**

Create `src/ipc/doctor.rs`:

```rust
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
    println!("daemon_pid: {}", pid.map(|p| p.to_string()).unwrap_or_else(|| "none".to_string()));
    println!("daemon_alive: {alive}");
    println!("socket_exists: {}", socket_path.exists());

    if !alive && socket_path.exists() {
        println!("warning: socket exists but daemon is not alive; run `gatemini status` or `gatemini stop` to clean stale files");
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
```

- [ ] **Step 5: Add a smoke test for the command**

If the project already has CLI invocation tests, add this there. If not, add a small test in `src/cli.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_accepts_doctor_command() {
        Cli::command().debug_assert();
    }
}
```

- [ ] **Step 6: Verify command behavior**

Run:

```bash
cargo run -- doctor
```

Expected output includes:

```text
gatemini doctor
version: 1.12.2
socket:
daemon_alive:
current_exe:
```

- [ ] **Step 7: Commit**

```bash
git add src/cli.rs src/main.rs src/ipc/mod.rs src/ipc/doctor.rs
git commit -m "feat: add gatemini doctor diagnostics"
```

---

### Task 7: Full Verification And Release Readiness

**Files:**
- No source edits unless verification exposes a bug.

- [ ] **Step 1: Run focused stability tests**

```bash
cargo test pool
cargo test cli_adapter
cargo test health restart
```

Expected:

```text
test result: ok
```

- [ ] **Step 2: Run full test suite**

```bash
cargo test
```

Expected:

```text
test result: ok
```

- [ ] **Step 3: Build release binary**

```bash
cargo build --release
target/release/gatemini --version
```

Expected:

```text
gatemini 1.12.2
```

- [ ] **Step 4: Smoke test daemon/proxy startup**

Use a temporary config with one cheap stdio backend if possible. If using the live config, do not kill Jon's active daemon without asking.

Safe live check:

```bash
gatemini --version
gatemini status
gatemini doctor
```

Expected:

```text
gatemini 1.12.2
Daemon running ...
gatemini doctor
```

- [ ] **Step 5: Confirm process count no longer grows for dedicated backends**

Before test:

```bash
ps -axo pid,ppid,stat,etime,command | rg 'mcp-server-sequential-thinking|server-sequential-thinking'
```

Run three short manual MCP sessions through Gatemini that call sequential-thinking, then repeat the `ps` command. Expected: process count returns to `primary + min_idle + active client sessions`, not monotonic growth.

- [ ] **Step 6: Prepare PR**

```bash
git status --short
git log --oneline -7
gh pr create --title "fix: harden backend lifecycle stability" --body "$(cat <<'BODY'
## Summary
- Fix dedicated pool capacity accounting for prewarmed instances.
- Stop stale dedicated pools during backend replacement/restart.
- Align health recovery with full dedicated backend lifecycle restart.
- Kill CLI adapter process groups on timeout.
- Respect configured backend shutdown grace periods.
- Add `gatemini doctor` diagnostics for local proxy/daemon state.

## Verification
- cargo test pool
- cargo test cli_adapter
- cargo test health restart
- cargo test
- cargo build --release
- gatemini doctor
BODY
)"
```

---

## Self-Review

**Spec coverage:** The plan covers all six stability items from the review: pool capacity, stale pool cleanup, dedicated health recovery, CLI adapter process cleanup, shutdown timeout correctness, and diagnostics.

**Placeholder scan:** No `TBD`, `TODO`, or "implement later" placeholders remain. Every task has concrete files, code shape, commands, and expected results.

**Type consistency:** New names are consistent across tasks: `PoolStats`, `cleanup_backend_components`, `stop_timeout`, `RestartPolicy`, and `doctor`.

## Execution Choice

Plan complete and saved to `docs/superpowers/plans/2026-04-14-gatemini-stability-hardening.md`.

Two execution options:

1. **Subagent-Driven (recommended)** - Dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints.

