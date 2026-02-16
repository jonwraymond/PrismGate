# Progressive Tool Disclosure & Execution for Gatemini

**Status:** COMPLETE
**Created:** 2026-02-14
**Worktree:** No

## Problem Statement

Gatemini aggregates 30 backends with 258+ tools. When AI agents interact with it:
- `tool_info` responses can be 10.7k+ tokens (e.g., auggie's `codebase-retrieval`)
- `list_tools_meta` dumps 258 tool names in one response
- `search_tools` returns verbose results
- Tool definitions consume ~67k tokens (33.7% of 200k context) with multiple MCP servers active
- Agents get "⚠ Large MCP response (~10.7k tokens)" warnings that fill context fast

**Goal:** Implement progressive disclosure so agents discover tools incrementally, load schemas on-demand, and keep context lean.

## Research Summary (40 sources)

### Industry Patterns

| Pattern | Source | Key Idea |
|---------|--------|----------|
| **Bounded Context Packs (BCP)** | [SynapticLabs blog series](https://blog.synapticlabs.ai/bounded-context-packs-meta-tool-pattern) | 2-tool architecture: `getTools` + `useTools`. 33 tools → ~600 tokens at startup |
| **Progressive Disclosure SEP #1888** | [MCP GitHub](https://github.com/modelcontextprotocol/modelcontextprotocol/issues/1888) | Protocol-level `searchTools` with `mode: "operations"` / `mode: "types"` |
| **ProDisco** | [harche/ProDisco](https://github.com/harche/ProDisco) | Reference impl for Kubernetes: single meta-tool indexes TypeScript library APIs |
| **Tool Gating MCP** | [ajbmachon/tool-gating-mcp](https://github.com/ajbmachon/tool-gating-mcp) | Dynamic tool enable/disable. 100+ tools → only 2-3 active at a time |
| **Dynamic Tool Discovery** | [Speakeasy](https://www.speakeasy.com/mcp/tool-design/dynamic-tool-discovery) | `notifications/tools/list_changed` to add/remove tools at runtime |
| **Microsoft MCP Gateway** | [microsoft/mcp-gateway](https://github.com/microsoft/mcp-gateway) | Tool registration + dynamic routing in Kubernetes |
| **Glama MCPGateway** | [glama.ai](https://glama.ai/mcp/servers/@abdullah1854/MCPGateway) | Sandboxed JS execution to filter/transform results before they reach context |

### Client Support (Claude Code)

| Feature | Supported | Reference |
|---------|-----------|-----------|
| MCP Resources (`@` mentions) | Yes | [claude-code docs](https://code.claude.com/docs/en/mcp) |
| MCP Prompts (`/mcp__` commands) | Yes | [claude-code docs](https://code.claude.com/docs/en/mcp) |
| Resource Templates | Yes | Via `@` autocomplete |
| `notifications/tools/list_changed` | Yes | Dynamic tool updates |
| Resource Subscriptions | No | [Issue #7252](https://github.com/anthropics/claude-code/issues/7252) |
| Lazy-load tool definitions | Requested | [Issue #11364](https://github.com/anthropics/claude-code/issues/11364) (dup) |

### Context Window Impact Data

| Scenario | Token Cost | Source |
|----------|------------|--------|
| 7 MCP servers, all tools loaded | 67,300 tokens (33.7%) | [Issue #11364](https://github.com/anthropics/claude-code/issues/11364) |
| 3 MCP servers | 42,600 tokens | Same issue |
| 33 tools traditional | ~7,000 tokens at startup | [BCP Part 1](https://blog.synapticlabs.ai/bounded-context-packs-tool-bloat-tipping-point) |
| 33 tools with BCP meta-pattern | ~600 tokens at startup | [BCP Part 2](https://blog.synapticlabs.ai/bounded-context-packs-meta-tool-pattern) |
| Each additional tool schema | ~150 tokens | Same |
| `tool_info` for auggie | ~10,700 tokens | Measured |

### rmcp 0.15 SDK Support

| Feature | API | Status |
|---------|-----|--------|
| Resources | `ServerHandler::list_resources`, `read_resource` | Available ([docs.rs](https://docs.rs/rmcp)) |
| Resource Templates | `ServerHandler::list_resource_templates` | Available |
| Prompts | `ServerHandler::list_prompts`, `get_prompt` | Available |
| Completions | `ServerHandler::complete` | Available |
| Capabilities | `.enable_resources()`, `.enable_prompts()` | Available |
| Resources handler status | [Issue #337](https://github.com/modelcontextprotocol/rust-sdk/issues/337) | Open (basic API works) |

---

## Implementation Plan

### Phase 1: Tool Response Optimization (Quick Wins)

**Estimated token savings: 80-90% on individual tool responses**

#### Task 1.1: Add `brief` mode to `tool_info`
- [ ] Add `detail` field to `ToolInfoParams`: `"brief"` | `"full"` (default: `"brief"`)
- [ ] `brief` returns: `{ name, description (first sentence), backend, parameters: [names only] }` (~200 tokens)
- [ ] `full` returns: current behavior with complete schema (~10k tokens)
- [ ] Update tool description to mention brief/full modes

#### Task 1.2: Add `brief` mode to `search_tools`
- [ ] Add `brief` boolean to `SearchToolsParams` (default: `true`)
- [ ] `brief=true`: returns `{ name, backend, description (first sentence) }` per result
- [ ] `brief=false`: returns current full results with schemas

#### Task 1.3: Paginate `list_tools_meta`
- [ ] Add optional `cursor` and `page_size` (default: 50) to `list_tools_meta`
- [ ] Return `{ tools: [...], next_cursor: "..." }` format
- [ ] First page returns tools sorted by usage frequency (if tracked) or alphabetically

#### Task 1.4: Add annotations to large responses
- [ ] When any tool response exceeds 2000 chars, add MCP `annotations`:
  ```json
  { "priority": 0.3, "audience": ["assistant"] }
  ```
- [ ] Normal responses get `priority: 0.7`

### Phase 2: MCP Resources & Templates

**Estimated token savings: 95%+ for schema discovery (schemas stay out of context unless explicitly loaded)**

#### Task 2.1: Enable resources capability
- [ ] Update `ServerCapabilities` in `server.rs`:
  ```rust
  ServerCapabilities::builder()
      .enable_tools()
      .enable_resources()
      .enable_prompts()
      .build()
  ```

#### Task 2.2: Implement static resources
- [ ] `gatemini://overview` — Short (500 token) guide: what gatemini is, how to discover tools, when to use resources vs tools
- [ ] `gatemini://backends` — JSON list: `[{ name, tool_count, status, description }]` for all 30 backends
- [ ] `gatemini://tools` — Compact index: `[{ name, backend, one_line_desc }]` for all 258+ tools (~3k tokens vs ~40k for full schemas)

#### Task 2.3: Implement resource templates
- [ ] `gatemini://tool/{tool_name}` — Full schema + description for one tool (on-demand)
- [ ] `gatemini://backend/{backend_name}` — Backend details: name, type (stdio/http), tool count, list of tool names
- [ ] `gatemini://backend/{backend_name}/tools` — All tools for one backend with brief descriptions

#### Task 2.4: Implement `list_resources` handler
- [ ] Override `ServerHandler::list_resources` to return static resources
- [ ] Support pagination via cursor

#### Task 2.5: Implement `list_resource_templates` handler
- [ ] Override `ServerHandler::list_resource_templates` to return URI templates
- [ ] Include helpful descriptions for each template

#### Task 2.6: Implement `read_resource` handler
- [ ] Route `gatemini://overview` → static markdown text
- [ ] Route `gatemini://backends` → live backend status from `BackendManager`
- [ ] Route `gatemini://tools` → compact tool index from `ToolRegistry`
- [ ] Route `gatemini://tool/{name}` → full tool schema from registry
- [ ] Route `gatemini://backend/{name}` → backend details
- [ ] Route `gatemini://backend/{name}/tools` → tools for backend with brief desc
- [ ] Return `McpError` (-32002) for unknown URIs

#### Task 2.7: Implement completion for resource template arguments
- [ ] Override `ServerHandler::complete` for `ref/resource` references
- [ ] `gatemini://tool/{tool_name}` → autocomplete from tool names in registry
- [ ] `gatemini://backend/{backend_name}` → autocomplete from backend names

#### Task 2.8: Resource change notifications
- [ ] When backends come online/offline (via health checker), send `notifications/resources/list_changed`
- [ ] When tool registry changes (register/deregister), send notification

### Phase 3: MCP Prompts

**Enables guided workflows via `/mcp__gatemini__promptname` in Claude Code**

#### Task 3.1: Implement `discover` prompt
- [ ] Name: `discover`
- [ ] No arguments
- [ ] Returns messages guiding the agent through progressive discovery:
  1. Overview of gatemini (30 backends, 258+ tools)
  2. Instructions to use `@gatemini://backends` to see what's available
  3. Instructions to use `search_tools` with `brief=true` for finding tools
  4. Instructions to use `@gatemini://tool/{name}` for full schema only when needed
  5. Instructions to use `call_tool_chain` for execution

#### Task 3.2: Implement `find_tool` prompt
- [ ] Name: `find_tool`
- [ ] Argument: `task` (required) — description of what the agent needs
- [ ] Returns: embedded resource with search results + the top match's full schema
- [ ] Uses `search_tools` internally with brief mode

#### Task 3.3: Implement `backend_status` prompt
- [ ] Name: `backend_status`
- [ ] No arguments
- [ ] Returns: health/status of all backends with tool counts, latency stats if available

#### Task 3.4: Implement `list_prompts` and `get_prompt` handlers
- [ ] Override `ServerHandler::list_prompts`
- [ ] Override `ServerHandler::get_prompt`
- [ ] Support argument validation

### Phase 4: Dynamic Tool Visibility

**Reduces the 7 meta-tools to context-relevant subset**

#### Task 4.1: Smart tool description with capability index
- [ ] Update `search_tools` description to include a brief capability index:
  ```
  "Search 258 tools across 30 backends: web search (tavily, exa, zai),
   code intelligence (auggie, serena, octocode), browser automation (playwright,
   chrome-devtools), AI models (cerebras, pal, minimax), databases (supabase),
   file processing (repomix, firecrawl), docs (context7, deepwiki, ref)..."
  ```
- [ ] This lets the AI know what categories exist without loading any schemas

#### Task 4.2: `notifications/tools/list_changed` on backend events
- [ ] When a backend comes online → emit `notifications/tools/list_changed`
- [ ] When a backend goes offline → emit notification
- [ ] Client re-fetches `tools/list` and sees updated meta-tools
- [ ] Requires access to the `Peer` from `RequestContext` in server handlers

#### Task 4.3: Track tool usage frequency
- [ ] Add `call_count` to `ToolRegistry` entries
- [ ] Use frequency data to:
  - Sort `search_tools` results (more-used tools rank higher)
  - Sort `list_tools_meta` pages (hot tools first)
  - Include usage hint in resource annotations

### Phase 5: Response Optimization in `call_tool_chain`

**The sandbox is the highest-value optimization target since it executes arbitrary tool chains**

#### Task 5.1: Automatic response summarization
- [ ] When `call_tool_chain` output exceeds `max_output_size`, instead of hard truncation:
  - Truncate at limit with `\n...[truncated, use gatemini://tool/{name} for full schema]`
  - Add annotation with `priority: 0.3`

#### Task 5.2: Result caching in sandbox
- [ ] Cache frequently-requested tool results in the sandbox runtime
- [ ] Return cached results with `[cached]` annotation
- [ ] TTL configurable per backend

#### Task 5.3: Streaming-friendly response format
- [ ] For large responses, structure as: summary header (key findings) + details section
- [ ] Summary stays under 500 tokens even for massive results
- [ ] Details reference resource URIs for full content

---

## Token Budget Analysis

### Before (Current State)

| Action | Tokens | Notes |
|--------|--------|-------|
| gatemini tool definitions at startup | ~1,500 | 7 meta-tools |
| `list_tools_meta` | ~3,000 | 258 tool names |
| `tool_info` (auggie) | ~10,700 | Full schema + rules |
| `search_tools` (10 results) | ~5,000 | Full descriptions |
| `call_tool_chain` (typical) | ~2,000-15,000 | Varies widely |
| **Typical discovery flow** | **~20,000+** | List → search → info → execute |

### After (With Progressive Disclosure)

| Action | Tokens | Savings |
|--------|--------|---------|
| gatemini tool definitions at startup | ~1,500 | Same (meta-tools only) |
| `@gatemini://overview` | ~500 | Resource, on-demand |
| `@gatemini://backends` | ~800 | Resource, on-demand |
| `search_tools` brief (10 results) | ~600 | **88% reduction** |
| `tool_info` brief | ~200 | **98% reduction** |
| `@gatemini://tool/{name}` (when needed) | ~10,700 | On-demand only |
| **Typical discovery flow** | **~3,600** | **82% reduction** |

---

## File Changes

| File | Changes |
|------|---------|
| `src/server.rs` | Add resource/prompt handlers, update capabilities, brief modes |
| `src/tools/discovery.rs` | Add brief mode to search/info, pagination |
| `src/registry.rs` | Add usage tracking, compact index generation |
| `src/backend/mod.rs` | Expose backend status for resources |
| `src/backend/health.rs` | Emit notifications on state changes |
| `src/resources.rs` | **New** — Resource routing, URI parsing, template matching |
| `src/prompts.rs` | **New** — Prompt definitions and handlers |
| `src/sandbox/` | Response summarization, caching |
| `config/gatemini.yaml` | (No changes needed) |

## Dependencies

- rmcp 0.15 (`enable_resources()`, `enable_prompts()` confirmed available)
- rmcp `ServerHandler` trait: `list_resources`, `read_resource`, `list_resource_templates`, `list_prompts`, `get_prompt`, `complete`
- Note: rmcp [Issue #337](https://github.com/modelcontextprotocol/rust-sdk/issues/337) is open — may need workarounds for resource handler gaps

## Implementation Order

```
Phase 1 (quick wins) → Phase 2.1-2.3 (resources) → Phase 3.1-3.2 (prompts)
→ Phase 2.4-2.8 (handlers) → Phase 3.3-3.4 (handlers) → Phase 4 → Phase 5
```

Phase 1 can ship independently for immediate token savings. Phases 2-3 are the core progressive disclosure. Phases 4-5 are optimizations.

## Success Metrics

- [ ] Typical discovery flow tokens: 20k → <4k (80% reduction)
- [ ] `tool_info` brief response: <300 tokens
- [ ] `search_tools` brief response: <100 tokens per result
- [ ] Resources accessible via `@gatemini://` in Claude Code
- [ ] Prompts accessible via `/mcp__gatemini__` in Claude Code
- [ ] No "⚠ Large MCP response" warnings during normal discovery
- [ ] All 258 tools still fully accessible when needed (no functionality loss)
