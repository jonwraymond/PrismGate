# Contributing to Gatemini

Thanks for your interest in contributing to Gatemini! This guide covers everything you need to get started.

## Prerequisites

- **Rust 1.85+** (edition 2024) via [rustup](https://rustup.rs/)
- **Node.js 18+** (many MCP backends are npx-based)
- **D2** (optional, for diagram rendering): `brew install d2` or [d2lang.com](https://d2lang.com)

### Optional dependencies

| Feature | Dependency | Purpose |
|---------|-----------|---------|
| `sandbox` | V8 via rustyscript | TypeScript execution in `call_tool_chain` |
| `semantic` | model2vec-rs + hf-hub | Embedding-based tool search |
| `admin` | axum | HTTP admin API |

All three are enabled by default. To build without optional features:

```bash
cargo build --no-default-features
```

## Getting Started

```bash
# Clone the repo
git clone https://github.com/jonwraymond/prismgate.git
cd prismgate

# Build (debug)
cargo build

# Build (release, includes V8 + semantic + admin)
cargo build --release

# Run tests
cargo test

# Run clippy
cargo clippy -- -D warnings

# Check formatting
cargo fmt --check
```

## Project Structure

```
src/
├── main.rs                    # Entry point, InitializedGateway setup
├── cli.rs                     # clap CLI (proxy, serve, status, stop)
├── config.rs                  # Config parsing, validation, hot-reload
├── server.rs                  # MCP server, per-client sessions
├── registry.rs                # ToolRegistry (BM25 + RRF hybrid search)
├── cache.rs                   # Tool cache persistence
├── embeddings.rs              # Semantic embedding search (model2vec)
├── tracker.rs                 # CallTracker (usage, latency, recents)
├── resources.rs               # MCP resources (@-mention URIs)
├── prompts.rs                 # MCP prompts (discover, find_tool, etc.)
├── admin.rs                   # Optional axum admin API
│
├── ipc/
│   ├── proxy.rs               # stdio <-> Unix socket bridge
│   ├── daemon.rs              # Socket listener, accept loop, idle shutdown
│   ├── socket.rs              # Path resolution, PID files, liveness
│   ├── status.rs              # `gatemini status` command
│   └── stop.rs                # `gatemini stop` command
│
├── backend/
│   ├── mod.rs                 # BackendManager, Backend trait, DashMap
│   ├── stdio.rs               # Child process backends (MCP over stdio)
│   ├── http.rs                # HTTP backends (streamable-HTTP)
│   ├── health.rs              # Health checker, circuit breaker, backoff
│   ├── composite.rs           # Virtual backend for composite tools
│   └── lenient_client.rs      # HTTP wrapper for missing Content-Type
│
├── tools/
│   ├── discovery.rs           # search_tools, list_tools_meta, tool_info
│   ├── register.rs            # register_manual, deregister_manual
│   └── sandbox.rs             # call_tool_chain routing
│
├── sandbox/
│   ├── mod.rs                 # V8 sandbox execution
│   └── bridge.rs              # JS preamble generation
│
└── secrets/
    ├── resolver.rs            # secretref: pattern resolution
    └── bws.rs                 # Bitwarden Secrets Manager provider
```

## Development Workflow

### 1. Create a branch

```bash
git checkout -b feat/your-feature main
```

### 2. Write tests first

We follow TDD where practical. Tests live alongside source code in `#[cfg(test)]` modules:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_your_feature() {
        // ...
    }
}
```

For async tests:

```rust
#[tokio::test]
async fn test_async_feature() {
    // ...
}
```

### 3. Implement the feature

Key patterns to follow:

- **Concurrent state**: Use `DashMap` for shared data, `Arc<T>` for shared ownership
- **Backend trait**: Implement `Backend` for new transport types
- **Config**: Add new fields with `#[serde(default)]` for backwards compatibility
- **Error handling**: Use `anyhow::Result` for application errors, `thiserror` for library errors
- **Logging**: Use `tracing` macros (`info!`, `warn!`, `error!`) with structured fields

### 4. Verify

```bash
cargo test                           # All tests pass
cargo clippy -- -D warnings          # No warnings
cargo fmt --check                    # Formatting clean
```

### 5. Submit a PR

- Keep PRs focused on a single concern
- Write a clear description of what changed and why
- Reference related issues if applicable
- CI runs tests, clippy, and fmt checks automatically

## Architecture Overview

Gatemini uses a **shared daemon architecture**:

```
Claude Code ──stdio──> proxy ──┐
Claude Code ──stdio──> proxy ──┤ Unix socket
Claude Code ──stdio──> proxy ──┘
                                │
                         daemon (1 process)
                           ├── backend #1 (stdio)
                           ├── backend #2 (stdio)
                           └── backend #3 (HTTP)
```

**Proxy mode** (default): Lightweight byte pipe bridging stdio to Unix socket. Auto-spawns daemon on first use.

**Daemon mode** (`serve`): Binds Unix socket, manages backends, serves multiple clients concurrently. Exits on idle timeout (default 5 min).

### Key subsystems

| Subsystem | Owner | Description |
|-----------|-------|-------------|
| `BackendManager` | `backend/mod.rs` | DashMap of running backends, lifecycle management |
| `ToolRegistry` | `registry.rs` | BM25 + semantic search index, tool namespace collision detection |
| `HealthChecker` | `backend/health.rs` | Periodic pings, circuit breaker, auto-restart with backoff |
| `GateminiServer` | `server.rs` | Per-client MCP session, tool router |
| `CallTracker` | `tracker.rs` | Usage counts, latency histograms, recent calls |

### Concurrency model

- **Tokio** async runtime for all I/O
- **DashMap** for lock-free concurrent reads (one shard per CPU)
- **Arc** for shared ownership across tasks
- **RwLock** for infrequently-written shared state
- **AtomicUsize/AtomicU8** for counters and state flags
- **V8 sandbox** runs on a dedicated OS thread (V8 isolates are `!Send`)

## Testing

```bash
# Run all tests
cargo test

# Run a specific test
cargo test test_name

# Run tests for a specific module
cargo test registry::tests

# Run with output
cargo test -- --nocapture
```

Current test count: 172+ unit tests covering registry, config, cache, daemon, backend concurrency, embeddings, MCP compliance, and more.

## Code Style

- Follow standard Rust conventions (`rustfmt` defaults)
- Prefer `anyhow::Result` for fallible functions
- Use structured logging: `info!(field = %value, "message")`
- Keep functions focused and small
- Document public APIs with `///` doc comments
- Use `#[cfg(feature = "...")]` for optional functionality

## Questions?

Open an issue on GitHub or check the [detailed docs](docs/README.md) for architecture deep-dives.
