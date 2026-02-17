# Resources & Prompts

Gatemini uses all three MCP primitives -- tools, resources, and prompts -- to provide multiple discovery pathways for AI agents.
Tool counts in this document use a representative registry snapshot and vary by deployment.

## MCP Primitive Roles

| Primitive | Control | Gatemini Use |
|-----------|---------|---------------|
| **Tools** | Model-controlled (AI decides when to call) | 7 meta-tools for search, inspect, execute |
| **Resources** | Application-controlled (client loads via @-mention) | Compact indices, on-demand schemas |
| **Prompts** | User-controlled (slash commands, menus) | Guided discovery workflows |

This separation follows the [MCP specification](https://modelcontextprotocol.io/specification/2025-11-25): tools are for actions the AI initiates, resources are for data the application provides, and prompts are for user-initiated workflows.

## Resources

**Source**: [`src/resources.rs`](../src/resources.rs)

### Static Resources

| URI | Tokens | Content |
|-----|--------|---------|
| `gatemini://overview` | ~500 | Gateway guide: how to use meta-tools, discovery workflow, token savings tips |
| `gatemini://backends` | Variable | JSON list of all backends with status and tool counts |
| `gatemini://tools` | ~3,000 | Compact index of ALL tools (name, backend, first sentence) |

The `gatemini://tools` resource is the flagship optimization: loading it provides awareness of all 258+ tools in a representative catalog for ~3,000 tokens versus ~40,000 tokens for full schemas -- a **92.5% reduction**.

### Resource Templates

| URI Pattern | Content | On-Demand Cost |
|-------------|---------|----------------|
| `gatemini://tool/{tool_name}` | Full schema for one tool | 200-10,000 tokens |
| `gatemini://backend/{backend_name}` | Backend details, status, tool count | ~200 tokens |
| `gatemini://backend/{backend_name}/tools` | Brief tool list for one backend | Variable |

Templates enable targeted lookups: instead of loading all tool schemas at once, an agent can load the compact index and then request full schemas only for tools it intends to use.

### First Sentence Truncation

Resources use a more aggressive 120-character truncation than discovery tools (200 characters) for maximum compactness:

```rust
// In resources.rs: 120 char limit
fn first_sentence(text: &str) -> String { ... }

// In discovery.rs: 200 char limit
fn first_sentence(text: &str) -> String { ... }
```

### Resource Completions

Gatemini provides autocomplete for resource URI parameters:

```
gatemini://tool/web_     → ["web_search_exa", "web_search_tavily", ...]
gatemini://backend/e     → ["exa"]
```

This enables IDE-like completion when constructing resource URIs.

## Prompts

**Source**: [`src/prompts.rs`](../src/prompts.rs)

### discover

A 4-step guided walkthrough teaching the progressive disclosure workflow:

```
Step 1: View available backends
  → Load @gatemini://backends

Step 2: Search for tools
  → search_tools("your task", brief=true) — ~60 tokens/result

Step 3: Get full schema when ready to execute
  → tool_info("tool_name", detail="full") — or load @gatemini://tool/{name}

Step 4: Execute
  → call_tool_chain('backend.tool_name({args})')
```

The prompt explicitly documents token savings: brief mode saves 80-98% tokens during discovery.

### find_tool

Takes a task description as input and automates the search workflow:

1. Performs BM25 search for top 5 matches
2. Displays results in a markdown table (name, backend, brief description)
3. Includes the full schema for the top match
4. Provides an executable TypeScript example:

```typescript
const r = await backend.tool_name({param: "value"});
return r;
```

### backend_status

Generates a health dashboard showing all backends:

```markdown
| Backend | Status | Tools | Transport |
|---------|--------|-------|-----------|
| exa | Healthy | 3 | stdio |
| tavily | Healthy | 1 | stdio |
| custom | Starting | 0 | http |
```

## Server Instructions

**Source**: [`src/server.rs`](../src/server.rs)

Gatemini embeds discovery instructions directly in its MCP `get_info()` response. These instructions are delivered to the AI agent before it makes any tool calls:

```
gatemini is an MCP gateway that aggregates tools from multiple backend MCP servers.
Use search_tools to find tools, tool_info for details, and call_tool_chain to execute
TypeScript code that calls backend tools.

## Discovery Workflow (use progressive disclosure to save context)
1. search_tools("your task") → brief results by default (~60 tokens/result)
2. tool_info("name") → brief: name, backend, description, param names (~200 tokens)
3. tool_info("name", detail="full") → complete schema, ONLY when ready to call
4. call_tool_chain("code") → execute TypeScript
```

This approach teaches agents the progressive disclosure pattern without requiring external documentation, and it works across all MCP clients (Claude Code, Cursor, etc.).

## When to Use Each Primitive

### Use Resources When

- Loading context at conversation start (e.g., `@gatemini://tools` for tool awareness)
- Agent needs static data that doesn't require a decision (backend list, tool schemas)
- Client supports @-mention syntax

### Use Tools When

- Agent is searching for relevant tools (`search_tools`)
- Agent needs to inspect a specific tool (`tool_info`)
- Agent is executing an action (`call_tool_chain`)

### Use Prompts When

- User wants a guided walkthrough (`/discover`)
- User wants to find a tool interactively (`/find_tool`)
- User wants to check backend health (`/backend_status`)

## Sources

- [`src/resources.rs`](../src/resources.rs) -- Resource implementation
- [`src/prompts.rs`](../src/prompts.rs) -- Prompt implementations
- [`src/server.rs`](../src/server.rs) -- Server instructions and MCP integration
- [MCP Specification (2025-11-25)](https://modelcontextprotocol.io/specification/2025-11-25) -- Official protocol spec
- [MCP Resources](https://modelcontextprotocol.info/docs/concepts/resources/) -- Resource concept guide
- [MCP Features Guide (WorkOS)](https://workos.com/blog/mcp-features-guide) -- Comprehensive feature overview
- [Laurent Kubaski stdio transport](https://medium.com/@laurentkubaski/understanding-mcp-stdio-transport-protocol-ae3d5daf64db) -- Transport protocol
- [CodeSignal MCP Primitives](https://codesignal.com/learn/courses/developing-and-integrating-a-mcp-server-in-python/lessons/exploring-and-exposing-mcp-server-capabilities-tools-resources-and-prompts) -- Primitive roles
