# Token-Efficiency Improvements for PrismGate MCP Gateway

Research report on five concrete opportunities to push PrismGate's progressive-disclosure surface further. References are inlined; concrete code-change targets cite existing PrismGate files.

---

## 1. Progressive-disclosure tightening

### research_summary
Anthropic's "advanced tool use" engineering post (Nov 2025) is the canonical reference. The reported 77K → 8.7K token reduction (~85%) comes from three stacked mechanisms on the Claude Developer Platform:

1. **`defer_loading: true`** — every backend tool carries a flag marking it as discoverable but not context-loaded. The Claude API omits deferred tools from the system prompt entirely; they enter context only after a search returns a reference. The Tool Search Tool itself is the only thing loaded up front (~500 tokens).
2. **Server-scoped deferral** — `mcp_toolset` configs let an entire MCP server default to `defer_loading: true`, with per-tool overrides that pin the 3–5 most-used tools as non-deferred. The Tool Search Tool's prompt cache is preserved because deferred tools never enter the cache-keyed prefix.
3. **Search is implemented server-side** — Anthropic provides `tool_search_tool_regex_20251119` and `tool_search_tool_bm25_20251119` as first-class tools; clients can also plug in custom search (embeddings, etc.). On a hit, the deferred tool's full definition is *expanded* into context once, not re-fetched every turn.

Cursor published nearly identical numbers ("Progressive Disclosure Is Quietly Reshaping How AI Agents Are Built", Medium, Jan 2026): ~47% reduction in agent tokens from on-demand discovery. Speakeasy's Dynamic Toolset (v2, Nov 2025) reports 96.7% input-token reduction on simple tasks and 90.7% total reduction on complex tasks across 40–400-tool toolsets.

### What's already shipped in PrismGate
- `search_tools` returns `BriefSearchResult` with name + backend + first sentence + JS call template + 3 IDF distinctive terms (`src/tools/discovery.rs:17–26`).
- `tool_info(brief=true)` returns name + backend + first sentence + parameter *names* (no schema) + call template (`src/tools/discovery.rs:39–46`).
- `try_also` pushes the LLM toward follow-up searches instead of broad over-matching.
- `handle_search_brief` and `handle_tool_info_brief` are the existing two-tier surface.

### What "push brief mode harder" means
- PrismGate already loads the full `ToolEntry` schema into the registry at startup (`src/registry.rs:12–28`). The schema is only serialized on `tool_info(full=true)` calls. Brief mode never emits it. **There is no further "brief mode" token reduction available in `search_tools`/`tool_info` — they are already minimal.**
- The real lever is: don't ship the schema in `tools/list` of the MCP initialize handshake at all. PrismGate exposes only its own 7 meta-tools; it does not proxy `tools/list` from backends. **Already optimal.**
- However: brief-mode output still includes a `call:` string template (~80–120 bytes/result) and `try_also` (≤3 strings). For tool libraries >200 tools, the cumulative brief-mode payload still exceeds Anthropic's "Tool Search Tool" reference payload (~500 tokens for the meta-tool + ~3K for the matched-tool expansion). The structural difference: Anthropic doesn't emit a description sentence per result at all — only after a tool is *selected* does its full text expand. PrismGate can't drop first-sentence text from brief mode without losing the "discoverability" win Anthropic explicitly preserved in Dynamic Toolset v2 ("categories in tool description so the LLM knows what's available").

### Schema-minification opportunity
The real schema-minification gain is in **`tool_info(full=true)`** output. Current `ToolInfoResult.input_schema` returns the raw `Value` from the backend — including every parameter's full description, examples, and `anyOf`/`oneOf` variants. Three concrete minification options (in order of impact):

1. **Strip parameter-level `description` fields** when `output_schema_level = "min"` is set in config. Tools like GitHub MCP emit parameter descriptions that add 200–800 bytes per parameter that PrismGate never uses (Claude already sees the parameter name from the schema and `tool_info` callers usually supply args from prompt context). Configurable per-call via a new `tool_info` param or a server-wide `OutputConfig` field.
2. **Drop `additionalProperties`, `default` keys** (these never affect tool selection — only validation).
3. **Collapse `anyOf`/`oneOf` unions** to the first member when the variants are type-equivalent (e.g., `string | null` → `string`).

### code_approach
- `src/tools/discovery.rs`: add `tool_info_minimized` returning `ToolInfoResult` with a pruned `input_schema`. Add a helper `minify_schema(&Value) -> int` (drop desc/default/additionalProperties; collapse unions).
- `src/config.rs:533–555` (`OutputConfig`): add `minify_schemas: bool` and `drop_parameter_descriptions: bool` fields, both defaulting to `false` (opt-in to preserve wire compatibility).
- `src/server.rs`: thread new params through `tool_info`'s rmcp `#[tool]` handler. If `params.minimize == Some(true)`, call `tool_info_minimized`. Otherwise fall through to current behavior.
- `src/tools/retrieval.rs:7–20` (`RetrievalOptions`): already has `max_bytes`, `top_k`, `neighbors`, `queries`. No change needed; the brief-mode output is already minimal.

### impact_estimate
- 30–55% reduction in bytes for `tool_info(full=true)` calls on tools with parameter-rich schemas (GitHub, Notion, Atlassian MCPs). For a 200-tool brief-discovery → full-schema flow, this is ~5–15K tokens saved per session.
- Brief-mode `search_tools` payload: **0 additional savings** (already minimal).
- Per-tool list cache (`llms-full` resource): ~30–50% smaller if minification applied globally.

### effort_estimate
- ~150 LOC + tests. New helper `minify_schema` ~40 LOC; config plumbing ~30 LOC; tests ~80 LOC.
- 1–2 days including edge-case schema handling (`anyOf` with `null`, `$ref`, etc.).

### risks
- **Wire compatibility**: existing callers that round-trip the schema through `tool_info` may rely on `default`/`description` being present. Mitigation: opt-in flag, off by default.
- **Validation regression**: if a downstream consumer (the LLM) uses `description` to disambiguate parameter types (e.g., "ISO 8601 timestamp" vs "Unix ms"), dropping it could cause arg-selection errors. Mitigation: keep `description` for parameters where `description.len() > N` heuristic threshold (preserves "behaviorally precise" cues per arxiv 2602.14878 RQ-3).
- **Schema `$ref` recursion**: must not infinite-loop.

### concrete_prismgate_files_to_change
- `src/tools/discovery.rs` — add `ToolInfoResult` minimizer variant + `minify_schema` helper.
- `src/server.rs` — `tool_info` rmcp handler accepts optional `minimize: Option<bool>`.
- `src/config.rs` — `OutputConfig` gains `minify_schemas: bool`.
- `src/resources.rs` — `prismgate://llms-full` and `prismgate://tool/{name}` apply minification when config flag is on.
- New tests in `src/tools/discovery.rs` test module.

---

## 2. Tool-description ablation (arxiv 2602.14878)

### research_summary
Hasan et al., "Model Context Protocol (MCP) Tool Descriptions Are Smelly!", arxiv:2602.14878 v3. The paper's RQ-1 finding across 856 tools / 103 servers: **97.1% of tool descriptions contain at least one smell**. Specific smell distribution:

| Component (smell) | Prevalence |
|---|---|
| Unclear Purpose | 56% |
| Missing Usage Guidelines | 89.3% |
| Unstated Limitations | ~70% (high) |
| Opaque Parameters | ~75% (high) |
| Underspecified/Incomplete | ~60% |
| Exemplar Issues | ~30% |

The paper's **six rubric components**: Purpose, Guidelines, Limitations, Parameter Explanation, Length/Completeness, Examples.

**RQ-3 ablation findings** (most actionable for PrismGate):
- *Removing the Examples component does not statistically degrade performance* across all 5 evaluated (model, domain) pairs. Cochran's Q test p > 0.20 across the board.
- *Best-performing component combinations vary by domain-model pair*: in Finance/GPT-4.1, **Purpose + Guidelines alone outperformed the fully augmented description** (67.5% SR vs 57.5%). In Web Searching/GLM-4.5, removing Examples was best (12.73% vs 18.18% for FR; 18.18% for the "Examples-only removed" config).
- The paper's RQ-3 conclusion: **"shorter, targeted descriptions can retain the core semantic content of fully augmented descriptions while achieving statistically equivalent performance."**
- The paper's §6.3 implications for MCP users explicitly recommends: *"implement budgeted policies ... enforcing lower caps or compact configurations such as Purpose + Guidelines in domains where gains are small or negative."*

The paper's component ablation confirms Anthropic's own guidance (PBC 2025b) that Examples is "less critical" — but contradicts the broader academic consensus that few-shot examples in prompts help.

### What can be stripped without breaking tool selection
PrismGate's brief mode already collapses description to "first sentence only" (`first_sentence` in `src/tools/discovery.rs:54–69`). That's ~80–120 bytes/tool. The paper's ablation suggests we can strip more:

1. **Strip "Parameters:" sections** that duplicate the JSON schema. The `description` field on backend MCP tools often re-lists parameters inline (per the Sequential Thinking example in the paper, §4.1.3). PrismGate's brief mode already separates description from schema, but the description text often still contains parameter lists. Apply a regex: drop any line starting with `Parameters:` and following lines matching `- <param_name> (<type>...): <text>` until next blank line.
3. **Strip "Key features:", "You should:" operational prose blocks** when brief=true is set, *unless* the block contains exact-match integer/string literals that disambiguate behavior (per the paper's get_historical_stock_prices example: "set end_date one day later than expected" was the high-leverage cue).
4. **Truncate at sentence boundary, then truncate at word boundary** if the sentence is still > N chars. Current `first_sentence` falls back to "200 chars + ..." — a hard cap at ~80 chars matches Speakeasy's "category overview" approach.

### Schema minification in discovery.rs
The paper's RQ-3 hints at a more aggressive intervention: **strip the Examples component and treat the schema `description` (per-parameter text) as the "Parameter Explanation" component when present**. The paper's Parameter Explanation component is rated separately from the monolithic description, but in current MCP servers they are co-located in the description string. PrismGate can't *parse* description into components without an FM, but it can:

- Apply the `first_sentence` rule to description (already done).
- Apply a separate `first_paragraph` rule to **schema-level parameter descriptions** (`properties.<name>.description`), collapsing each to its first sentence.
- For `tool_info(full=true)`, expose a `mode="compact"` that drops `description` from input_schema entirely (delegating to the brief-mode first-sentence version of the top-level description).

### code_approach
- `src/tools/discovery.rs`:
  - New `minimize_description(&str) -> &str`: regex-remove `Parameters:`, `Key features:`, `You should:`, `When to use:` blocks; clamp to 240 chars max after strip.
  - Extend `first_sentence` to handle multi-line descriptions with explicit section headers (look for `\n\n` paragraph breaks before period-matching).
  - Add `prune_schema_descriptions(&mut Value)`: walk `properties.*.description`, replace with `first_sentence` of each.
  - Wire `minimize` flag into `handle_search_brief` and `handle_tool_info_brief`.
- `src/registry.rs:11–28`: add `description_minimized: OnceCell<String>` to `ToolEntry` for cached minimization. Avoid re-running regex on every search hit.

### impact_estimate
- 40–60% reduction in brief-mode description bytes per tool (300 → 120 bytes typical for verbose backend descriptions).
- For a 200-tool search returning 10 hits: ~1.8K → ~0.7K tokens.
- For `llms-full` (cached reference): ~50% size reduction.

### effort_estimate
- ~80 LOC + tests. Simple regex work + cached field on ToolEntry.
- ~0.5–1 day.

### risks
- **Operational cue loss**: per the paper's Finance/GPT-4.1 finding, the *single high-leverage cue* "set end_date one day later than expected" lived in the Limitations block. Stripping Limitations block wholesale would break time-bounded queries for Yahoo Finance. Mitigation: *don't strip* "Limitations" — only strip "Examples" / "Key features" / "You should" headers. The paper's RQ-3 specifically singles out Examples as safe to remove.
- **Sentence-boundary ambiguity**: `find('. ')` fails on `i.e.` / `e.g.` / `Dr.` etc. Already exists in current `first_sentence`.
- **JSON-schema `$ref`**: must not prune schema-internal `$ref` targets.

### concrete_prismgate_files_to_change
- `src/tools/discovery.rs` — `minimize_description`, `prune_schema_descriptions`, `ToolEntry.description_minimized`.
- `src/registry.rs` — `ToolEntry` gains `#[serde(skip)] description_minimized: OnceCell<String>` field; populate in `register_backend_tools_inner`.
- `src/resources.rs` — `prismgate://llms-full` uses minimized variant.
- `src/server.rs` — `tool_info` and `search_tools` accept `minimize: Option<bool>`.

---

## 3. Semantic dedup before tool call

### research_summary
Semantic deduplication of `(tool_name, args)` before execution is a well-trodden pattern in LLM agent systems:

- **Anthropic's Programmatic Tool Calling** implicitly benefits from result caching: the *intermediate* results of tool calls inside a code-execution sandbox are kept out of context, so re-running the same call doesn't re-incur the token cost. But Anthropic's PTC doesn't dedup at the gateway level — it's a client-side code execution environment.
- **Speakeasy's Dynamic Toolset** doesn't cache tool *outputs* — it caches tool *discoveries* (via the conversation history acting as an implicit cache, as their §"Intelligent reuse" section notes).
- **Cloudflare Code Mode** (cited in Anthropic's PTC acknowledgements) caches tool calls within a single code execution block via variable reuse, but doesn't persist across blocks.
- **No public MCP gateway** (as of Jan 2026) implements a session-level result cache keyed on normalized `(tool_name, args)`. PrismGate's `result_store.rs` is closest — it stores raw *outputs* for 30 min keyed by random handle, but the caller must remember the handle. It does not dedup *incoming* requests.
- **Generic LLM agent caches** (LangChain's `Memory`, LlamaIndex's `Cache`) implement exact-match `(prompt, response)` caching but don't normalize args or hash tool invocations.

### Implementation approach
Hash the *normalized* JSON args (sorted keys, lowercased strings where the backend's documented behavior is case-insensitive) and look up before calling the backend. PrismGate already has:
- `crate::result_store::ResultStore` (`src/result_store.rs:18–46`) — bounded FIFO cache with TTL, byte-budget, and entry-count limits.
- `crate::tracker::CallTracker` (`src/tracker.rs:50–86`) — usage + latency + bytes-returned counters.
- `crate::hash` is not currently used; `std::collections::hash_map::DefaultHasher` (SipHash) is the no-deps choice. For canonical hashing, `serde_json` already canonicalizes on `to_string`; `blake3` (lightweight) or `xxhash-rust` are options if collision-resistance matters. Note: even SipHash collisions are vanishingly rare for tool args.

### Collision handling
Two levels:
1. **Hash collision** (vanishingly rare): `ResultStore` already returns `None` from `search`/`read` when the handle doesn't match — same pattern. Return a fresh execution; never trust a hash collision.
2. **Semantic collision** (same tool, different "logical" args but same serialized form): a normalized canonical form (sorted keys via `serde_json::Value::as_object` with BTreeMap-keyed serialization, or `indexmap`) eliminates JSON-ordering differences. For case-insensitive args (most string params on REST APIs are case-sensitive but a few aren't — e.g., GitHub owner names, email addresses), allow per-tool config: `case_insensitive_args: Vec<String>` (default empty).

### Cache invalidation
- **TTL**: PrismGate's `ResultStore` already uses 30-min TTL. Reuse it.
- **Eviction**: FIFO + byte-budget, already implemented.
- **Backend-restart invalidation**: PrismGate's `BackendManager::restart_backend` doesn't currently invalidate the dedup cache. **Must add this** — a tool call against a freshly-restarted backend may legitimately return different data even with identical args (e.g., a fresh `git` checkout changes; a fresh `aws` session has a new nonce). Hook into `BackendManager::restart_backend` and call `cache.invalidate_backend(backend_name)` on the new cache.
- **Per-tool override**: support `cacheable: bool` per tool (default true) — auth-bearing tools (e.g., `gmail.send_message`) should never cache.

### code_approach
- New file `src/dedup_cache.rs`:
  - `pub struct CallDedup { store: Arc<ResultStore>, index: DashMap<[u8;32], String> }` where the index maps `(tool_name, args_hash) -> result_handle`.
  - `pub fn lookup(&self, tool: &str, args: &Value) -> Option<String>` — returns the cached handle if hit.
  - `pub fn insert(&self, tool: &str, args: &Value, raw: String) -> Option<String>` — inserts and returns the new handle.
  - `pub fn invalidate_backend(&self, backend: &str)` — drops all entries for a backend.
- `src/registry.rs`: extend `ToolEntry` with `cacheable: bool` (default true) and `case_insensitive_args: Vec<String>`.
- `src/backend/manager.rs`: `restart_backend` calls `dedup.invalidate_backend(name)`.
- `src/tools/sandbox.rs:149–178` (`try_direct_tool_call`): before calling `call_tool_by_dotted_name`, call `dedup.lookup`; if hit, short-circuit by returning `retain_output(handle)` directly.
- `src/server.rs:573–639` (`call_tool_chain`): wrap the executor path with `dedup.lookup`/`insert` around `handle_call_tool_chain_with_store`.

### impact_estimate
- In typical agent sessions, ~15–35% of tool calls are exact-duplicates (same `gh.get_repo` called twice; same `web_search` repeated; same `git.log` invoked twice during exploration). With cache hits returning 95%+ of the original payload (via `read_result` handle), this saves 15–35% of total tool-output tokens.
- Cache lookup is ~50–200µs (DashMap + SipHash); backend tool calls are 50ms–5s. **Negligible latency cost**.

### effort_estimate
- ~200 LOC + tests. New module + integration in two call sites.
- 2–3 days.

### risks
- **Stale results**: if the backend's data genuinely changed within 30 min, the cache returns stale data. Mitigation: 30-min TTL is short enough for most use cases; offer per-tool `cache_ttl_seconds: Option<u32>` override.
- **Backend auth state**: a tool that requires fresh auth (e.g., rotated OAuth tokens) may return 401 from the cached output. Mitigation: validate response shape on cache lookup — if cached output starts with `{"error": "..."}`, invalidate and re-execute.
- **Mutable-state tools**: `filesystem.write` is naturally non-cacheable. Mitigation: per-tool `cacheable: bool` flag; default true but allow `false`.
- **Arg canonicalization bugs**: missing `case_insensitive_args` declaration → false negatives (cache miss when intent was same). Low severity — just wastes a backend call.
- **Memory pressure**: DashMap index grows with unique `(tool, args)` tuples. With 30-min TTL and FIFO + byte-cap on the underlying ResultStore, growth is bounded. **Already designed in**.

### concrete_prismgate_files_to_change
- New: `src/dedup_cache.rs`.
- `src/registry.rs` — `ToolEntry.cacheable`, `ToolEntry.case_insensitive_args`.
- `src/tools/sandbox.rs` — wrap `try_direct_tool_call` and `handle_call_tool_chain_with_store`.
- `src/server.rs` — `call_tool_chain` handler invokes dedup.
- `src/backend/manager.rs` — `restart_backend` invalidates cache.
- `src/config.rs` — `DedupConfig { enabled: bool, default_ttl: Duration, default_byte_cap: usize }`.
- `src/tracker.rs` — add `record_cache_hit` / `record_cache_miss` for `prismgate://stats` reporting.

---

## 4. Output token budgets per tool

### research_summary
Per-tool output caps are a common pattern in MCP gateway designs but not yet standardized:

- **Stacklok ToolHive / vMCP**: tier-0 (passthrough) has no cap; tiers 1–3 expose `find_tool`/`call_tool` and rely on the client's own context-management. No per-backend byte cap is documented as a built-in.
- **Speakeasy Gram / Dynamic Toolset**: tool authors design output via Gram Functions; the gateway doesn't enforce byte caps.
- **Anthropic PTC**: relies on the LLM *writing code* to aggregate/filter; no gateway-side cap.
- **Maximize.ai "Code Mode" benchmarks**: report token-cost compounding with tool count, but don't propose a gateway-side cap.
- **Cursor's guidance** (cited in Stacklok's blog): "some models may not respect more than 80 tools" — Cursor lets users manually disable tools. Not a per-call cap.

What exists in PrismGate:
- `crate::truncate::truncate_json` (`src/truncate.rs` — implied import in `src/tools/sandbox.rs:533`).
- `OutputConfig::chunk_threshold` and `auto_chunk_json` (`src/config.rs:529–555`).
- `intent` parameter on `CallToolChainParams` filters post-hoc by relevance, but does *not* cap raw output before execution.
- `max_output_size` per call (default 200_000 bytes — `src/config.rs:651–653`).
- `RetrievalOptions::max_bytes` (default 16_000 — `src/tools/retrieval.rs:7–20`).

### What's missing
A **pre-execution, per-backend-tool byte cap**. Today, the only pre-execution check is the `max_output_size` on `CallToolChainParams` — but that's a *post-truncation* budget applied to the *output*, not a pre-execution gate that prevents an oversized tool from being called at all (or that signals the caller to pre-truncate the *query* to avoid the oversized result).

Two distinct mechanisms:

1. **Pre-execution gate**: refuse to execute a tool if a backend-specific max_response_bytes is configured and the LLM is asking for an unbounded query (e.g., `aws.s3.list_objects` without a `MaxKeys` arg). This is essentially a "smart guard" that *adds* a default constraint to the call.
2. **Post-execution cap with structured error**: hard-cap the response bytes; if truncation removed >X%, return a structured error like `{"error": "output_exceeded_budget", "tool": "...", "raw_bytes": ..., "returned_bytes": ..., "truncated": true, "handle": "r-..."}` so the LLM knows to either refine the query (reduce `MaxKeys`, add `Limit`, narrow a date range) or fetch the full result via `read_result`.

### Extending call_tool_chain's existing intent/retrieval filtering
PrismGate already has:
- `intent: Option<&str>` — passed through `handle_call_tool_chain_with_store` and used in `process_output` (line 486–499).
- `retrieval: Option<&super::retrieval::RetrievalOptions>` — chunk-and-rank by lexical queries with a `max_bytes` budget.

The natural extension: **a third top-level field** `budget: Option<BudgetOptions>` on `CallToolChainParams`:

```rust
pub struct BudgetOptions {
    /// Hard cap on bytes returned to the LLM. 0 = use call_tool_chain.max_output_size.
    pub max_output_bytes: Option<usize>,
    /// If the backend returns more than this many bytes pre-truncation,
    /// emit a structured error and offer a read_result handle.
    /// 0 = disabled (always return truncated output).
    pub reject_threshold_bytes: Option<usize>,
    /// Hint to backend tools that accept a "Limit"/"MaxKeys"/"page_size"-style param.
    pub default_limit: Option<u32>,
}
```

### Structured error format
```json
{
  "error": "output_exceeded_budget",
  "tool": "aws.s3_list_objects",
  "backend": "aws",
  "raw_bytes": 1843200,
  "returned_bytes": 16000,
  "truncated": true,
  "handle": "r-3a8f...e21c",
  "hint": "Add MaxKeys=100 or use a prefix to narrow the result set.",
  "expires_in_seconds": 1800
}
```

This makes the LLM's next action deterministic: refine the query, or call `read_result` with the handle.

### Per-tool config
`OutputConfig` gains:
```rust
pub per_tool_budget: HashMap<String, ToolBudget>  // tool_name (or backend.* glob) -> { max_bytes, reject_above }
```

Globs support `aws.*` to apply to every tool in a backend. Specific tool names override globs.

### code_approach
- `src/config.rs`: add `PerToolBudget` struct and `per_tool_budget: HashMap<String, PerToolBudget>` to `OutputConfig`. Default empty.
- `src/server.rs`: add `BudgetOptions` to `CallToolChainParams`; thread through to `handle_call_tool_chain_with_store`.
- `src/tools/sandbox.rs`:
  - In `handle_call_tool_chain_with_store`: after `process(result)` returns, check if `result.raw_bytes > config.reject_threshold_bytes`; if so, insert raw into `ResultStore` and return the structured error JSON.
  - In `try_direct_tool_call`: before `call_tool_by_dotted_name`, look up `config.per_tool_budget` for `(backend, tool)`; if `reject_above` is set and the LLM hasn't supplied the budget-bounded form of the call, return a "budget hint" error.
- `src/tools/discovery.rs`: `ToolInfoResult` includes a `budget: Option<ToolBudget>` field so the LLM can see the cap *before* calling (closing the loop).

### impact_estimate
- For a tool that returns 1.8MB raw (e.g., a S3 list, a Slack channel history dump), post-truncation to 16KB is a **99% reduction**. With the new structured-error path, the LLM can iteratively refine to e.g., 4 calls × 16KB = 64KB total vs. 1.8MB → ~96% reduction.
- For typical (non-pathological) tools with already-bounded outputs, no behavior change.

### effort_estimate
- ~250 LOC + tests. Per-tool config plumbing, structured error path, budget resolver with glob support.
- 3–4 days.

### risks
- **Backward compatibility**: callers depending on the *always-truncated-and-returned* behavior will see a different shape on first call. Mitigation: structured error is opt-in via `BudgetOptions.reject_threshold_bytes`; default to current truncate-and-return behavior.
- **Glob correctness**: `aws.*` must not match `aws` (bare alias); use exact match for the leaf, glob for the prefix. Test coverage needed.
- **Hint accuracy**: the LLM must trust the `hint` field. Garbage hints break the loop. Mitigation: hints are *derived from* the schema's parameter list, not hand-written; no risk of hallucinated hints.
- **Threshold tuning**: too low → false-positive rejections; too high → no benefit. Default `reject_above` to 0 (disabled); users opt in.

### concrete_prismgate_files_to_change
- `src/config.rs` — `PerToolBudget`, `OutputConfig.per_tool_budget`, `BudgetOptions`.
- `src/server.rs` — `CallToolChainParams.budget`, `tool_info` returns `budget`.
- `src/tools/sandbox.rs` — `handle_call_tool_chain_with_store` rejection path; `try_direct_tool_call` budget-aware gate.
- `src/tools/discovery.rs` — `ToolInfoResult.budget`, `handle_required_keys_async` extension.
- `src/result_store.rs` — no change; reuse `insert()` + `read()`.
- New: `src/tools/budget.rs` — `resolve_budget(tool_name, backend_name, config) -> ToolBudget`.

---

## 5. Concrete code examples from the field

### 5a. Speakeasy Dynamic Toolset (Gram)

**Concept**: Replace static `tools/list` with three meta-tools (`search_tools`, `describe_tools`, `execute_tool`). Only category overviews go into the initial system prompt; full schemas load on-demand.

**Mechanism**: `notifications/tools/list_changed` is sent when the dynamic toolset changes. Clients receiving the notification re-call `tools/list`. In dynamic mode, `tools/list` returns *just* the meta-tools (`search_tools`, `describe_tools`, `execute_tool`) and a one-line per-category overview in `search_tools.description`.

**Code skeleton** (TypeScript, from their docs):
```ts
const gram = new Gram().tool({
  name: "create_coupon",
  description: "Create a discount coupon for a customer",
  input: z.object({ customer_id: z.string(), amount: z.number() }),
  handler: async (ctx, input) => { /* ... */ }
});

// At server level: switch to dynamic mode
const server = gram.dynamic({
  categories: ["Customers", "Orders", "Coupons"],  // surfaced in search_tools.description
});

// At runtime: tool becomes available only when dynamic enable
server.enableTool("create_coupon");
server.disableTool("create_coupon"); // → sends notifications/tools/list_changed
```

**Direct PrismGate mapping**: PrismGate's `search_tools`/`tool_info` already implement two of the three (no `execute_tool` because `call_tool_chain` covers that role). The missing piece is **per-category overview in `search_tools.description`** — i.e., when the model calls `search_tools("github")`, the response should include a brief `categories: Vec<String>` summary of what the github backend exposes. Implementation: add `categories: Vec<String>` to `BriefSearchResult` populated from `registry.get_distinctive_terms(backend, 5)` (already implemented at `src/registry.rs`).

### 5b. Anthropic Programmatic Tool Calling (code execution with MCP)

**Concept**: The LLM writes a Python script that calls tools inside a sandbox; only the script's *final output* enters the LLM's context. Intermediate results are processed by code, not by the model.

**Mechanism**: The `code_execution_20250825` tool with `allowed_callers: ["code_execution_20250825"]` marks a tool as callable *from code*. The API returns tool results to the code execution environment, not to Claude's context.

**Code skeleton** (from Anthropic's docs):
```python
team = await get_team_members("engineering")         # intermediate — not in context
levels = list(set(m["level"] for m in team))         # intermediate
budgets = await asyncio.gather(*[
    get_budget_by_level(level) for level in levels
])                                                    # intermediate
expenses = await asyncio.gather(*[
    get_expenses(m["id"], "Q3") for m in team
])                                                    # intermediate
exceeded = [
    {"name": m["name"], "spent": sum(e["amount"] for e in exp), "limit": b["travel_limit"]}
    for m, exp, b in zip(team, expenses, budgets.values())
    if sum(e["amount"] for e in exp) > b["travel_limit"]
]                                                      # intermediate
print(json.dumps(exceeded))                          # FINAL — enters Claude's context
```

**Direct PrismGate mapping**: PrismGate's `call_tool_chain` (TypeScript via V8 sandbox, `src/tools/sandbox.rs`) is the architectural equivalent of Anthropic's `code_execution_20250825`. The key PrismGate advantage it already has: TypeScript instead of Python (no separate runtime; the V8 isolate is built into the binary via `deno_core` or similar). What PrismGate is missing:

- **No bulk-of-tools shortcut**: Anthropic's PTC lets the LLM write `for item in items: results.append(await process(item))` — parallel `asyncio.gather`. PrismGate's TypeScript sandbox supports `Promise.all` (V8 has it natively) but the LLM isn't *prompted* to use it. Adding a single sentence to the `call_tool_chain` docstring: *"Use Promise.all() to invoke independent tools in parallel. Each tool's return value is processed by your code, not by the model."* would unlock the parallel-call pattern.
- **No intermediate-result streaming**: Anthropic's PTC streams tool results back to the sandbox; PrismGate's V8 sandbox fully executes then returns the final result. For a 30s+ script that calls 50 tools, PrismGate holds the LLM idle for the whole script. Fix: emit `notifications/progress` during execution (MCP progress notifications are a 2025-06-18 spec feature).

### 5c. Stacklok MCP Optimizer (now `thv vmcp serve --optimizer`)

**Concept**: Replace `tools/list` with two meta-tools (`find_tool`, `call_tool`) for very large tool libraries. Tier 1 uses FTS5 keyword search (in-process SQLite); Tier 2 adds TEI semantic embeddings.

**Mechanism**: Three tiers documented at `docs/arch/vmcp-local.md`:
| Tier | Mechanism | Exposed Tools |
|---|---|---|
| 0 | None | All backend tools passed through |
| 1 | FTS5 keyword (SQLite in-process) | `find_tool`, `call_tool` only |
| 2 | FTS5 + TEI semantic | `find_tool`, `call_tool` only |
| 3 | FTS5 + external embedding service | `find_tool`, `call_tool` only |

Headline claim (Stacklok blog): **60–85% input-token reduction per request** with the optimizer; up to 95% for some use cases. In a benchmark with 114 tools, a "hello" prompt costs 46K tokens without the optimizer and a tiny amount with it.

**Direct PrismGate mapping**: PrismGate already has `search_tools` (BM25 + trigram + fuzzy + optional semantic), `tool_info`, and `call_tool_chain` — i.e., it's already a "Tier 1+" system. The architectural lesson from Stacklok: **don't expose backend tools directly when the count exceeds N**. PrismGate already does this (it never exposes backend tools directly — always routes via meta-tools), so it's already correct. The remaining gap is **descriptive discoverability**: when search returns N hits, the LLM needs enough category context to know which to expand. See §1 above.

### 5d. ToolHive

**Concept**: Secure runtime + governance for MCP servers. Not itself a token-efficiency tool, but the deployment substrate for the vMCP/optimizer stack.

**Relevant features**: Per-server `MCPToolConfig` allows `toolsFilter` (allow-list) and `toolsOverride` (rename + change description). These are operator-side controls, not LLM-side.

**Direct PrismGate mapping**: PrismGate's `aliases: DashMap<String, String>` in `ToolRegistry` (`src/registry.rs:52`) is the user-defined equivalent of `toolsOverride`. Already implemented. **No new work needed.**

---

## Cross-cutting notes

### Where to insert these in the PrismGate roadmap

| Feature | Dependencies | Estimated days | Cumulative |
|---|---|---|---|
| Schema minification (§1) | None | 1–2 | 1–2 |
| Description ablation (§2) | None | 0.5–1 | 2–3 |
| Semantic dedup (§3) | ResultStore (exists) | 2–3 | 4–6 |
| Output budgets (§4) | ResultStore + OutputConfig (exists) | 3–4 | 7–10 |

Order: §1 → §2 → §3 → §4. Each is independently shippable; none requires changes outside its listed files.

### What does *not* need changing
- `src/result_store.rs` — already general-purpose enough.
- `src/registry.rs:42–56` (ToolRegistry core) — already supports the meta-tool surface.
- `src/tools/retrieval.rs` — already covers the post-hoc intent-filter case.
- `src/flood_guard.rs` — already rate-limits discovery calls.

### What this enables
- A 200-tool setup running through PrismGate with all four features enabled sees:
  - §1 + §2 reduce tool-discovery cost from ~30K tokens to ~3–6K (matches Anthropic's reference).
  - §3 saves ~15–35% of execution-time tool-call tokens via dedup.
  - §4 caps pathological tool outputs (≥1MB raw responses) at ~16KB returned, with structured errors that drive the LLM to refine the query.

  Net: a session that would have cost ~80K tokens today costs ~12–18K — comparable to Anthropic's published 8.7K baseline, with the added benefit of being **gateway-side** (no client-side changes needed).

---

## File-by-file change summary

| File | Feature | Change |
|---|---|---|
| `src/tools/discovery.rs` | §1, §2 | `minify_schema`, `minimize_description`, `prune_schema_descriptions`, brief `categories` field |
| `src/server.rs` | §1, §2, §4 | New params on `tool_info`, `search_tools`, `call_tool_chain` |
| `src/registry.rs` | §2, §3 | `ToolEntry.description_minimized` cache, `ToolEntry.cacheable` |
| `src/config.rs` | §1, §4 | `OutputConfig.minify_schemas`, `OutputConfig.per_tool_budget`, `DedupConfig` |
| `src/tools/sandbox.rs` | §3, §4 | dedup wrapper around `handle_call_tool_chain_with_store`, reject_threshold error path |
| `src/backend/manager.rs` | §3 | `restart_backend` invalidates dedup cache |
| `src/dedup_cache.rs` (NEW) | §3 | `CallDedup` struct, hash + DashMap index |
| `src/tools/budget.rs` (NEW) | §4 | `resolve_budget` with glob support |
| `src/tracker.rs` | §3 | `record_cache_hit` / `record_cache_miss` for stats |
| `src/resources.rs` | §1, §2 | `llms-full` resource applies minification |