<p align="center">
  <h1 align="center">Gatemini</h1>
  <p align="center">
    Shared-daemon MCP gateway that multiplexes many backend servers behind one stable MCP endpoint.
  </p>
</p>

<p align="center">
  <a href="https://github.com/jonwraymond/prismgate/actions/workflows/ci.yml"><img src="https://github.com/jonwraymond/prismgate/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
  <a href="https://github.com/jonwraymond/prismgate/releases"><img src="https://img.shields.io/github/v/release/jonwraymond/prismgate" alt="Release"></a>
</p>

Gatemini is the runtime and binary name. The source repository and release channel live under the `PrismGate` GitHub project.

## Why this exists

Most MCP clients launch one process tree per session. If you configure a few dozen backends, each new terminal or editor session pays the same startup and memory cost again.

Gatemini changes that model:

- One daemon owns the backend connections.
- Lightweight proxy processes bridge each client session over stdio.
- The daemon is reused until it has been idle for the configured timeout.
- Agents discover backend tools through 7 gateway meta-tools instead of receiving every schema up front.

<p align="center">
  <img src="docs/diagrams/ipc-architecture.svg" alt="Gatemini IPC architecture" width="860">
</p>

## What it provides

| Capability | What the code does today |
|-----------|---------------------------|
| Shared daemon | Proxy mode connects to a single Unix socket daemon instead of starting backends per session |
| Auto-start and restart | First proxy spawns `gatemini serve`; `gatemini restart` drains clients and lets proxies reconnect |
| Progressive discovery | `search_tools`, `list_tools_meta`, `tool_info`, `get_required_keys_for_tool`, `call_tool_chain`, `register_manual`, `deregister_manual` |
| Multiple backend transports | `stdio`, `streamable-http`, and `cli-adapter` backends in one config |
| Health management | Periodic pinging, failure thresholds, internal circuit-breaker tracking, restart backoff, pending-backend retry |
| Tool cache | Cached namespaced tools load before backends reconnect; cache version is currently `4` |
| TypeScript execution | `call_tool_chain` fast-paths JSON/simple calls and falls back to the V8 sandbox when needed |
| Secrets | Environment interpolation, `.env` loading, `secretref:` resolution, and Bitwarden Secrets Manager integration |

## Quick start

### Install

Download a release from [GitHub Releases](https://github.com/jonwraymond/prismgate/releases), or build from source:

```bash
git clone https://github.com/jonwraymond/prismgate.git
cd prismgate
cargo install --path .
```

### Configure

The default config path is the platform config directory plus `gatemini/config.yaml`:

- macOS/Linux: `~/.config/gatemini/config.yaml`
- Windows: `%APPDATA%\\gatemini\\config.yaml`

Minimal example:

```yaml
log_level: info

daemon:
  idle_timeout: 5m

backends:
  exa:
    command: npx
    args: ["-y", "exa-mcp-server"]
    env:
      EXA_API_KEY: "${EXA_API_KEY}"

  github:
    transport: streamable-http
    url: "https://api.githubcopilot.com/mcp/"
    headers:
      Authorization: "Bearer ${GITHUB_PAT_TOKEN}"
```

For a fuller example covering secrets, CLI adapters, admin settings, and health tuning, see [`config/example.yaml`](config/example.yaml).

### Register Gatemini as an MCP server

Example Claude Code configuration:

```json
{
  "mcpServers": {
    "gatemini": {
      "command": "/path/to/gatemini",
      "args": ["-c", "/path/to/config.yaml"]
    }
  }
}
```

### CLI modes

```bash
gatemini            # Proxy mode (default)
gatemini --direct   # Single-process direct mode, no daemon or socket
gatemini serve      # Run the daemon in the foreground
gatemini status     # Read PID/socket state
gatemini stop       # Gracefully stop the daemon
gatemini restart    # Stop, drain clients, let proxies reconnect
```

## Runtime model

The daemon binds its socket early, before the heavier initialization path completes. That means proxies can connect while the daemon is still loading config, resolving secrets, restoring cache, and starting backends.

<p align="center">
  <img src="docs/diagrams/daemon-lifecycle.svg" alt="Gatemini daemon lifecycle" width="860">
</p>

Proxy mode is not just a raw byte pipe. It also:

- acquires a flock lock to avoid duplicate daemon startup
- reconnects with backoff if the daemon disappears
- caches the MCP initialize request and initialized notification
- replays that cached handshake when reconnecting

<p align="center">
  <img src="docs/diagrams/proxy-startup.svg" alt="Gatemini proxy startup" width="760">
</p>

## Discovery model

Backend tools are not exposed as first-class MCP tools. Instead, Gatemini exposes a small discovery and execution surface:

| Meta-tool | Purpose |
|-----------|---------|
| `search_tools` | Search live registry entries by task description |
| `list_tools_meta` | Page through registered tool names |
| `tool_info` | Inspect one tool in `brief` or `full` detail |
| `get_required_keys_for_tool` | Show required env keys for the owning backend |
| `call_tool_chain` | Execute JSON, simple TS, or full sandboxed TS |
| `register_manual` | Register a dynamic backend at runtime |
| `deregister_manual` | Remove a dynamic backend |

Resources and prompts round out the MCP surface:

- Static resources: `gatemini://overview`, `gatemini://backends`, `gatemini://tools`, `gatemini://recent`
- Resource templates: `gatemini://tool/{tool_name}`, `gatemini://backend/{backend_name}`, `gatemini://backend/{backend_name}/tools`, `gatemini://recent/{limit}`
- Prompts: `discover`, `find_tool`, `backend_status`

<p align="center">
  <img src="docs/diagrams/tool-discovery.svg" alt="Gatemini progressive discovery" width="860">
</p>

## Health and lifecycle behavior

Backend state exposed publicly is limited to:

- `Starting`
- `Healthy`
- `Unhealthy`
- `Stopped`

Circuit-breaker timing is tracked internally by the health checker and surfaced through those states rather than a separate public enum.

Current default health settings come from `src/config.rs`:

- interval: `30s`
- timeout: `5s`
- failure threshold: `3`
- max restarts per window: `5`
- restart window: `60s`
- restart backoff: `1s` initial, `30s` max
- restart timeout: `30s`
- recovery multiplier: `3`
- drain timeout: `10s`

<p align="center">
  <img src="docs/diagrams/health-checker.svg" alt="Gatemini health checker" width="860">
</p>

## Configuration and secrets

Config loading is intentionally simple and code-backed:

1. Load `.env` files once.
2. Read YAML.
3. Expand environment variables with `shellexpand::env`.
4. Deserialize config.
5. Resolve `secretref:` values.
6. Validate required fields and supported transport combinations.

`.env` files are loaded from:

1. `~/.env`
2. the standard Gatemini config directory, for example `~/.config/gatemini/.env`
3. the config file's sibling directory

Supported secret modes:

- direct environment references such as `${EXA_API_KEY}`
- `secretref:bws:...` with environment fallback when BWS is disabled
- Bitwarden Secrets Manager when `secrets.providers.bws.enabled: true`

## Documentation

The repo docs were rewritten against the current Rust implementation and diagrams:

- [Docs index](docs/README.md)
- [Architecture](docs/architecture.md)
- [Codebase map](docs/codebase-map.md)
- [Tool discovery](docs/tool-discovery.md)
- [Backend management](docs/backend-management.md)
- [Secrets and config](docs/secrets-and-config.md)
- [Resources and prompts](docs/resources-and-prompts.md)
- [Token efficiency](docs/token-efficiency.md)
- [Telemetry strategy](docs/telemetry-strategy.md)
- [Sandbox](docs/sandbox.md)

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for development setup, docs build commands, and review expectations.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
