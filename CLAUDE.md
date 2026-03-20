# Gatemini

Rust MCP gateway that exposes a small discovery surface over many backend MCP servers.

## Naming

- Runtime name: `gatemini`
- Repository and releases: `PrismGate`

## Runtime summary

- default mode is a proxy that bridges stdio to a Unix socket daemon
- the daemon binds early, then completes shared initialization
- backend tools are discovered through 7 gateway meta-tools
- proxy reconnect can replay the cached MCP initialize handshake
- public backend states are `Starting`, `Healthy`, `Unhealthy`, and `Stopped`

## Command surface

- `gatemini`
- `gatemini --direct`
- `gatemini serve`
- `gatemini status`
- `gatemini stop`
- `gatemini restart`

## Public MCP surface

Tools:

- `search_tools`
- `list_tools_meta`
- `tool_info`
- `get_required_keys_for_tool`
- `call_tool_chain`
- `register_manual`
- `deregister_manual`

Resources:

- `gatemini://overview`
- `gatemini://backends`
- `gatemini://tools`
- `gatemini://recent`
- `gatemini://stats`
- `gatemini://health`
- `gatemini://llms`
- `gatemini://llms-full`
- `gatemini://call_tool_chain`
- `gatemini://tool/{tool_name}`
- `gatemini://backend/{backend_name}`
- `gatemini://backend/{backend_name}/tools`
- `gatemini://recent/{limit}`
- `gatemini://guide/{topic}`

Prompts:

- `discover`
- `find_tool`
- `backend_status`

## Module map

| Module | Purpose |
|--------|---------|
| `src/main.rs` | shared initialization, mode dispatch |
| `src/cli.rs` | CLI and platform paths |
| `src/config.rs` | defaults, config loading, validation, hot-reload |
| `src/server.rs` | public MCP tool surface |
| `src/registry.rs` | tool registry, three-tier search (BM25 → trigram → fuzzy), IDF terms |
| `src/cache.rs` | cache restore and persistence |
| `src/tracker.rs` | recent calls, usage counts, backend latency, session byte tracking |
| `src/resources.rs` | resources, template completion, llms.txt generation |
| `src/prompts.rs` | live prompts driven by registry and tracker |
| `src/ipc/` | proxy/daemon/socket lifecycle |
| `src/backend/` | transport implementations and health management |
| `src/tools/` | meta-tool handlers, intent filtering, JSON chunking |
| `src/sandbox/` | V8 execution bridge |
| `src/secrets/` | secret providers and resolver |

## Dedicated instance mode

- `instance_mode: dedicated` gives each proxy session its own backend instance from a pool
- only applies to `stdio` and `cli-adapter` transports; HTTP backends ignore it
- pool pre-warms `min_idle` instances (default 1), lazy-spawns on demand up to `max_instances` (default 20)
- instances are recycled (stop + respawn) on session disconnect for clean state
- session_id is threaded from daemon accept loop through GateminiServer → sandbox → BackendManager
- direct mode uses session_id 0
- pool implementation lives in `src/backend/pool.rs`
- health checker calls `restart_pool_primary()` instead of `restart_backend()` for dedicated backends

## Context efficiency features

- search has three-tier fallback: BM25 → trigram substring → fuzzy Levenshtein correction
- search results include `try_also` distinctive terms (IDF-scored) for follow-up queries
- `call_tool_chain` supports `intent` param for filtering large outputs to relevant sections
- output truncation uses head 60% + tail 40% split (preserves both beginning and end)
- `gatemini://stats` shows per-session bytes returned vs processed, savings ratio
- `gatemini://llms` and `gatemini://llms-full` auto-generate machine-readable tool references
- JSON chunking utility in `src/tools/json_chunker.rs` for key-path decomposition

## Important implementation notes

- transport names are `stdio`, `streamable-http`, and `cli-adapter`
- backend stderr is currently discarded with `Stdio::null()` for stdio backends
- cache version is `4` and defaults to the platform cache directory
- composite tool changes are detected by the watcher but require restart
- `admin.allowed_cidrs` exists in config but is not enforced in the current admin routes

# context-mode — MANDATORY routing rules

You have context-mode MCP tools available. These rules are NOT optional — they protect your context window from flooding. A single unrouted command can dump 56 KB into context and waste the entire session.

## BLOCKED commands — do NOT attempt these

### curl / wget — BLOCKED
Any Bash command containing `curl` or `wget` is intercepted and replaced with an error message. Do NOT retry.
Instead use:
- `ctx_fetch_and_index(url, source)` to fetch and index web pages
- `ctx_execute(language: "javascript", code: "const r = await fetch(...)")` to run HTTP calls in sandbox

### Inline HTTP — BLOCKED
Any Bash command containing `fetch('http`, `requests.get(`, `requests.post(`, `http.get(`, or `http.request(` is intercepted and replaced with an error message. Do NOT retry with Bash.
Instead use:
- `ctx_execute(language, code)` to run HTTP calls in sandbox — only stdout enters context

### WebFetch — BLOCKED
WebFetch calls are denied entirely. The URL is extracted and you are told to use `ctx_fetch_and_index` instead.
Instead use:
- `ctx_fetch_and_index(url, source)` then `ctx_search(queries)` to query the indexed content

## REDIRECTED tools — use sandbox equivalents

### Bash (>20 lines output)
Bash is ONLY for: `git`, `mkdir`, `rm`, `mv`, `cd`, `ls`, `npm install`, `pip install`, and other short-output commands.
For everything else, use:
- `ctx_batch_execute(commands, queries)` — run multiple commands + search in ONE call
- `ctx_execute(language: "shell", code: "...")` — run in sandbox, only stdout enters context

### Read (for analysis)
If you are reading a file to **Edit** it → Read is correct (Edit needs content in context).
If you are reading to **analyze, explore, or summarize** → use `ctx_execute_file(path, language, code)` instead. Only your printed summary enters context. The raw file content stays in the sandbox.

### Grep (large results)
Grep results can flood context. Use `ctx_execute(language: "shell", code: "grep ...")` to run searches in sandbox. Only your printed summary enters context.

## Tool selection hierarchy

1. **GATHER**: `ctx_batch_execute(commands, queries)` — Primary tool. Runs all commands, auto-indexes output, returns search results. ONE call replaces 30+ individual calls.
2. **FOLLOW-UP**: `ctx_search(queries: ["q1", "q2", ...])` — Query indexed content. Pass ALL questions as array in ONE call.
3. **PROCESSING**: `ctx_execute(language, code)` | `ctx_execute_file(path, language, code)` — Sandbox execution. Only stdout enters context.
4. **WEB**: `ctx_fetch_and_index(url, source)` then `ctx_search(queries)` — Fetch, chunk, index, query. Raw HTML never enters context.
5. **INDEX**: `ctx_index(content, source)` — Store content in FTS5 knowledge base for later search.

## Subagent routing

When spawning subagents (Agent/Task tool), the routing block is automatically injected into their prompt. Bash-type subagents are upgraded to general-purpose so they have access to MCP tools. You do NOT need to manually instruct subagents about context-mode.

## Output constraints

- Keep responses under 500 words.
- Write artifacts (code, configs, PRDs) to FILES — never return them as inline text. Return only: file path + 1-line description.
- When indexing content, use descriptive source labels so others can `ctx_search(source: "label")` later.

## ctx commands

| Command | Action |
|---------|--------|
| `ctx stats` | Call the `ctx_stats` MCP tool and display the full output verbatim |
| `ctx doctor` | Call the `ctx_doctor` MCP tool, run the returned shell command, display as checklist |
| `ctx upgrade` | Call the `ctx_upgrade` MCP tool, run the returned shell command, display as checklist |
