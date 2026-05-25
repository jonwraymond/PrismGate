# PrismGate

> The open-source MCP gateway for scalable tool discovery, session isolation, and enterprise-grade secrets management.

[![Tests](https://img.shields.io/badge/tests-302%20passing-brightgreen)](https://github.com/jonwraymond/PrismGate/actions)
[![Clippy](https://img.shields.io/badge/clippy-clean-brightgreen)](https://github.com/jonwraymond/PrismGate/actions)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

## What is PrismGate?

PrismGate (codename **Gatemini**) is a high-performance MCP gateway written in Rust. It acts as a reverse proxy and management layer for MCP (Model Context Protocol) servers, providing scalable, session-aware routing, tool discovery, and lifecycle management.

### Key Features

- **3-Tier Tool Discovery** — BM25 → trigram → fuzzy Levenshtein search across all registered tools
- **7 Gateway Meta-Tools** — `search_tools`, `list_tools_meta`, `tool_info`, `required_keys`, `call_tool_chain`, `register_manual`, `deregister_manual`
- **V8 Sandbox** — Execute TypeScript composite tools in an isolated sandbox
- **Per-Session Isolation** — Dedicated backend instances per MCP client session (InstancePool)
- **Enterprise Secrets** — BWS (Bitwarden Secrets Manager) integration with environment variable fallback
- **Multi-Transport** — stdio, streamable-http, and CLI adapter transports
- **Circuit Breakers** — Automatic health checks, exponential backoff restart, graceful degradation

## Architecture

```
┌─────────────────────────────────────────────────┐
│                  PrismGate                       │
│                                                  │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐      │
│  │ Server   │  │ Registry │  │ Sandbox  │      │
│  │ (MCP     │  │ (3-tier  │  │ (V8      │      │
│  │  surface)│  │  search) │  │  bridge) │      │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘      │
│       │              │              │            │
│  ┌────▼──────────────▼──────────────▼────┐      │
│  │           Backend Manager             │      │
│  │  ┌────────┐ ┌────────┐ ┌────────┐    │      │
│  │  │ stdio  │ │ http   │ │ cli    │    │      │
│  │  │ pool   │ │ pool   │ │ adapter│    │      │
│  │  └────────┘ └────────┘ └────────┘    │      │
│  └───────────────────────────────────────┘      │
│                                                  │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐      │
│  │ Secrets  │  │ Health   │  │ Audit    │      │
│  │ Resolver │  │ Checker  │  │ Logger   │      │
│  └──────────┘  └──────────┘  └──────────┘      │
└─────────────────────────────────────────────────┘
       ▲                              │
       │ MCP Protocol                 │ MCP Protocol
       ▼                              ▼
  ┌──────────┐                  ┌──────────┐
  │ MCP      │                  │ Backend  │
  │ Clients  │                  │ MCP      │
  │ (Claude, │                  │ Servers  │
  │  Cursor, │                  │ (GitHub, │
  │  etc.)   │                  │  etc.)   │
  └──────────┘                  └──────────┘
```

## Quickstart

### Prerequisites
- Rust 1.75+
- Node.js 18+ (for V8 sandbox)

### Install
```bash
git clone https://github.com/jonwraymond/PrismGate.git
cd PrismGate
cargo build --release
```

### Configure
Create `~/.prismgate/config.yaml`:
```yaml
backends:
  - name: github
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-github"]
    env:
      GITHUB_TOKEN: "secretref:bws:project/dotenv/key/GITHUB_TOKEN"

  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

health:
  check_interval: 30s
  restart_initial_backoff: 1s
  restart_max_backoff: 60s
  max_restarts: 5
```

### Run
```bash
prismgate --config ~/.prismgate/config.yaml
```

## Configuration

### Transport Types

| Transport | Use Case | Example |
|-----------|----------|---------|
| `stdio` | Local MCP servers | `npx @modelcontextprotocol/server-github` |
| `streamable-http` | Remote MCP servers | `https://api.example.com/mcp` |
| `cli-adapter` | CLI tools as MCP servers | Custom scripts, legacy tools |

### Secrets Management

PrismGate supports `secretref:<provider>:<reference>` patterns in config values:

```yaml
env:
  API_KEY: "secretref:bws:project/dotenv/key/API_KEY"
```

When BWS is disabled, falls back to environment variables with the same key name.

### Composite Tools

Define multi-step orchestrations as TypeScript snippets:

```yaml
composite_tools:
  - name: create_pr_and_notify
    description: Create a PR and send notification
    code: |
      const pr = await github.create_pull({ title, body, head, base });
      await slack.post_message({ channel: "#prs", text: `PR created: ${pr.html_url}` });
      return pr;
```

## API Reference

### Gateway Meta-Tools

| Tool | Description |
|------|-------------|
| `search_tools` | Search tools by natural language description |
| `list_tools_meta` | List all tools with metadata (paginated) |
| `tool_info` | Get detailed tool information (brief/full) |
| `required_keys` | Get required environment variables for a tool |
| `call_tool_chain` | Execute TypeScript code with access to all tools |
| `register_manual` | Register a manual backend endpoint |
| `deregister_manual` | Remove a manual backend registration |

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Run tests (`cargo test --all-features`)
4. Run clippy (`cargo clippy --all-features`)
5. Run fmt (`cargo fmt --check`)
6. Commit your changes
7. Push to the branch
8. Open a Pull Request

### Non-Negotiable Rules
- **Never push to main** — always use feature branches
- **Full test suite** must pass before every commit
- **No assumptions** — verify everything

## License

MIT License - see [LICENSE](LICENSE) for details.
