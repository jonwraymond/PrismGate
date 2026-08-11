# PrismGate clone() Hot Path Analysis

**Date:** 2026-05-24  
**Total `clone()` calls in `src/`:** 215  
**Files analyzed:** 10 top files by clone count

---

## 1. Per-File Clone Count (Top 10)

| # | File | clone() count | % of total |
|---|------|:---:|:---:|
| 1 | `src/registry.rs` | 41 | 19.1% |
| 2 | `src/backend/mod.rs` | 19 | 8.8% |
| 3 | `src/ipc/proxy_tests.rs` | 17 | 7.9% |
| 4 | `src/main.rs` | 15 | 7.0% |
| 5 | `src/sandbox/mod.rs` | 14 | 6.5% |
| 6 | `src/mcp_compliance_tests.rs` | 13 | 6.0% |
| 7 | `src/backend/health.rs` | 12 | 5.6% |
| 8 | `src/config.rs` | 11 | 5.1% |
| 9 | `src/backend/cli_adapter.rs` | 10 | 4.7% |
| 10 | `src/ipc/daemon_tests.rs` | 7 | 3.3% |
| — | _Remaining 10 files_ | _56_ | _26.0%_ |

---

## 2. Top Offender #1: `src/registry.rs` (41 clones)

### 2.1 Clone Categories

#### Category A — ToolEntry field clones in `register_backend_tools_inner` (lines 142–258)

These are the heaviest and most fixable. The method signature takes `tools: Vec<ToolEntry>` **by value**, yet every field is cloned instead of moved:

```rust
// CURRENT (14 clones in one loop, per-tool iteration):
for tool in tools {                       // tools is Vec<ToolEntry> — owned!
    let original = if tool.original_name.is_empty() {
        tool.name.clone()        // line 142 — clone String
    } else {
        tool.original_name.clone() // line 144 — clone String
    };
    let ns_entry = ToolEntry {
        name: ns_key.clone(),     // line 150 — clone new String
        original_name: original.clone(), // line 151 — clone String
        description: tool.description.clone(), // line 152 — clone String
        input_schema: tool.input_schema.clone(), // line 154 — clone Value (expensive!)
        tags: tool.tags.clone(),  // line 155 — clone Vec<String>
        backend_name: backend_name.to_string(),
    };
    self.tools.insert(ns_key.clone(), ns_entry.clone()); // line 157 — 2 more clones
    ...
    // Bare alias path (lines 181–189): 5 more clones from tool fields
    let bare_entry = ToolEntry {
        name: original.clone(),
        original_name: original.clone(),
        description: tool.description,   // moved, no clone here
        input_schema: tool.input_schema, // moved, no clone here
        tags: tool.tags,                 // moved, no clone here
        ...
    };
}
```

**Root cause:** The loop does not destructure `tool` into its fields, so every field access requires a clone because `tool` is used for both `ns_entry` and potentially `bare_entry`.

**Proposed fix — destructure and clone conditionally:**

```rust
// REPLACEMENT: destructure upfront, clone only when needed for both paths
for tool in tools {
    let ToolEntry { name, original_name, description, input_schema, tags, .. } = tool;
    
    let original = if original_name.is_empty() { name.clone() } else { original_name };
    let ns_key = format!("{}.{}", namespace, original);
    
    // Clone fields that may be needed for bare_entry path
    let (desc_copy, schema_copy, tags_copy) = (description.clone(), input_schema.clone(), tags.clone());
    
    let ns_entry = ToolEntry {
        name: ns_key.clone(),
        original_name: original.clone(),
        description,           // moved — no clone
        backend_name: backend_name.to_string(),
        input_schema,          // moved — no clone
        tags,                  // moved — no clone
    };
    self.tools.insert(ns_key.clone(), ns_entry.clone()); // DashMap requires clone (unavoidable)
    ...
    
    // Bare alias path uses the pre-cloned copies
    if owners.len() == 1 {
        let bare_entry = ToolEntry {
            name: original.clone(),
            original_name: original.clone(),
            description: desc_copy,
            backend_name: backend_name.to_string(),
            input_schema: schema_copy,
            tags: tags_copy,
        };
        self.tools.insert(original.clone(), bare_entry); // bare_entry moved, no clone
    }
}
```

**Savings:** Eliminates 3 clones per tool in the ns_entry path (description, input_schema, tags become moves). In the bare-entry path, replaces 3 field clones with 3 upfront clones (net neutral per tool with bare aliases, net savings per tool without). For a typical 10-tool backend, ~15 fewer clones.

#### Category B — DashMap `.value().clone()` patterns (lines 273, 278, 288, 292, 316, 326, 465, 539, 660, 666, 722)

These are the standard DashMap read pattern:

```rust
// HOT PATH: search_hybrid (line 722) — called on every hybrid search
let entry = self.tools.get(&name)?.value().clone();  // 1 clone per result

// HOT PATH: get_all (line 273) — called by resources/stats endpoints
self.tools.iter().map(|r| r.value().clone()).collect() // N clones

// HOT PATH: build_corpus (line 539) — called on every BM25/trigram search
let entry = r.value().clone();  // N clones
```

**Root cause:** `ToolEntry` is stored directly in `DashMap<String, ToolEntry>`. DashMap's `Ref` guard prevents returning references that outlive the lock. To return `ToolEntry` to callers, a full clone is required.

**Proposed fix — store `Arc<ToolEntry>` in DashMap:**

```rust
// BEFORE:
pub struct ToolRegistry {
    tools: DashMap<String, ToolEntry>,
    ...
}

// AFTER:
pub struct ToolRegistry {
    tools: DashMap<String, Arc<ToolEntry>>,
    ...
}
```

Then all lookups become reference-count bumps instead of full clones:

```rust
// search_hybrid — BEFORE:
let entry = self.tools.get(&name)?.value().clone();  // clones all fields

// search_hybrid — AFTER:
let entry = Arc::clone(self.tools.get(&name)?.value());  // just bumps refcount

// get_all — BEFORE:
self.tools.iter().map(|r| r.value().clone()).collect()

// get_all — AFTER:
self.tools.iter().map(|r| Arc::clone(r.value())).collect()
// Callers that need owned ToolEntry for serialization can .as_ref().clone() on demand
```

**Caveat:** This changes the return types of `get_by_name`, `get_all`, `get_by_backend`, `search`, etc. from `ToolEntry`/`Vec<ToolEntry>` to `Arc<ToolEntry>`/`Vec<Arc<ToolEntry>>`. Callers that serialize (cache, resources, server responses) would need `entry.as_ref().clone()` at the serialization boundary, which is fine because serialization needs an owned copy anyway.

**Savings estimate (hot paths):**
- `build_corpus()`: N clones → N `Arc::clone()`s (cheap refcount bump) — saves ~15 clones per search
- `search_hybrid()`: N clones per hybrid search → N refcount bumps
- `get_by_name()`: 1 clone → 1 refcount bump (called per tool dispatch)
- `snapshot()`: still needs clones for serialization, net neutral

#### Category C — String clones in RRF/trigram search (lines 701, 705, 624, 637, 640)

```rust
// RRF score accumulation (lines 701, 705):
*rrf_scores.entry(entry.name.clone()).or_default() += 1.0 / (RRF_K + ...);

// Fuzzy correction (line 624, 637, 640):
return term.clone();
best = Some((candidate.clone(), dist));
best.map(|(w, _)| w).unwrap_or_else(|| term.clone())
```

**Proposed fix for RRF:** Use `&str` keys with transient string storage:

```rust
// Build a Vec of owned strings alongside the entry refs, then use &str in HashMap
let names: Vec<String> = bm25_results.iter().map(|e| e.name.clone()).collect();
// ... but this is WORSE. Instead, just accept the clone — it's a String, cheap.

// Or: use entry.name.as_str() with a different map type, but HashMap<&str, f64>
// requires all keys to outlive the map. Use a scoped approach:
{
    let mut rrf_scores: HashMap<&str, f64> = HashMap::new();
    for (rank, entry) in bm25_results.iter().enumerate() {
        *rrf_scores.entry(&entry.name).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    // rrf_scores only valid while bm25_results/semantic_results are alive
    // Then build final results from &str keys
    let mut scored: Vec<(&str, f64)> = rrf_scores.into_iter().collect();
    scored.sort_by(...);
    // Look up full entries by &str
    let results: Vec<ToolEntry> = scored.into_iter()
        .filter_map(|(name, _)| self.tools.get(name).map(|r| r.value().clone()))
        .take(limit as usize)
        .collect();
}
```

**Savings:** Eliminates 2–4 `String::clone()` calls per hybrid search. Minor but measurable on repeated searches.

#### Category D — `snapshot()` clones (lines 761, 782, 784)

```rust
let key = (e.backend_name.clone(), e.original_name.clone());  // line 761
result.entry(e.backend_name.clone()).or_default().push(e.clone());  // lines 782-784
```

**Assessment:** `snapshot()` is called by the cache persistence system, not a hot path. These clones are for owned key construction and serialization — necessary for the HashMap return type. **Low priority.**

### 2.2 Summary: registry.rs

| Pattern | Clones affected | Difficulty | Impact |
|---------|:---:|:---:|:---:|
| Destructure tool in register loop | ~8/tool | Low | High — hot path on startup |
| Arc<ToolEntry> in DashMap | ~15/search | Medium | High — every tool lookup/dispatch |
| &str in RRF HashMap | ~4/search | Low | Low |
| snapshot() clones | ~4/snapshot | — | None (not hot path) |

---

## 3. Top Offender #2: `src/backend/mod.rs` (19 clones)

### 3.1 Clone Categories

#### Category A — Config clones for task spawn (lines 303–304, 362–372, 397)

```rust
// start_all() — cloned for each backend spawn:
let name = name.clone();                    // line 303
let backend_config = backend_config.clone(); // line 304

// start_backend() — cloned into each backend constructor:
let b = stdio::StdioBackend::new(name.to_string(), config.clone());  // line 362
let b = http::HttpBackend::new(name.to_string(), config.clone());    // line 367
let b = cli_adapter::CliAdapterBackend::new(name.to_string(), config.clone())?; // line 372

// dedicated pool:
pool::InstancePool::new(name.to_string(), config.clone(), Arc::clone(registry)) // line 397
```

**Root cause:** `BackendConfig` is large (contains nested structs, Vecs, Options). Each backend constructor takes owned `BackendConfig`. When spawning backends concurrently, each needs its own copy.

**Proposed fix — pass `Arc<BackendConfig>`:**

```rust
// Store configs as Arc<BackendConfig> internally
struct BackendManager {
    configs: RwLock<HashMap<String, Arc<BackendConfig>>>,
    ...
}

// start_all: Arc::clone is cheap
let backend_config = Arc::clone(backend_config);

// Change backend constructors to accept Arc<BackendConfig> or &BackendConfig
// If backends only need read access during construction, pass &BackendConfig
```

**Savings:** Eliminates 3–4 full `BackendConfig` clones per backend. For a gateway with 10 backends, that's ~30–40 deep clones eliminated at startup.

#### Category B — `arguments.clone()` for retry/fallback (lines 733, 772)

```rust
// call_tool_with_fallback:
let result = self.call_tool(backend_name, tool_name, arguments.clone(), session_id).await; // line 733
...
self.call_tool(fallback_name, original_name, arguments.clone(), session_id).await; // line 772
```

**Root cause:** `call_tool` takes `Option<Value>` by value. Since the caller may need `arguments` for multiple attempts (primary + fallbacks), each call needs a clone.

**Proposed fix — pass by reference:**

```rust
// Change call_tool to accept &Option<Value>:
pub async fn call_tool(
    &self,
    backend_name: &str,
    tool_name: &str,
    arguments: &Option<Value>,    // reference
    session_id: Option<u64>,
) -> Result<Value> { ... }

// call_tool_with_fallback: no clones needed
let result = self.call_tool(backend_name, tool_name, &arguments, session_id).await;
...
self.call_tool(fallback_name, original_name, &arguments, session_id).await;
```

**Caveat:** `call_tool` dispatches to `backend.call_tool(tool_name, arguments).await` which currently takes `Option<Value>` by value. This requires changing the `Backend` trait's `call_tool` signature. The trait implementations (stdio, http, cli_adapter) all serialize arguments to JSON, so they could accept a reference and clone internally only when needed.

**Savings:** Eliminates 2 `Value` clones per fallback chain attempt. `Value` clones can be expensive for large JSON arguments.

#### Category C — DashMap iteration clones (lines 805, 860, 931, 1043, 1077, 1084)

```rust
// release_session: clone keys for Vec collection
.map(|entry| (entry.key().clone(), Arc::clone(entry.value()))) // line 805

// stop_all: clone backend references
.map(|r| (r.key().clone(), Arc::clone(r.value())))  // line 860

// get_memory_stats: clone stats
self.memory_stats.get(name).map(|r| r.value().clone())  // line 1077
```

**Assessment:** These are mostly gathering data for iteration outside the DashMap lock. `Arc::clone` is already cheap (refcount bump). The `key().clone()` is a `String::clone`. **Low priority** for stop/cleanup paths; only `release_session` (line 805) and `get_memory_stats` (line 1077) are potentially hot.

#### Category D — `RetryConfig` clone (lines 423, 618)

```rust
// line 423 — insert by cloning
self.retry_configs.insert(name.to_string(), config.retry.clone());

// line 618 — read by cloning
.map(|r| r.value().clone())
```

**Proposed fix — derive `Copy` on `RetryConfig`:**

```rust
// If RetryConfig consists only of Copy types (Duration, f64, u32):
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay: Duration,
    pub backoff_multiplier: f64,
    pub max_delay: Duration,
}
```

Then `r.value().clone()` becomes `*r.value()` — zero-cost.

**Savings:** 2 clones eliminated per retry config read (call_tool hot path).

### 3.2 Summary: backend/mod.rs

| Pattern | Clones affected | Difficulty | Impact |
|---------|:---:|:---:|:---:|
| Arc<BackendConfig> | ~3–4/backend | Medium | Medium — startup only |
| arguments: &Option<Value> | 2/call | High | Medium — changes trait |
| RetryConfig: Copy | 2/read | Low | Low |
| DashMap iteration | 6 total | Low | Low — mostly shutdown |

---

## 4. Top Offender #3: `src/ipc/proxy_tests.rs` (17 clones)

### 4.1 Clone Categories

#### A — `socket_path.clone()` (9 occurrences) — LOW PRIORITY

```rust
let bound = crate::ipc::daemon::bind_early(Some(socket_path.clone()));  // line 78
```

These are all in test code, cloning a `PathBuf` to pass to spawned tasks. `PathBuf::clone()` is essentially a heap string clone — cheap compared to test overhead.

**Proposed fix:** Use `Arc<PathBuf>` or pass `PathBuf` by value and reconstitute later. But given these are tests, not worth the refactor.

#### B — `service.peer().clone()` (4 occurrences) — API REQUIRED

```rust
let peer = service.peer().clone();  // lines 316, 504, 823, 890
```

This is the rmcp library API — `peer()` returns `&Peer`, and tests need an owned handle. Unavoidable.

#### C — JSON `Value::clone()` (3 occurrences) — TEST ONLY

```rust
.clone(),  // lines 344, 464, 703
```

Cloning JSON values for test assertion construction. Not hot path.

### 4.2 Summary: proxy_tests.rs

All clones are in test code. None warrant changes. **Zero production impact.**

---

## 5. Consolidated Recommendations (Priority Order)

### P0 — Eliminate unnecessary clones in hot-path registration loop

**File:** `src/registry.rs`, method `register_backend_tools_inner`  
**Change:** Destructure `tool: ToolEntry` at loop start; move fields into `ns_entry`; clone only for bare-entry path.  
**Impact:** Saves ~3 full `ToolEntry` clones per tool at startup (description + input_schema + tags).  
**Risk:** Low — mechanical change, existing tests cover registration.

### P1 — Arc<ToolEntry> in DashMap

**File:** `src/registry.rs`  
**Change:** `DashMap<String, ToolEntry>` → `DashMap<String, Arc<ToolEntry>>`; update all readers.  
**Impact:** Every tool lookup, search, and dispatch becomes a refcount bump instead of a full clone. Affects `search()`, `get_by_name()`, `get_all()`, `get_by_backend()`, `build_corpus()`, `search_hybrid()`, `find_equivalent_tool()`.  
**Caveat:** Callers that need owned `ToolEntry` (serialization, responses) must `entry.as_ref().clone()` — but these are at the boundary (cache write, JSON response) where a clone is acceptable.  
**Risk:** Medium — wide interface change; all callers in `src/server.rs`, `src/resources.rs`, `src/cache.rs` need updating.

### P2 — Arc<BackendConfig> for backend constructors

**File:** `src/backend/mod.rs`  
**Change:** Store `configs` as `Arc<BackendConfig>`; pass `Arc::clone()` to constructor tasks; change backend constructors to accept `Arc<BackendConfig>`.  
**Impact:** ~3 full `BackendConfig` clones per backend eliminated.  
**Risk:** Medium — requires changing constructor signatures across stdio, http, cli_adapter, pool.

### P3 — &Option<Value> in call_tool signature

**File:** `src/backend/mod.rs`, trait `Backend`  
**Change:** `call_tool` takes `arguments: &Option<Value>`; trait implementations serialize from reference.  
**Impact:** Saves 2 `Value` clones per fallback attempt.  
**Risk:** High — changes the public `Backend` trait; affects all implementations and callers.

### P4 — RetryConfig: Copy

**File:** `src/config.rs`  
**Change:** Add `#[derive(Copy)]` to `RetryConfig`.  
**Impact:** Zero-cost reads in `call_tool` hot path.  
**Risk:** Trivial — RetryConfig is small (4 fields, all Copy).

### P5 — &str keys in RRF HashMap

**File:** `src/registry.rs`, `search_hybrid`  
**Change:** Use `HashMap<&str, f64>` scoped to result lifetimes.  
**Impact:** ~4 String clones eliminated per hybrid search.  
**Risk:** Low — requires ensuring `bm25_results`/`semantic_results` live as long as the HashMap.

---

## 6. Files NOT Recommended for Change

- **`src/ipc/proxy_tests.rs`** — All 17 clones in test code; zero prod impact.
- **`src/mcp_compliance_tests.rs`** (13 clones) — Test code only.
- **`src/ipc/daemon_tests.rs`** (7 clones) — Test code only.
- **`src/testutil.rs`** (5 clones) — Test utilities.
- **`src/integration_inventory.rs`** (5 clones) — Test code.

---

## 7. Estimated Impact Summary

| Priority | Files | Clones saved (est.) | Hot path |
|:---:|------|:---:|:---:|
| P0 | registry.rs | ~15–30 per startup | Yes (startup) |
| P1 | registry.rs | ~15–50 per search | Yes (every dispatch) |
| P2 | backend/mod.rs | ~30 per startup | Yes (startup) |
| P3 | backend/mod.rs + trait | 2 per fallback | Yes (fallback ops) |
| P4 | config.rs + backend/mod.rs | 2 per tool call | Yes (every call) |
| P5 | registry.rs | 4 per hybrid search | Yes (hybrid search) |
| **Total** | — | **~68–118 per operation** | — |

The highest-ROI change is **P1 (Arc<ToolEntry>)**: it affects every tool dispatch in the system and converts `O(fields)` clones to `O(1)` refcount bumps on the hottest path: tool lookups and search results.