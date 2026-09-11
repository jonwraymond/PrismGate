# gatemini — Claude Code host hooks

Install: copy the `hooks/claude-code/hooks.json` to your Claude Code hooks directory (e.g. `~/.claude/hooks/hooks.json`) and point `CLAUDE_PLUGIN_ROOT` at this repo.

These hooks redirect large host operations through gatemini so raw output never enters the conversation. They require gatemini running in proxy mode.

## PreToolUse hooks

| Tool | Behavior |
|------|----------|
| **Bash** | Commands with output likely >20 lines are redirected to `call_tool_chain`. Single git/mkdir/rm/mv/cd/ls/npm/pip calls pass through. |
| **Read** | Files opened for analysis are read server-side via `execute_file`. Only the result enters context. |
| **WebFetch** | Blocked. Use `fetch_and_index(url)` then `session_search(query)`. Raw HTML never enters context. |

## SessionStart hook

On session start and after compact, the hook injects a resume card:
1. Call `session_search(card=true)` for open handles, decisions, constraints
2. Call `read_result(handle)` for any listed payload
3. Search prior context before asking what we're working on

## Setup

```bash
# Add to ~/.claude/hooks/hooks.json
cp hooks/claude-code/hooks.json ~/.claude/hooks/hooks.json

# Or for a specific project
cp hooks/claude-code/hooks.json .claude/hooks/hooks.json
```

The hook uses `${CLAUDE_PLUGIN_ROOT}` which should point at the PrismGate repo root.

## Manual routing (no hooks)

If hooks aren't installed, manually route:

| Need | Instead of | Use |
|------|-----------|-----|
| Analyze file | `Read file.txt` | `execute_file("file.txt")` |
| Fetch URL | `WebFetch url` | `fetch_and_index("url")` then `session_search` |
| Long shell output | `bash cmd` | `call_tool_chain` with shell code |
| Resume after compact | Ask "what were we working on?" | `session_search(card=true)` then `read_result` |
