# PrismGate

A high-performance MCP gateway that multiplexes dozens of backend MCP servers through a single shared daemon, eliminating the resource explosion caused by per-session process spawning.

## The Problem

AI coding agents like Claude Code connect to MCP (Model Context Protocol) servers for capabilities like web search, file analysis, and code generation. Each server runs as a separate child process. With 30+ backends configured, every Claude Code session spawns its own copy of every server:

```
Session 1 → 30 backend processes
Session 2 → 30 backend processes
Session 3 → 30 backend processes
───────────────────────────────
Total: 90 processes (and growing)
```

This doesn't scale. Memory usage explodes, startup is slow, and backends that maintain state (databases, browser sessions) can't be shared.

## The Solution

PrismGate runs a single daemon that manages all backend connections. Multiple Claude Code sessions connect to the daemon via Unix socket through lightweight proxy processes:

```
Claude Code ──stdio──▸ proxy ──┐
Claude Code ──stdio──▸ proxy ──┤ Unix socket
Claude Code ──stdio──▸ proxy ──┘
                                │
                         daemon (1 process)
                           ├── backend #1
                           ├── backend #2
                           └── ... (30+ backends, shared)
```

**Result:** 30 backend processes total, regardless of how many sessions are active.

## Features

- **Shared daemon architecture** — one set of backends serves all clients via Unix socket IPC
- **Auto-start** — the first proxy connection spawns the daemon automatically; flock-based coordination prevents races
- **Idle shutdown** — daemon exits after configurable timeout with no active clients; restarts transparently on next use
- **Progressive tool discovery** — 7 meta-tools with brief/full modes keep token usage low (~60 tokens per search result vs ~500)
- **V8 sandbox** — `call_tool_chain` executes TypeScript for multi-tool orchestration in a single MCP call
- **Prerequisite processes** — auto-start dependent services (e.g., a web app) before launching its MCP server
- **Health monitoring** — periodic pings, circuit breaker, auto-restart with exponential backoff
- **Tool cache** — instant tool availability on daemon restart (loaded before backends reconnect)
- **Secret resolution** — `secretref:bws:...` patterns resolved at startup via Bitwarden Secrets Manager
- **Dual transport** — stdio child processes and streamable-HTTP backends in the same config
- **BM25 + semantic search** — tool discovery via keyword and embedding-based similarity

## Quick Start

### Prerequisites

- Rust 1.75+
- Backends you want to connect (Node.js tools, Python servers, HTTP APIs, etc.)

### Install

```bash
cargo install --path .
```

### Configure

```bash
cp config/example.yaml config/gatemini.yaml
# Edit config/gatemini.yaml with your backends
```

### Run

PrismGate works as an MCP server itself. Add it to your Claude Code config:

```json
{
  "mcpServers": {
    "gatemini": {
      "command": "gatemini"
    }
  }
}
```

The first connection auto-starts the daemon. Subsequent sessions share the same daemon.

### CLI Commands

```bash
gatemini              # Default: proxy mode (stdio ↔ Unix socket)
gatemini serve        # Run as daemon directly
gatemini status       # Show daemon PID and state
gatemini stop         # Gracefully stop the daemon
```

## Configuration

See [`config/example.yaml`](config/example.yaml) for a full example covering:

- Stdio backends (command + args)
- Streamable-HTTP backends (URL + headers)
- Prerequisite processes (auto-start dependencies)
- Secret references (`secretref:bws:...` and `${ENV_VAR}`)
- Health check tuning
- Sandbox settings

## Architecture

See [CLAUDE.md](CLAUDE.md) for detailed module documentation, key patterns, and the complete architecture overview.

## Meta-Tools

PrismGate exposes 7 meta-tools to clients instead of proxying hundreds of individual tools:

| Tool | Purpose |
|------|---------|
| `search_tools` | BM25 + semantic search across all backends |
| `list_tools_meta` | Compact index of all available tools |
| `tool_info` | Full schema for a specific tool |
| `get_required_keys` | List env vars needed by a backend |
| `call_tool_chain` | Execute tool calls (direct, JSON batch, or TypeScript via V8) |
| `register_manual` | Add a backend at runtime |
| `deregister_manual` | Remove a backend at runtime |

## Building

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # Run tests
```

## Releases

Binary builds and packages are published from GitHub tags that match `v*` (for example `v0.3.0`).

From a release tag `vX.Y.Z`, GitHub Releases will include:

- `gatemini-vX.Y.Z-darwin-x86_64.tar.gz` (macOS Intel)
- `gatemini-vX.Y.Z-darwin-arm64.tar.gz` (macOS Apple Silicon)
- `gatemini-vX.Y.Z-windows-x86_64.zip` (Windows)
- `gatemini-vX.Y.Z-linux-x86_64.tar.gz` (Linux x86_64)
- `gatemini_X.Y.Z_amd64.deb` (Debian/Ubuntu package)
- `gatemini-X.Y.Z-1.x86_64.rpm` (RPM package)
- `checksums.txt` (SHA-256 for all assets)

### Install examples

```bash
tar -xzf gatemini-vX.Y.Z-darwin-arm64.tar.gz   # macOS (Intel/ARM: pick the matching tarball)
tar -xzf gatemini-vX.Y.Z-linux-x86_64.tar.gz    # Linux tarball

dpkg -i gatemini_X.Y.Z_amd64.deb                # Debian/Ubuntu
rpm -i gatemini-X.Y.Z-1.x86_64.rpm             # RHEL/Fedora/SLES

# Windows
# Extract gatemini-vX.Y.Z-windows-x86_64.zip and run gatemini.exe

# Verify checksum
sha256sum -c checksums.txt
```

## License

MIT
