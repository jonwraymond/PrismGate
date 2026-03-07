# Tool Discovery

Gatemini keeps the MCP surface intentionally small and pushes backend-tool detail behind discovery calls. The point is not to hide tools; it is to avoid sending every backend schema into the model context before the agent knows what it needs.

![Progressive discovery](diagrams/tool-discovery.svg){ .diagram-wide }

## Public discovery surface

The gateway exposes exactly 7 meta-tools:

| Tool | Default behavior |
|------|------------------|
| `search_tools` | brief search results for a natural-language task |
| `list_tools_meta` | paginated tool-name listing |
| `tool_info` | brief detail for one tool unless `detail="full"` |
| `get_required_keys_for_tool` | required env keys for the owning backend |
| `call_tool_chain` | execute JSON or TypeScript |
| `register_manual` | add a dynamic backend |
| `deregister_manual` | remove a dynamic backend |

The registry entries behind those tools are live data derived from configured backends and dynamic registrations.

## Search implementation

The registry implementation lives in `src/registry.rs`.

### BM25

Keyword search uses Okapi BM25 with:

- `k1 = 1.2`
- `b = 0.75`

Tool names get extra weight by duplicating the name tokens before description tokens are added.

### Optional semantic search

When the `semantic` cargo feature is enabled, Gatemini also builds model2vec embeddings from:

```text
{tool_name} {tool_description}
```

The default semantic model path is:

```text
minishlab/potion-base-8M
```

### Hybrid fusion

When both retrievers are available, the gateway fuses them with Reciprocal Rank Fusion. The registry fetches at least 30 candidates from each retriever before the final merge so small `limit` values do not starve the fusion step.

## Brief versus full

Two defaults are important for context hygiene:

- `search_tools` defaults to `brief=true`
- `tool_info` defaults to `detail="brief"`

Brief search results contain:

- tool name
- backend name
- first sentence of the description
- a generated call example

Brief tool info contains:

- tool name
- backend name
- first sentence of the description
- parameter names
- a generated call example

Full tool info returns the entire input schema for the tool.

## Registry rules

Tool registration has a few rules that matter when you debug discovery behavior:

- namespaced entries are the source of truth
- bare aliases are added only when a tool name is unique across backends
- if another backend later registers the same bare name, the alias is removed
- cached tools are restored under namespaced keys before the backend reconnects

## Resources and prompts

The discovery story is not just tools.

Resources:

- `gatemini://overview`
- `gatemini://backends`
- `gatemini://tools`
- `gatemini://recent`
- `gatemini://tool/{tool_name}`
- `gatemini://backend/{backend_name}`
- `gatemini://backend/{backend_name}/tools`
- `gatemini://recent/{limit}`

Prompts:

- `discover`
- `find_tool`
- `backend_status`

These are implemented in `src/resources.rs` and `src/prompts.rs`.

## Execution handoff

Discovery ends in one place: `call_tool_chain`.

That handler tries, in order:

1. direct JSON tool-call parsing
2. simple single-call TypeScript parsing
3. full sandbox execution

See [Sandbox](sandbox.md) for the execution details.
