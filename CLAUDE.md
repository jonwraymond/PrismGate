# Gatemini

Rust MCP gateway that connects to 30+ backend MCP servers and exposes 7 meta-tools to Claude Code via a shared daemon architecture.

## Architecture

### IPC & Process Model

```
Claude Code ──stdio──▸ gatemini (proxy) ──┐
Claude Code ──stdio──▸ gatemini (proxy) ──┤ Unix socket
Claude Code ──stdio──▸ gatemini (proxy) ──┘ /tmp/gatemini-{UID}.sock
                                           │
                                    gatemini daemon (1 process)
                                      ├── backend MCP server #1
                                      ├── backend MCP server #2
                                      └── ... (20-30+ backends, shared)
```

- **Proxy mode** (default): thin byte pipe bridging stdio ↔ Unix socket, auto-spawns daemon on first use
- **Daemon mode** (`serve`): binds Unix socket, manages backends, serves multiple clients concurrently
- **Idle shutdown**: daemon exits after configurable timeout (default 5m) with no active clients; proxy auto-restarts on next use

### Modules

| Module | Purpose |
|--------|---------|
| `src/main.rs` | Entry point, `InitializedGateway` setup, backend startup orchestration |
| `src/cli.rs` | clap CLI parser — commands: (default proxy), `serve`, `status`, `stop` |
| **IPC** | |
| `src/ipc/proxy.rs` | Proxy mode — stdio↔socket bridge, auto-start daemon with flock coordination |
| `src/ipc/daemon.rs` | Daemon mode — Unix socket listener, per-client `GateminiServer`, idle shutdown |
| `src/ipc/socket.rs` | Socket path resolution, PID file management, `is_daemon_alive`, `try_acquire_lock` |
| `src/ipc/status.rs` | `status` command — show daemon PID and alive/dead state |
| `src/ipc/stop.rs` | `stop` command — send SIGTERM to daemon, poll for exit |
| **Backend** | |
| `src/backend/mod.rs` | BackendManager — DashMap of running backends, start/stop/add/remove lifecycle, CallGuard drain |
| `src/backend/stdio.rs` | StdioBackend — spawns child in process group, MCP handshake via rmcp, reaper task |
| `src/backend/http.rs` | HttpBackend — streamable-HTTP transport via rmcp |
| `src/backend/lenient_client.rs` | HTTP client wrapper tolerating missing Content-Type headers (z.ai compat) |
| `src/backend/health.rs` | HealthChecker — periodic ping, circuit breaker, auto-restart with exponential backoff |
| **Core** | |
| `src/config.rs` | Config parsing, validation, hot-reload file watcher, `DaemonConfig` with idle_timeout |
| `src/registry.rs` | ToolRegistry — BM25 + optional semantic search index |
| `src/cache.rs` | Tool cache persistence — instant tool availability on daemon restart |
| `src/embeddings.rs` | Semantic embedding search via model2vec, L2-normalized cosine similarity |
| `src/server.rs` | MCP server — builds tool router, handles per-client sessions |
| `src/resources.rs` | MCP resources for @-mention discovery (`gatemini://overview`, tools, backends) |
| `src/prompts.rs` | MCP prompts for guided discovery (`discover`, `find_tool`, `backend_status`) |
| **Tools** | |
| `src/tools/discovery.rs` | search_tools, list_tools_meta, tool_info, get_required_keys — with brief/full modes |
| `src/tools/register.rs` | register_manual, deregister_manual — runtime backend management |
| `src/tools/sandbox.rs` | call_tool_chain — fast-path JSON/direct call detection, V8 sandbox fallback |
| **Other** | |
| `src/sandbox/` | rustyscript V8 sandbox for call_tool_chain TypeScript execution |
| `src/secrets/` | BWS integration, SecretProvider trait, regex-based secretref resolution |
| `src/admin.rs` | Optional axum admin API (feature-gated: `admin`) |

## Key Patterns

- Backends stored in `Arc<DashMap<String, RunningBackend>>` — concurrent access without mutex
- rmcp crate for MCP protocol: `ServiceExt`, `RunningService<RoleClient>`, `ClientHandler`
- Config pipeline: shellexpand → YAML parse → resolve_secrets_async → validate
- Health checker runs on tokio interval, respects `max_restarts` and `restart_window`
- Tool cache enables instant availability on daemon restart (loaded before backends connect)
- Proxy auto-start uses flock + double-check pattern to prevent duplicate daemon spawning
- Process group isolation (`process_group(0)`) for clean backend termination
- Brief discovery modes minimize token usage (~60 vs ~500 per search result)

## Building & Testing

```bash
cargo build                    # debug build
cargo build --release          # release build
cargo test                     # 62 unit tests
```
