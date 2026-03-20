# Process Supervision Overhaul

**Date:** 2026-03-20
**Status:** Approved
**Goal:** Fix memory/swap issues caused by improper process cleanup at the supervision layer

## Problem

Gatemini's child process supervision has several gaps that cause memory and swap pressure:

1. `kill_child()` waits only 200ms after SIGTERM before SIGKILL — heavy backends like serena (Python, 16GB RSS) don't have time to clean up
2. Backend stderr is discarded (`Stdio::null()`) — OOM kills, tracebacks, and memory warnings are invisible
3. Pool `release()` immediately spawns a replacement instance before the OS has reclaimed the old process's memory — briefly 2x usage
4. No visibility into child process memory consumption — issues discovered externally via `ps aux`
5. `stop_all()` doesn't enforce `drain_timeout` — in-flight calls can be interrupted
6. `gatemini stop` gives up after 5s but daemon drain is 30s — user force-kills during graceful shutdown
7. Prerequisite cleanup sends SIGTERM but doesn't wait for exit — zombie processes

## Design

### 1. Graceful Kill with Configurable Grace Period

Replace the hardcoded 200ms sleep in `kill_child()` with an active wait loop:

1. Send SIGTERM to process group (Unix) or `taskkill /T /PID` (Windows)
2. Poll `child.try_wait()` every 100ms for up to `shutdown_grace_period`
3. If still alive after deadline → SIGKILL (Unix) or `taskkill /F /T /PID` (Windows)
4. Log whether process exited gracefully or was force-killed

**Config:** `shutdown_grace_period` on `BackendConfig` (default: 5s)

```yaml
backends:
  serena:
    shutdown_grace_period: 15s
  sequential-thinking:
    shutdown_grace_period: 3s
```

### 2. Stderr Capture Ring Buffer

Replace `Stdio::null()` with `Stdio::piped()` on stderr. Spawn a tokio task per backend that reads stderr lines into a bounded ring buffer (200 lines, ~20KB per backend).

**Data structure:** `Arc<Mutex<VecDeque<String>>>` on `StdioBackend`

**Exposure:**
- New method `StdioBackend::recent_stderr(&self, limit: usize) -> Vec<String>`
- Added to `gatemini://backend/{name}` resource JSON as `recent_stderr` field
- Logged at `warn!` level when backend exits unexpectedly (reaper task)

**No config needed** — always on, minimal overhead.

### 3. Pool Replenish Delay

After `release()` stops an instance, wait before checking `min_idle` replenishment:

1. `release()` calls `instance.stop()` (waits up to `shutdown_grace_period`)
2. Sleep `replenish_delay` before spawning replacement
3. Gives OS time to reclaim swap/RSS from dead process

**Config:** `pool.replenish_delay` on `PoolConfig` (default: 2s)

```yaml
backends:
  serena:
    pool:
      replenish_delay: 5s
```

### 4. Per-Backend Memory Tracking

**Collection:** Every `memory_check_interval` (default 30s), run a single `ps -o pid,rss=` call (Unix) or `tasklist /FI "PID eq"` (Windows) for all live backend PIDs. Parse RSS, store in `DashMap<String, MemoryStats>`.

```rust
pub struct MemoryStats {
    pub pid: u32,
    pub rss_kb: u64,
    pub peak_rss_kb: u64,
    pub last_sampled: Instant,
}
```

**Memory limit with auto-restart:**
- Optional `max_memory_mb` on `BackendConfig` (default: None)
- When exceeded, log warning and trigger restart (same path as health checker)
- Cooldown: `memory_restart_cooldown` (default 60s) prevents restart loops

```yaml
backends:
  serena:
    max_memory_mb: 2048
health:
  memory_check_interval: 30s
  memory_restart_cooldown: 60s
```

**Exposure:**
- `gatemini://health` — new resource: per-backend PID, RSS, peak RSS, memory limit, % used
- `gatemini://backend/{name}` — add `memory` section with current/peak RSS

### 5. Fix stop_all() Drain and gatemini stop Timeout

**5a. Per-backend stop timeout:** Wrap each `backend.stop()` in `tokio::time::timeout(shutdown_grace_period, ...)`. On timeout, SIGKILL.

**5b. Stop command reads config:** `gatemini stop` reads `client_drain_timeout + drain_timeout` from config and uses that as its wait timeout instead of hardcoded 5s. Shows progress message.

**5c. Cleanup guard:** Socket/PID/lock file cleanup guaranteed via Drop guard, even if `stop_all()` panics:

```rust
struct CleanupGuard<'a> { path: &'a Path }
impl Drop for CleanupGuard<'_> {
    fn drop(&mut self) { socket::cleanup_files(self.path); }
}
```

### 6. Windows Support

| Component | Unix | Windows |
|-----------|------|---------|
| Graceful kill | `kill(-pgid, SIGTERM)` → poll → `kill(-pgid, SIGKILL)` | `taskkill /T /PID` → poll → `taskkill /F /T /PID` |
| Process group | `cmd.process_group(0)` | `CREATE_NEW_PROCESS_GROUP` flag |
| Memory query | `ps -o pid,rss= -p PIDs` | `tasklist /FI "PID eq" /FO CSV /NH` |
| Prerequisite stop | `kill(-pgid, SIGTERM)` + wait | `taskkill /T` + wait |
| Daemon mode | Unix domain socket | Stubbed (existing) — separate effort |

## Config Reference

### BackendConfig (new fields)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `shutdown_grace_period` | Duration | `5s` | Time between SIGTERM and SIGKILL |
| `max_memory_mb` | Option<u64> | None | Auto-restart if RSS exceeds this |

### PoolConfig (new fields)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `replenish_delay` | Duration | `2s` | Wait after stop before spawning replacement |

### HealthConfig (new fields)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `memory_check_interval` | Duration | `30s` | RSS sampling interval |
| `memory_restart_cooldown` | Duration | `60s` | Min time between memory restarts |

## Files

| File | Changes |
|------|---------|
| `src/config.rs` | New fields on BackendConfig, PoolConfig, HealthConfig |
| `src/backend/stdio.rs` | Rewrite `kill_child()`, stderr ring buffer, expose PID |
| `src/backend/mod.rs` | MemoryStats, memory DashMap, fix `stop_all()` |
| `src/backend/pool.rs` | `replenish_delay` in release() |
| `src/backend/health.rs` | Memory check cycle, auto-restart, cooldown |
| `src/backend/prerequisite.rs` | Grace period + SIGKILL fallback |
| `src/resources.rs` | `gatemini://health`, memory/stderr in backend resource |
| `src/ipc/stop.rs` | Config-based timeout |
| `src/ipc/daemon.rs` | CleanupGuard |
| `CLAUDE.md` | Document supervision features |

**Estimated: ~600 new lines, ~100 modified, 10 files**

## Success Criteria

1. `gatemini stop` waits for actual drain completion
2. Heavy backends (serena) exit gracefully within `shutdown_grace_period`
3. Backend stderr visible via `gatemini://backend/{name}`
4. `gatemini://health` shows live RSS per backend
5. Backends exceeding `max_memory_mb` are auto-restarted
6. Pool replenishment doesn't cause 2x memory spike
7. All 273+ tests pass, clippy clean
8. Windows: graceful kill and memory tracking functional
