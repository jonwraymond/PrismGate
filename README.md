<p align="center">
  <h1 align="center">Gatemini</h1>
  <p align="center">
    A high-performance MCP gateway that multiplexes dozens of backend servers through a single shared daemon.
  </p>
</p>

<p align="center">
  <a href="https://github.com/jonwraymond/gatemini/actions/workflows/ci.yml"><img src="https://github.com/jonwraymond/gatemini/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
  <a href="https://github.com/jonwraymond/gatemini/releases"><img src="https://img.shields.io/github/v/release/jonwraymond/gatemini" alt="Release"></a>
</p>

---

## Why Gatemini?

Note: backend counts and catalog size in this documentation are examples; the actual toolset and backend mix can vary by deployment and configuration.

AI coding agents connect to [MCP](https://modelcontextprotocol.io/) (Model Context Protocol) servers for capabilities like web search, file analysis, and code generation. Each server runs as a separate process. In deployments with larger backend sets, every session can quickly multiply process count:

```
Session 1 -> 30 backend processes
Session 2 -> 30 backend processes
Session 3 -> 30 backend processes
─────────────────────────────────
Total: 90 processes (and growing)
```

**This doesn't scale.** Memory explodes, startup is slow, and stateful backends (databases, browser sessions) can't be shared across sessions.

## How It Works

Gatemini runs a single daemon that manages all backend connections. Multiple sessions connect through lightweight proxy processes via Unix socket:

<p align="center">
  <img src="docs/diagrams/ipc-architecture.svg" alt="IPC Architecture" width="700">
</p>

**Result:** 30 backend processes total, regardless of how many sessions are active. The daemon auto-starts on first connection and shuts down after 5 minutes of inactivity.

## Features

| Feature | Description |
|---------|-------------|
| **Shared daemon** | One set of backends serves all clients via Unix socket IPC |
| **Auto-start** | First proxy connection spawns the daemon; flock coordination prevents races |
| **Idle shutdown** | Daemon exits after configurable timeout (default 5 min) with no active clients |
| **Progressive discovery** | 7 meta-tools with brief/full modes — ~60 tokens per search result vs ~500 |
| **V8 sandbox** | `call_tool_chain` executes TypeScript for multi-tool orchestration |
| **Health monitoring** | Periodic pings, circuit breaker, auto-restart with exponential backoff |
| **Tool cache** | Instant tool availability on daemon restart before backends reconnect |
| **BM25 + semantic search** | Hybrid keyword and embedding-based tool discovery |
| **Flexible secrets** | Env vars, `.env` files, hardcoded YAML, or BWS — with automatic fallback |
| **Dual transport** | Stdio child processes and streamable-HTTP backends in one config |
| **Hot-reload** | Config changes apply without daemon restart (backends, aliases, tags) |
| **Fallback chains** | Automatic failover to alternative backends on transient errors |

## Quick Start

### Install from Release

Download the latest binary from [GitHub Releases](https://github.com/jonwraymond/gatemini/releases):

```bash
# Replace with tag you want:
#   - release tag: main
#   - versioned release: v0.3.0, v1.0.0, etc.
export RELEASE_TAG=main

# macOS (Apple Silicon)
curl -L "https://github.com/jonwraymond/gatemini/releases/download/${RELEASE_TAG}/gatemini-${RELEASE_TAG}-darwin-arm64.tar.gz" \
  | tar -xz -C /tmp
install -m 755 /tmp/gatemini ~/.local/bin/

# macOS (Intel)
curl -L "https://github.com/jonwraymond/gatemini/releases/download/${RELEASE_TAG}/gatemini-${RELEASE_TAG}-darwin-x86_64.tar.gz" \
  | tar -xz -C /tmp
install -m 755 /tmp/gatemini ~/.local/bin/

# Linux (tar.gz)
curl -L "https://github.com/jonwraymond/gatemini/releases/download/${RELEASE_TAG}/gatemini-${RELEASE_TAG}-linux-x86_64.tar.gz" \
  | tar -xz -C /tmp
install -m 755 /tmp/gatemini ~/.local/bin/

# Linux package managers (tarball package versions include release tag directly)
#
# If RELEASE_TAG is a semver tag (for example, v0.3.0), package files are:
#   - Debian/Ubuntu: gatemini_${RELEASE_TAG#v}_amd64.deb
#   - RHEL/Fedora:  gatemini-${RELEASE_TAG#v}-1.x86_64.rpm
#
# If RELEASE_TAG is `main`, package versions are:
#   - Debian/Ubuntu: gatemini_0.0.0.<sha8>_amd64.deb
#   - RHEL/Fedora:  gatemini-0.0.0.<sha8>-1.x86_64.rpm
```

Download from GitHub Releases and pick the matching files from the release you selected above.

```powershell
# Windows (zip)
$RELEASE_TAG = "main"  # Replace with desired release tag (main, v0.3.0, etc.)
$out = "$env:TEMP\gatemini-${RELEASE_TAG}-windows-x86_64.zip"
$dest = "$env:USERPROFILE\bin"

Invoke-WebRequest `
  -Uri "https://github.com/jonwraymond/gatemini/releases/download/$RELEASE_TAG/gatemini-$RELEASE_TAG-windows-x86_64.zip" `
  -OutFile $out
Expand-Archive -Path $out -DestinationPath $dest -Force
```

### Install from Source

```bash
git clone https://github.com/jonwraymond/gatemini.git
cd gatemini
cargo install --path .
# or: make install  (installs to ~/.local/bin/)
```

### Configure

Create a config file at the platform config directory (`~/.config/gatemini/config.yaml` on macOS/Linux):

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

  firecrawl:
    command: npx
    args: ["-y", "firecrawl-mcp"]
    env:
      FIRECRAWL_API_KEY: "${FIRECRAWL_API_KEY}"

  custom-api:
    transport: http
    url: "https://api.example.com/mcp"
    headers:
      Authorization: "Bearer ${API_TOKEN}"
```

See [`config/example.yaml`](config/example.yaml) for a full annotated example covering secrets, prerequisites, health tuning, and sandbox settings.

### Connect to Claude Code

Add Gatemini as an MCP server in `~/.claude.json`:

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

The first connection auto-starts the daemon. Subsequent sessions share the same daemon transparently.

### CLI Commands

```bash
gatemini              # Proxy mode (default): stdio <-> Unix socket bridge
gatemini serve        # Run as daemon directly (foreground)
gatemini status       # Show daemon PID and alive/dead state
gatemini stop         # Gracefully stop the daemon
```

## Architecture

### Daemon Lifecycle

<p align="center">
  <img src="docs/diagrams/daemon-lifecycle.svg" alt="Daemon Lifecycle" width="700">
</p>

The daemon binds its Unix socket **before** initialization (secret resolution, model loading, backend startup). This means proxies can connect immediately — MCP bytes queue in the kernel socket buffer until the accept loop starts.

### Proxy Startup

<p align="center">
  <img src="docs/diagrams/proxy-startup.svg" alt="Proxy Startup Sequence" width="600">
</p>

A flock + double-check pattern prevents multiple proxies from racing to spawn duplicate daemons. The first proxy acquires an exclusive lock, spawns the daemon, and waits for the socket. All other proxies connect to the existing daemon.

### Health Monitoring

<p align="center">
  <img src="docs/diagrams/health-checker.svg" alt="Health Checker" width="650">
</p>

Backends are monitored with periodic MCP pings. Failed backends enter a circuit breaker: after 3 failures, the circuit opens and auto-restart begins with exponential backoff (1s, 2s, 4s... capped at 30s). A half-open probe after the recovery window tests if the backend is healthy again.

## Progressive Tool Discovery

With a representative catalog (example: 258+ tools across 30+ backends), sending all tool schemas would consume ~67,000 tokens (33% of a 200K context window). Gatemini solves this with **progressive disclosure**:

<p align="center">
  <img src="docs/diagrams/tool-discovery.svg" alt="Tool Discovery Flow" width="700">
</p>

### Meta-Tools

Instead of exposing hundreds of individual tools, Gatemini provides 7 meta-tools:

| Tool | Purpose | Token Cost |
|------|---------|------------|
| `search_tools` | BM25 + semantic search across all backends | ~60/result (brief) |
| `list_tools_meta` | Paginated index of all available tools | ~3/name |
| `tool_info` | Schema for a specific tool (brief or full) | ~200 (brief) |
| `get_required_keys_for_tool` | List env vars needed by a backend | Minimal |
| `call_tool_chain` | Execute tools (JSON, TypeScript, or direct) | N/A |
| `register_manual` | Add a backend at runtime | N/A |
| `deregister_manual` | Remove a dynamic backend | N/A |

**Typical discovery flow uses ~3,600 tokens vs ~20,000 tokens naive — an 82% reduction.**

### MCP Resources

Resources provide compact read-only views for `@`-mention context loading:

| URI | Content | Tokens |
|-----|---------|--------|
| `gatemini://overview` | Gateway guide and discovery workflow | ~500 |
| `gatemini://tools` | Compact index of ALL tools | ~3,000 |
| `gatemini://tool/{name}` | Full schema for one tool | 200-10,000 |
| `gatemini://backends` | Backend list with health status | Variable |
| `gatemini://backend/{name}` | Single backend details | ~200 |
| `gatemini://recent` | Recent tool call history | Variable |

### MCP Prompts

| Prompt | Purpose |
|--------|---------|
| `discover` | 4-step guided discovery walkthrough |
| `find_tool` | Search, display top 5, show full schema for #1 |
| `backend_status` | Health dashboard with latency stats |

## V8 Sandbox

`call_tool_chain` supports three execution tiers for multi-tool orchestration:

<p align="center">
  <img src="docs/diagrams/sandbox-execution.svg" alt="Sandbox Execution" width="650">
</p>

```typescript
// Tier 1: Direct JSON (no V8)
{"tool": "exa.web_search_exa", "arguments": {"query": "MCP gateway"}}

// Tier 2: Simple TypeScript (regex-parsed, no V8)
await exa.web_search_exa({query: "MCP gateway"})

// Tier 3: Full V8 sandbox (multi-step orchestration)
const results = await exa.web_search_exa({query: "Rust MCP"});
const url = results.results[0].url;
const page = await firecrawl.firecrawl_scrape({url});
return {search: results, page};
```

The sandbox auto-generates typed accessors for all backends and provides introspection via `__interfaces` and `__getToolInterface()`.

## Configuration Reference

### Backend Types

**Stdio** (child process):
```yaml
backends:
  my-backend:
    command: npx
    args: ["-y", "my-mcp-server"]
    env:
      API_KEY: "${MY_API_KEY}"
    timeout: 30
```

**HTTP** (remote server):
```yaml
backends:
  remote-api:
    transport: http
    url: "https://api.example.com/mcp"
    headers:
      Authorization: "Bearer ${TOKEN}"
```

### Secret Resolution

Gatemini supports three modes for providing secrets — no BWS required:

**Mode 1: Environment variables** (simplest — no config needed)
```yaml
backends:
  my-backend:
    env:
      API_KEY: "${MY_API_KEY}"   # Direct env var or .env file
```

**Mode 2: secretref with env fallback** (default when BWS is disabled)
```yaml
backends:
  my-backend:
    env:
      API_KEY: "secretref:bws:project/dotenv/key/MY_API_KEY"
      # When BWS disabled, extracts "MY_API_KEY" and resolves via env var
```

**Mode 3: Bitwarden Secrets Manager** (full secret management)
```yaml
secrets:
  strict: true
  providers:
    bws:
      enabled: true
      access_token: "${BWS_ACCESS_TOKEN}"
      organization_id: "${BWS_ORG_ID}"
```

`.env` files are loaded from three locations (later overrides earlier):
1. `~/.env`
2. `~/.config/gatemini/.env` (platform config dir)
3. Sibling of the config file

### Hot-Reloadable Settings

| Setting | Hot-Reload | Notes |
|---------|-----------|-------|
| Backends (add/remove/change) | Yes | Removes old, starts new |
| Aliases | Yes | Updates alias map |
| Tags | Yes | Via backend restart |
| Fallback chains | Yes | Part of backend config |
| Composite tools | No | Requires daemon restart |
| Daemon settings | No | Read once at startup |

### Health Tuning

```yaml
# Values below match current defaults; tune for your deployment.
health:
  interval: 30         # Check every 30 seconds
  timeout: 5           # Ping timeout in seconds
  failure_threshold: 3 # Failures before circuit opens
  max_restarts: 5      # Max restarts per window
  restart_window: 60   # Window in seconds (1 min)
```

## Building

### From Source

```bash
cargo build                        # Debug build
cargo build --release              # Release with all features
cargo build --no-default-features  # Minimal (no V8, no semantic, no admin)
cargo test                         # Run unit tests
cargo clippy -- -D warnings        # Lint check
```

### Cross-Platform Targets

The CI/CD pipeline builds for all major platforms:

| Platform | Target | Artifact |
|----------|--------|----------|
| macOS (Apple Silicon) | `aarch64-apple-darwin` | `.tar.gz` |
| macOS (Intel) | `x86_64-apple-darwin` | `.tar.gz` |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz`, `.deb`, `.rpm` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

Release artifacts and SHA-256 checksums are published to [GitHub Releases](https://github.com/jonwraymond/gatemini/releases) on every tag.

### Makefile

```bash
make build    # cargo build --release
make install  # Install to ~/.local/bin/
make clean    # cargo clean
```

## Documentation

Detailed documentation lives in the [`docs/`](docs/) directory:

| Document | Description |
|----------|-------------|
| [Architecture](docs/architecture.md) | IPC model, daemon lifecycle, socket coordination |
| [Tool Discovery](docs/tool-discovery.md) | BM25 + semantic search, brief/full modes, RRF fusion |
| [Token Efficiency](docs/token-efficiency.md) | Measured savings (82-98%), industry comparison |
| [Sandbox](docs/sandbox.md) | V8 execution, bridge preamble, 3-tier strategy |
| [Backend Management](docs/backend-management.md) | Health checks, circuit breaker, runtime registration |
| [Secrets & Config](docs/secrets-and-config.md) | Secret resolution, hot-reload, BWS integration |
| [Resources & Prompts](docs/resources-and-prompts.md) | MCP resources and guided discovery workflows |
| [Telemetry Strategy](docs/telemetry-strategy.md) | OpenTelemetry integration plan |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, testing, and PR guidelines.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

Copyright 2025-2026 Jon Raymond.
