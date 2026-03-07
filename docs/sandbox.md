# Sandbox

`call_tool_chain` is Gatemini's execution tool for backend orchestration. It accepts JSON or TypeScript and routes it through the cheapest viable execution path.

![Sandbox execution](diagrams/sandbox-execution.svg){ .diagram-wide }

## Execution tiers

The handler in `src/tools/sandbox.rs` tries three paths in order.

### Tier 1: direct JSON

Example:

```json
{"tool": "exa.web_search_exa", "arguments": {"query": "MCP protocol"}}
```

If the payload matches that shape, Gatemini dispatches it directly with no V8 startup cost.

### Tier 2: simple single-call TypeScript

Example:

```typescript
const result = await exa.web_search_exa({query: "MCP protocol"});
return result;
```

The fast path strips common boilerplate and extracts a single `backend.tool(args)` call.

### Tier 3: full V8 sandbox

If the code contains loops, branching, or multiple tool calls, Gatemini executes it in the V8 sandbox from `src/sandbox/mod.rs`.

## Runtime model

The sandbox runs on a dedicated OS thread because V8 isolates are not `Send`.

Key defaults:

| Setting | Default |
|---------|---------|
| execution timeout | `30s` unless overridden |
| max heap size | `50 MiB` |
| thread name | `gatemini-sandbox` |

The tool handler also gates full sandbox execution with a semaphore so too many concurrent isolates do not exhaust memory.

## Bridge contract

Before user code runs, `src/sandbox/bridge.rs` generates a preamble that exposes:

- one JS object per backend
- async functions for each tool
- `__interfaces` for programmatic schema inspection
- `__getToolInterface(name)` for backend-qualified or bare lookups

Backend and tool names are sanitized into valid JS identifiers, but the original names are still passed through to the backend call boundary.

## User-code wrapping

If the submitted code does not export a `main` function, Gatemini wraps it in one before evaluation.

That means these both work:

```typescript
const r = await exa.web_search_exa({query: "test"});
return r;
```

```typescript
export default async function main() {
  return await exa.web_search_exa({query: "test"});
}
```

## Error handling

Tool dispatch happens back on the main Tokio runtime through `Handle::spawn()`.

When a tool exists in the cache but its backend has not reconnected yet, the sandbox augments the returned error so the caller sees a clearer "still starting" explanation instead of a generic unavailable message.

## Security model

The sandbox inherits rustyscript and V8 restrictions and only exposes one controlled bridge back into the host runtime: tool invocation through the backend manager.

In practice that means:

- no direct filesystem capability
- no direct network capability
- no direct subprocess capability
- tool access only through the registered backend surface
