# Codebase Map

This page is the high-level source map for Gatemini as it exists today.

## Startup path

The shared initialization pipeline lives in `src/main.rs` and is reused by direct mode and daemon mode.

Initialization order:

1. load `.env` files once
2. load and validate config
3. initialize tracing
4. resolve secrets
5. create the registry, tracker, and backend manager
6. restore cached tools and usage data
7. register aliases and composite tools
8. start background tasks for backends, health checks, config watching, and optional admin API

Key file:

- `src/main.rs`

## CLI and platform paths

`src/cli.rs` defines the public command surface and the standard platform paths:

- config home: platform config directory plus `gatemini/`
- cache home: platform cache directory plus `gatemini/`
- commands: `serve`, `status`, `stop`, `restart`
- direct mode: `--direct`

## IPC layer

The IPC layer is the difference between Gatemini and a one-process-per-session MCP setup.

Files:

- `src/ipc/proxy.rs`: proxy mode, handshake caching, reconnect logic, daemon auto-start
- `src/ipc/daemon.rs`: early socket bind, accept loop, idle shutdown, client drain timeout
- `src/ipc/socket.rs`: socket path resolution, PID files, flock lock path, cleanup helpers
- `src/ipc/status.rs`: daemon status command
- `src/ipc/stop.rs`: graceful stop command
- `src/ipc/restart.rs`: restart command, wait budget, reconnect expectation
- `src/ipc/mcp_framing.rs`: small JSON-RPC framer used for handshake interception

Important runtime facts:

- Linux prefers `$XDG_RUNTIME_DIR/gatemini.sock` when available.
- macOS and the fallback path use `/tmp/gatemini-$UID.sock`.
- the daemon binds the socket before the heavy initialization path finishes
- proxy startup uses flock plus a second connect check to avoid duplicate daemons
- reconnect replays the cached MCP initialize handshake

## Public MCP surface

The gateway server is implemented in `src/server.rs`.

Public tools:

- `search_tools`
- `list_tools_meta`
- `tool_info`
- `get_required_keys_for_tool`
- `call_tool_chain`
- `register_manual`
- `deregister_manual`

Public resources and prompts are implemented separately:

- `src/resources.rs`
- `src/prompts.rs`

The advertised protocol version is `2025-06-18`.

## Backend system

The backend subsystem owns transport differences, lifecycle, concurrency, and restart behavior.

Files:

- `src/backend/mod.rs`: `Backend` trait, `BackendManager`, public backend state, retry and drain behavior
- `src/backend/stdio.rs`: child-process MCP backends over stdin/stdout
- `src/backend/http.rs`: streamable HTTP backends
- `src/backend/cli_adapter.rs`: CLI templates exposed as tools without a dedicated MCP server
- `src/backend/prerequisite.rs`: prerequisite process dedup and lifecycle
- `src/backend/health.rs`: health checker, restart windows, internal circuit-breaker timing
- `src/backend/pool.rs`: per-session dedicated instance pool for stateful backends
- `src/backend/composite.rs`: virtual backend for composite tools
- `src/backend/lenient_client.rs`: HTTP client wrapper for servers with imperfect content-type behavior

Public backend states are only:

- `Starting`
- `Healthy`
- `Unhealthy`
- `Stopped`

The health checker keeps extra circuit-breaker timing internally instead of exposing extra enum states.

## Discovery and search

The discovery system is spread across three files:

- `src/registry.rs`: registry storage, BM25 search, optional hybrid RRF search, alias rules
- `src/tools/discovery.rs`: tool handlers for search, paging, brief/full views, required keys
- `src/embeddings.rs`: optional model2vec-powered semantic search when the `semantic` feature is enabled

Design details worth knowing:

- the registry always stores namespaced tools
- bare aliases are added only when there is no collision
- cached tools restore namespaced entries before a backend is healthy
- `search_tools` defaults to `brief=true`
- `tool_info` defaults to `detail="brief"`

## Sandbox execution

`call_tool_chain` is split across:

- `src/tools/sandbox.rs`: routing and fast-path parsing
- `src/sandbox/mod.rs`: dedicated V8 thread and runtime bridge
- `src/sandbox/bridge.rs`: generated JS accessors and introspection helpers

Execution tiers:

1. direct JSON tool call
2. simple single-call TypeScript parse
3. full V8 sandbox

The sandbox feature is optional at compile time and enabled by default in this repo.

## Config, secrets, and reload behavior

The main source of truth is `src/config.rs`.

What it owns:

- config structs and defaults
- environment interpolation
- secret resolution
- validation
- config watching and hot-reload

Hot-reload behavior today:

- backend changes: applied
- alias changes: applied
- composite tool changes: detected but not hot-reloaded; restart required
- daemon-level settings: read on startup

## State persistence and telemetry

Two files own runtime snapshots:

- `src/cache.rs`: tool cache, embedding cache, usage stats cache
- `src/tracker.rs`: recent tool calls, per-tool usage counts, backend latency histograms

Current cache version: `4`

Current cache contents:

- backend tool snapshots
- optional embeddings
- per-tool usage stats

## Optional admin API

The feature-gated admin server lives in `src/admin.rs`.

Current routes:

- `/api/health`
- `/api/backends`
- `/api/discovery`

Current limitation:

- `admin.allowed_cidrs` exists in config, but the route layer does not currently enforce CIDR filtering

## Tests

Tests are embedded throughout the codebase plus a few focused integration modules:

- backend concurrency tests
- daemon and proxy tests
- MCP compliance tests
- registry, config, cache, tracker, sandbox, and secrets tests

Useful files:

- `src/backend/concurrency_tests.rs`
- `src/ipc/daemon_tests.rs`
- `src/ipc/proxy_tests.rs`
- `src/mcp_compliance_tests.rs`
- `src/testutil.rs`
