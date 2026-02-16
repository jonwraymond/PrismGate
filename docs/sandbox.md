# Sandbox

PrismGate's `call_tool_chain` meta-tool lets AI agents execute TypeScript code that calls backend tools. This enables multi-tool orchestration (loops, conditionals, error handling) in a single execution context, reducing LLM round-trips and token usage.

## Three-Tier Execution Strategy

`call_tool_chain` uses a performance-optimized tiered approach:

```
Input code
  ├─ Tier 1: Direct JSON parsing ──────── O(1), no V8 overhead
  ├─ Tier 2: Simple TypeScript parsing ── Regex-based, no V8 overhead
  └─ Tier 3: V8 sandbox ─────────────── Full TypeScript execution
```

### Tier 1: Direct JSON Parsing

**Source**: [`src/tools/sandbox.rs`](../src/tools/sandbox.rs)

Detects pure JSON tool calls:

```json
{"tool": "exa.web_search_exa", "arguments": {"query": "MCP protocol"}}
```

Parsed directly, dispatched to the backend, no V8 involved.

### Tier 2: Simple TypeScript Parsing

Detects single-tool TypeScript patterns by stripping common boilerplate:

```typescript
const result = await exa.web_search_exa({query: "MCP protocol"}); return result;
```

The parser strips `const result =`, `await`, `return result`, and semicolons to extract the core call pattern `backend.tool_name({args})`. This handles the vast majority of LLM-generated tool calls without V8 startup overhead.

### Tier 3: V8 Sandbox

**Source**: [`src/sandbox/mod.rs`](../src/sandbox/mod.rs)

For multi-step, conditional, or complex orchestration that can't be parsed as a simple call:

```typescript
const results = await exa.web_search_exa({query: "Rust MCP"});
const urls = results.results.map(r => r.url);
const details = [];
for (const url of urls.slice(0, 3)) {
  details.push(await firecrawl.firecrawl_scrape({url}));
}
return {urls, details};
```

## V8 Sandbox Architecture

### The Thread Boundary Problem

V8 isolates are `!Send` -- they cannot cross thread boundaries. But PrismGate's backend services (`BackendManager`, `ToolRegistry`) live in the main tokio async runtime. The sandbox solves this with a dedicated OS thread and runtime bridge:

```
Main tokio runtime                    Dedicated V8 thread
  │                                     │
  ├─ execute() called                   │
  │   ├─ Clone Arcs (registry, mgr)     │
  │   ├─ Spawn "gatemini-sandbox" ──────┤
  │   │   oneshot channel               │
  │   │                                 ├─ Create V8 Runtime
  │   │                                 ├─ Register __call_tool
  │   │                                 ├─ Generate preamble
  │   │                                 ├─ Load user module
  │   │                                 ├─ Execute main()
  │   │                                 │   ├─ JS calls __call_tool()
  │   │  ◀── Handle::spawn ────────────│   │   (dispatches to tokio)
  │   ├─ mgr.call_tool() ──────────────│   │
  │   ├─ Returns result ───────────────▸│   │
  │   │                                 │   └─ Continues execution
  │   │                                 ├─ Send result via oneshot
  │   ◀─────────────────────────────────┤
  │                                     │ Thread exits
  └─ Return result
```

### Runtime Configuration

| Parameter | Default | Purpose |
|-----------|---------|---------|
| `timeout` | 30s | Maximum execution time |
| `max_heap_size` | 50 MB | V8 heap limit |
| Thread name | `gatemini-sandbox` | OS thread identifier |

### Output Handling

Results are truncated to 200,000 characters (configurable via `max_output_size`) with UTF-8 boundary awareness:

```rust
output.truncate(output.floor_char_boundary(max_size));
```

`floor_char_boundary` ensures truncation doesn't split multi-byte UTF-8 characters, preventing corrupted JSON responses.

## Bridge Preamble

**Source**: [`src/sandbox/bridge.rs`](../src/sandbox/bridge.rs)

Before user code executes, PrismGate generates a JavaScript preamble that creates tool accessor objects for every registered backend:

```javascript
// Helper alias for the registered async function
const __ct = rustyscript.async_functions['__call_tool'];

// Backend accessor: exa
const exa = {
  web_search_exa: async (args) => await __ct("exa", "web_search_exa", args || {}),
  find_similar: async (args) => await __ct("exa", "find_similar", args || {}),
};

// Backend accessor: tavily
const tavily = {
  tavily_search: async (args) => await __ct("tavily", "tavily_search", args || {}),
};

// Introspection API
const __interfaces = {
  "exa": {
    "web_search_exa": { name: "web_search_exa", description: "...", input_schema: {...} },
    "find_similar": { name: "find_similar", description: "...", input_schema: {...} },
  },
  // ...
};

function __getToolInterface(dotted_name) {
  const parts = dotted_name.split('.');
  if (parts.length === 2) {
    return __interfaces[parts[0]]?.[parts[1]] || null;
  }
  // Search all backends for bare tool name
  for (const backend of Object.values(__interfaces)) {
    if (backend[dotted_name]) return backend[dotted_name];
  }
  return null;
}
```

### Identifier Sanitization

Backend and tool names are sanitized for use as JavaScript identifiers:

| Input | Sanitized | Rule |
|-------|-----------|------|
| `exa` | `exa` | Already valid |
| `my-search-backend` | `my_search_backend` | Hyphens → underscores |
| `123start` | `_123start` | Leading digit → prefix underscore |
| `has.dots` | `has_dots` | Dots → underscores |

The original names are preserved in the `__ct()` calls as JSON-escaped strings, so the backend receives the correct name regardless of sanitization.

### String Escaping

All backend and tool names in `__ct()` calls are JSON-escaped via `serde_json::to_string()`:

```javascript
// Safe: embedded quotes and backslashes are properly escaped
web_search: async (args) => await __ct("exa", "web_search_exa", args || {}),
```

This prevents injection attacks from malicious tool names.

## User Code Wrapping

If the user's code doesn't export a `main` function, PrismGate wraps it automatically:

**User writes**:
```typescript
const r = await exa.web_search_exa({query: "test"});
return r;
```

**PrismGate wraps as**:
```typescript
// ... preamble ...
export default async function main() {
  const r = await exa.web_search_exa({query: "test"});
  return r;
}
```

If the code already contains `export default` or `export async function main`, it's used as-is.

## Tool Dispatch

When JavaScript calls `__ct(backend, tool, args)`:

1. **Validate arguments**: backend_name and tool_name must be strings
2. **Dispatch to tokio**: `Handle::spawn()` bridges back to the main async runtime
3. **Call backend**: `manager.call_tool(backend_name, tool_name, arguments)`
4. **Error enhancement**: If the tool exists in the registry but the backend isn't ready, the error message is enhanced:

```
Backend 'exa' is still starting. Tool 'web_search_exa' is cached
but the backend hasn't connected yet. Try again shortly.
```

This handles the common case where the tool cache provides tool definitions before all backends have connected.

## Introspection API

The sandbox exposes two introspection mechanisms:

### `__interfaces`

A nested object containing full tool schemas grouped by backend. Useful for programmatic schema inspection:

```typescript
const schema = __interfaces.exa.web_search_exa.input_schema;
const params = Object.keys(schema.properties);
```

### `__getToolInterface(name)`

Look up a tool by dotted notation (`backend.tool_name`) or bare name:

```typescript
const info = __getToolInterface("exa.web_search_exa");
// or
const info = __getToolInterface("web_search_exa"); // searches all backends
```

## Security Considerations

The V8 sandbox inherits [Deno's security model](https://docs.deno.com/runtime/fundamentals/security/) through rustyscript:

- **No filesystem access** by default
- **No network access** by default
- **No environment variable access** by default
- **No subprocess spawning** by default

The only capability is calling `__call_tool`, which is mediated through PrismGate's `BackendManager`. This provides a controlled execution environment where the TypeScript code can only interact with registered backend tools.

The V8 engine also provides [in-process sandboxing](https://v8.dev/blog/sandbox) with ~1% performance overhead, isolating heap memory to prevent memory corruption from spreading.

## Sources

- [`src/tools/sandbox.rs`](../src/tools/sandbox.rs) -- Three-tier execution strategy
- [`src/sandbox/mod.rs`](../src/sandbox/mod.rs) -- V8 sandbox implementation
- [`src/sandbox/bridge.rs`](../src/sandbox/bridge.rs) -- Preamble generation
- [rustyscript](https://github.com/rscarson/rustyscript) -- V8 sandbox wrapper for Rust
- [Deno Security](https://docs.deno.com/runtime/fundamentals/security/) -- Permission model
- [V8 Sandbox](https://v8.dev/blog/sandbox) -- In-process memory isolation
- [Glama Code Execution](https://glama.ai/blog/2025-12-14-code-execution-with-mcp-architecting-agentic-efficiency) -- Token savings from code execution
- [Block Goose: Code Mode + MCP](https://block.github.io/goose/blog/2025/12/21/code-mode-doesnt-replace-mcp/) -- Complementary pattern validation
- [E2B Architecture](https://memo.d.foundation/breakdown/e2b) -- Firecracker alternative comparison
