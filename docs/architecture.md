# Architecture

Gatemini uses a shared daemon model to multiplex backend MCP servers across multiple AI agent sessions. A single daemon process manages all backends, while lightweight proxy processes bridge each Claude Code session's stdio to the daemon via Unix domain sockets.

## Process Model

![IPC & Process Model](diagrams/architecture.svg)

```
Claude Code ──stdio──▸ gatemini (proxy) ──┐
Claude Code ──stdio──▸ gatemini (proxy) ──┤ Unix socket
Claude Code ──stdio──▸ gatemini (proxy) ──┘ /tmp/gatemini-{UID}.sock
                                           │
                                    gatemini daemon (1 process)
                                      ├── backend MCP server #1 (stdio child)
                                      ├── backend MCP server #2 (stdio child)
                                      ├── backend MCP server #3 (HTTP)
                                      └── ... (for example, 30+ backends, shared)
```

This architecture delivers three key benefits:

1. **Resource sharing** -- backend processes run once, shared across all Claude Code sessions
2. **Instant startup** -- proxy connects to existing daemon in ~2s; no backend initialization per session
3. **Independent lifecycle** -- daemon survives client disconnects; auto-shuts down after 5 minutes idle

## Proxy Mode

**Source**: [`src/ipc/proxy.rs`](../src/ipc/proxy.rs)

The proxy is a zero-initialization byte pipe. It performs no config loading, no tracing setup, and no backend management. Its sole job is bridging Claude Code's stdio to the daemon's Unix socket.

### Startup Sequence

```
1. cleanup_stale_socket()
   └─ If socket exists but daemon is dead → remove stale socket + PID file

2. try_connect() [2s timeout]
   └─ Success → bridge_stdio() → done (fast path)

3. try_acquire_lock() [exclusive flock on .lock file]
   ├─ Won lock:
   │   ├─ Double-check: try_connect() again (race protection)
   │   ├─ spawn_daemon() as detached child (stdin/stdout null, stderr inherit)
   │   └─ wait_for_socket() [exponential backoff: 50ms→1s, 30s timeout]
   └─ Lock held by another proxy:
       └─ wait_for_socket() [another proxy is already spawning]

4. bridge_stdio()
   └─ Bidirectional: stdin→socket, socket→stdout
   └─ Exits on either EOF or BrokenPipe
```

### Flock + Double-Check Pattern

When multiple Claude Code sessions start simultaneously, only one proxy should spawn the daemon. Gatemini uses a file lock with double-checking:

1. **First check**: `try_connect()` -- most common path, daemon already running
2. **Acquire exclusive flock** on `{socket_path}.lock` -- non-blocking, fails if held
3. **Second check**: `try_connect()` again -- another proxy may have just finished spawning
4. **Spawn if needed**: only the lock holder spawns; others wait for the socket

The lock is held throughout daemon spawning and released after the socket becomes connectable. This prevents duplicate daemon instances without polling or retry loops.

### Daemon Spawning

The proxy spawns itself with the `serve` subcommand:

```
gatemini -c /path/to/config.yaml serve
```

- `stdin`/`stdout` set to `Stdio::null()` -- daemon doesn't hold proxy's stdio
- `stderr` inherited -- daemon logs via tracing appear in the terminal
- Process is detached -- proxy can exit without killing the daemon

## Daemon Mode

**Source**: [`src/ipc/daemon.rs`](../src/ipc/daemon.rs)

The daemon is the heavyweight process that manages all backend MCP servers. It initializes once and serves multiple clients concurrently.

### Initialization (shared with all modes)

**Source**: [`src/main.rs`](../src/main.rs) -- `initialize()`

```
1. load_dotenv()           -- Load ~/.env (Once pattern, thread-safe)
2. Load config             -- shellexpand → YAML parse
3. Initialize tracing      -- Structured logging to stderr
4. resolve_secrets_async   -- Resolve secretref: patterns
5. Create ToolRegistry     -- With or without EmbeddingIndex
6. Create BackendManager   -- DashMap-backed concurrent store
7. Load tool cache         -- Instant tool availability from previous run
8. Spawn background tasks:
   ├── start_all()         -- Connect all backends, discover tools
   ├── health_checker()    -- Periodic pings, circuit breaker
   ├── watch_config()      -- File watcher for hot-reload
   └── admin_api()         -- Optional HTTP admin (feature-gated)
```

### Accept Loop

After initialization, the daemon binds the Unix socket and enters the accept loop:

```rust
loop {
    select! {
        accept = listener.accept() => { /* spawn client task */ }
        () = idle_sleep, if idle_enabled && active_sessions == 0 => { break; }
        _ = sigterm.recv() => { break; }
        _ = sigint.recv() => { break; }
    }
}
```

Each connected client gets a new `GateminiServer` instance (cheap: clones `Arc` references to the shared registry and backend manager). The rmcp crate handles the full MCP protocol per session.

**Key concurrency primitives**:

| Primitive | Purpose |
|-----------|---------|
| `TaskTracker` | Tracks active client tasks for graceful shutdown |
| `AtomicUsize` | Counts active sessions for idle timeout |
| `Arc<Notify>` | Broadcasts shutdown signal to background tasks |

### Idle Shutdown

The daemon exits after `idle_timeout` (default 5 minutes) with zero active clients:

- Timer resets on every new client connection
- Timer is pushed forward while any client is connected
- The proxy auto-restarts the daemon on next use

### Graceful Shutdown Sequence

```
1. Stop accepting new connections (break accept loop)
2. client_tracker.close() + wait() -- drain active client sessions
3. shutdown_notify.notify_waiters() -- signal background tasks
4. backend_manager.stop_all() -- stop all backends, wait for in-flight calls
5. socket::cleanup_files() -- remove socket + PID file
```

## Socket Coordination

**Source**: [`src/ipc/socket.rs`](../src/ipc/socket.rs)

### Path Determination

```
/tmp/gatemini-{UID}.sock          -- socket
/tmp/gatemini-{UID}.sock.pid      -- PID file
/tmp/gatemini-{UID}.sock.lock     -- flock coordination
```

On Linux with `$XDG_RUNTIME_DIR`, the base path uses that instead of `/tmp`. The UID suffix ensures multi-user isolation on shared machines.

### Liveness Check

`is_daemon_alive()` reads the PID file and sends signal 0:

```rust
libc::kill(pid as libc::pid_t, 0)  // 0 = no signal, just check existence
```

Returns `true` if the process exists and belongs to the current user.

### Lock Acquisition

`try_acquire_lock()` uses `flock(LOCK_EX | LOCK_NB)`:

- **Non-blocking**: returns immediately if another process holds the lock
- **File descriptor ownership**: lock auto-releases if the process crashes
- **Deliberate retention**: lock file is never deleted (it's the coordination mechanism)

## Status and Stop Commands

**Sources**: [`src/ipc/status.rs`](../src/ipc/status.rs), [`src/ipc/stop.rs`](../src/ipc/stop.rs)

| Command | Action |
|---------|--------|
| `gatemini status` | Read PID file, check if alive via signal 0, print status |
| `gatemini stop` | Send SIGTERM to daemon PID, poll for exit (100ms intervals, 5s timeout) |

## Process Group Isolation

**Source**: [`src/backend/stdio.rs`](../src/backend/stdio.rs)

Each stdio backend spawns with `process_group(0)`, creating a new process group:

```rust
cmd.process_group(0)  // setsid-like: child gets its own PGID = PID
```

On termination, Gatemini sends SIGTERM to the entire process group:

```rust
libc::kill(-(pid as i32), libc::SIGTERM)  // negative PID = process group
```

This ensures the backend and all its children (subprocesses, scripts) are terminated together. Without process groups, killing the parent would orphan children to init.

### Termination Sequence

```
1. SIGTERM to process group: kill(-(pid), SIGTERM)
2. Wait 200ms for graceful exit
3. Force kill child: child.kill() as fallback
```

## Transport Performance

Gatemini chose Unix domain sockets over TCP for proxy-daemon IPC:

| Metric | Unix Socket | TCP Localhost |
|--------|-------------|---------------|
| Latency | ~2-3 us | ~3.6 us |
| Throughput (100B msg) | 130k msg/s | 70k msg/s |
| Overhead | Kernel bypass | Full TCP/IP stack |

Benchmarks from [Baeldung IPC comparison](https://www.baeldung.com/linux/ipc-performance-comparison). UDS avoids TCP checksum, congestion control, and routing overhead. The tradeoff is local-machine only -- which is exactly Gatemini's deployment model.

## Three Operating Modes

| Mode | Command | Use Case |
|------|---------|----------|
| **Proxy** (default) | `gatemini` | Claude Code integration via stdio |
| **Daemon** | `gatemini serve` | Started automatically by proxy; can also run standalone |
| **Direct** | `gatemini --direct` | Single-session mode, no daemon/socket (debugging) |

## Sources

- [`src/ipc/proxy.rs`](../src/ipc/proxy.rs) -- Proxy mode implementation
- [`src/ipc/daemon.rs`](../src/ipc/daemon.rs) -- Daemon accept loop and shutdown
- [`src/ipc/socket.rs`](../src/ipc/socket.rs) -- Socket path, PID, flock coordination
- [`src/main.rs`](../src/main.rs) -- Initialization and mode dispatch
- [rmcp](https://github.com/4t145/rmcp) -- Rust MCP SDK used for protocol handling
- [Baeldung IPC Performance](https://www.baeldung.com/linux/ipc-performance-comparison) -- UDS vs TCP benchmarks
- [flock(2) man page](https://man7.org/linux/man-pages/man2/flock.2.html) -- File locking semantics
- [Process group kill semantics](https://www.baeldung.com/linux/kill-members-process-group) -- kill(-pgid)
