# PrismGate Efficiency Improvements — Master Tracker

**Branching strategy:** one feature branch per improvement, one PR per improvement.  
**Target:** `main` branch of `jonwraymond/PrismGate`.  
**Kanban:** this file + per-wave `docs/plans/wave-*.md` implementation plans.

---

## Wave 1 — Quick wins, high token/context impact

| # | Feature | Branch | PR | Status | Notes |
|---|---------|--------|-----|--------|-------|
| T2 | Tool description ablation | `feat/desc-ablation` | [#145](https://github.com/jonwraymond/PrismGate/pull/145) | 🟢 Open | Strip non-essential description components per arxiv 2602.14878 |
| T1 | Schema minification | `feat/schema-minify` | — | ⏳ Next | Drop `description`/`default`/`additionalProperties` from `tool_info(full=true)` |
| R5 | ResultStore TTL janitor | `feat/result-ttl-janitor` | [#146](https://github.com/jonwraymond/PrismGate/pull/146) | 🟢 Open | Background cleanup of expired handles |
| S1 | Secrets redaction in FTS5 | `feat/secrets-redact` | [#147](https://github.com/jonwraymond/PrismGate/pull/147) | 🟢 Open | Regex redaction before indexing tool output |
| O1 | Structured tracing (OTEL spans) | `feat/otel-tracing` | #148 | ⏳ Planned | Instrument core paths with `tracing` + optional `opentelemetry` |
| K1 | Exact-match result cache | `feat/result-dedup-cache` | #149 | ⏳ Planned | Hash `(tool_name, params)` → handle; short-circuit duplicate calls |

## Wave 2 — Medium-lift, core efficiency

| # | Feature | Branch | PR | Status | Notes |
|---|---------|--------|-----|--------|-------|
| T3 | Semantic dedup before tool call | `feat/semantic-dedup` | #150 | ⏳ Planned | `(tool_name, normalized_args_hash) → handle` index |
| C3 | Tool output middleware pipeline | `feat/output-middleware` | #151 | ⏳ Planned | Make `execute_file`/`fetch_and_index` default path |
| R2 | Async actor for session store (Track A) | `feat/session-async-actor` | #152 | ⏳ Planned | Swap `std::sync::mpsc` → `tokio::sync::mpsc` |
| R1 | HTTP connection pool & keep-alive | `feat/http-pool` | #153 | ⏳ Planned | Shared `reqwest::Client` pool for HTTP backends |
| S3 | Tool call rate limiting per backend | `feat/exec-rate-limit` | #154 | ⏳ Planned | Extend flood guard to execution layer |
| O3 | Cost attribution | `feat/cost-attribution` | #155 | ⏳ Planned | Tag `call_tool_chain` with estimated USD cost |

## Wave 3 — Medium-lift, advanced efficiency

| # | Feature | Branch | PR | Status | Notes |
|---|---------|--------|-----|--------|-------|
| C4 | `prismgate compact` CLI + MCP tool | `feat/compact-cli` | #156 | ⏳ Planned | First-class compaction trigger for hosts |
| C5 | Context budget dashboard | `feat/context-dashboard` | #157 | ⏳ Planned | Per-session rolling budgets, projected exhaustion, top-N tools |
| R7 | Chunked tool responses + progress | `feat/chunked-responses` | #158 | ⏳ Planned | Emit `notifications/progress` every 16KB for large payloads |
| K2 | Prompt/prefix cache for catalogs | `feat/catalog-prefix-cache` | #159 | ⏳ Planned | Cache generated per-backend tool catalogs with TTL |
| K3 | Backend schema cache | `feat/schema-cache` | #160 | ⏳ Planned | Cache `listTools` responses; detect `tools/changed` notifications |
| S2 | Parameter allowlisting | `feat/param-allowlist` | #161 | ⏳ Planned | Whitelist allowed parameter values/patterns for sensitive tools |
| O2 | Token accounting per backend | `feat/token-accounting` | #162 | ⏳ Planned | Track input/output tokens per backend per session |
| O4 | `prismgate profile` CLI + resource | `feat/profile-cli` | #163 | ⏳ Planned | Per-backend latency p50/p95, call volume, error rate table |

## Wave 4 — High-lift, new capabilities

| # | Feature | Branch | PR | Status | Notes |
|---|---------|--------|-----|--------|-------|
| C1 | Session summarization worker | `feat/session-summarizer` | #164 | ⏳ Planned | Background compaction into synthetic summary events |
| C2 | Hierarchical session memory (hot/warm/cold) | `feat/hierarchical-memory` | #165 | ⏳ Planned | Three-tier session recall; resume_card pulls warm+hot |
| R6 | Streamable HTTP transport | `feat/streamable-http` | #166 | ⏳ Planned | New `transport-streamable-http-server` listener |
| R3 | Lazy backend initialization | `feat/lazy-backends` | #167 | ⏳ Planned | Start backends on first use; cold start ~30s → ~3s |
| R4 | Tool registry hot-reload | `feat/registry-hot-reload` | #168 | ⏳ Planned | Watch schema changes, reload BM25 index without restart |
| K4 | Semantic cache for tool results | `feat/semantic-cache` | #169 | ⏳ Planned | Embed results, similarity search, cache hit return |
| S4 | Audit log with hash chain | `feat/audit-log` | #170 | ⏳ Planned | Append-only SQLite log with hash-chaining between events |
| S5 | OPA sidecar for tool authorization | `feat/opa-policy` | #171 | ⏳ Planned | External policy engine for fine-grained tool access control |
| O5 | Session export/import | `feat/session-portability` | #172 | ⏳ Planned | Portable JSON/SQLite session dump for debugging |
