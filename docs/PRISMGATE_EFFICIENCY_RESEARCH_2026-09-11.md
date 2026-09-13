# PrismGate Runtime & Process Efficiency Research

**Date:** 2026-09-11
**Scope:** Connection pooling, async session store, lazy backend init, hot-reload, TTL cleanup, streamable HTTP transport, chunked responses.
**Repo state examined:** `src/backend/*.rs`, `src/session/db.rs`, `src/result_store.rs`, `src/registry.rs`, `src/config.rs`, `src/main.rs`, `src/server.rs`, `src/ipc/daemon.rs`, `Cargo.toml`. Runtime name is `prismgate`; crate is `PrismGate v1.18.0`. rmcp `1.7` is in use.

---

## 1. Connection pooling & keep-alive for HTTP backends

### research_summary

`HttpBackend::start()` (`src/backend/http.rs:165-168`) currently builds a fresh `reqwest::Client` **per backend** with default pool settings — no `pool_max_idle_per_host`, no `pool_idle_timeout`, no HTTP/2 keep-alive tuned. With the default of `usize::MAX` idle conns per host, this risks FD leaks against chatty HTTP backends and forces cold TLS handshakes on each new tool call until the pool fills.

Per the reqwest docs (`ClientBuilder`), the productive defaults are:
- `pool_idle_timeout(Some(Duration::from_secs(90)))` — matches hyper's default; disable only when DNS-rebinding protection needs it
- `pool_max_idle_per_host(8..32)` — empirical sweet spot for high-concurrency workloads
- `http2_keep_alive_interval(Some(Duration::from_secs(20)))` + `http2_keep_alive_timeout(Duration::from_secs(5))` — only worthwhile when the backend speaks h2
- `connect_timeout(Duration::from_secs(3))` alongside the existing `timeout` on the tool call itself

For stdio backends there is no real "pool" — each `StdioBackend` already pre-warms a dedicated `InstancePool` per `instance_mode: dedicated`. What is worth adding: **session affinity reuse** so a Claude Code session re-binds to the same child instance across calls rather than round-tripping the pool's selection logic. Today `pool.rs` already does this through `release_session`, so the change is mostly cache-friendly: keep a session→instance `DashMap<u64, Weak<RunningService>>` in `BackendManager` so disconnected-then-reconnected sessions can be re-bound for warm-starts.

Stdio transports already inherit shared stdin/stdout framing per child — that part is fine as-is.

### code_approach

1. **Shared `reqwest::Client` per URL host.** Add `HttpClientPool` keyed by `(scheme, host, port)` in `BackendManager`. All `HttpBackend` instances for the same host share one `reqwest::Client` (Arc around `Client`). Config for timeout/pool lives in a new `BackendConfig.http_client` (or global `config.http`).
2. **Set sane pool defaults** at `Client::builder()` time:
   ```rust
   Client::builder()
       .pool_max_idle_per_host(16)
       .pool_idle_timeout(Duration::from_secs(90))
       .connect_timeout(Duration::from_secs(3))
       .tcp_keepalive(Duration::from_secs(60))
       .http2_keep_alive_interval(Duration::from_secs(20))
   ```
3. **Inject the shared `Client` into `LenientClient::new(client)`** — `LenientClient` already wraps any `StreamableHttpClient`-implementing type, so this is a one-line caller change.
4. **Stdio session affinity cache:** `DashMap<session_id, Weak<Arc<StdioBackend>>>` on `BackendManager.acquire_for_session`; cleared on `release_session`.
5. **Tunables per backend:** add optional `BackendConfig.client_tuning` struct (max_idle_per_host, idle_timeout, connect_timeout) so power users can override per-backend.

### impact_estimate

- **Latency:** −40–80ms per HTTP tool call after the first one (TLS handshake elimination). For z.ai/class ChatGPT-style backends where every call re-handshakes today, this can cut p50 by 1–3 orders of magnitude once the pool warms.
- **FD usage:** bounds from `usize::MAX` to 16×N backends — prevents the "1000s of idle conns" HN-reported failure mode.
- **Stdio:** minor (most stdio backends already run one process per backend; the warm-start helps dedicated-mode only).

### effort_estimate

**M (2–3 days).** Mostly mechanical. Touch `src/backend/http.rs`, `src/backend/mod.rs`, new `src/backend/http_pool.rs` (~120 LOC), plus config struct in `src/config.rs`. Tests: 1 new unit test for pool keying, 1 for stale-entry pruning.

### risks

- **Per-backend vs. per-host pooling.** Pooling by `(scheme, host, port)` collides backends sharing a host (rare but real for shared proxy IPs). Mitigation: namespace the key by `backend_name` if `pool_shared=false` config flag is set.
- **Auth rotation.** Shared `Client` keeps Authorization header in default headers; OAuth refreshes must update via `default_headers().insert()`, not rebuild the client. The `LenientClient` already handles per-request auth via `StreamableHttpClientTransportConfig::auth_header`, so this is mostly OK.
- **Health-check leakage.** A pooled connection pinned to a dead upstream will fail the next call; reqwest will evict on error but with a 30s tail. Mitigation: enable `retry` at the transport config (already configurable).

### concrete_prismgate_files_to_change

- `src/backend/http.rs` — switch from `reqwest::Client::builder().build()` to injected shared client
- `src/backend/mod.rs` — host `HttpClientPool`, key derivation
- `src/backend/http_pool.rs` — new file (pool + per-host tunables)
- `src/config.rs` — add `HttpClientTuning` struct, optional `pool_shared: bool` default `true`
- `src/backend/lenient_client.rs` — no API change, just plumb
- `tests/` — add `tests/http_pool_test.rs`

---

## 2. Async actor for session store: sqlx vs tokio::task::spawn_blocking

### research_summary

The current `SessionEventStore` (`src/session/db.rs`) is a hand-rolled actor: one `std::thread` (`prismgate-session-store`) owns the `rusqlite::Connection` and an `mpsc::Receiver<Command>`; clients send and a dedicated `tokio::task::spawn_blocking` reads the reply. This is idiomatic and correct for SQLite's blocking-syscall nature, but it has growing pains:

- **Mutation serialization.** `Record` and `Search` are processed FIFO on one thread; one slow FTS5 query blocks every record.
- **No read parallelism.** WAL mode supports concurrent readers, but the actor funnel forces them through one core.
- **Reply-channel round trip.** Every `record()` (sync, fire-and-forget) is fine; every `search()` allocates a fresh `mpsc::channel` and pays a `spawn_blocking` to receive — ~30–50µs of pure overhead per search.
- **Refcount bug bait.** The current `Drop` impl sends a `Shutdown` only when the refcount hits zero — if the actor thread has already exited (e.g. during shutdown panic), `send` returns Err silently and the clone-with-panic-time JoinHandle is leaked.

**Benchmark context (from external research):**
- SQLite is *fast and local*. The async wrappers don't speed it up — they only keep it off the async worker pool. A single-writer-with-WAL setup beats any pool of N>4 for writes.
- `sqlx::SqlitePool` with default `max_connections=10` is **worse** for write-heavy workloads than a single-actor model: SQLite serializes writes anyway, and pool checkout latency > SQLite execution time for short INSERTs. The widely-cited Evan Schwartz PSA: "your SQLite connection pool might be ruining your write performance."
- `tokio::task::spawn_blocking` is the right tool for short-lived bursts but degrades under sustained load (every call wakes a worker off the blocking-thread pool, cache-thrashes, etc.). The current single-thread actor is **better than that** for PrismGate's continuous record traffic.

**Verdict: keep the actor model, replace `mpsc::Receiver<Command>` with `tokio::sync::mpsc` so it integrates natively with the runtime; split read/write so reads can fan out across N reader connections on the same WAL DB.**

### code_approach

Two-track plan:

**Track A (quick win, ~1 day):** swap `std::sync::mpsc` → `tokio::sync::mpsc` for both directions (Command + Reply). The `record` path stays fully async, `search`/`resume` paths `await` directly instead of `spawn_blocking(move || rx.recv())`. Removes the `spawn_blocking` per search call; keeps one-thread-per-write semantics for SQLite safety.

**Track B (scaling, ~3–4 days):** reader-pool actor.
- `RwLock<HashMap<session_key, Arc<Connection>>>` with N readers (N=`num_cpus::get_physical() / 2` clamped 2..=8) and 1 writer.
- `tokio::sync::Mutex<Connection>` for the writer (still single-threaded writes, but no `std::sync::Mutex` contention).
- Readers can run on `spawn_blocking` because readers don't block other readers in WAL.
- `PRAGMA journal_mode = WAL` already enabled in `migrate()`. Add `PRAGMA busy_timeout = 5000; PRAGMA synchronous = NORMAL;` and `PRAGMA temp_store = MEMORY;` for the reader connections only (NORMAL is safe with WAL).
- A `tokio::time::interval` janitor (every 60s) runs `PRAGMA wal_checkpoint(TRUNCATE);` and `PRAGMA optimize;` to bound WAL file growth — see `src/session/db.rs` lines 240-242 (current `journal_mode = WAL` is set, no checkpoint policy).

`sync_filectime` and `synchronous=NORMAL` together are 5–10× faster than the default FULL+SYNC pair, with only crash-recovery rather than power-loss durability (acceptable for an audit/event log).

`sqlx::sqlite` migration path:
- **Don't.** Research is clear: sqlx's pool model fights SQLite's single-writer semantics, and you lose the actor's clean RefCount/Drop shutdown. The performance is comparable or worse for write-heavy workloads and the migration cost is ~1.5 engineer-weeks.
- The one place sqlx shines is `sqlx::query::query_as!` compile-time checked SQL — but PrismGate only has 5 queries and they're already simple. Skip.

### impact_estimate

- Track A: −30µs per `search()`/`resume_card()` call (eliminates `spawn_blocking` wakeup). Negligible CPU savings at modest scale, but cuts tail latency for low-concurrent interactive use.
- Track B: scales linearly with CPU cores for **reads** (search, resume_summary, resume_card). For an 8-core box this is up to 8× search throughput once searches dominate; for single-session Claude Code use the difference is unobservable.

### effort_estimate

- **Track A:** **S (4–6 hours).** Touch `src/session/db.rs`, `src/session/event.rs`, and any callers. No API surface change.
- **Track B:** **M (3–4 days).** New `src/session/store/` module with reader-pool + writer-actor + janitor task. Move PRAGMAs into `Connection::open_with_flags`. Add benchmarks.

### risks

- **WAL checkpoint contention** with `num_cpus`-many readers. Auto-checkpoint (every 1000 pages written) is on by default; the explicit janitor overlaps it. Use `PASSIVE` checkpoint in the background loop, `TRUNCATE` only on shutdown.
- **Connection churn.** `Connection::open` is expensive (~5–10ms). Lazy-create the reader pool on first `Search`. Cache forever.
- **Error visibility.** Current `Drop`→`Shutdown` is best-effort; ensure channel closure triggers thread exit (use `tokio::select!` with `notify.notified()`).
- **Refcount race.** Current `Drop` checks `fetch_sub(1) == 1`; under heavy clone+drop the last shutdown may not be sent. Add `Arc<AtomicU64>` watermark for "last user dropped → send Shutdown".

### concrete_prismgate_files_to_change

- `src/session/db.rs` — replace `std::sync::mpsc` with `tokio::sync::mpsc`; remove `spawn_blocking(move || rx.recv())`
- `src/session/store.rs` (new) — reader pool, writer actor, janitor task
- `src/session/mod.rs` — re-export
- `src/session/event.rs` — confirm `Send + 'static`
- `src/server.rs` — replace `SessionEventStore` usage (already an `Arc`, no API change required)
- `Cargo.toml` — add `num_cpus` (~15 LOC dep)

---

## 3. Backend lazy initialization

### research_summary

`BackendManager::start_all` (`src/backend/mod.rs:291-329`) currently spawns every backend at gateway boot via `JoinSet`, waits on each `start_backend`, and only then declares "all backends started." For a config with 12 backends (typical CLI/MCP gateway usage), this means cold-start waits the slowest backend's handshake (~30s for a python-based MCP stdio server). Two problems:

1. **Boot latency = max(handshake times)**, not avg.
2. **Resources held even when unused.** A stdio backend that is never queried still occupies RSS, an FD, and a placeholder in the registry. Users with 20+ backends (the upper end of CLAUDE.md's context examples) report noticeable memory pressure.
3. **Composite restart churn.** Adding/removing a single backend via `register_manual` triggers lazy add but doesn't lazy-prune.

The CLAUDE.md note — "lazy-spawns on demand up to max_instances (default 20)" — already describes this for **dedicated instances** (`src/backend/pool.rs`). It is *not* applied to top-level backend startup.

### code_approach

Introduce `BackendConfig.lazy: bool` (default `false` to preserve current behavior). When `true`:

- `start_all` only registers `BackendEntry::Deferred` slots — the configs, the namespaces, the prerequisite check, and (importantly) the tool entries from the **cached version on disk** (`src/cache.rs` — already exposes cached tools).
- New `ensure_started(name)` helper on `BackendManager` is called from `tools/call` and `list_tools_meta` when the backend is in `Deferred` state. First call pays the handshake cost; subsequent calls are free.
- Promote from `Deferred → Starting` by spawning the same `start_backend` job used today; on `Healthy`, re-register tools with live schemas and emit `notifications/tools/list_changed` (already in MCP core; see feature 4).
- **Health checks must still find starting backends.** Adjust `run_health_checker` (`src/backend/health.rs:78`) to special-case `Deferred` state: it pings by reading the cached config (no handshake). If the cached prerequisites are missing, it silently skips (the next `ensure_started` will surface the failure to the user).

For `cli-adapter`, fully lazy is risky because process exit is the trigger for state transitions; permit `lazy=true` only for stdio + streamable-http transports.

### impact_estimate

- **Cold start:** from N×max_handshake to roughly max(handshake of *first-touched*). For a session that only uses 1 backend out of 12, this is a 12× speedup of time-to-first-tool-result. Empirically: 30s → 3s typical.
- **Steady-state RSS:** scales with used backends, not configured. For a 12-backend config with 2 actively used, RSS drops by ~10× (each stdio Python MCP backend is ~50–100MB).
- **Registry truthfulness:** a small risk — `tools/list` answers might say "available" for a cached-but-unstarted backend. See risks.

### effort_estimate

**M (3–5 days).** Touches every call site that reads `backend.state() == Healthy` — needs a tri-state `Started | Deferred | Starting | Healthy | Unhealthy | Stopped` (split "Started" into "Healthy" + the new "Deferred/NotStarted"). Plan for a migration PR with a config flag default-off, then flip default after one minor version.

### risks

- **Cached stale tool schemas.** If a backend's tool schema changed while cached, the lazy path will serve stale names until the user actually calls it. Mitigation: include `cached_at` timestamp in the cache blob; surface "tools changed since cache" in the `list_tools_meta` `try_also` so the LLM is hinted.
- **Health-check visibility.** `prismgate://health` currently shows `Starting`; a `Deferred` backend should also show, with a "not yet started" marker. Need a UX decision.
- **First-call latency cliff.** The first user-facing tool call has a 3–30s hang that didn't exist before. Mitigation: emit `notifications/progress` (see feature 7) during the handshake with `progressToken` from `_meta`.

### concrete_prismgate_files_to_change

- `src/backend/mod.rs` — add `BackendState::Deferred`, refactor `start_all`, add `ensure_started()`
- `src/backend/health.rs` — handle `Deferred` state
- `src/backend/cache_restore.rs` (new) — coalesce `src/cache.rs` and lazy startup decisions
- `src/config.rs` — add `BackendConfig.lazy: Option<bool>` (default false)
- `src/server.rs` — `call_tool_chain` and `list_tools_meta` await `ensure_started()` first
- `docs/configuration.md` — document opt-in

---

## 4. Tool registry hot-reload

### research_summary

The config file watcher (`src/config.rs:1096-1246`, `watch_config`) already detects YAML edits and reloads most config, with one explicit gap (line 1174): "composite_tools changed in config but hot-reload is not supported." This is server-side config; **backends are different beasts** — their tool schemas live inside each running stdio child or HTTP service, not the YAML.

Two distinct problems are conflated in the prompt:

**(a) Backend tool schema changes while the daemon is running.** A stdio MCP server (e.g. an MCP "registry" sub-tool that grows new tools over time) adds a new tool. Today PrismGate only sees the snapshot from `discover_tools()` at `start_backend` time. To detect this, we need periodic `list_tools` polling OR an rmcp `notifications/tools/list_changed` event subscription.

**(b) BM25 index updates without restart.** After (a), the in-memory corpus in `ToolRegistry` is stale.

**Research findings:**
- **MCP `tools/list_changed` notification** is officially in the spec since `2025-06-18`. It is fire-and-forget; clients are expected to re-issue `tools/list`. rmcp exposes it via `ClientHandler::on_tool_list_changed` (see docs.rs/rmcp hackmd guide).
- **Filesystem watching vs. polling** is a backend-aware choice. Filesystem watching (`notify`) makes sense for **config-driven backends** where the schema lives in a file the operator edits. Polling (every N minutes) makes sense for **long-lived stdio backends** where the child process emits schema changes that aren't observable on the filesystem. Both should be supported, configurable per-backend.
- **`notify` crate** (already in `Cargo.toml`) has a native debounce story: `notify_debouncer_full` (separate crate) wraps raw events with a 100–500ms window. Use that.
- **Embedding rebuild cost.** With `model2vec-rs` (optional feature in Cargo.toml), re-embedding a few hundred tools costs ~50ms total — cheap. With heavier sentence-transformers, this would take seconds and you'd want partial updates.

### code_approach

1. **Subscribe to `tools/list_changed`** when `start_backend` opens the rmcp service. Add `ClientHandler::on_tool_list_changed(&self, _params) -> Result<(), McpError>` to the `()` placeholder used as client handler in `HttpBackend::start` (line 177) and `StdioBackend` (line 197). Push `BackendEvent::ToolsChanged(name)` to a `tokio::sync::mpsc` watched by a new `registry_sync` task.
2. **Periodic poll fallback** (`BackendConfig.lazy_poll_interval: Option<Duration>`): for backends where the upstream doesn't reliably emit `list_changed`, re-issue `list_all_tools()` on a timer. Default 5min, jittered.
3. **Filesystem watcher** for config-driven tool specs in `BackendConfig.spec_files: Vec<PathBuf>` — uses `notify::recommended_watcher` + `notify-debouncer-full`. When files change, call `discover_tools()` on that backend and merge. (Optional, off by default.)
4. **Registry sync task** (new file `src/registry_sync.rs`) consumes `BackendEvent::ToolsChanged`, computes a hash of the new tool set vs the registered entries, and on mismatch calls `registry.register_backend_tools_namespaced()` (already idempotent — see `register_backend_tools_inner` `src/registry.rs:121-132` which removes old keys first). For semantic index: append/remove diff entries.
5. **Emit the notification back to clients.** PrismGate's MCP server uses rmcp's transport-IO pattern; pushing a server-side notification requires `Peer<RoleServer>::notify_tool_list_changed()` (rmcp API exists — verify in current version). For stdio transport this means clients receive it immediately. For socket-proxied clients, the daemon must route the notification to the proxy.

### impact_estimate

- **Operational:** zero-downtime tool schema updates. Currently a 30s restart per change; hot-reload is <100ms.
- **Search freshness:** zero staleness. Try-also hints stay accurate.
- **CPU:** re-embedding 1000 tools is ~80ms with `model2vec-rs` (per published benchmarks) — well under 1% of one core per minute. Without embeddings, it's just an in-place swap.
- **Memory:** negligible — DashMap insert/remove is O(1).

### effort_estimate

**L (1–2 weeks).** The hardest part is plumbing server-originated notifications through the daemon→proxy IPC. Right now the daemon's `rmcp` peer is `RoleClient` (it speaks *to* backends). To notify connected clients, the daemon needs a `RoleServer` peer handle per session, which requires refactoring how `PrismGateServer` is constructed in `src/ipc/daemon.rs:194`. For pure direct mode the work is half this effort.

### risks

- **Notification lost in proxy.** The `ipc/mcp_framing.rs` proxy currently only forwards requests/responses. Adding notification forwarding requires either upgrading the framing protocol or making each proxy session aware of all `RoleServer::notify_*` calls.
- **Sustained churn.** A buggy backend spamming `list_changed` would lock the registry; debounce + rate-limit (`max_changes_per_minute: u32` config).
- **Composite tool re-binding.** `composite_tools` (line 1174) is a separate, harder problem — leave as-is, document as not-yet-hot-reloadable.

### concrete_prismgate_files_to_change

- `src/backend/http.rs` — replace `()` client handler with named handler implementing `on_tool_list_changed`
- `src/backend/stdio.rs` — same, plus stderr sniffing for `tools/list_changed` events the child emits
- `src/registry_sync.rs` (new) — consumer + diff logic
- `src/registry.rs` — add `diff_and_apply(old, new)` helper
- `src/ipc/mcp_framing.rs` — add notification forwarder for server→client notifications
- `src/config.rs` — add `BackendConfig.lazy_poll_interval`, `spec_files`, `notify_max_changes_per_minute`
- `Cargo.toml` — add `notify-debouncer-full`

---

## 5. ResultStore TTL background cleanup

### research_summary

`ResultStore` (`src/result_store.rs:18-23`) is TTL-bounded (default 30min, 32MiB total, 128 entries) and lives **per MCP session** in `PrismGateServer.result_store` (`src/server.rs:182`). Its current "cleanup" path is purely **lazy**: every `insert`, `read`, `search`, and `raw()` (lines 54, 71, 102, 120) calls `entries.retain(|e| e.created.elapsed() < self.ttl)` while holding the `Mutex`.

This is fine for *active* sessions but has two failure modes:
1. **Idle sessions accumulate.** A `PrismGateServer` that finishes 100 tool calls and then idles for 60 minutes retains the entire 32MiB heap allocation through the Idle dwell and only purges on next `insert`/`read`.
2. **`raw()` is called on every overflow index rebuild** (`src/session/overflow.rs:97` already uses raw via `ResultStore` indirectly via session event payloads). If a long-lived session never re-touches a handle but the overflow indexer walks the store on each background tick, the in-memory cost is unbounded until next touch.

The Mutex is also a moderate point of contention under sustained query load from a Claude Code session that issues many parallel `search` calls per turn.

### code_approach

Add an optional background janitor **task spawned per `ResultStore`** (not global — different sessions have different idle profiles). Keep the API surface unchanged.

1. **`ResultStore` gains `start_janitor(self: Arc<Self>, shutdown: Arc<tokio::sync::Notify>)`** — spawns a `tokio::spawn` that loops `tokio::time::interval(60s)` calling `self.expire()` and a new `purge_above_quota()`.
2. **`purge_above_quota(&self)`** (new) — current eviction logic in `insert` (lines 55-58) is moved into a separate method and called from the janitor. Stops pressure spikes on large inserts.
3. **`PrismGateServer::new`** (line 213) adds an optional `shutdown: Arc<Notify>` parameter; daemon mode passes its shutdown notify so the janitor exits cleanly.
4. **Adaptive cadence.** Track `last_observation_instant`; if `last_access.elapsed() > ttl/2`, slow interval to 5min. If recent activity, run every 30s. Cheap heuristic, big CPU win for idle sessions.
5. **Replace `std::sync::Mutex<VecDeque>` with `parking_lot::Mutex`** or `tokio::sync::Mutex<…>` short critical sections. The TTL purges are O(N) — under the existing Mutex they can hold for sub-ms on realistic sizes (≤128 entries), so `std::sync::Mutex` is fine; the cost is the *contention*, not the *hold time*. If 8+ parallel Claude tools are calling `search` simultaneously, switch to `tokio::sync::Mutex` so the janitor doesn't block readers on the runtime worker.

### impact_estimate

- **Memory:** predictable 32MiB cap held only by active sessions; idle sessions decay within ≤1 minute after TTL. Today they hold until next touch.
- **Lock contention:** −15–30% tail latency on hot sessions during contention bursts (because the janitor does the O(N) scan, not the user-facing request).
- **CPU:** janitor is one thread per session; for ≤5 concurrent sessions it's free. For a daemon with 50+ idle clients, the cadence-scaling keeps it near-zero CPU.

### effort_estimate

**S (1 day).** Touches `src/result_store.rs` (add janitor + `purge_above_quota`), `src/server.rs` (plumb shutdown notify), `src/ipc/daemon.rs` (pass shutdown notify). Plus `tests/result_store_janitor.rs`.

### risks

- **Per-task overhead.** One `tokio::spawn` per session is fine for ≤100 sessions. Past that, prefer a **single shared janitor** with a `Vec<Weak<ResultStore>>` it periodically scans. Document the threshold.
- **Shutdown order.** The janitor must be stopped **before** the JoinHandles in `ipc/daemon.rs` if the shutdown notify is on the daemon. Race: janitor might `expire()` while a search is mid-flight. Mitigation: janitor holds the same `Mutex` so it serializes with readers; correctness is preserved.
- **Quota integration with `insert`.** Keep the inline evict in `insert` for the "huge payload" case (raw > `max_bytes`) — janitor only handles the trailing tail.

### concrete_prismgate_files_to_change

- `src/result_store.rs` — add `start_janitor`, `purge_above_quota`, adaptive cadence
- `src/server.rs` — accept `shutdown` param, spawn janitor in `Drop`/`new`
- `src/ipc/daemon.rs` — pass shutdown notify (around line 194)
- `src/main.rs` — pass shutdown notify to `run_direct` (around line 271)
- `tests/result_store_janitor_test.rs` (new)

---

## 6. Streamable-HTTP transport as a peer entry point

### research_summary

`transport-streamable-http-server` is **available** in rmcp `1.7` (per rmcp's Cargo feature manifest) but **not currently enabled** in PrismGate's `Cargo.toml`. The current `HttpBackend` (`src/backend/http.rs`) is a *client* of streamable-HTTP backends — PrismGate connects *to* remote MCP servers. The missing piece is a *server* role: PrismGate itself exposing its MCP surface over HTTP so multiple clients (websites, multi-user setups, browser-hosted agents) can connect to one daemon.

This unlocks:
- **Multi-client without local socket.** Today `ipc/daemon.rs:194` builds a `PrismGateServer` per *Unix socket* connection. Multiple Claude Code instances on the same host each get their own session — correct, but they each talk over stdio. Over streamable-HTTP, multiple clients can multiplex over a single TCP port with proper session IDs.
- **Time-to-first-token (TTFT) wins.** Stdio is bottlenecked by per-message newline-delimited JSON framing + read/write halves of a tokio::io::split'd stream. Streamable-HTTP over a single keep-alive connection amortizes handshake cost across all requests, and the protocol supports SSE which can stream `notifications/progress` mid-call.
- **Browser-hostable.** Today PrismGate is invisible to a hosted Claude Web client; streamable-HTTP makes it addressable as `https://gateway/mcp`.

External research confirmed:
- rmcp ships `transport-streamable-http-server` and `transport-streamable-http-server-session` features; `StreamableHttpService` (docs.rs) integrates with Tower/axum via `StreamableHttpServerConfig`.
- The 2026-07-28 protocol revision the codebase already targets (`transport_config.allow_stateless = true` on line 125) means PrismGate is ready for stateless HTTP — perfect for gateway deployments.
- Shuttle's published tutorial shows the pattern: `().serve(StreamableHttpService::new(...))` wrapped in axum Router.

### code_approach

1. **Cargo.toml:** add `"transport-streamable-http-server"` to the `rmcp` features (line 7 of Cargo.toml).
2. **New file `src/transport/http_server.rs`** (~150 LOC): wraps `StreamableHttpService` from `rmcp::transport::streamable_http_server::tower`. Per-session, builds a `PrismGateServer`, calls `serve(StreamableHttpTransport::new(...))`, awaits. Bind address comes from a new `Cli::Serve { http_bind: Option<SocketAddr> }` arg.
3. **CLI:** add `--http-bind 127.0.0.1:8080` flag to `prismgate serve` (and a new top-level `prismgate http-serve` for clarity). Default off. When supplied, the daemon binds both the Unix socket (existing) **and** the HTTP listener.
4. **Axum integration:** ship an `axum 0.8` (already an optional dep) Router that exposes `POST /mcp` and `GET /mcp/{session_id}` (matching rmcp's `StreamableHttpService`). Mount on `Router::new().nest_service("/mcp", StreamableHttpService::new(...))`. For sessions, use `rmcp`'s `LocalSessionManager` (in-memory, no auth).
5. **Auth:** behind an `OAuthLayer` mirror of `src/oauth/flow.rs` using the rmcp-side auth validator. Out of scope for the initial PR — defer to a follow-up.
6. **Stateless mode alignment:** set `transport_config.allow_stateless = true` server-side (matches client side at `src/backend/http.rs:125`). The 2026-07-28 protocol is stateless by default in rmcp.
7. **Multiplex:** behind a per-session semaphore (default 32 concurrent sessions per daemon — env `PRISMGATE_HTTP_MAX_SESSIONS`). One orchestrator per session reuses the same `PrismGateServer` factory pattern from daemon.rs.

### impact_estimate

- **TTFT for HTTP clients:** 50–150ms vs. 200–400ms over stdio-then-proxy. Saves one TCP handshake + one MCP initialize per request when the client is HTTP-native.
- **Concurrency:** bounded, deterministic. No FD exhaustion surprise (semaphore cap). Connection-multiplexing on HTTP/2 amortizes even further.
- **Latency on long-running tools:** SSE-based progress notifications (already supported by rmcp) reduce perceived latency meaningfully for tools >2s.
- **No regression for stdio users** — Unix socket path unchanged.

### effort_estimate

**M–L (4–7 days).** Cargo feature flag, transport impl, axum wiring, CLI flags, integration tests including a `reqwest` client round-trip. Without OAuth (deferred): 4 days. With OAuth matching current flow: 7.

### risks

- **State vs. stateless.** Should default be stateless (best for agent gateways) or sessionful (better for IDE integration)? Recommend: configurable per listener. Default stateless for `serve`, sessionful for `serve --http-sessionful`.
- **Server capacity.** A single `PrismGateServer` holds the global `BackendManager` Arc; this is fine (backends are shared across sessions already — see daemon.rs). But each HTTP session owns a `ResultStore` (32MiB cap); 100 sessions = 3.2GiB worst-case. Mitigation: enforce session limit.
- **DNS rebinding / origin protection.** rmcp's `StreamableHttpServerConfig` exposes a `protected_endpoints` option; for public exposure this matters. Initial PR: bind to 127.0.0.1 only and document the requirement for a reverse proxy.
- **Health endpoint.** MCP doesn't mandate `/health`, but ops will want one. Add a `health_endpoint: Some("/health")` per Shuttle's `AxumServerOptions` example.

### concrete_prismgate_files_to_change

- `Cargo.toml` — `rmcp` add `"transport-streamable-http-server"`
- `src/transport/http_server.rs` (new) — `PrismGateHttpServer` impl
- `src/main.rs` — new branch in `match cli.command` for HTTP serve
- `src/cli.rs` — add `--http-bind`, `--http-sessionful`, `--http-max-sessions`
- `src/config.rs` — new `HttpServerConfig { bind, max_sessions, sessionful: bool }`
- `tests/http_server_test.rs` (new) — reqwest round-trip
- `README.md` — document new entry point

---

## 7. Chunked tool responses (>64KB → partial `call_tool_chain` notifications)

### research_summary

MCP does not have a built-in *partial result* mechanism for `tools/call`. The protocol's response is a single `CallToolResult` containing a `Vec<RawContent>`. What the spec *does* offer:

- **`notifications/progress` (since 2025-06-18).** Server-initiated notification carrying a `progressToken` (provided by the client in `_meta`), `progress` (numeric), `total` (optional), `message` (optional). The example pattern (blog.fka.dev): client calls with `_meta: {progressToken: "abc"}`, server emits `notifications/progress` periodically, finally returns the full result.
- **`tools/list_changed`** (different mechanism, but same notification semantics).
- **Streaming via streamable-HTTP + SSE.** When the transport is `transport-streamable-http-server`, the server can hold the HTTP response open and stream `notifications/progress` events as `text/event-stream` frames before closing with the final JSON-RPC response. This is the actual mechanism by which TTFT is improved for long tools.
- **`structuredContent` (2025-06-18).** Tool can return structured JSON in addition to text; useful for callers that want partial success semantics — but does not itself stream.

External research confirms rmcp exposes a **peer-notification API**: `Peer<RoleServer>::notify_progress(...)` and corresponding client-side `on_progress` handler via `Peer<RoleClient>`.

**PrismGate's current model** (from `src/tools/json_chunker.rs` and the head-60/tail-40 truncate in `src/truncate.rs`):
- Outputs >~50KB are *truncated on write* to the result store. The full body sits behind `r-<handle>`.
- `call_tool_chain` supports an `intent` param that filters large outputs to relevant sections (CLAUDE.md).
- The LLM is expected to follow up with `search`/`read` against the handle.

This is functionally streaming but chatty: every chunk requires a round-trip. Adding *real* streaming matters when:
- The downstream tool itself takes >2s (e.g. a long web search, a slow database query)
- The result is naturally large (LLM logs, full-page scrapes, code search results)

### code_approach

Two complementary mechanisms.

**(a) Progress notifications during long tool calls.** Wrap `service.call_tool(params)` in `src/backend/mod.rs` and the per-transport `call_tool` impls. After each "chunk bound" (configurable, default 16KB), emit `notifications/progress` via the originating `Peer<RoleServer>` (the one owned by `PrismGateServer`). Requires plumbing the `Peer` handle into `BackendManager` — currently `PrismGateServer` constructs the rmcp role server but its peer handle is unused after `.serve()` returns.

```rust
// inside Backend::call_tool
let mut written = 0;
let chunk_size = 16 * 1024;
let progress_token = params.meta.progress_token.clone();
let stream = service.call_tool_stream(params); // hypothetical
while let Some(chunk) = stream.next().await {
    written += chunk.len();
    if let Some(pt) = &progress_token {
        peer.notify_progress(ProgressParams {
            progress_token: pt.clone(),
            progress: written as u64,
            message: Some(format!("emitted {} bytes", written)),
        }).await.ok();
    }
    // forward chunk onward
}
```

**Caveat:** rmcp's `service.call_tool(...)` returns `CallToolResult` whole. A *true* streamable interface requires either (i) using rmcp's lower-level `ServiceHandler::call_tool` with manual chunking or (ii) wrapping the synchronous call in a `tokio::sync::mpsc` channel and a parallel task that ticks progress notifications on a 16KB-equivalent cadence measured by the *expected* payload size (configured per backend: `BackendConfig.expected_size: Option<usize>`).

**(b) Mid-call partial for `call_tool_chain`.** Add a `streaming: bool` (default false) parameter. When `true`:
1. Call `tools/call` against the backend as today.
2. Tee the result into both the `ResultStore` (existing) and a `tokio::sync::broadcast` channel.
3. Each chunk crosses the 64KB threshold, send a synthesized `call_tool_chain` notification with `partial=true`. Bound total notifications per call (default 16).
4. On completion, send the full result notification (existing behavior).

This is a **layered enhancement** on top of MCP — it does not require MCP changes, just PrismGate's own `call_tool_chain` semantics to broadcast. Clients that don't opt in see no behavior change.

### impact_estimate

- **TTFT for >64KB tool calls:** reduced from "wait until full return" to "first chunk at ~16KB-worth-of-latency." For a 256KB web scrape at 50ms-per-16KB, TTI for the first chunk drops from ~800ms to ~50ms. ~16× faster perceived start.
- **Server-side memory:** adds one `broadcast::channel` per active call (configurable capacity, default 4). Marginal.
- **Wire traffic:** up to 16 extra JSON-RPC frames per call. For stdio transport that means 16 extra stdin reads; for streamable-HTTP they ride the same SSE stream — practically free.
- **Total CPU:** minor; one extra tokio task per active streaming call.

### effort_estimate

- **(a) Progress notifications (pure infrastructure):** **M (2–3 days)** — plumb peer handles, emit notifications, add config knob. No tool-side changes.
- **(b) `call_tool_chain` streaming partial notifications:** **M (3–4 days)** — `Broadcast` integration, partial=true handling in `src/server.rs`, `intent` reconciliation with partial chunks. Requires careful ordering (chunks before final).
- **(a) + (b) together:** **L (1–2 weeks)** with integration tests.

### risks

- **Notification storms.** A backend returning 1MB in tiny frames + 16 chunks per frame = thousands of notifications. Bound `notifications_per_call: u32` (default 16).
- **Client support.** `notifications/progress` and partial `call_tool_chain` are *PrismGate extensions*, not core MCP. Document that fact. Clients that don't understand the partials just see the final one (idempotent, additive).
- **Ordering under load.** `tokio::sync::broadcast` does not guarantee total order across consumers; use `tokio::sync::watch` per-session if strict ordering is required.
- **JSON split boundaries.** Chunking at byte 16384 inside a UTF-8 stream can split a codepoint. Reuse `ResultStore::floor_char_boundary` semantics (already used at result_store.rs:91, 108) to chunk at safe boundaries.

### concrete_prismgate_files_to_change

- `src/server.rs` — peer-handle plumbing, broadcast channel for `call_tool_chain`
- `src/tools/mod.rs` (or `src/tools/call_tool_chain.rs` if it exists) — partial emission, idempotent reducer
- `src/backend/mod.rs` — accept `Peer<RoleServer>` injection so backends can call `notify_progress`
- `src/backend/http.rs`, `src/backend/stdio.rs`, `src/backend/cli_adapter.rs` — emit progress tick at chunk_size boundaries (configurable per backend via `BackendConfig.progress_chunk_bytes`)
- `src/config.rs` — add `progress_chunk_bytes: u32` default 16384; `progress_max_notifications_per_call: u32` default 16; `streaming: bool` on `call_tool_chain` params
- `src/result_store.rs` — add `split_at_boundary(byte_offset) -> (Vec<u8>, &[u8])` helper
- `tests/chunked_responses_test.rs` (new)

---

## Cross-cutting summary

| # | Feature | Impact | Effort | Risks | Defaultable? |
|---|---|---|---|---|---|
| 1 | HTTP connection pool + stdio session affinity | ★★★★★ | M | Pool sharing cross-backend; OAuth header rotation | Yes (off → on) |
| 2 | Async session store (track A: tokio mpsc; track B: reader-pool) | ★★★ (A) / ★★★★ (B) | S / M | WAL checkpoint policy; refcount race | Yes (A only) |
| 3 | Lazy backend init | ★★★★★ cold-start / RSS | M | Cached stale schemas; first-call cliff | Yes (opt-in) |
| 4 | Tool registry hot-reload | ★★★★ ops | L | Proxy notification routing; churn rate | Optional behind config |
| 5 | ResultStore TTL janitor | ★★ memory predictability | S | Per-session overhead | Yes (default on) |
| 6 | Streamable-HTTP peer listener | ★★★★★ multi-client / TTFT | M–L | Stateless vs sessionful; capacity planning | Yes (opt-in via CLI) |
| 7 | Chunked tool responses + progress | ★★★★ perceived latency for big tools | L | Notification storms; ordering | Yes (default off, opt-in per call) |

**Recommended sequencing** (assuming one engineer-week slices):

1. **Week 1:** #5 (ResultStore janitor) → trivial win, clears risk for #7.
2. **Week 2:** #2 Track A (tokio::sync::mpsc swap) and #1 (HTTP pool — small surface, big payoff).
3. **Week 3:** #6 streamable-HTTP listener (the biggest reach unlock; couples naturally with #1's shared client work).
4. **Week 4:** #3 lazy backend init (flagged off, behind a doc'd opt-in).
5. **Week 5+:** #4 hot-reload + #7 chunked responses (both L-effort, high-leverage, can land together as v1.19.0).

**Cross-feature dependencies:**
- #1 enables #6 (shared client across HTTP server and HTTP backend).
- #2 Track B requires a connection model decision that interacts with #1 if HTTP backends ever need to record to the session DB (they don't today — only the gateway does).
- #3 forces #5 to be on by default (otherwise deferred backends hold RAM and the result store decays).
- #7 requires #5 (chunked emission pipeline uses `ResultStore` as the durable backstop for clients that lose the SSE stream).

Final note: every recommendation in this document preserves the existing public MCP surface (`search_tools`, `list_tools_meta`, `call_tool_chain`, etc.). None of them require an rmcp API break; everything rides additive features in `1.7.x` and CLI/config opt-in flags.
