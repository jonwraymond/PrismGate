# Backend Management

The backend subsystem turns a config file into running MCP transports, restart behavior, and a unified registry.

![Backend lifecycle](diagrams/backend-lifecycle.svg){ .diagram-wide }

## Public backend states

The public enum in `src/backend/mod.rs` is intentionally small:

- `Starting`
- `Healthy`
- `Unhealthy`
- `Stopped`

There are no public states such as "Degraded", "Restarting", or "Circuit Open". Circuit-breaker timing exists internally in the health checker and is reflected through `Unhealthy` and `Stopped`.

## BackendManager

`BackendManager` owns:

- the live backend map
- the backend config map
- per-backend semaphores
- per-backend retry configs
- rate limiters
- dynamic backend tracking
- managed prerequisite PIDs
- in-flight call draining
- optional call tracking hooks

The manager is transport-agnostic; it delegates concrete behavior to `Backend` implementations.

## Supported transports

### `stdio`

Child process backends communicate over stdin/stdout using rmcp.

Key details from `src/backend/stdio.rs`:

- stdin and stdout are piped
- stderr is set to `Stdio::null()`
- Unix builds place the child in a new process group
- a reaper task watches for unexpected exit and marks the backend stopped

That process-group isolation is what lets shutdown send `SIGTERM` to the whole backend tree instead of only the parent process.

### `streamable-http`

Remote HTTP backends are implemented in `src/backend/http.rs`.

Use this transport in config:

```yaml
backends:
  github:
    transport: streamable-http
    url: "https://api.githubcopilot.com/mcp/"
    headers:
      Authorization: "Bearer ${GITHUB_PAT_TOKEN}"
```

The lenient client wrapper exists to tolerate some imperfect servers that omit expected response headers.

### `cli-adapter`

CLI adapter backends let you publish tools without writing a separate MCP server.

You can either define tools inline:

```yaml
backends:
  jq-tools:
    transport: cli-adapter
    timeout: 30s
    tools:
      filter:
        description: "Apply a jq filter to JSON input"
        input_schema:
          type: object
          properties:
            filter: { type: string }
            input: { type: string }
          required: [filter, input]
        command: "jq '{{filter}}'"
        stdin: "{{input}}"
        output: json
```

Or point to an external adapter file:

```yaml
backends:
  ffmpeg-tools:
    transport: cli-adapter
    adapter_file: ~/.config/gatemini/adapters/ffmpeg.yaml
```

The adapter file path supports `~` expansion in the CLI adapter loader.

## Concurrency, retries, and fallback

Per-backend limits come from config:

- `max_concurrent_calls`
- `semaphore_timeout`
- `retry`
- `rate_limit`
- `fallback_chain`

Retry behavior only applies to the `Starting` state, where the manager waits briefly for a backend that is still connecting. Calls to `Unhealthy` or `Stopped` backends fail immediately unless the manager routes into a fallback backend for a transient error.

## Health checker

The health loop in `src/backend/health.rs` runs in three phases:

1. ping healthy backends
2. handle unhealthy and stopped backends
3. retry pending configured backends that never became live

![Health checker](diagrams/health-checker.svg){ .diagram-wide }

Current defaults from `src/config.rs`:

| Setting | Default |
|---------|---------|
| `health.interval` | `30s` |
| `health.timeout` | `5s` |
| `health.failure_threshold` | `3` |
| `health.max_restarts` | `5` |
| `health.restart_window` | `60s` |
| `health.restart_initial_backoff` | `1s` |
| `health.restart_max_backoff` | `30s` |
| `health.restart_timeout` | `30s` |
| `health.recovery_multiplier` | `3` |
| `health.drain_timeout` | `10s` |

Internal circuit-breaker behavior:

- healthy backends are pinged
- failures increment `consecutive_failures`
- once the threshold is reached, the backend is marked `Unhealthy`
- the health checker records `circuit_open_since`
- after `interval * recovery_multiplier`, a half-open probe is attempted
- if the probe fails, restart logic or another recovery window applies

## Prerequisites

Some backends depend on another process already running. That is handled by `src/backend/prerequisite.rs`.

Features:

- optional `pgrep -f` dedup via `process_match`
- optional managed lifecycle on shutdown
- startup delay before backend connect

If `managed: true`, Gatemini records the spawned prerequisite PID and terminates the process group during shutdown.

## Composite tools

Composite tools are not a separate transport. They are registered under the virtual `__composite` backend and executed through the sandbox layer.

Important limitation:

- config watcher notices composite tool changes
- those changes are logged
- they are not hot-reloaded; daemon restart is required
