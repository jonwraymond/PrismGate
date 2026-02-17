# Backend Management

Gatemini manages many backend MCP servers through a concurrent lifecycle system with health monitoring, circuit breaking, and automatic recovery.

![Backend Health & Circuit Breaker](diagrams/backend-lifecycle.svg)

## BackendManager

**Source**: [`src/backend/mod.rs`](../src/backend/mod.rs)

The `BackendManager` is the central authority for backend lifecycle:

```rust
struct BackendManager {
    backends: Arc<DashMap<String, Arc<dyn Backend>>>,  // Running backends
    configs: RwLock<HashMap<String, BackendConfig>>,    // All backend configs
    in_flight_calls: AtomicUsize,                       // Global call counter
    dynamic_backends: RwLock<HashSet<String>>,           // Runtime-registered
    prerequisite_pids: DashMap<String, u32>,             // Managed prerequisites
}
```

### Why DashMap?

`DashMap` provides lock-free concurrent reads through internal sharding (one shard per CPU core, each with an independent `RwLock`). Multiple backends can register tools simultaneously at startup without contention. This is critical during initialization when larger deployments connect and discover tools concurrently.

## Backend Trait

All backends implement a common trait:

| Method | Purpose |
|--------|---------|
| `start()` | Spawn process / connect to HTTP endpoint, perform MCP handshake |
| `stop()` | Graceful shutdown, terminate child process |
| `call_tool(name, args)` | Forward tool call to the backend |
| `discover_tools()` | List available tools via MCP |
| `is_available()` | Quick state check (non-blocking) |
| `state()` | Current state: Starting, Healthy, Unhealthy, Stopped |
| `wait_for_exit()` | Monitor child process for unexpected exit (stdio only) |

## Stdio Backends

**Source**: [`src/backend/stdio.rs`](../src/backend/stdio.rs)

Stdio backends are MCP servers spawned as child processes communicating over stdin/stdout.

### Spawn and Handshake

```
1. tokio::process::Command::new(command)
   ├─ .args(args)
   ├─ .envs(env)                     -- Resolved secrets injected here
   ├─ .stdin(Stdio::piped())
   ├─ .stdout(Stdio::piped())
   ├─ .stderr(Stdio::inherit())      -- Backend logs visible in daemon stderr
   └─ .process_group(0)              -- New process group for clean termination

2. rmcp handshake: ().serve((stdout, stdin))
   └─ MCP initialize → peer_info (server_name, version)

3. Spawn reaper task (monitors for unexpected exit)
```

### Process Termination

```rust
// Step 1: SIGTERM to process group (child + all its children)
libc::kill(-(pid as i32), libc::SIGTERM);

// Step 2: Wait 200ms for graceful exit
tokio::time::sleep(Duration::from_millis(200)).await;

// Step 3: Force kill as fallback
child.kill().await;
```

Process group isolation (`process_group(0)`) ensures that if a backend spawns subprocesses (e.g., Node.js spawning worker threads), they're all terminated together. Without this, killing the parent would orphan children to init.

### Reaper Task

Each stdio backend spawns a background task that monitors for unexpected exits:

```rust
async fn reaper(backend: Arc<StdioBackend>) {
    backend.wait_for_exit().await;
    // Mark as Stopped — health checker will auto-restart
}
```

This provides immediate crash detection rather than waiting for the next health check interval.

## HTTP Backends

**Source**: [`src/backend/http.rs`](../src/backend/http.rs)

HTTP backends connect to remote MCP servers via Streamable HTTP transport:

```
gatemini daemon ──HTTP──▸ remote MCP server
                          (e.g., z.ai, custom HTTP servers)
```

### LenientClient

**Source**: [`src/backend/lenient_client.rs`](../src/backend/lenient_client.rs)

Some MCP servers (notably z.ai) omit the `Content-Type` header in responses. Gatemini wraps the reqwest client to tolerate this, treating missing Content-Type as `application/json`.

### Header Forwarding

HTTP backends support `Authorization` and custom headers:

```yaml
backends:
  my_api:
    transport: http
    url: "https://api.example.com/mcp"
    headers:
      Authorization: "Bearer secretref:bws:project/dotenv/key/MY_API_KEY"
      X-Custom: "value"
```

## Tool Call Forwarding

### CallGuard (RAII In-Flight Tracking)

Every tool call is wrapped in a `CallGuard` that increments `in_flight_calls` on creation and decrements on drop:

```rust
let _guard = CallGuard::new(&self.in_flight_calls);
// ... execute tool call ...
// Guard dropped here → counter decremented
```

This ensures accurate tracking even if the call panics or errors.

### Retry Logic for Starting Backends

If a backend is in `Starting` state (connecting but not yet ready), tool calls retry with backoff:

| Attempt | Delay | Total Wait |
|---------|-------|------------|
| 1 | 500ms | 500ms |
| 2 | 1s | 1.5s |
| 3 | 2s | 3.5s |

After 3 attempts, the call fails with an error. Backends in `Unhealthy` or `Stopped` states fail immediately (no retry).

### Graceful Shutdown

`stop_all()` waits for all in-flight calls to complete before terminating backends:

```
1. Stop accepting new calls (mark all backends as Stopping)
2. Wait for in_flight_calls.load() == 0
3. Stop each backend (SIGTERM → force kill)
4. Clean up prerequisite processes
```

## Health Checker

**Source**: [`src/backend/health.rs`](../src/backend/health.rs)

The health checker runs on a configurable interval (default 30s) and manages backend health through three phases:

### Phase 1: Ping Healthy Backends

- Concurrent MCP ping requests to all `Healthy` backends
- Staggered across 80% of the interval to avoid thundering herd
- Configurable timeout per ping (default 5s)

### Phase 2: Handle Failed Backends

For `Stopped` or `Unhealthy` backends:

```
Circuit open?
  ├─ Yes, within recovery window → Skip (circuit stays open)
  ├─ Yes, recovery window expired → Half-open probe
  │   ├─ Probe succeeds → Circuit closed, mark Healthy
  │   └─ Probe fails → Reset circuit timer, stay open
  └─ No → Auto-restart with exponential backoff
```

### Phase 3: Retry Pending Backends

Backends that failed initial handshake (config exists but never entered DashMap) are retried with the same backoff logic.

### Circuit Breaker

| Parameter | Default | Purpose |
|-----------|---------|---------|
| `failure_threshold` | 3 | Consecutive failures before circuit opens |
| `max_restarts` | 5 | Maximum restarts per window |
| `restart_window` | 1 min | Window for counting restarts |

States:

```
Closed (Healthy)
  ── N consecutive failures ──▸ Open (Unhealthy)
                                  ── 3x check interval ──▸ Half-Open
                                                            ├─ Probe OK ──▸ Closed
                                                            └─ Probe fail ──▸ Open
```

### Exponential Backoff

Restart delay doubles with each attempt:

| Attempt | Delay |
|---------|-------|
| 0 | 1s |
| 1 | 2s |
| 2 | 4s |
| 3 | 8s |
| 4 | 16s |
| 5+ | 30s (capped) |

The restart window resets after `restart_window` (default 1 min) expires, allowing fresh restart attempts for transient failures.

## Prerequisite Processes

**Source**: [`src/backend/prerequisite.rs`](../src/backend/prerequisite.rs)

Some backends require a prerequisite process (e.g., a local API server) before they can start:

```yaml
backends:
  vibe_kanban:
    command: npx
    args: ["-y", "vibe-kanban-mcp"]
    prerequisite:
      command: "python3"
      args: ["-m", "http.server", "8080"]
      process_match: "http.server 8080"  # pgrep pattern for dedup
      managed: true                       # Stop on daemon shutdown
      startup_delay: 2s                   # Wait before starting backend
```

### Deduplication

Before spawning, Gatemini checks if the prerequisite is already running:

```bash
pgrep -f "http.server 8080"
```

If found, the spawn is skipped (idempotent). This prevents duplicate processes when the daemon restarts.

### Lifecycle

- `managed: true` -- Gatemini sends SIGTERM to the process group on daemon shutdown
- `managed: false` -- Gatemini leaves the process running (external management)

## Runtime Registration

**Source**: [`src/tools/register.rs`](../src/tools/register.rs)

Backends can be added and removed at runtime via meta-tools:

### register_manual

```json
{
  "manual_call_template": {
    "name": "my-backend",
    "command": "npx",
    "args": ["-y", "my-mcp-server"],
    "env": {"API_KEY": "..."}
  }
}
```

**Validation**:
- Name must match `[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}`
- Dynamic backend limit enforced (default 10, prevents DoS)
- Transport auto-detected: stdio if `command` present, HTTP if `url` present
- All registrations logged for audit

### deregister_manual

- Only dynamic (runtime-registered) backends can be removed
- Static config backends are protected from deregistration
- Distinguishes "not found" from "static/protected" in error messages

## Sources

- [`src/backend/mod.rs`](../src/backend/mod.rs) -- BackendManager and Backend trait
- [`src/backend/stdio.rs`](../src/backend/stdio.rs) -- Stdio backend lifecycle
- [`src/backend/http.rs`](../src/backend/http.rs) -- HTTP backend transport
- [`src/backend/health.rs`](../src/backend/health.rs) -- Health checker and circuit breaker
- [`src/backend/prerequisite.rs`](../src/backend/prerequisite.rs) -- Prerequisite processes
- [`src/tools/register.rs`](../src/tools/register.rs) -- Runtime registration
- [DashMap](https://github.com/xacrimon/dashmap) -- Concurrent HashMap
- [rmcp](https://github.com/4t145/rmcp) -- Rust MCP SDK
- [AWS Circuit Breaker](https://docs.aws.amazon.com/prescriptive-guidance/latest/cloud-design-patterns/circuit-breaker.html) -- Pattern reference
- [Azure Circuit Breaker](https://learn.microsoft.com/en-us/azure/architecture/patterns/circuit-breaker) -- Health check probing
- [AWS Exponential Backoff](https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/) -- Backoff with jitter
- [Process group termination](https://www.baeldung.com/linux/kill-members-process-group) -- kill(-pgid)
