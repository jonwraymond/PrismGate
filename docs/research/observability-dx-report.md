# PrismGate Observability & DX — Deep Research

> Scope: gaps between the current `CallTracker` / `prismgate://stats` / structured logging and what a serious observability + developer-experience layer would look like for a Rust MCP gateway that fans out to many stdio/HTTP backends.
>
> Code references are pinned to the workspace at `/home/hermes-exec/workspace/PrismGate/` (prismgate v1.18.0, Cargo.toml dated 2025-09-11).

## Current state (recap from `docs/telemetry-strategy.md` + source)

| Surface | Today | Where |
|---|---|---|
| Structured logging | `tracing` + `tracing-subscriber` (env-filter, JSON feature already enabled), stderr writer, no spans | `src/main.rs:81-87` |
| Per-call latency | HDR histogram per backend, p50/p95/p99/avg | `src/tracker.rs:52-148` |
| Recent ring buffer | 500-event bounded FIFO (Mutex<VecDeque>) | `src/tracker.rs:54` |
| Usage counts | `DashMap<String, u64>` per tool | `src/tracker.rs:56` |
| Byte accounting | per-tool returned, total processed (raw), `estimated_tokens_saved = (processed-returned)/4` | `src/tracker.rs:223-275` |
| Session events | SQLite + FTS5 via `rusqlite`, actor-thread mpsc, `session_search`, `resume_card` | `src/session/db.rs:239-292` |
| `prismgate://stats` resource | session_stats only (uptime, calls, bytes, savings ratio, per_tool) | `src/resources.rs:271-276` |
| `prismgate://recent/{n}` resource | last N events | `src/resources.rs:299-310` |
| `prismgate://health` | per-backend PID/RSS/stderr ring | `src/resources.rs:242-270` |
| `prismgate doctor` | socket/alive/version check (Unix only) | `src/ipc/doctor.rs:6-65` |
| OTEL spans | **none** — `info_span!` / `#[instrument]` count = 0 across the crate |
| Per-backend error rate | **not tracked** (success bool is recorded but not aggregated) |
| Token counts | **not tracked** (only `/4` heuristic on raw bytes) |
| Cost attribution | **none** |
| `prismgate profile` | **none** |
| Session export/import | **none** |

The architecture already has the right *scaffolding* (lock-free DashMaps, HDR histograms, mpsc-backed SQLite actor, flood-guard-aware server). What is missing is a structured-tracing spine and a per-backend aggregation layer that the MCP resources can render.

---

## Feature 1 — Structured tracing: OpenTelemetry spans

### research_summary

PrismGate has `tracing = "0.1"` and `tracing-subscriber` with the `json` feature already on (`Cargo.toml:65-67`), but the code uses only event macros (`info!`, `warn!`, `debug!`, `error!`) and emits them to stderr. There is not a single `info_span!` or `#[instrument]` in `src/`, so there is no parent-child span tree — every log line stands alone. The `docs/telemetry-strategy.md` page is explicit that "OTEL export remains future work."

The Rust ecosystem's idiomatic path is the **two-layer subscriber**:
1. `tracing` for instrumentation API (no API change for existing macros).
2. `tracing-opentelemetry` to convert `tracing::Span` → `opentelemetry::Span`.
3. `opentelemetry-otlp` exporter to push spans over OTLP/HTTP to a Collector, Langfuse (`/api/public/otel`), or any OTel-compatible backend.
4. **Pin the whole family** together — `opentelemetry 0.32` ↔ `opentelemetry_sdk 0.32` ↔ `opentelemetry-otlp 0.32` ↔ `tracing-opentelemetry 0.33` (verified against `openobserve.ai/opentelemetry/rust`, 2026).

Hot-path instrumentation points already exist as natural span boundaries:

| Span name | Where | Useful fields |
|---|---|---|
| `mcp.request` | top of every `#[tool]` macro in `src/server.rs` | `rpc.method`, `session_id`, `request_id` |
| `mcp.discovery.search_tools` | `src/tools/discovery.rs:99-116` | `query.len`, `limit`, `brief`, `result_count` |
| `mcp.discovery.tool_info` | `src/tools/discovery.rs:179-187` | `tool_name`, `detail`, `hit` |
| `mcp.call_tool_chain` | `src/tools/sandbox::handle_call_tool_chain_with_store` | `code.len`, `intent`, `timeout`, `tool_count` |
| `backend.call` | `BackendManager::call_tool` (`src/backend/mod.rs:535`) | `backend`, `tool`, `attempt`, `queue_wait_ms` |
| `session_store.record` | `src/session/db.rs:295-312` | `category`, `event_type`, `name` |
| `session_store.search` | `src/session/db.rs:314-347` | `query.len`, `limit`, `hit_count` |
| `health_check` | `src/backend/health.rs` | `backend`, `restart_count`, `consecutive_failures` |

### code_approach

1. **Cargo.toml** — add an `observability` feature (default-on):
   ```toml
   opentelemetry = "0.32"
   opentelemetry_sdk = { version = "0.32", features = ["rt-tokio"] }
   opentelemetry-otlp = { version = "0.32", features = ["grpc-tonic", "trace"] }
   tracing-opentelemetry = "0.33"
   ```
   All gates on `feature = "observability"` so users who do not want OTel pay zero compile cost. The existing `tracing` and `tracing-subscriber` deps are untouched.

2. **Layered subscriber in `src/main.rs`** — replace the single `.init()` at line 83-87 with a feature-gated `init_tracing(&config)` helper:
   ```rust
   let fmt_layer = tracing_subscriber::fmt::layer()
       .with_writer(std::io::stderr)
       .with_ansi(false)
       .json();
   let env_filter = EnvFilter::try_new(&config.log_level)?;
   #[cfg(feature = "observability")]
   let otel_layer = build_otel_layer(&config.observability)?;
   let registry = tracing_subscriber::registry()
       .with(env_filter)
       .with(fmt_layer);
   #[cfg(feature = "observability")]
   let registry = registry.with(otel_layer);
   registry.init();
   ```
   `build_otel_layer` reads `OTEL_EXPORTER_OTLP_ENDPOINT` (and a new `observability.otlp_endpoint` config block) and constructs a `BatchSpanProcessor` with `BatchConfig::default().with_max_queue_size(2048)` so a slow OTLP collector cannot backpressure the request path. **Performance cost on hot path = a single `Arc` increment and a small `HashMap` allocation per `info_span!` exit** — benchmarks in the wider Rust ecosystem show <1 µs overhead per span when the batch processor is active.

3. **Span injection** — add `#[instrument(skip_all, fields(...))]` to the four hot handlers in `src/server.rs` (`search_tools`, `list_tools_meta`, `tool_info`, `call_tool_chain`). Use `skip_all` (do not capture arguments automatically — `params.code` is a sandbox payload that may be megabytes). Carry `session_id` as a span field so traces group by client.

4. **`tracing-futures` already pulled in via `tokio` feature set** — `.in_current_span()` on the awaited futures inside `BackendManager::call_tool` so the backend span becomes a child of the MCP span.

5. **Drop OTel gracefully** — wrap the `BatchSpanProcessor` so it `shutdown()`s on `Ctrl-C`/`SIGTERM`, flushed by the existing `shutdown_notify: Arc<tokio::sync::Notify>` (`src/main.rs:214`).

### impact_estimate

| Dimension | Today | After |
|---|---|---|
| Per-call debuggability | log lines alone; no causal tree | full parent→child span tree, queryable in Jaeger/Tempo/Langfuse |
| Cross-backend correlation | manual log grep on `backend=...` | `trace_id`/`span_id` carried end-to-end |
| Latency attribution | one HDR bucket per backend | per-call percentiles via OTel histograms |
| Onboarding for new backends | read logs | OTEL trace shows backend startup sequence, first call, first failure |
| Cargo compile time | baseline | +8-12 s with `observability` feature (heuristic — `opentelemetry-otlp` is heavy) |

### effort_estimate

- **S** — feature-flag plumbing + 4 `#[instrument]` annotations + Cargo deltas. The hard part (event emission) already exists.
- ~1-2 days of focused work, plus docs update (`docs/telemetry-strategy.md` was explicit that OTEL export is future work — this PR flips that switch).

### risks

1. **Stdout/stderr already in use** — the gateway logs to stderr (line 86) so MCP stdio framing stays clean. OTLP exporter goes over TCP, so no contention. ✓
2. **Sampling** — without sampling, every discovery search creates a span. Add `Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(0.1)))` by default; expose `observability.sample_ratio` in config. Users can flip to `AlwaysOn` for debugging.
3. **`#[instrument]` on `call_tool_chain` is a hot path** — the macro expansion allocates a span field map per call. Mitigation: `skip_all` + explicit `fields()` so only string constants and small numerics are captured.
4. **OTEL crate churn** — pre-1.0 minors break APIs. Pin `opentelemetry = "=0.32.x"` exactly for the first release; bump in its own PR after a soak period.
5. **PII in spans** — `params.code` for `call_tool_chain` is user-written TypeScript that may contain secrets/paths. Never add it as a span field; use `tracing::Span::current().record("code_bytes", code.len())` if any size info is wanted.

### concrete_prismgate_files_to_change

- `Cargo.toml` — add 4 opentelemetry crates, gated on `observability` feature; add to `default = [...]` only if you want it on by default (recommend **off by default** for the first release, on in dev profile).
- `src/main.rs:81-87` — replace inline subscriber init with `init_tracing()` helper.
- `src/main.rs:319-440` (run modes) — call `otel::shutdown().await` in each path before process exit.
- `src/config.rs` — add `ObservabilityConfig { otlp_endpoint: Option<String>, sample_ratio: f64, service_name: String }`.
- `src/server.rs` — annotate `search_tools`, `list_tools_meta`, `tool_info`, `call_tool_chain` (lines ~336, 397, 418, 573) with `#[instrument]`.
- `src/backend/mod.rs:535-716` — wrap `b.call_tool(...)` in `tracing::info_span!("backend.call", backend, tool, attempt).in_scope(|| ...)` or convert to `instrument` on a small helper.
- `src/tools/sandbox.rs` — top-level `handle_call_tool_chain_with_store` gets a span with `tool_count`, `intent`, `max_output_size`.
- `src/session/db.rs:295-312` and `:314-347` — `#[instrument(skip_all, fields(name = %event.name))]` on `record` and `search`.
- `src/backend/health.rs` — span `health_check` with `restart_count` and `circuit_open` bool.
- **NEW** `src/observability/mod.rs` — `init_tracing(config) -> Result<OtelGuard>`, `OtelGuard::shutdown()`.
- **NEW** `src/observability/layers.rs` — OTLP layer construction with default batch config.
- `docs/telemetry-strategy.md` — flip the "future work" framing; document `observability.otlp_endpoint`.

---

## Feature 2 — Token accounting per backend

### research_summary

Today PrismGate tracks **bytes** (`processed`, `returned`) but reports `estimated_tokens_saved = (processed - returned) / 4` as a single global number (`src/tracker.rs:273-274`). This is fine for the headline stat but loses:

- The input/output split (raw bytes counted do not distinguish the request side from the response side).
- Per-backend attribution (every byte is rolled into the session total).
- Encoding-aware accuracy (`/4` overshoots by ~30% on CJK and undershoots on code).

The Rust ecosystem has two viable token counters:

| Crate | Pros | Cons | Verdict |
|---|---|---|---|
| `tiktoken-rs` (anysphere) | full BPE, OpenAI-compatible, `count()` returns `usize` | half-implemented, slow on non-ASCII | usable but not great |
| `tiktoken` (zhuobing-li, lib.rs #91, 12.6k downloads/month) | 11 encodings across 5 providers, hand-written ASCII/CJK scanners, **5-49x faster than tiktoken-rs natively**, `count()` allocates nothing, used by 32 crates (10 direct) | large vocab tables bundled in binary (~5.5 MB) | **pick this** |

Per-encoding coverage the package ships: `cl100k_base`, `o200k_base`, `o200k_harmony`, `p50k_base`, `p50k_edit`, `r50k_base`, `gpt2` (OpenAI), `llama3` (Meta), `deepseek_v3`, `qwen2`, `mistral_v3`. That covers every backend PrismGate is likely to see in practice.

### code_approach

1. **Cargo.toml** — `tiktoken = "3"` behind `feature = "token-counting"` (off by default to keep release binaries small).

2. **`src/tokenizer/mod.rs`** (new) — `TokenCounter` enum with variants:
   ```rust
   pub enum TokenCounter {
       O200kBase,        // GPT-4o
       Cl100kBase,       // GPT-4 / 3.5-turbo
       Llama3,
       DeepseekV3,
       Qwen2,
       MistralV3,
       Heuristic,        // the existing /4 fallback, used when feature off
   }
   impl TokenCounter {
       pub fn for_model(model_hint: Option<&str>) -> Self { ... }
       pub fn count(&self, text: &str) -> u64 { ... }   // returns 0 for Heuristic on empty
   }
   ```
   `count()` holds a `OnceLock<tiktoken::Encoding>` per variant to avoid re-parsing the BPE table on every call. **The CRITICAL perf property** is that `tiktoken::Encoding::count()` does *not* allocate a token id vector, so it is safe to call from hot paths (verified in the lib.rs benchmark).

3. **Extend `CallTracker`** — add `input_tokens` / `output_tokens` fields that mirror `bytes_processed`/`bytes_returned` but per-backend:
   ```rust
   // src/tracker.rs
   token_counts: DashMap<String, TokenStats>,    // keyed by backend_name
   ...
   pub struct TokenStats {
       pub input_tokens: AtomicU64,
       pub output_tokens: AtomicU64,
       pub input_chars: AtomicU64,    // for backends we cannot tokenize
       pub output_chars: AtomicU64,
   }
   ```

4. **Wire the counters** at the two natural chokepoints:
   - **`BackendManager::call_tool`** (`src/backend/mod.rs:535`) — currently calls `tracker.record(...)`. Add `tracker.record_tokens(backend, input_tokens, output_tokens)` where the counts come from a new method on the `Backend` trait: `fn estimate_tokens(&self, args: &Value, result: &Value) -> (u64, u64)`.
   - The stdio backend (`src/backend/stdio.rs:244`) is the only one that **actually sees** structured request/response JSON. The HTTP backend (`src/backend/http.rs`) and cli-adapter also have JSON bodies. Implement `estimate_tokens` per-backend with `tiktoken::count()` when the backend declares a `model_hint` in its config; otherwise use `chars/4` and tag with `TokenSource::Heuristic`.

5. **`prismgate://stats` enrichment** — extend `SessionStats` with `per_backend_tokens: Vec<BackendTokenStat>` so users see `exa → in=12,300 out=4,200`, `context7 → in=8,900 out=1,800`, etc.

### impact_estimate

| Dimension | Today | After |
|---|---|---|
| Token attribution | global aggregate `/4` heuristic | per-backend, per-session, with model-specific encodings |
| CJK/code accuracy | ~±30% | ±5% (within BPE rounding) |
| Heap pressure on hot path | none | one `AtomicU64::fetch_add` per call, plus the BPE encode (~few hundred ns) |
| Binary size | 0 | +5.5 MB if feature on (vocab tables); 0 if feature off |
| First-call latency | 0 | ~30 ms (one-time `OnceLock` init for the BPE tables) — must be done at startup, not first call |

### effort_estimate

- **M** — the per-backend counters and the new resource field are simple; the *interesting* work is the per-backend `estimate_tokens` implementation across `stdio`, `http`, `cli_adapter`, `composite`. Realistically 3-5 days including tests.

### risks

1. **BPE tables are large** — keep the feature opt-in. Add `count-estimated-tokens` to `prismgate doctor` so users can verify their setup.
2. **Schema drift** — some backends stream SSE; partial responses can't be tokenized accurately. Mark `TokenStats::source = Streaming` and fall back to char-based.
3. **Privacy** — token counting reads the full request body. Confirm the `oauth` and `secrets` resolvers do not leak env values into the args; they shouldn't (they are separate from the MCP transport) but verify.
4. **Race on `OnceLock`** — first-call init is blocking. Wrap in `tokio::task::spawn_blocking` if startup latency matters; otherwise it's a one-time 30 ms hit.
5. **`call_tool_chain` returns TS-evaluated JSON, not the backend response** — tokenize the *backend* return value inside `Backend::call_tool`, not the wrapper. This is what the code approach above does.

### concrete_prismgate_files_to_change

- `Cargo.toml` — add `tiktoken = { version = "3", optional = true }`, wire `token-counting` feature.
- **NEW** `src/tokenizer/mod.rs` — `TokenCounter` enum + `for_model` resolver.
- **NEW** `src/tokenizer/heuristic.rs` — chars/4 fallback.
- `src/tracker.rs` — add `TokenStats` struct, `token_counts: DashMap<String, TokenStats>`, `record_tokens(backend, in_tok, out_tok)`, extend `session_stats()` and `BackendTokenStat`.
- `src/backend/mod.rs:535-716` — call `tracker.record_tokens` after a successful `b.call_tool`. Add `Backend::estimate_tokens(&self, args: &Value, result: &Value) -> (u64, u64)` to the trait at `src/backend/mod.rs:157`.
- `src/backend/stdio.rs`, `src/backend/http.rs`, `src/backend/cli_adapter.rs`, `src/backend/composite.rs` — implement `estimate_tokens` in each (use `TokenCounter::for_model(self.model_hint())`).
- `src/resources.rs:271-276` — add `per_backend_tokens` to the `stats` JSON output.
- `src/ipc/doctor.rs` — print "token counter: tiktoken o200k_base (5.5 MB)" line.
- `docs/token-efficiency.md` — new section explaining per-backend token attribution.

---

## Feature 3 — Cost attribution

### research_summary

Cost attribution is the natural follow-on to per-backend token counts. Today the gateway has zero cost data because it has zero pricing data. Three layers are needed:

1. **Price table** — per (provider, model) pair: input $/1k tokens, output $/1k tokens, currency.
2. **Pricing source** — either bundled static table (cheap, stale) or live lookup (expensive, fresh). The Rust ecosystem has no equivalent to Helicone's hosted model-cost API; the closest is the open-source `models.dev` JSON dataset (~hundreds of models, MIT-licensed, refreshed nightly).
3. **Cost event** — annotate each `record_tokens` call with `$cost = input_tokens * in_price + output_tokens * out_price`. Store in the existing `CallTracker` (already thread-safe, lock-free writes).

**Survey of comparable work** (all verified live, 2026):
- **Bifrost** (Maxim AI, open source, Go) — stores provider/model/status/latency/token/cost logs in SQLite by default, Postgres for production; queryable through built-in dashboard, REST API, WebSocket stream. They made observability a "first-class concern" — the direct analog to what PrismGate should do.
- **LiteLLM** — free/open-source for self-host, integrates with external tracing tools, has per-key/per-user budgets. Missing native trace viewer; uses Postgres/Redis. Their cost attribution is table-driven (`model_prices.yaml`).
- **Portkey** — 40+ data points per request via OTEL ingestion, workspace/team/user-level segmentation, no native cost calculation (must be derived from token counts × external pricing).
- **OpenRouter** — pay-as-you-go with prepaid credits, per-model API key dashboard, no native gateway.
- **Helicone** — closed source, only enterprise self-host, OpenInference conventions (not OpenTelemetry GenAI). Expensive.

**For a Rust-first MCP gateway that values self-hosting**: the LiteLLM-style bundled YAML table + per-call `($/tok_in × tok_in) + ($/tok_out × tok_out)` is the right balance. No external API dependency; pricing is explicit and auditable; users can override per-backend in `prismgate.yaml`.

### code_approach

1. **Bundled price table** at `config/model_pricing.yaml` (new), git-tracked:
   ```yaml
   - provider: openai
     model: gpt-4o
     input_per_1k: 0.0025
     output_per_1k: 0.01
     currency: USD
     updated: 2026-01-15
   - provider: openai
     model: gpt-4o-mini
     input_per_1k: 0.00015
     output_per_1k: 0.0006
     currency: USD
   - provider: anthropic
     model: claude-3-5-sonnet
     input_per_1k: 0.003
     output_per_1k: 0.015
     currency: USD
   # ... covers llama3, deepseek, qwen, mistral ...
   ```

2. **`src/cost/mod.rs`** (new) — `CostTable { entries: HashMap<(String, String), Price> }`, loaded at startup via `serde_yaml_ng`. Lookup:
   ```rust
   pub fn price_for(&self, backend: &str, model_hint: Option<&str>) -> Option<&Price>
   ```
   Falls back to `None` (no cost attribution), which is correct behavior — a backend without a price entry should not fabricate one.

3. **User override** in `prismgate.yaml`:
   ```yaml
   backends:
     exa:
       model_hint: openai/gpt-4o-mini
       cost_override:
         input_per_1k: 0.0001   # negotiated rate
         output_per_1k: 0.0004
   ```
   `cost_override` wins over the bundled table.

4. **Wire into `CallTracker`** — extend `TokenStats` (from Feature 2) with `cost_micros: AtomicU64` (micros-of-USD to avoid floating-point atomics):
   ```rust
   tracker.record_cost(backend, cost_micros);
   ```
   `session_stats()` exposes `total_cost_usd: f64 = total_cost_micros / 1_000_000.0`.

5. **`prismgate://stats` new fields** — `cost_per_backend: Vec<BackendCostStat { backend, input_tokens, output_tokens, input_cost_usd, output_cost_usd, total_cost_usd }>`.

### impact_estimate

| Dimension | Today | After |
|---|---|---|
| Cost visibility | none | per-backend, per-session, cumulative USD |
| Budget enforcement | none | already possible via `record_cost` aggregation; pair with `flood_guard`-style circuit on `cost_per_minute > threshold` (future) |
| Pricing data freshness | n/a | table snapshot + user override; `models.dev` re-fetch as opt-in |
| Accuracy | n/a | 100% accurate for backends declaring `model_hint`; "unknown" for the rest, which is correct |

### effort_estimate

- **S-M** — straightforward once Feature 2 (tokens) is done. ~2-3 days including the bundled YAML and per-backend override plumbing.

### risks

1. **Pricing table staleness** — gate the table on `updated:` dates; warn at startup if any entry is >90 days old. Provide `prismgate pricing refresh` to pull from `models.dev`.
2. **Currency confusion** — store everything in micros-of-USD by convention; convert at display time. Document this in `docs/secrets-and-config.md`.
3. **Backends without model_hint** — many MCP backends wrap arbitrary HTTP services (exa, tavily) and have no "model" to price. For these, accept that cost attribution is N/A and show `"model": null` in the resource.
4. **Float precision** — using micros-of-USD as the atomic unit sidesteps `f64::fetch_add` not existing; convert to `f64` only at the resource-render boundary.
5. **Negotiated/enterprise rates** — explicitly support per-backend `cost_override` so corporate users don't see misleading blended rates.

### concrete_prismgate_files_to_change

- **NEW** `config/model_pricing.yaml` — bundled snapshot.
- **NEW** `src/cost/mod.rs` — `CostTable`, `Price`, `load()`, `price_for()`.
- **NEW** `src/cost/override.rs` — parse `cost_override` blocks from `BackendConfig`.
- `Cargo.toml` — no new deps (reuses `serde_yaml_ng`).
- `src/config.rs` — add `cost_override: Option<Price>` to backend config schema; validate on load.
- `src/tracker.rs` — `cost_micros: AtomicU64` in `TokenStats`, `record_cost(backend, micros)`, extend `SessionStats`.
- `src/main.rs:99` — load `CostTable::load("config/model_pricing.yaml")` at startup; expose as `Arc<CostTable>`.
- `src/backend/mod.rs` — after every `b.call_tool`, look up price via `cost_table.price_for(backend, model_hint)` and record.
- `src/resources.rs:271-276` — `cost_per_backend` field in `stats`.
- `docs/secrets-and-config.md` — new "Cost attribution" section.
- `README_NEW.md` — call this out as a feature.

---

## Feature 4 — `prismgate profile` command

### research_summary

Today `prismgate doctor` (Unix-only, `src/ipc/doctor.rs`) is the only diagnostic CLI command and it only reports socket/PID/daemon-version state. The data that *would* go in a profile (per-backend latency p50/p95, call volume, error rate, token usage, cost) is **already collected** in `CallTracker` (`src/tracker.rs:107-148`, `session_stats()` at line 233). The gap is purely presentation.

PrismGate's CLI uses `clap` with subcommands (`src/cli.rs:49-103`); adding a `Profile` subcommand is the natural fit. The complication: `profile` reads from a **running daemon**, not from the local CLI invocation, so it must speak the IPC socket protocol — same path `status` uses (`src/ipc/status.rs`).

Render options to consider:
1. **Plain table** — `prettytable` or hand-rolled padding, fits terminal-first DX. Zero new deps.
2. **JSON** — already what `prismgate://stats` does; good for scripting.
3. **Markdown table** — paste-friendly for issue reports.

Recommend: **all three** behind `--format={table,json,md}` (default `table`).

### code_approach

1. **`src/cli.rs:49-103`** — add a new variant:
   ```rust
   Profile {
       /// Optional backend name to focus the report.
       #[arg(long)]
       backend: Option<String>,
       /// Output format.
       #[arg(long, value_enum, default_value_t = ProfileFormat::Table)]
       format: ProfileFormat,
       /// Reset tracker stats after reading.
       #[arg(long)]
       reset: bool,
   },
   ```
   `ProfileFormat { Table, Json, Markdown }` derives `clap::ValueEnum`.

2. **`src/ipc/profile.rs`** (new) — speaks the daemon socket, fetches a new `prismgate://profile` resource (see step 3), prints via `crate::cli::render::format_profile(...)`. Reuses the existing `ipc::socket::connect()` helper.

3. **New resource `prismgate://profile`** — add to `src/resources.rs:list_static_resources()` and `read_resource`:
   ```rust
   "prismgate://profile" => {
       let report = build_profile_report(&tracker, &backend_manager, &registry);
       Ok(text_resource(uri, &serde_json::to_string_pretty(&report)?))
   }
   ```
   `build_profile_report` joins three data sources:
   - `CallTracker::session_stats()` — totals, per-tool, per-byte savings.
   - For each backend in `CallTracker::backends_with_latency()`: latency p50/p95/p99 from `CallTracker::latency_stats(backend)`; call count = sum of usage for tools whose `backend_name == backend` (registry lookup); error count = `CallTracker::error_count_for(backend)` (new method, see step 4); token stats from Feature 2; cost stats from Feature 3.
   - Optional `?backend=foo` filter narrows the table.

4. **Tracker enrichment** (`src/tracker.rs`) — add error counter per backend:
   ```rust
   error_counts: DashMap<String, AtomicU64>,
   ```
   Update `record()` to increment both `usage_counts[tool]` and (if `!success`) `error_counts[backend]`. Public method:
   ```rust
   pub fn error_count(&self, backend: &str) -> u64 { ... }
   pub fn error_rate(&self, backend: &str) -> f64 { ... }   // errors / total
   ```

5. **`src/cli/render.rs`** (new, no clap dep) — pure formatting:
   ```rust
   pub fn format_table(report: &ProfileReport, writer: &mut dyn Write) -> io::Result<()>
   pub fn format_markdown(report: &ProfileReport, writer: &mut dyn Write) -> io::Result<()>
   ```
   Three tables side-by-side as columns: latency, traffic, cost. Example terminal output:
   ```
   Backend      Calls  Err%   p50ms   p95ms   p99ms   InTok    OutTok   $USD
   exa         1,243   1.2%    142     389     712   82.3k    14.1k   0.42
   context7      218   0.5%     87     201     340   18.7k     3.2k   0.09
   tavily        341   4.1%    612   1,840   3,200   9.1k     2.8k   0.03
   ────────────────────────────────────────────────────────────────────────
   Total      1,802   1.8%    180     520     980  110.1k    20.1k   0.54
   ```

6. **`--reset` semantics** — when set, send the same IPC message as `prismgate purge --yes` for the requested filter (whole daemon or single backend). The current `tracker.reset()` is per-process; extending to per-backend requires:
   ```rust
   impl CallTracker {
       pub fn reset_backend(&self, backend: &str) { ... }
   }
   ```
   that drops that backend's `latency`, `usage_counts` entries for its tools, and the backend's `TokenStats`/`cost_micros` row.

### impact_estimate

| Dimension | Today | After |
|---|---|---|
| Per-backend DX | log scraping | single terminal table |
| Issue triage | screenshot of MCP client | paste-ready markdown |
| CI integration | n/a | `prismgate profile --format=json` parses cleanly |
| Cost auditing | none | per-backend USD totals |

### effort_estimate

- **M** — most of the data is already in `CallTracker`; the work is rendering + IPC + per-backend reset. ~3-4 days.

### risks

1. **`prismgate profile` while daemon is down** — needs a clear "daemon not running" message. The `status` command already does this; copy its pattern (`src/ipc/status.rs`).
2. **Cost figures shock users** — backends without pricing data show `—`, which is correct. Document this.
3. **Per-backend reset is destructive** — require confirmation when `--reset --backend=X`. Default to refusing if no `--yes`.
4. **High-cardinality backends** — many PrismGate setups have 30+ backends. Default `--format=table` paginates at 20 rows, `--no-truncate` shows all.
5. **`CallTracker::reset()` is shared across the daemon** — `reset_backend` is a partial reset; confirm with the `purge` flow that it does not race with concurrent `record()` calls (the existing `reset_gate: Mutex<()>` already serializes; reuse it).

### concrete_prismgate_files_to_change

- `src/cli.rs:49-103` — `Profile` variant + `ProfileFormat` enum.
- **NEW** `src/ipc/profile.rs` — socket client.
- `src/ipc/mod.rs` — re-export `profile` module.
- **NEW** `src/cli/render.rs` — table/markdown/json formatters.
- `src/resources.rs` — register `prismgate://profile` resource + handler.
- `src/tracker.rs` — `error_counts: DashMap<String, AtomicU64>`, `record()` updates, `error_count()`, `error_rate()`, `reset_backend()`, new `ProfileReport` struct.
- `src/main.rs:329-440` — add `(Some(cli::Command::Profile { ... }), _) => ipc::profile::run(...)` arm.
- `docs/backend-management.md` — new "Profiling backends" section.

---

## Feature 5 — Session export/import

### research_summary

Session data already lives in **two** stores:
- `CallTracker` — in-memory (lost on restart unless persisted by `cache::save`).
- `SessionEventStore` — SQLite + FTS5 actor, gated on `session-store` feature (`src/session/db.rs`). Schema:
  ```sql
  session_events(id, session_key, category, event_type, name, payload,
                 outcome, timestamp, handle, bytes_avoided, bytes_returned)
  session_events_fts(category, event_type, name, payload, outcome, content='session_events')
  ```

There is no export path. The existing schema is **already portable** — SQLite files are by definition portable. The work is:
1. **Export**: copy the SQLite file to a path; emit the `CallTracker` usage/latency snapshot as JSON sidecar.
2. **Import**: point a new daemon at the SQLite file (or merge into the existing one).
3. **Sanitization**: redact env var values, secrets, and `payload` strings flagged as `category=secret`.

The session event store uses `rusqlite` with the `bundled` feature (currently absent — feature gates on `session-store`, which gates on `dep:rusqlite` without `bundled`). For export portability across machines, add `bundled` to `rusqlite` features so the SQLite file is byte-compatible regardless of system libsqlite.

### code_approach

1. **New CLI subcommand** `prismgate session`:
   ```
   prismgate session export [--output PATH] [--include-tracker] [--redact-secrets]
   prismgate session import PATH [--merge | --replace] [--yes]
   prismgate session info                          # counts, size, oldest/newest event
   prismgate session purge SESSION_KEY [--yes]     # existing, wire through CLI
   ```

2. **`src/ipc/session.rs`** (new) — three sub-routines:
   - `export(output, include_tracker, redact) -> Result<()>` — uses the `purge` IPC pattern. Asks the daemon to:
     1. If `redact`: walk `session_events` rows where `category='secret'` or `category='tool'` and `name` matches known secret names (`*_API_KEY`, `*_TOKEN`, etc.); replace `payload` with `***REDACTED***`. Use a single `UPDATE … WHERE …` so it is one transaction.
     2. Issue `VACUUM INTO 'output_path'` (`SQLite` ≥ 3.27). This produces a compacted, defragmented copy that is also more portable than a hot WAL file.
     3. If `include_tracker`: serialize `CallTracker::snapshot_usage()` and `CallTracker::snapshot_latency()` as a `tracker.json` sidecar (next to the `.sqlite3`).
   - `import(path, mode)` — atomic `cp` from `path` into the daemon's `prismgate_cache_home()/session-store/session_events.sqlite3`; open + `PRAGMA integrity_check`; on success, restart the actor (the actor holds the connection; design an `ActorCommand::Reload` in `src/session/db.rs`).
   - `info` — `SELECT COUNT(*), MIN(timestamp), MAX(timestamp) FROM session_events` + on-disk size.

3. **Tracker snapshot** in `src/tracker.rs`:
   ```rust
   pub fn snapshot_latency(&self) -> HashMap<String, LatencyStats> { ... }
   ```
   Already have `snapshot_usage()` at line 159.

4. **Import = reload** — extend `src/session/db.rs` actor command set:
   ```rust
   enum Command {
       ...
       Reload,  // close current connection, re-open from disk
   }
   ```
   Send `Reload` after `cp` lands; actor re-runs `migrate()`. Existing FTS triggers fire automatically because `migrate()` is idempotent.

5. **Schema portability note** — current `migrate()` (`src/session/db.rs:239-292`) uses `PRAGMA journal_mode = WAL`. WAL files are split into `*.sqlite3` + `*.sqlite3-wal` + `*.sqlite3-shm`. `VACUUM INTO` writes a single self-contained file, which is exactly what we want for export. Document this in the new `prismgate session` help.

### impact_estimate

| Dimension | Today | After |
|---|---|---|
| Debugging production | "send me your logs" | one .sqlite3 file |
| Cross-machine replay | impossible | trivial |
| Tracker persistence | cache-restore only (counts, not histograms) | full snapshot sidecar |
| Threat surface | n/a | redaction must be safe-by-default (off) |

### effort_estimate

- **M-L** — `prismgate session` is a new IPC surface, but each subcommand is small. The non-trivial piece is the actor `Reload` command + careful handling of `journal_mode` switching on hot WAL. ~4-5 days.

### risks

1. **Concurrent writes during export** — `VACUUM INTO` holds an exclusive lock briefly; the actor is single-threaded so no race. Document the lock window (~10ms-1s for huge DBs).
2. **Secret leakage** — default `redact=false` for backward compat (users opt in). Print a warning to stderr when secrets are detected but not redacted. Recommend `prismgate session export --redact-secrets` in the README.
3. **Schema version skew** — if a v2 daemon imports a v1 export, migration runs and may fail. Add `PRAGMA user_version` and refuse to import if the on-disk version is newer than the running daemon's migrator can handle.
4. **Atomicity on import** — `cp` is not atomic. Use `tempfile::NamedTempFile::new_in(target_dir)` + `persist()` for crash-safe move.
5. **Disk usage** — `VACUUM INTO` doubles disk briefly. Cap import size with `metadata.len()` check before copy.

### concrete_prismgate_files_to_change

- `Cargo.toml` — `rusqlite` features `["bundled"]` (always); add `tempfile` if not already (it is, in `dev-dependencies`; promote to runtime).
- **NEW** `src/ipc/session.rs` — `export`, `import`, `info`, `purge` IPC handlers.
- `src/session/db.rs` — add `Reload` command, expose `actor.reload()` sender; add `user_version` PRAGMA to `migrate()`.
- `src/tracker.rs` — `snapshot_latency()` returning `HashMap<String, LatencyStats>`.
- `src/cli.rs:49-103` — `Session { action: SessionAction }` subcommand variant; `SessionAction { Export, Import, Info, Purge }`.
- `src/main.rs:329-440` — dispatch the new subcommand.
- `src/ipc/mod.rs` — re-export `session` module.
- `docs/session-store.md` — new "Export & import" section.

---

## Feature 6 — Survey: Langfuse, Helicone, Bifrost (self-hosted feasibility)

### research_summary

| Platform | License | Self-host | OTEL native | LLM-specific | Verdict for PrismGate |
|---|---|---|---|---|---|
| **Langfuse** | MIT (core), enterprise for SSO/eval | yes, single Docker Compose; OTLP endpoint at `/api/public/otel` (Basic Auth, HTTP/JSON + HTTP/protobuf, **no gRPC**) | yes | yes (prompts, completions, token counts, cost) | **recommended primary** |
| **Helicone** | proprietary | only enterprise | OpenInference conventions, not OpenTelemetry GenAI | yes | avoid — closed-source, semconv mismatch |
| **Bifrost** (Maxim AI) | open source, Apache-2.0 (verify before adoption) | yes, Go binary, SQLite default, Postgres prod | yes, plugs into existing observability stack | yes (built-in dashboards, REST API, WebSocket) | **recommended benchmark** — direct architectural analog |
| **Arize Phoenix** | source-available, free self-host | yes | OpenInference conventions | yes | alternative if Langfuse semantics don't fit |
| **OpenObserve** | AGPL-3.0 | single binary | yes, vendor-neutral OTel-native | yes (LLM spans alongside infra) | best *consolidation* play; heavier than PrismGate needs alone |
| **Braintrust LOGS / gateway** | proprietary | yes (gateway is in beta, self-host needs enterprise) | partial | yes | monitoring-only — does not own the gateway path |

**Langfuse feasibility** — verified live against `langfuse.com/integrations/native/opentelemetry`:
- Endpoint: `POST {LANGFUSE_HOST}/api/public/otel/v1/traces` with Basic Auth.
- Transports: OTLP/HTTP+JSON and OTLP/HTTP+protobuf. gRPC unsupported as of 2026 (verify on the version you target).
- All Langfuse SDKs become unnecessary — OTel spans flow directly. **A `tracing-opentelemetry` subscriber configured with `opentelemetry-otlp::SpanExporter::builder().with_http()` pointing at Langfuse is enough.**

**Bifrost feasibility** — verified live against `getmaxim.ai/articles/complete-guide-to-llm-logging-otel-tracing-and-observability-in-bifrost`:
- 11 µs overhead at 5,000 RPS on a 3XL machine (40x faster than LiteLLM). Go, not Rust, so not embeddable.
- Storage: SQLite default for dev/self-host, Postgres for high-volume. **Same shape PrismGate already has** (rusqlite + optional Postgres via feature flag).
- Built-in REST + WebSocket query surface; this is what `prismgate://stats`/`profile` are reaching toward in spirit.

### code_approach

**Langfuse wiring** (with Feature 1 already in place):
```rust
// src/observability/exporters/langfuse.rs
pub fn build_langfuse_exporter(host: &str, public_key: &str, secret_key: String)
    -> opentelemetry_otlp::SpanExporter
{
    let endpoint = format!("{}/api/public/otel/v1/traces", host.trim_end_matches('/'));
    let auth = format!("{}:{}", public_key, secret_key);
    let auth_header = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(auth));
    opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .with_headers(HashMap::from([("Authorization".into(), auth_header)]))
        .build()
}
```
Auth keys come from the existing `secrets` resolver (`src/secrets/`) — never embed in `prismgate.yaml` plaintext.

**Bifrost integration pattern** — Bifrost speaks OTel out of the box, so the same OTLP exporter from Feature 1 works. Add a `observability.bifrost_endpoint` config field for symmetry with `langfuse_endpoint` if desired; in practice one endpoint is enough.

**Self-host boundary** — neither platform needs to be installed *inside* PrismGate. The integration is:
```
PrismGate (Rust, tracing-opentelemetry)
        │
        │ OTLP/HTTP+protobuf
        ▼
OTel Collector (optional)  ◄── can fan-out to multiple backends
        │
        ├─▶ Langfuse       (LLM trace UI)
        ├─▶ Prometheus     (metrics — derive from histogram exports)
        ├─▶ Tempo/Jaeger   (raw trace search)
        └─▶ Loki           (log aggregation)
```

**Helicone alternative is non-trivial** — their conventions are OpenInference, not OTel GenAI. Bridging would require a custom layer that renames span attributes (`gen_ai.usage.input_tokens` → `llm.usage.prompt_tokens` etc.). Not recommended unless there is a hard business reason.

### impact_estimate

| Outcome | Effort |
|---|---|
| Ship to Langfuse (cloud or self-host) | ~50 LOC after Feature 1 |
| Ship to Helicone | ~300 LOC of attribute translation + ongoing semconv drift |
| Ship to Bifrost | same 50 LOC (Bifrost speaks OTel natively) |
| Cross-platform via OTel Collector | 0 LOC in PrismGate — config-only on the collector side |

### effort_estimate

- **S** — Langfuse/Bifrost is a config change once Feature 1 lands. Helicone is the only outlier and is not recommended.

### risks

1. **Langfuse gRPC gap** — must use HTTP exporter, not tonic. Lock the dependency feature set.
2. **Auth in env vars** — load Langfuse keys via the existing `secrets` resolver; never accept them on the CLI.
3. **Schema attribute drift** — the OTel GenAI semantic conventions are still evolving (current draft is `gen_ai.*` namespace). Pin a version, validate on startup that required attributes (`gen_ai.system`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`) are present.
4. **Network availability** — if Langfuse is unreachable, `BatchSpanProcessor` should drop spans, not block the gateway. `BatchConfig::default().with_max_export_batch_size(512).with_max_queue_size(2048)` and `with_scheduled_delay(Duration::from_secs(5))` is the right shape.
5. **Cost** — Langfuse self-host on a 4-core box handles ~50k events/sec; well above what a single prismgate daemon emits.

### concrete_prismgate_files_to_change

- **NEW** `src/observability/exporters/langfuse.rs` — `build_langfuse_exporter()`.
- **NEW** `src/observability/exporters/collector.rs` — generic OTLP collector endpoint (the default).
- `src/observability/mod.rs` — choose exporter from `config.observability.backend`.
- `src/config.rs` — add `ObservabilityBackend` enum (`None`, `Otlp`, `Langfuse`), `endpoint`, `auth_env_keys`.
- `src/secrets/mod.rs` — expose `lookup_pair(pub_env, secret_env) -> (String, String)` for Basic Auth.
- `docs/telemetry-strategy.md` — replace "future work" with concrete quick-start: a single `docker compose up` snippet.

---

## Sequencing recommendation

| Wave | Features | Why |
|---|---|---|
| **1 (week 1-2)** | Feature 1 (OTEL spans) + Feature 6 (Langfuse wiring) | Unblocks every later feature; cheap; immediate debugging wins |
| **2 (week 3)** | Feature 2 (tokens) + Feature 3 (cost) | Tokens is the prerequisite; cost is thin glue once tokens exist |
| **3 (week 4)** | Feature 4 (`prismgate profile`) | Data is ready; render + IPC is the work |
| **4 (week 5+)** | Feature 5 (session export/import) | Largest surface, smallest immediate user impact; nice-to-have after the rest |

Each wave is independently shippable. Feature 6 is not a numbered deliverable — it is a configuration of Feature 1.

## Cross-cutting concerns

- **Config schema migration** — every feature above adds to `config.rs`. Use the existing hot-reload path (`src/config.rs::watch_config`); for new top-level keys, additive-only.
- **Tests** — `src/tracker.rs` already has good unit tests; mirror that density for the new `TokenStats`, `error_counts`, `reset_backend`. For `prismgate profile`, add an integration test in `src/mcp_compliance_tests.rs`.
- **Feature gating** — recommend:
  - `observability` (default off): `tracing-opentelemetry`, OTLP exporter, Langfuse.
  - `token-counting` (default off): `tiktoken`.
  - `cost-attribution` (default on): zero deps, just YAML + arithmetic.
  - `session-store` already exists.
- **Documentation** — update `docs/telemetry-strategy.md` (it's already explicit about future work), `docs/token-efficiency.md`, `docs/secrets-and-config.md`, `README_NEW.md`.
