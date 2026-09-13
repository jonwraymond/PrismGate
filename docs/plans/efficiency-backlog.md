# PrismGate Efficiency Improvements — Unified Backlog

Generated from 6 parallel research subagents. Each item includes: category, research summary, code approach, impact, effort, risks, and concrete files to change.

---

## Token Efficiency

| # | Feature | Impact | Effort | Risks | Files |
|---|---------|--------|--------|-------|-------|
| T1 | Schema minification for `tool_info(full=true)` | High | Low | Wire compat, validation regression | `src/tools/discovery.rs`, `src/config.rs`, `src/server.rs`, `src/resources.rs` |
| T2 | Tool description ablation (arxiv 2602.14878) | High | Low | Operational cue loss, sentence-boundary ambiguity | `src/tools/discovery.rs`, `src/registry.rs`, `src/resources.rs`, `src/server.rs` |
| T3 | Semantic dedup before tool call | High | Medium | Stale results, auth state, mutable-state tools | `src/dedup_cache.rs` new, `src/registry.rs`, `src/tools/sandbox.rs`, `src/server.rs`, `src/backend/manager.rs`, `src/config.rs`, `src/tracker.rs` |
| T4 | Output token budgets per tool | High | Low | Reject-path UX, per-tool config complexity | `src/tools/sandbox.rs`, `src/config.rs`, `src/server.rs` |

**Recommended order:** T2 → T1 → T3 → T4

---

## Context Efficiency

| # | Feature | Impact | Effort | Risks | Files |
|---|---------|--------|--------|-------|-------|
| C1 | Session summarization worker | High | Medium | Summary quality, actor thread contention | `src/session/db.rs`, `src/session/extract.rs`, `src/session/mod.rs` |
| C2 | Hierarchical session memory (hot/warm/cold) | Medium | High | Query complexity, resume_card UX | `src/session/db.rs`, `src/session/retrieval.rs`, `src/session/mod.rs` |
| C3 | Tool output middleware pipeline (default) | Medium | Low | Performance regression, backward compat | `src/tools/sandbox.rs`, `src/server.rs` |
| C4 | First-class `prismgate compact` CLI + MCP tool | Medium | Low | Host integration, hook ordering | `src/cli.rs`, `src/server.rs`, `skills/prismgate-context/SKILL.md` |
| C5 | Context budget dashboard | Medium | Low | Metric accuracy, time-bucketed storage | `src/tracker.rs`, `src/resources.rs`, `src/session/db.rs` |

**Recommended order:** C3 → C4 → C5 → C1 → C2

---

## Runtime / Process Efficiency

| # | Feature | Impact | Effort | Risks | Files |
|---|---------|--------|--------|-------|-------|
| R1 | HTTP connection pool & keep-alive | High | Medium | Pool sharing, OAuth header rotation | `src/backend/http.rs`, `src/backend/http_pool.rs` new, `src/backend/mod.rs`, `src/config.rs` |
| R2 | Async actor for session store (Track A: tokio mpsc) | Medium | Low | WAL checkpoint contention | `src/session/db.rs`, `src/session/store.rs` new, `src/session/mod.rs` |
| R3 | Lazy backend initialization | Medium | Medium | Stale schema UX, first-call latency cliff | `src/backend/mod.rs`, `src/backend/health.rs`, `src/backend/cache_restore.rs` new, `src/config.rs`, `src/server.rs` |
| R4 | Tool registry hot-reload | Medium | High | Notification loss, churn DoS | `src/registry_sync.rs` new, `src/registry.rs`, `src/backend/*.rs`, `src/ipc/mcp_framing.rs` |
| R5 | ResultStore TTL janitor | Low | Low | Shutdown ordering, per-task overhead | `src/result_store.rs`, `src/server.rs`, `src/ipc/daemon.rs`, `src/main.rs` |
| R6 | Streamable HTTP transport | High | High | Stateless vs sessionful, memory cap, DNS rebinding | `src/transport/http_server.rs` new, `src/main.rs`, `src/cli.rs`, `src/config.rs` |
| R7 | Chunked tool responses + progress notifications | High | Medium | Notification storms, partial-ordering | `src/server.rs`, `src/backend/*.rs`, `src/result_store.rs`, `src/config.rs` |

**Recommended order:** R5 → R2 → R1 → R7 → R6 → R3 → R4

---

## Caching

| # | Feature | Impact | Effort | Risks | Files |
|---|---------|--------|--------|-------|-------|
| K1 | Exact-match result cache (hash tool+args → handle) | High | Low | Stale results, memory pressure | `src/dedup_cache.rs` new, `src/tools/sandbox.rs`, `src/server.rs`, `src/tracker.rs` |
| K2 | Prompt/prefix cache for tool catalogs | Medium | Low | Cache invalidation on schema change | `src/registry.rs`, `src/resources.rs`, `src/cache.rs` |
| K3 | Backend schema cache (listTools TTL) | Medium | Low | Stale schema detection | `src/backend/*.rs`, `src/registry.rs`, `src/cache.rs` |
| K4 | Semantic cache for tool results | High | High | Embedding quality, false positives | Reuse `model2vec-rs`, new `src/semantic_cache.rs` |

**Recommended order:** K1 → K2 → K3 → K4

---

## Security & Governance

| # | Feature | Impact | Effort | Risks | Files |
|---|---------|--------|--------|-------|-------|
| S1 | Secrets redaction in FTS5 | High | Low | False positives, performance | `src/session/overflow.rs`, `src/session/extract.rs`, `src/tools/fetch.rs`, `src/tools/execute_file.rs` |
| S2 | Parameter allowlisting for sensitive tools | Medium | Low | Policy maintenance, false rejects | `src/tools/sandbox.rs`, `src/config.rs`, `src/server.rs` |
| S3 | Tool call rate limiting per backend | High | Low | Throttling UX, health coupling | `src/flood_guard.rs`, `src/backend/health.rs`, `src/tools/sandbox.rs` |
| S4 | Audit log with hash chain | Medium | Low | Write amplification, WAL contention | `src/session/db.rs`, `src/session/event.rs`, `src/tracker.rs` |
| S5 | OPA sidecar for tool authorization | Medium | High | Availability, fail-open vs fail-closed | New `src/policy/opa.rs`, `src/config.rs`, `src/server.rs` |

**Recommended order:** S1 → S3 → S2 → S4 → S5

---

## Observability & DX

| # | Feature | Impact | Effort | Risks | Files |
|---|---------|--------|--------|-------|-------|
| O1 | Structured tracing (OTEL spans) | Medium | Low | Cardinality, perf overhead | `src/server.rs`, `src/session/db.rs`, `src/backend/mod.rs`, `Cargo.toml` |
| O2 | Token accounting per backend | Medium | Medium | Token estimation accuracy, binary size | `src/tracker.rs`, `src/backend/mod.rs`, `src/session/db.rs`, `Cargo.toml` (`tiktoken`) |
| O3 | Cost attribution | Medium | Low | Pricing drift, unknown models | `src/cost/mod.rs` new, `src/tracker.rs`, `src/resources.rs`, `config/model_pricing.yaml` |
| O4 | `prismgate profile` CLI + resource | Low | Low | Output formatting, tracker enrichment | `src/cli.rs`, `src/resources.rs`, `src/tracker.rs` |
| O5 | Session export/import | Low | Medium | Schema versioning, atomicity | `src/cli.rs`, `src/session/db.rs`, `src/session/mod.rs` |

**Recommended order:** O1 → O3 → O2 → O4 → O5

---

## Sequencing Recommendation

**Wave 1 (Weeks 1–2): Quick wins, high token/context impact**
- T2, T1, R5, S1, O1, K1

**Wave 2 (Weeks 3–4): Medium-lift, core efficiency**
- T3, C3, R2, R1, S3, O3

**Wave 3 (Weeks 5–6): Medium-lift, advanced efficiency**
- C4, C5, R7, K2, K3, S2, O2, O4

**Wave 4 (Weeks 7–8+): High-lift, new capabilities**
- C1, C2, R6, R3, R4, K4, S4, S5, O5
