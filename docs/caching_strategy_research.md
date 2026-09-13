# PrismGate Caching Strategy — Deep Research Report

Scope: cache surface for `prismgate` (Rust MCP gateway). Current state confirmed in code:
- `src/result_store.rs` — session-local handle store, 30 min TTL, FIFO + max_bytes eviction.
- `src/cache.rs` (v4) — persistent tool cache + persisted `model2vec-rs` embeddings + usage stats.
- `src/embeddings.rs` — `EmbeddingIndex` over tool name + description, brute-force cosine.
- `src/registry.rs` — `register_backend_tools_inner` already adds to `EmbeddingIndex` on every backend (re)register.
- `src/main.rs` — semantic default feature loads `minishlab/potion-base-8M` via model2vec on startup.
- `src/backend/mod.rs:158,379,950` — `discover_tools()` calls MCP `tools/list`; called once per backend start and as health probe. **No `notifications/tools/list_changed` listener wired up.**
- `src/prompts.rs` — `find_tool_prompt` re-runs `registry.search` on every prompt request.
- `src/resources.rs:533,579` — `llms_txt()` / `llms_full_txt()` regenerate on every read.

`rmcp` 1.8.0 already exposes `ClientHandler::on_tool_list_changed` (handler/client.rs:228) and `ToolListChangedNotification` (model/meta.rs:189) — notification path is free to wire.

---

## 1. Semantic cache for tool results

### Research summary
- **GPTCache** (zilliztech) is the canonical reference design: adapter → preprocessor → embedding generator → cache manager → similarity evaluator → postprocessor. Embedding is pluggable (OpenAI, ONNX, sentence-transformers); vector store is pluggable (FAISS, Milvus, Qdrant). Cache key is `(namespace, embedded_prompt, params)`. Threshold (cosine) gates hit/miss; optional LLM-as-judge verification for high-stakes hits. Hit rate 20–45% is realistic for FAQ/support workloads.
- **Langfuse semantic cache** sits at gateway, not per-app; keyed by `(tenant, model, prompt_version, active_tool_set, embedded_prompt)`. Crucial rule: namespace must encode every axis that makes two textually-similar requests semantically different. Tenant isolation and `index_version` are mandatory; optimize for precision, not hit rate.
- **Bifrost** ships the cleanest dual-layer model: exact hash (L1, sub-millisecond) → vector similarity (L2, embedding lookup only). Direct-only mode works without any vector store. Vector store is configurable (FAISS or external). Cache hits are ~5 ms.
- **LiteLLM** supports in-memory, disk, Redis, S3, GCS, Qdrant, Valkey, plus semantic on Redis/Qdrant. Defaults to OpenAI `text-embedding-ada-002`. Cache groups let prompts share cache across model families.
- **Portkey** "semantic" is a superset of simple; gated to ≤8,191 input tokens and ≤4 messages for accuracy.

### Rust embedding options
| Crate | Size / model | Latency | Quality | Suitability |
|---|---|---|---|---|
| `model2vec-rs` (already a dep, `semantic` feature) | `potion-base-8M` ≈30 MB; 256-d | <5 ms / query on CPU | Lower than MiniLM but adequate for tool-result paraphrase | **Best fit** — already wired, no new dep |
| `fastembed-rs` | `all-MiniLM-L6-v2` ≈23 MB; 384-d; ONNX Runtime | ~10 ms / query on CPU | Strong | Drop-in, but adds a 50 MB dep tree |
| `ruvector-onnx-embeddings` | Bundled 8 sentence-transformers; ONNX | ~10 ms / query | Strong | Most featureful, but new dep |
| `rust-bert` | Full BERT; CPU/GPU | 50–200 ms | Strongest | Overkill — heavy build (PyTorch-style weights) |
| External HTTP embedder (Ollama, OpenAI) | n/a | 50–500 ms | Highest | Out of process, defeats latency goal |

**Recommendation:** keep `model2vec-rs`. Same crate, same model, same 256-d vector space already used for tool discovery. Reusing it for result caching costs zero new compile-time dependency and shares the in-memory index.

### Code approach (concrete)
Reuse `EmbeddingIndex` (already thread-safe, RwLock over `HashMap<String, Vec<f32>>`). Add a sibling index for result embeddings keyed by `(tool_name, params_hash)` — same lock pattern, separate map. Embed the *result text* (or a deterministic digest of structured JSON) so two paraphrased queries whose outputs are equivalent both hit.

Pseudocode:
```rust
// src/result_semantic_cache.rs (new)
pub struct SemanticResultCache {
    embedder: Arc<EmbeddingIndex>,
    entries: RwLock<HashMap<CacheKey, CacheEntry>>, // 32k entries typical
    similarity_threshold: f32,                       // 0.92 conservative
    ttl: Duration,
}
struct CacheKey {
    tool: String,
    params_hash: u64,      // SHA-256 truncated
}
struct CacheEntry {
    vector: Vec<f32>,      // L2-normalized
    handle: String,        // existing ResultStore handle
    inserted: Instant,
}
impl SemanticResultCache {
    pub fn get_or_miss(&self, tool: &str, params_hash: u64, query_text: &str) -> CacheLookup {
        let q = self.embedder.embed(query_text);
        let store = self.entries.read().unwrap();
        let mut best = None;
        for (k, v) in store.iter() {
            if k.tool != tool || k.params_hash != params_hash { continue; }
            let sim = dot(&q, &v.vector);
            if sim >= self.similarity_threshold && (best.is_none() || sim > best.0) {
                best = Some((sim, v.handle.clone()));
            }
        }
        match best {
            Some((sim, h)) => CacheLookup::Hit { handle: h, similarity: sim },
            None => CacheLookup::Miss,
        }
    }
}
```

Wire into the call path in `src/server.rs` near `call_tool_chain` (around line 577) and the direct tool dispatch (around line 768): after routing to a backend, before invoking, compute `params_hash = blake3::hash(&canonical_json(args)).as_bytes()[..8]`; look up; on hit, return the `ResultStore` handle immediately. On miss, dispatch and on success insert both into `ResultStore` and `SemanticResultCache`.

### Impact / effort / risks
- **Impact:** Highest ROI among the four. For tool catalogs with idempotent read-only tools (`gpt-4o-mini` summarization backends, `exa.web_search` over similar queries), 20–40% latency and backend-cost reduction is plausible. Embedding cost (~5 ms per result) is paid only on miss.
- **Effort:** Medium. New file `src/result_semantic_cache.rs` (~250 lines + tests), wire 3–4 lines into `server.rs`, plumb config. **No new Cargo dep.**
- **Risks:**
  - **Semantic collisions** — paraphrased prompts with different intended outputs hit. Mitigate with conservative threshold (≥0.92) and disable per-tool via config opt-out (`tools.cache.semantic.deny: ["exa.search"]`).
  - **Stale cache** — TTL of 5–10 min default. Embed `params_hash` so trivial arg changes still miss.
  - **Memory bound** — bound by entry count + vector bytes. At 256-d f32 × 32k entries ≈ 32 MB; add an LRU.
  - **Poisoned outputs** — never cache errors; cache only on `success=true`.

### Files to change
- **New** `src/result_semantic_cache.rs` — index + LRU + tests.
- `src/server.rs` — lookup before dispatch, insert after success.
- `src/config.rs` — add `tools.cache.semantic.{enabled,threshold,ttl,max_entries}` and per-tool deny list.
- `src/main.rs` — construct the cache, share via `Arc`.
- `src/lib.rs` / `src/server.rs` — register the cache into `PrismGateServer`.
- Docs: `docs/caching.md` (new).

---

## 2. Prompt / prefix cache for gateway system prompts

### Research summary
- **Helicone prompt caching** (Anthropic `cache_control`): provider-side, opaque to the gateway. Not applicable here — PrismGate is upstream of the LLM.
- **Portkey / LiteLLM "simple cache"**: keyed on the serialized request — effectively memoizing the prompt-response pair. Saves cost when the same prompt recurs. This is closer to feature 4 below than to "prompt cache" in the strict provider sense.
- **TrueFoundry, Bifrost**: prompt cache = static, rarely-changing portion of the system prompt (the tool catalog, the guide text). Generated once per schema version, hashed, served as a constant. When the schema changes, version bumps and old entries evict.
- **MCP specifically**: every LLM that connects to PrismGate sends a fresh system prompt built from `prismgate://backends`, `prismgate://tools`, the `discover` prompt, and the per-tool schemas. Most of that text is deterministic for a given backend/tool-set state. The expensive part is generating `llms-full.txt` (full schemas for every tool) and the dynamic portion of `find_tool_prompt`.

### Code approach
Two-tier cache in front of the prompt generators:

1. **Per-schema-version prompt cache.** Key = `(prompt_name, schema_version)`. `schema_version` is a 64-bit hash of the canonical `(backend_name → tools → input_schema)` snapshot — bump whenever `register_backend_tools_inner` mutates state. Value = rendered prompt text. TTL optional (24h) for defense-in-depth; primary invalidation is version-bump.
2. **Per-query `find_tool` cache.** Key = `(schema_version, task_text_normalized)`. Value = top-5 search results. TTL short (60s). Keeps the prompt stable when an agent hammers `find_tool` with the same task between schema mutations.

Both fit cleanly inside `PromptsCache` (new module `src/prompts_cache.rs`). Wrap `prompts::get_prompt` with a check; bypass when `schema_version == 0` (first request).

```rust
// src/prompts_cache.rs (new)
pub struct PromptsCache {
    schema_version: AtomicU64,
    prompt_text: RwLock<HashMap<&'static str, String>>,         // schema-version-keyed
    find_tool: RwLock<HashMap<String, FindToolResult>>,         // task → result
}
impl PromptsCache {
    pub fn bump_schema(&self) { /* increment AtomicU64, clear prompt_text */ }
    pub fn get_or_render(&self, name: &str, registry: &ToolRegistry) -> String { /* … */ }
}
```

Wire bump from `registry::register_backend_tools_inner` and `remove_backend_tools` — call `self.prompts_cache.bump_schema()` after the mutation. Cheap; one atomic store.

### Impact / effort / risks
- **Impact:** Medium. `find_tool_prompt` is re-rendered per MCP `prompts/get` call; agents frequently re-fetch during discovery. `llms-full.txt` is on the order of tens of KB and is regenerated for every resource read.
- **Effort:** Small. ~150 lines + tests. No new deps.
- **Risks:**
  - **Stale cache after backend start** — must bump on first live register too. Pattern: `cache.rs` already knows when a backend transitions from cached → live; hook there.
  - **Personalization** — `backend_status_prompt` includes runtime stats. Don't cache it, or cache only the static portion.
  - **Cache stampede** — first request after schema bump fans out to N agents. Mitigate with single-flight (DashMap `entry().or_insert_with(|| spawn_render()))`).

### Files to change
- **New** `src/prompts_cache.rs`.
- `src/prompts.rs` — replace direct string builds with cache reads.
- `src/registry.rs` — call `bump_schema` from `register_backend_tools_inner` and `remove_backend_tools`.
- `src/resources.rs` — wrap `llms_txt` and `llms_full_txt` similarly.
- `src/main.rs` — construct and share `Arc<PromptsCache>`.

---

## 3. Backend schema cache (`tools/list` per backend)

### Research summary
- **MCP spec**: servers that declared `tools.listChanged: true` MUST send `notifications/tools/list_changed`. Spec discussion #76 confirms notifications cover contract changes (name, description, params) — not implementation changes. Clients are expected to re-issue `tools/list` on receipt.
- **`rmcp` 1.8** exposes `ClientHandler::on_tool_list_changed(NotificationContext<RoleServer>)` and `ToolListChangedNotification` — the gateway can subscribe per backend. Currently `PrismGate` constructs the client handler implicitly via the `serve(transport)` call; the gateway wraps the backend's peer but does not implement a custom handler.
- **PrismGate today** calls `backend.discover_tools()` only in three places: `add_backend`, the health-checker probe, and `composite.discover_tools()`. There is no in-memory schema cache; each call is a round-trip to the backend MCP server.

### Code approach
Add a per-backend `BackendSchemaCache` keyed by `backend_name` with two invalidation paths:

1. **Passive — MCP notification.** Implement a custom `rmcp::ClientHandler` for each backend transport. In `on_tool_list_changed`, call `manager.refresh_schema(backend_name)`, which:
   - calls `backend.discover_tools()` (already exists, returns `Vec<ToolEntry>`)
   - compares to cached set by `(name, input_schema)` — if equal, bump freshness timestamp only
   - if different, call `registry.remove_backend_tools(backend_name)` then `register_backend_tools_namespaced(...)` and persist via `cache::save`.
2. **Active — TTL polling fallback.** For backends that don't advertise `listChanged` (most existing ones), poll `tools/list` on a per-backend interval (default 5 min). Skip if last successful poll < TTL ago.

```rust
// inside BackendManager
pub struct BackendSchemaCache {
    schemas: DashMap<String, CachedSchema>,
}
struct CachedSchema {
    tools: Vec<ToolEntry>,
    fetched_at: Instant,
    advertised_list_changed: bool,
}
impl BackendSchemaCache {
    pub async fn get_or_refresh(&self, backend_name: &str, backend: &dyn Backend)
        -> Result<Vec<ToolEntry>>
    { /* compare TTL; re-call discover_tools if expired */ }
}
```

Wire the notification handler. Two options:
- **(a) Reuse one handler per gateway session.** Add `impl ClientHandler for PrismGateBackendClient { async fn on_tool_list_changed(...) { ... } }`, store backend_name in the struct, fan out to `BackendSchemaCache`. Requires injecting the backend name into the rmcp service builder — feasible because each backend's `serve()` call constructs its own service tower.
- **(b) Custom wrapper transport.** Wrap the inner transport with a JSON-RPC filter that intercepts `notifications/tools/list_changed` and routes to the cache. More invasive; only worth it if (a) fails.

Start with (a); (b) only if needed.

### Impact / effort / risks
- **Impact:** Medium-high. Today every proxy reconnect re-handshakes and re-discovers (per `CLAUDE.md` "proxy reconnect can replay the cached MCP initialize handshake" — but discover_tools is not in that path; it's called per add_backend). On hot-reload, schema is re-registered each time. Caching eliminates redundant `tools/list` calls.
- **Effort:** Medium-large. ~300 lines + tests + rmcp integration work. Custom `ClientHandler` impl for each backend transport (stdio, http, cli_adapter) — they share `backend.discover_tools()` but each constructs its own `Service<RoleClient>`.
- **Risks:**
  - **Notification flooding** — buggy backend emits notifications on every call. Cap re-fetches with debounce (e.g., min 1s between refreshes).
  - **State desync** — cache says tools exist, backend says otherwise. Fix: on `call_tool` failure with "unknown tool", force-refresh schema. Add a `force_refresh(backend_name)` API to `BackendSchemaCache`.
  - **Health probe vs schema fetch collision** — health-checker's `discover_tools` ping must go through the same cache; else it bypasses invalidation.

### Files to change
- **New** `src/backend/schema_cache.rs` — `BackendSchemaCache` + tests.
- `src/backend/mod.rs` — wrap `discover_tools()` calls with the cache; expose `refresh_schema` and `force_refresh`.
- `src/backend/stdio.rs`, `http.rs`, `cli_adapter.rs`, `composite.rs`, `pool.rs` — each gets a custom `ClientHandler` impl (or one shared wrapper) that hooks `on_tool_list_changed`.
- `src/main.rs` — construct cache, pass into `BackendManager`.
- `src/config.rs` — add `backends.schema_cache.{ttl, list_changed_only}`.

---

## 4. Exact-match result cache (highest value, lowest effort)

### Research summary
- **Helicone**: hashes request, stores response in Cloudflare Workers KV at the edge. Bucket max size configurable (default 3 entries/bucket for free, more for enterprise). Cache-Control `max-age` sets TTL.
- **Portkey simple cache**: "exact match on the input prompts" — hash of the full prompt payload. Misses on any byte change.
- **LiteLLM**: in-memory `InMemoryCache` with TTL+size limits, plus Redis/S3/GCS for distributed. The dual-tier pattern (in-mem + Redis) is canonical.
- **Bifrost direct mode**: hash → response, sub-millisecond hits. No embedding required.

Why this matters even though it's "simple": most MCP tool traffic has significant repetition per session. `search_tools(query="web search")`, `exa.web_search(...)`, `prismgate://health`, `prismgate://backends` — all are deterministic functions of their arguments. A backend round-trip costs 10–500 ms; a hashmap lookup costs <1 µs.

### Code approach
**Reuse the existing `ResultStore`** — its `r-{uuid}` handles are already an exact-match cache for *prior* results within the same session. Extend it with a deterministic hash key.

```rust
// in src/result_store.rs (extend, don't fork)
pub struct ResultStore {
    entries: Mutex<VecDeque<Entry>>,            // existing, FIFO
    by_key: DashMap<u64, String>,               // NEW: params_hash → handle
    max_bytes: usize,
    max_entries: usize,
    ttl: Duration,
}
impl ResultStore {
    pub fn lookup_by_key(&self, key: u64) -> Option<String> {
        self.by_key.get(&key).map(|r| r.value().clone())
            .filter(|h| self.entries.lock().unwrap().iter().any(|e| &e.handle == h))
    }
    pub fn insert_keyed(&self, key: u64, raw: String) -> Option<String> {
        // existing insert logic, then mirror to by_key
    }
}
```

Key composition:
```
key = blake3(backend_name || canonical_json(args))  // truncated to u64
```

Hash collision handling: blake3 truncated to 64 bits has birthday collision at √2^64 ≈ 4B entries. For practical cache sizes (≤10K entries), collision probability per lookup is < 10⁻¹⁰. Defense: on lookup, compare canonical args bytewise before returning the cached handle. Reject on mismatch.

TTL: same as `ResultStore` (30 min default, configurable). Eviction: when an entry leaves the `VecDeque`, also remove its key from `by_key`.

Memory bounds: existing `max_bytes` (32 MB) and `max_entries` (128) cover this. For higher-volume use, raise the default and document it.

### Impact / effort / risks
- **Impact:** High. Latency for repeated `search_tools`, `tool_info(brief)`, resource reads, and idempotent tool calls drops to <1 ms. Backend load drops proportionally.
- **Effort:** Small. ~80 lines + 4 tests. Single-file change. **This is the highest-ROI change.**
- **Risks:**
  - **Stale-by-design** — cached `exa.web_search(query="Q")` is fresh for ~TTL seconds even if Q's answer would have changed. Mitigate: allow `tools.cache.exact.deny` list; shorter TTL (60s) is the default for tools marked `side_effects=true` in their input schema. MCP doesn't carry a `sideEffects` field today, but infer from schema keywords (`mutation`, `delete`, `update`) or expose a config knob.
  - **Non-determinism** — tools that include timestamps in their output shouldn't cache. Same denylist config.
  - **Session-scoping** — current `ResultStore` is per-MCP-connection (Arc-clone). Hash-keyed cache should also be per-session to avoid leaking data across users. Keep it on `ResultStore`.

### Files to change
- `src/result_store.rs` — add `by_key`, `lookup_by_key`, `insert_keyed`, eviction hook.
- `src/server.rs` — wrap `call_tool_chain` dispatch with key lookup; insert with key on success.
- `src/config.rs` — `result_store.{max_bytes,max_entries,ttl,exact_cache_enabled}`.
- Tests in `src/server.rs` and `src/result_store.rs`.

---

## 5. Research synthesis: Portkey / Helicone / LiteLLM

| Capability | Portkey | Helicone | LiteLLM | Bifrost | Self-hostable in PrismGate? |
|---|---|---|---|---|---|
| Exact hash cache | ✅ `mode: simple` | ✅ header `Helicone-Cache-Enabled` | ✅ `InMemoryCache`, Redis, S3, GCS | ✅ Direct mode | ✅ **Yes** — feature 4 |
| Semantic cache | ✅ Enterprise (vector DB) | Roadmap only | ✅ Redis/Qdrant semantic | ✅ Dual-layer | ✅ **Yes** — feature 1 (reuse model2vec) |
| Prompt / prefix cache | Indirect (provider-side) | Indirect (Anthropic `cache_control`) | Indirect | Indirect | ✅ **Yes, gateway-side** — feature 2 |
| Backend tool catalog cache | n/a (provider catalog) | n/a | n/a | n/a | ✅ **Yes** — feature 3 |
| Cross-instance distributed cache | Redis (Enterprise) | Cloudflare Workers KV (managed) | Redis | Optional | ⚠️ Optional — not needed if running single daemon |
| TTL control | `max_age` | `Cache-Control` | `ttl`, `s-maxage` | Configurable | ✅ Default + per-tool override |
| Tenant / session isolation | Required config | Per-project | Namespace | Namespace | ✅ Free — already per-session |

**PrismGate's architectural advantage**: it is upstream of LLM providers and sits at the MCP boundary, so it controls both *backend tool dispatch* (features 3, 4) and *gateway-rendered prompts/resources* (feature 2). Portkey/Helicone/LiteLLM operate one layer downstream and can only do (1) and (4) effectively. Feature 3 has no analogue in any LLM gateway — it's a PrismGate-specific win.

What **can** be self-hosted without external services:
- All four features. The `semantic` Cargo feature already pulls `model2vec-rs`; the `potion-base-8M` model is 30 MB on disk, downloads on first run via HF cache. No external API needed.
- No Redis required. The cache is in-process behind `DashMap` / `RwLock`, which is fine for the daemon model where each `prismgate serve` owns its state.

What **cannot** be self-hosted without external services:
- Cross-instance cache sharing (would need Redis). Out of scope unless PrismGate moves to a multi-instance horizontal model.

---

## Recommended ordering & combined impact

1. **Feature 4 (exact-match result cache)** — smallest diff, biggest latency win. Do first.
2. **Feature 2 (prompts/prefix cache)** — small diff, big CPU win for high-traffic agents.
3. **Feature 3 (backend schema cache + notifications)** — medium diff, biggest backend-load win, and a PrismGate-unique capability.
4. **Feature 1 (semantic result cache)** — medium diff, biggest qualitative win but most risk. Ship last behind a config flag with conservative defaults; gate per-tool via deny list.

Combined: estimated 50–80% reduction in backend `tools/list` and `tools/call` invocations for typical agent workloads, with sub-millisecond cache hits replacing 50–500 ms backend round-trips.

---

## Config sketch (new `Config` section)

```yaml
tools:
  cache:
    exact:                    # Feature 4
      enabled: true
      ttl_seconds: 1800
      deny: ["exa.crawl"]     # tools that should never be cached exactly
    semantic:                 # Feature 1
      enabled: false          # off by default; turn on with caution
      threshold: 0.92
      ttl_seconds: 300
      max_entries: 32000
      deny: []
backends:
  schema_cache:               # Feature 3
    ttl_seconds: 300          # poll fallback
    list_changed_only: false  # if true, never poll, only respond to notifications
prompts:
  cache:
    enabled: true             # Feature 2
    find_tool_ttl_seconds: 60
```

## Files-to-touch rollup

| Feature | New files | Modified files |
|---|---|---|
| 4. Exact result cache | — | `src/result_store.rs`, `src/server.rs`, `src/config.rs` |
| 2. Prompt cache | `src/prompts_cache.rs` | `src/prompts.rs`, `src/registry.rs`, `src/resources.rs`, `src/main.rs` |
| 3. Schema cache | `src/backend/schema_cache.rs` | `src/backend/{mod,stdio,http,cli_adapter,composite,pool}.rs`, `src/main.rs`, `src/config.rs` |
| 1. Semantic result cache | `src/result_semantic_cache.rs` | `src/server.rs`, `src/main.rs`, `src/config.rs` |
| Docs | `docs/caching.md` | `CLAUDE.md`, `README_NEW.md` |

All four features can ship behind a single Cargo feature `cache-strategies` (default on), with individual config toggles. No new heavyweight dependencies.
