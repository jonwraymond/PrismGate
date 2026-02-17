# Tool Discovery

Gatemini implements progressive disclosure to prevent tool definition bloat from consuming the AI agent's context window. Instead of exposing the full backend toolset directly, it exposes 7 meta-tools that let the agent discover, inspect, and execute backend tools on demand.

Note: the 258+ tool example below is from a typical production snapshot and will vary by deployment.

## The Problem

With larger backend sets (for example, 30+ backends, each exposing 5-20 tools), Gatemini commonly manages 258+ tools. Naive approaches fail at this scale:

| Approach | Token Cost | Impact |
|----------|-----------|--------|
| Expose all tools in `tools/list` | ~67,000 tokens | 33.7% of 200k context consumed before conversation starts |
| Single tool `tool_info` response | ~10,700 tokens | Largest tool (auggie's codebase-retrieval) |
| 10 full search results | ~5,000 tokens | Half a turn of context for one search |

Research confirms this is an industry-wide problem. Performance degrades after ~40 tools ([Demiliani, 2025](https://demiliani.com/2025/09/04/model-context-protocol-and-the-too-many-tools-problem/)), and 5-7 tools is the practical accuracy limit ([Jenova AI](https://www.jenova.ai/en/resources/mcp-tool-scalability-problem)). Gatemini exposes exactly 7 meta-tools, hitting the optimal range.

## The Solution: 4-Step Progressive Disclosure

![Progressive Disclosure Workflow](diagrams/discovery-flow.svg)

```
Step 1: search_tools(brief=true)     ~60 tokens/result    "What tools exist?"
Step 2: tool_info(detail="brief")    ~200 tokens           "Tell me more about this one"
Step 3: tool_info(detail="full")     ~200-10,700 tokens    "Give me the full schema"
Step 4: call_tool_chain              Execute                "Run it"
```

Each step reveals progressively more detail, and the agent can stop at any step. A typical discovery flow uses ~3,600 tokens instead of ~20,000 -- an **82% reduction**.

### Meta-Tools

| Tool | Purpose | Brief Mode |
|------|---------|------------|
| `search_tools` | BM25/hybrid search by task description | Default: brief=true (~60 tok/result) |
| `list_tools_meta` | Paginated list of all tool names | Names only (~3 tok/name) |
| `tool_info` | Full or brief schema for one tool | Default: detail="brief" (~200 tok) |
| `get_required_keys_for_tool` | Env var keys a backend needs | Key names only |
| `call_tool_chain` | Execute TypeScript calling backend tools | N/A |
| `register_manual` | Add a backend at runtime | N/A |
| `deregister_manual` | Remove a dynamic backend | N/A |

## BM25 Search

**Source**: [`src/registry.rs`](../src/registry.rs)

Gatemini implements BM25 (Okapi BM25) for keyword-based tool search with standard IR parameters:

| Parameter | Value | Purpose |
|-----------|-------|---------|
| k1 | 1.2 | Term frequency saturation -- prevents long descriptions from dominating |
| b | 0.75 | Document length normalization -- adjusts for varying description lengths |

### Tokenization

Text is split on non-alphanumeric characters and lowercased:

```
"get_current_time"  →  ["get", "current", "time"]
"streamable-http"   →  ["streamable", "http"]
"Search the WEB"    →  ["search", "the", "web"]
```

### Name Boost (2x)

Tool names receive a 2x weight by duplicating name tokens in the document representation:

```rust
let mut tokens = tokenize(&entry.name);    // ["web", "search"]
let name_tokens = tokens.clone();
tokens.extend(name_tokens);                // ["web", "search", "web", "search"]
tokens.extend(tokenize(&entry.description)); // + description tokens
```

This ensures exact name matches rank higher than description-only matches. A query for "web search" will rank `web_search` above a tool that merely mentions searching in its description.

### Scoring Formula

For each query term *t* in document *d*:

```
IDF(t) = ln((N - df(t) + 0.5) / (df(t) + 0.5) + 1)

TF_norm(t,d) = (tf(t,d) * (k1 + 1)) / (tf(t,d) + k1 * (1 - b + b * |d| / avgdl))

score(d) = sum(IDF(t) * TF_norm(t,d)) for all t in query
```

Results are sorted by score descending, then by name for stable ordering.

## Semantic Search

**Source**: [`src/embeddings.rs`](../src/embeddings.rs) (feature-gated: `semantic`)

When compiled with the `semantic` feature, Gatemini adds vector-based search using model2vec:

| Property | Value |
|----------|-------|
| Model | minishlab/potion-base-8M |
| Parameters | 8M (50x smaller than typical sentence transformers) |
| Speed | ~500x faster than full transformers on CPU |
| Normalization | L2-normalized (dot product = cosine similarity) |

### Embedding Text

Each tool is embedded as the concatenation of its name and description:

```
"{tool_name} {tool_description}"
```

### Search

Brute-force cosine similarity over all vectors. At a representative size of 258 tools, this takes ~5 microseconds -- fast enough that approximate nearest neighbor (ANN) indices like HNSW are unnecessary until ~10,000+ tools.

### When Semantic Helps

BM25 excels at exact term matching but fails on conceptual queries:

| Query | BM25 Result | Semantic Result |
|-------|-------------|-----------------|
| "web search" | `web_search` (exact match) | `web_search` |
| "find information online" | No match (no shared terms) | `web_search` (conceptual match) |
| "code analysis" | `codebase_retrieval` (partial) | `codebase_retrieval` + `code_search` |

## Hybrid RRF Fusion

**Source**: [`src/registry.rs`](../src/registry.rs) -- `search_hybrid()`

When both BM25 and semantic search are available, Gatemini combines them using Reciprocal Rank Fusion (RRF):

```
RRF_score(tool) = sum(1 / (K + rank_i)) for each retriever i
```

where K=60 is the [standard IR constant](https://learn.microsoft.com/en-us/azure/search/hybrid-search-ranking).

### Why RRF?

BM25 produces scores in the 0-15+ range. Cosine similarity produces 0-1. These scales are incomparable -- you can't just add them. RRF sidesteps this by converting both to rank-based scores:

| Tool | BM25 Score | BM25 Rank | Cosine | Semantic Rank | RRF Score |
|------|-----------|-----------|--------|---------------|-----------|
| web_search | 8.3 | 1 | 0.92 | 1 | 1/(61) + 1/(61) = 0.0328 |
| tavily_search | 6.1 | 2 | 0.85 | 3 | 1/(62) + 1/(63) = 0.0320 |
| find_similar | 2.4 | 3 | 0.88 | 2 | 1/(63) + 1/(62) = 0.0320 |

The fusion fetches at least 30 candidates from each retriever before ranking to ensure quality results.

## Brief vs Full Modes

### search_tools

**Brief** (default, ~60 tokens/result):
```json
{"name": "web_search_exa", "backend": "exa", "description": "Search the web."}
```

**Full** (~500 tokens/result):
```json
{"name": "web_search_exa", "backend": "exa", "description": "Search the web using Exa's neural search engine. Returns results with titles, URLs, and optional text content..."}
```

### tool_info

**Brief** (default, ~200 tokens):
```json
{"name": "web_search_exa", "backend": "exa", "description": "Search the web.", "parameters": ["query", "num_results", "type"]}
```

**Full** (~200-10,700 tokens depending on schema complexity):
```json
{"name": "web_search_exa", "backend": "exa", "description": "...", "input_schema": {"type": "object", "properties": {"query": {"type": "string", "description": "..."}, ...}}}
```

### First Sentence Extraction

**Source**: [`src/tools/discovery.rs`](../src/tools/discovery.rs) -- `first_sentence()`

Brief mode extracts the first sentence by finding:
1. First `. ` (period + space)
2. First `.\n` (period + newline)
3. Trailing `.` (entire text is one sentence)
4. Truncation at 200 chars with `...` (no sentence boundary found)

### Parameter Name Extraction

Brief `tool_info` returns parameter names instead of full JSON schemas:

```rust
fn extract_param_names(schema: &Value) -> Vec<String> {
    schema.get("properties")
        .and_then(|p| p.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}
```

This reduces a typical tool schema from ~500 tokens to ~20 tokens (just the parameter names).

## MCP Resources for Discovery

**Source**: [`src/resources.rs`](../src/resources.rs)

Gatemini also exposes tool information as MCP resources (loaded via `@` mentions in Claude Code):

| Resource | Tokens | Purpose |
|----------|--------|---------|
| `gatemini://tools` | ~3,000 | Compact index of ALL tools (vs ~40,000 for full schemas) |
| `gatemini://tool/{name}` | 200-10,000 | Full schema for one tool on demand |
| `gatemini://overview` | ~500 | Gateway guide with discovery workflow |

Resources use an even more aggressive 120-character truncation (vs 200 in discovery tools) for maximum compactness.

## MCP Prompts for Guided Discovery

**Source**: [`src/prompts.rs`](../src/prompts.rs)

| Prompt | Purpose |
|--------|---------|
| `discover` | 4-step walkthrough teaching the progressive disclosure pattern |
| `find_tool` | Search + display top 5 matches + full schema for #1 + TypeScript example |
| `backend_status` | Health dashboard showing all backends, status, and tool counts |

## Server Instructions

Gatemini embeds discovery instructions directly in its MCP `get_info()` response. This teaches AI agents the progressive disclosure workflow before they make their first tool call, without requiring external documentation.

## Sources

- [`src/registry.rs`](../src/registry.rs) -- BM25 and RRF hybrid search
- [`src/embeddings.rs`](../src/embeddings.rs) -- Semantic embedding index
- [`src/tools/discovery.rs`](../src/tools/discovery.rs) -- Brief/full mode handlers
- [`src/resources.rs`](../src/resources.rs) -- MCP resource system
- [`src/prompts.rs`](../src/prompts.rs) -- Guided workflow prompts
- [Okapi BM25 (Wikipedia)](https://en.wikipedia.org/wiki/Okapi_BM25) -- BM25 algorithm
- [Elastic BM25 guide](https://www.elastic.co/blog/practical-bm25-part-2-the-bm25-algorithm-and-its-variables) -- Parameter tuning
- [Azure RRF documentation](https://learn.microsoft.com/en-us/azure/search/hybrid-search-ranking) -- Reciprocal Rank Fusion
- [Model2Vec](https://github.com/MinishLab/model2vec) -- Static embeddings
- [Speakeasy 100x token reduction](https://www.speakeasy.com/blog/how-we-reduced-token-usage-by-100x-dynamic-toolsets-v2) -- Industry validation
- [Anthropic Advanced Tool Use](https://www.anthropic.com/engineering/advanced-tool-use) -- Tool Search Tool pattern
- [RAG-MCP paper (arXiv)](https://arxiv.org/abs/2505.03275) -- Academic validation of retrieval-first tool discovery
- [Microsoft Tool-space Interference](https://www.microsoft.com/en-us/research/blog/tool-space-interference-in-the-mcp-era-designing-for-agent-compatibility-at-scale/) -- Tool scaling research
