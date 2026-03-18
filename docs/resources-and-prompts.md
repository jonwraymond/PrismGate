# Resources & Prompts

Gatemini exposes tools, resources, and prompts together. The goal is to give clients a compact, structured discovery surface without requiring every interaction to go through tool calls.

## Resources

Resources are implemented in `src/resources.rs`.

### Static resources

| URI | MIME type | Content |
|-----|-----------|---------|
| `gatemini://overview` | `text/plain` | gateway usage overview |
| `gatemini://backends` | `application/json` | backend list with status, availability, and live tool counts |
| `gatemini://tools` | `application/json` | compact tool index (~3k tokens vs ~40k for full schemas) |
| `gatemini://recent` | `application/json` | last 50 recorded tool calls with tool name, backend, duration, and success/failure |
| `gatemini://stats` | `application/json` | context savings stats: bytes returned vs processed, savings ratio, per-tool breakdown |
| `gatemini://llms` | `text/plain` | machine-readable gateway reference: tool names, descriptions, naming rules (~3k tokens) |
| `gatemini://llms-full` | `text/plain` | complete gateway reference with full input schemas for every tool |
| `gatemini://call_tool_chain` | `text/plain` | execution contract, return semantics, and examples for sandboxed TypeScript tool calls |

### Resource templates

| URI template | Content |
|--------------|---------|
| `gatemini://tool/{tool_name}` | one full tool entry from the registry |
| `gatemini://backend/{backend_name}` | one backend with status, availability, tool count, and tool names |
| `gatemini://backend/{backend_name}/tools` | the tools for one backend |
| `gatemini://recent/{limit}` | the last `N` tool calls |

The resource layer also implements template completion for tool and backend names.

## Prompt surface

Prompts are implemented in `src/prompts.rs` using the rmcp builder pattern (`Prompt::new(...).with_title(...)`, `PromptArgument::new(...).with_required(true)`).

Available prompts:

| Prompt | Arguments | What it returns |
|--------|-----------|-----------------|
| `discover` | none | live discovery walkthrough using current registry counts |
| `find_tool` | `task` (required) | search results plus the top match schema and example call |
| `backend_status` | none | a markdown table with backend state, availability, tool count, and latency stats |

`backend_status` currently includes:

- backend name
- state
- availability
- tool count
- p50 latency
- p95 latency
- sample count

## Recent-call data

The resources and prompts use `CallTracker` data from `src/tracker.rs`.

What is tracked today:

- bounded recent call history
- per-tool usage counts
- per-backend HDR latency histograms
- per-session and per-tool byte tracking (bytes returned vs bytes processed by the output pipeline)

That is why `gatemini://recent`, `gatemini://stats`, and `backend_status` can return live operational data without a separate telemetry backend.

## Server instructions

The server also embeds discovery instructions in its MCP info block, implemented in `src/server.rs`.

That instruction text tells agents, in effect:

- search first
- inspect second
- execute through `call_tool_chain`
- avoid assuming backend tools are directly exposed MCP tools

## Protocol note

The code currently advertises protocol version `2025-06-18`.

For general MCP concepts, use the living spec home rather than an older dated deep link:

- <https://modelcontextprotocol.io/specification>
