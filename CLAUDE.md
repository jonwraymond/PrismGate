# Gatemini

Rust MCP gateway that exposes a small discovery surface over many backend MCP servers.

## Naming

- Runtime name: `gatemini`
- Repository and releases: `PrismGate`

## Runtime summary

- default mode is a proxy that bridges stdio to a Unix socket daemon
- the daemon binds early, then completes shared initialization
- backend tools are discovered through 7 gateway meta-tools
- proxy reconnect can replay the cached MCP initialize handshake
- public backend states are `Starting`, `Healthy`, `Unhealthy`, and `Stopped`

## Command surface

- `gatemini`
- `gatemini --direct`
- `gatemini serve`
- `gatemini status`
- `gatemini stop`
- `gatemini restart`

## Public MCP surface

Tools:

- `search_tools`
- `list_tools_meta`
- `tool_info`
- `get_required_keys_for_tool`
- `call_tool_chain`
- `register_manual`
- `deregister_manual`

Resources:

- `gatemini://overview`
- `gatemini://backends`
- `gatemini://tools`
- `gatemini://recent`
- `gatemini://tool/{tool_name}`
- `gatemini://backend/{backend_name}`
- `gatemini://backend/{backend_name}/tools`
- `gatemini://recent/{limit}`

Prompts:

- `discover`
- `find_tool`
- `backend_status`

## Module map

| Module | Purpose |
|--------|---------|
| `src/main.rs` | shared initialization, mode dispatch |
| `src/cli.rs` | CLI and platform paths |
| `src/config.rs` | defaults, config loading, validation, hot-reload |
| `src/server.rs` | public MCP tool surface |
| `src/registry.rs` | tool registry, BM25, hybrid search |
| `src/cache.rs` | cache restore and persistence |
| `src/tracker.rs` | recent calls, usage counts, backend latency |
| `src/resources.rs` | resources and template completion |
| `src/prompts.rs` | live prompts driven by registry and tracker |
| `src/ipc/` | proxy/daemon/socket lifecycle |
| `src/backend/` | transport implementations and health management |
| `src/tools/` | meta-tool handlers |
| `src/sandbox/` | V8 execution bridge |
| `src/secrets/` | secret providers and resolver |

## Dedicated instance mode

- `instance_mode: dedicated` gives each proxy session its own backend instance from a pool
- only applies to `stdio` and `cli-adapter` transports; HTTP backends ignore it
- pool pre-warms `min_idle` instances (default 1), lazy-spawns on demand up to `max_instances` (default 20)
- instances are recycled (stop + respawn) on session disconnect for clean state
- session_id is threaded from daemon accept loop through GateminiServer → sandbox → BackendManager
- direct mode uses session_id 0
- pool implementation lives in `src/backend/pool.rs`
- health checker calls `restart_pool_primary()` instead of `restart_backend()` for dedicated backends

## Important implementation notes

- transport names are `stdio`, `streamable-http`, and `cli-adapter`
- backend stderr is currently discarded with `Stdio::null()` for stdio backends
- cache version is `4` and defaults to the platform cache directory
- composite tool changes are detected by the watcher but require restart
- `admin.allowed_cidrs` exists in config but is not enforced in the current admin routes
