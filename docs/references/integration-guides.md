# MCP Client Integration Guides

This document explains how to connect popular MCP clients — Cursor, Claude Desktop, Windsurf, and others — to PrismGate.

## Prerequisites

1. **PrismGate is installed and running.** Follow the [Quickstart](../README.md#quickstart) to build and configure PrismGate.
2. **At least one backend is defined** in `~/.config/gatemini/config.yaml`.
3. The `gatemini` binary is on your PATH (typically `~/.local/bin/gatemini` or installed via cargo).

Verify PrismGate works from a terminal before configuring any client:

```bash
gatemini status
# Expected output: daemon running, socket bound, backends listed
```

---

## General Integration Pattern

All MCP clients support the same basic mechanism: they launch PrismGate as a child process over stdio and exchange newline-delimited JSON-RPC messages on stdin/stdout.

When a client configures PrismGate, it needs to know:

| Field | Meaning |
|-------|---------|
| `command` | Path to the `gatemini` binary (e.g. `gatemini`, `/usr/local/bin/gatemini`) |
| `args` | Optional CLI flags (typically `["serve"]` or `["--direct"]`) |
| `env` | Optional environment variables passed to the child process (e.g. API keys if not using BWS) |

**Important**: PrismGate's stdio transport is a two-phase handshake. The client sends an `initialize` request; the gateway responds with `result` containing server capabilities, then sends an `initialized` notification. The client must complete this handshake before sending tool calls. All official MCP clients handle this correctly.

---

## Cursor

**Config location**: `~/.cursor/mcp.json`

### Minimal stdio config

```json
{
  "mcpServers": {
    "prismgate": {
      "command": "gatemini",
      "args": ["serve"]
    }
  }
}
```

If `gatemini` is not on your PATH, use the absolute path:

```json
{
  "mcpServers": {
    "prismgate": {
      "command": "/usr/local/bin/gatemini",
      "args": ["serve"]
    }
  }
}
```

### With environment variables

If your backends require API keys and you are not using Bitwarden Secrets Manager, pass them via `env`:

```json
{
  "mcpServers": {
    "prismgate": {
      "command": "gatemini",
      "args": ["serve"],
      "env": {
        "GITHUB_TOKEN": "ghp_your_token_here",
        "CONTEXT7_API_KEY": "your_context7_key"
      }
    }
  }
}
```

### Using streamable-HTTP mode

If PrismGate is running as a standalone daemon exposing an HTTP MCP endpoint (future feature; currently stdio-only), the Cursor config would instead use a URL:

```json
{
  "mcpServers": {
    "prismgate": {
      "url": "http://localhost:19999/mcp"
    }
  }
}
```

_Note: The streamable-HTTP transport is not yet implemented in PrismGate. This is included for forward compatibility._

### Verify the connection

After restarting Cursor, open the MCP tools panel. You should see:

- **Tools**: 7 gateway meta-tools (`search_tools`, `list_tools_meta`, `tool_info`, `get_required_keys_for_tool`, `call_tool_chain`, `register_manual`, `deregister_manual`)
- **Resources**: `gatemini://overview`, `gatemini://backends`, `gatemini://health`, etc.
- **Prompts**: `discover`, `find_tool`, `backend_status`

If Cursor shows "connection failed", check the Cursor output panel for errors and verify that `gatemini status` shows a healthy daemon.

---

## Claude Desktop

**Config locations**:

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |

### Minimal stdio config

```json
{
  "mcpServers": {
    "prismgate": {
      "command": "gatemini",
      "args": ["serve"]
    }
  }
}
```

### With environment variables

```json
{
  "mcpServers": {
    "prismgate": {
      "command": "gatemini",
      "args": ["serve"],
      "env": {
        "GITHUB_TOKEN": "ghp_your_token_here"
      }
    }
  }
}
```

### Restart Claude Desktop

After editing the config, fully quit and relaunch Claude Desktop. The new MCP server appears under the tools icon (🔌) in the sidebar.

### Known issue: "stdio transport requires initialize request"

If Claude Desktop shows "transport error", ensure:

1. The `gatemini` binary is executable and on PATH.
2. No other process is holding the socket lock (run `gatemini status` and `gatemini stop` if needed).
3. The config JSON is valid (use `jq . ~/.config/Claude/claude_desktop_config.json` to check).

---

## Windsurf (Codeium)

**Config location**: `~/.codeium/windsurf/mcp_config.json` (check Windsurf docs for your version)

```json
{
  "mcpServers": {
    "prismgate": {
      "command": "gatemini",
      "args": ["serve"]
    }
  }
}
```

Windsurf supports stdio MCP servers natively. The same config pattern applies.

---

## Zed

**Config location**: `~/.config/zed/settings.json`

Add under the `"mcp"` key:

```json
{
  "mcp_servers": {
    "prismgate": {
      "command": "gatemini",
      "args": ["serve"]
    }
  }
}
```

Zed requires the `mcp` feature flag to be enabled in settings.

---

## Continue Dev

**Config location**: `~/.continue/config.json`

```json
{
  "mcpServers": {
    "prismgate": {
      "command": "gatemini",
      "args": ["serve"]
    }
  }
}
```

---

## OpenWebUI

**Config location**: Via the web UI → Settings → MCP Servers

```
Command: gatemini serve
Environment: (leave blank unless using direct env vars)
```

---

## Direct mode for debugging

For single-session debugging without a daemon, use `--direct`:

```json
{
  "mcpServers": {
    "prismgate-debug": {
      "command": "gatemini",
      "args": ["--direct"]
    }
  }
}
```

Direct mode skips socket negotiation and runs everything in-process. It is slower for repeated calls because backend initialization happens on every launch, but it is useful for isolating client-side issues.

---

## Troubleshooting

### Client says "server not found" or "command not found"

- Verify `gatemini` is on PATH: `which gatemini` or `command -v gatemini`.
- If installed via cargo, the default location is `~/.cargo/bin/gatemini`. Add it to PATH or use the absolute path in the client config.
- Restart the client after any PATH change.

### Client connects but shows no tools

- PrismGate uses **search-first discovery**. The client should see the 7 meta-tools immediately.
- If meta-tools are missing, the MCP initialize handshake likely failed. Check client logs.
- Run `gatemini status` to confirm the daemon is healthy and backends are loaded.

### Tools fail with "backend not started"

- The backend process may have crashed during startup. Check health status:

  ```bash
  gatemini status --json
  ```

  Backends show state: `Healthy`, `Unhealthy`, `Starting`, or `Stopped`.
- Review backend logs: `gatemini logs` or check `~/.config/gatemini/logs/`.
- Verify required environment variables are set (or configured via secretref).

### "Too many tools" errors in Cursor

- Cursor imposes a limit of 40 MCP tools. PrismGate exposes only 7 meta-tools, so this should never trigger.
- If you manually register additional tools with `register_manual`, keep the total under Cursor's limit.

### Socket file already exists (daemon already running)

PrismGate uses a Unix domain socket with flock-based coordination. If a stale socket remains after a crash:

```bash
gatemini stop   # clean shutdown
gatemini serve  # restart fresh
```

Do **not** manually delete the socket file; use `gatemini stop` to ensure proper cleanup.

---

## Uninstalling

To remove PrismGate from a client, delete its entry from the client's MCP server config and restart the client. There is no persistent state in the client beyond the config entry.

To fully remove PrismGate from the system:

```bash
# Stop the daemon
gatemini stop

# Remove config and data
rm -rf ~/.config/gatemini
rm -f ~/.local/bin/gatemini   # if installed via cargo
cargo uninstall gatemini      # if installed via cargo install
```
