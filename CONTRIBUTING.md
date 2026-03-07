# Contributing to Gatemini

This guide reflects the current repo layout and build surface.

## Prerequisites

- Rust 1.85+
- Node.js 18+ if you want to exercise common `npx`-based MCP backends
- D2 if you want to edit and regenerate diagrams
- Python plus `requirements-docs.txt` if you want to build the MkDocs site locally

## Build and test

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

Docs build:

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements-docs.txt
mkdocs build
```

## Branching

Use a prefixed branch name:

```bash
git checkout -b codex/your-change
```

## Project structure

```text
src/
  main.rs                shared initialization and mode dispatch
  cli.rs                 CLI parsing and platform path helpers
  config.rs              config loading, defaults, validation, watcher
  server.rs              public MCP tools and session server
  registry.rs            tool registry and search
  cache.rs               cache load/save
  tracker.rs             recent-call and latency tracking
  resources.rs           MCP resources
  prompts.rs             MCP prompts
  admin.rs               optional admin API
  ipc/                   proxy, daemon, socket, status, stop, restart, framing
  backend/               transport implementations and lifecycle management
  sandbox/               V8 bridge and execution thread
  secrets/               secret providers and resolver
  tools/                 discovery, registration, and sandbox handlers
```

## Documentation rules for contributors

- Treat `src/config.rs` as the default-value source of truth.
- Treat `src/server.rs`, `src/resources.rs`, and `src/prompts.rs` as the public MCP-surface source of truth.
- Treat `src/backend/mod.rs` and `src/backend/health.rs` as the backend-state source of truth.
- Avoid baking registry-size snapshots into docs unless they are clearly labeled as examples.
- If you change D2 sources in `docs/diagrams/*.d2`, regenerate the matching SVGs before finishing.

## Regenerating diagrams

From the repo root:

```bash
for f in docs/diagrams/*.d2; do
  d2 -l elk "$f" "${f%.d2}.svg"
done
```

## Pull requests

Keep changes tight and verifiable:

- explain what changed
- explain why the change was needed
- include docs updates when the public surface or defaults changed
- mention any follow-up work if the implementation still has known limits
