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

The fast path uses `normalize_simple_call` to canonicalize common boilerplate patterns into a `backend.tool(args)` form before dispatch. Recognized forms include `const x = await b.t({...}); return x;`, `return await b.t({...})`, and bare `b.t({...})`.

If the normalized form matches `__interfaces` or `__getToolInterface(name)` introspection patterns, `try_introspection_call` handles them directly without entering the V8 isolate.

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

### globalThis mirrors

Every backend object is also mirrored onto `globalThis` so that both `globalThis['auggie']` and `globalThis.exa` work. If the sanitized name differs from the original (e.g. `my-backend` becomes `my_backend`), both names are mirrored. This handles LLM-generated code that references backends via `globalThis` instead of the local variable.

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

## Output processing pipeline

After execution, the raw output passes through a four-stage pipeline before being returned:

```
raw output → intent filter → auto-chunk JSON → truncate → size metadata
```

Each stage is configurable via `OutputConfig` in the sandbox config. All stages are enabled by default.

### Stage 1: intent filtering

If the caller provides an `intent` parameter in `call_tool_chain` and the output exceeds 5,000 bytes (`INTENT_SEARCH_THRESHOLD`), the output is split into paragraph-sized chunks and scored against the intent terms. Only chunks where at least 30% of intent terms appear are kept. Outputs below 5 KB bypass this stage entirely.

### Stage 2: auto-chunk large JSON

If `auto_chunk_json` is enabled and the output exceeds `chunk_threshold` (default 10240 bytes) and parses as JSON, the value is replaced with a compact key-path summary.

Two collapse strategies are tried in order:

1. **Uniform array detection** — if the JSON is an array where all items share the same key structure (common in loop patterns), the output is collapsed to a count, key list, first N items, and remaining identities.
2. **Key-path chunking** — otherwise, `src/tools/json_chunker.rs` recursively walks the value, producing chunks with hierarchical path titles (e.g. `results > items > [0-4]`). Identity fields (`id`, `name`, `title`, `slug`, `key`, `label`) are extracted for labeling.

If the JSON cannot be parsed or produces only one chunk, the raw text is passed through unchanged.

### Stage 3: smart truncation

If `smart_truncation` is enabled, long outputs are truncated using a head-60% / tail-40% split snapped to line boundaries. This preserves both the beginning (setup, context) and the end (results, errors, summaries) of tool output — a simple head-only cutoff would lose the tail, which is often the most important part.

When `smart_truncation` is false, simple head-only truncation is used instead (legacy behavior).

The truncation marker format:

```
... [truncated middle — X.XKB omitted] ...
```

For multi-line output:

```
... [N lines / X.XKB truncated — showing first M + last K lines] ...
```

### Stage 4: response size metadata

When the pipeline reduces output by more than 200 bytes, a size footer is appended:

```
[Output: X.XKB returned, Y.YKB processed, N% reduced]
```

This lets callers see exactly how much compression occurred and is also used by `gatemini://stats` for session-level byte tracking.

### OutputConfig reference

| Field | Default | Effect |
|-------|---------|--------|
| `auto_chunk_json` | `true` | chunk JSON outputs above threshold |
| `smart_truncation` | `true` | head 60% + tail 40% instead of head-only |
| `chunk_threshold` | `10240` | minimum bytes to trigger JSON chunking |

## Error handling

Tool dispatch happens back on the main Tokio runtime through `Handle::spawn()`.

When a tool exists in the cache but its backend has not reconnected yet, the sandbox augments the returned error so the caller sees a clearer "still starting" explanation instead of a generic unavailable message.

### Error hints

The `enhance_sandbox_error` function in `src/sandbox/mod.rs` intercepts V8 errors and appends actionable hints for five common LLM coding mistakes:

| Pattern | V8 error | Hint |
|---------|----------|------|
| Variable shadowing | `ReferenceError: Cannot access 'X' before initialization` | `X` is a backend name; use a different variable like `xResult` |
| Misspelled backend | `ReferenceError: X is not defined` with close name match | Suggests the correct backend name |
| Backend unavailable | `Backend 'X' is not available` | Load `@gatemini://backend/X` to check status |
| Meta-tool in sandbox | `ReferenceError: gatemini is not defined` (or `search_tools`, `tool_info`, `list_tools_meta`) | Meta-tools cannot be called inside `call_tool_chain`; use them as separate MCP tool calls |
| Bare tool name | `ReferenceError: X is not defined` with no backend match | `X` may be a tool; call it as `backend_name.X({args})` |

## Security model

The sandbox inherits rustyscript and V8 restrictions and only exposes one controlled bridge back into the host runtime: tool invocation through the backend manager.

In practice that means:

- no direct filesystem capability
- no direct network capability
- no direct subprocess capability
- tool access only through the registered backend surface
