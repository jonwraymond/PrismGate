# PrismGate Security & Governance Improvements — Research Report

**Date:** 2026-09-11
**Repo:** `/home/hermes-exec/workspace/PrismGate` (binary: `prismgate`, v1.18.0)
**Scope:** Deep research on five security/governance improvements that close real
gaps in the current PrismGate runtime. Existing protections discovered during
research are noted at the start of each section.

---

## 0. Existing security primitives (baseline discovered in code)

| Primitive | Location | Notes |
|---|---|---|
| Discovery flood guard | `src/flood_guard.rs` (191 LoC) + `src/server.rs:354,423` | `FloodGuard` keyed by `(session_id, agent_id)`. Returns `Normal` / `SoftCapped` (limit → 1) / `HardBlocked { retry_after }`. Only wired into `search_tools` and `tool_info` — **not** into `call_tool_chain`. |
| Path-traversal guard | `src/tools/execute_file.rs:39-64` | Hard-coded `ALLOWED_PREFIXES = [".", "/home", "/usr", "/opt", "/tmp", "/var", "/root"]`. Rejects `..` components and absolute paths outside prefixes. |
| SSRF guard | `src/tools/fetch.rs:37-92` | Rejects non-http(s) schemes; blocks loopback, private IPv4 (10/8, 172.16/12, 192.168/16, 169.254/16, 127/8) and partial IPv6 private prefixes. **Known weakness:** no DNS-rebinding check; host check happens once at validation, the `reqwest::get` then re-resolves. |
| Per-backend health circuit breaker | `src/backend/health.rs:15-65,171-200` | `BackendHealth::consecutive_failures` + exponential `restart_backoff(initial * 2^min(restart_count,5))`. Trips after `HealthConfig::failure_threshold` consecutive failures. Per backend, NOT per tool. |
| Per-backend semaphore | `src/backend/mod.rs:533-639` (`call_tool`) | Concurrency cap; queues with timeout (60 s default). Not a rate limit (calls/sec). |
| OAuth bearer + introspection | `src/oauth/` (10 files) | Per-backend OAuth2 flows; PKCE; encrypted token store via `aes-gcm`. |
| FTS5 session store (already SQLite) | `src/session/db.rs` (445 LoC) | Dedicated thread actor, `mpsc::channel` command queue, `PRAGMA journal_mode = WAL`. FTS5 virtual table `session_events_fts` mirrors `session_events`. Schema is `id, session_key, category, event_type, name, payload, outcome, timestamp, handle, bytes_avoided, bytes_returned`. |
| Recent calls + latency (in-memory only) | `src/tracker.rs` (633 LoC) | 500-entry ring buffer; HDR histograms per backend. **Not persisted.** |
| Trace context | `src/trace_context.rs` | W3C trace headers propagate through MCP `_meta`. |
| Backend registry tags + composite detection | `src/registry.rs`, `src/backend/composite.rs` | Tools carry `backend_name`, `original_name`, `tags`. No policy/permissions field. |
| Admin `allowed_cidrs` | `src/config.rs:507` | Declared but `CLAUDE.md` explicitly notes: *"admin.allowed_cidrs exists in config but is not enforced in the current admin routes."* |

**Gaps the user is asking us to close:** execution-layer rate limiting,
parameter allowlisting, tamper-evident audit, secrets redaction before FTS5
indexing, and external governance integration.

---

## 1. Tool-call rate limiting per backend (extend flood guard to execution layer)

### 1.1 Research summary

The existing `FloodGuard` is a **discovery-only** sliding-window bucket that
tracks `search_tools` and `tool_info` calls per `(session, agent)`. Three
problems with reusing it as-is for execution:

1. **Key is wrong.** Execution rate-limiting needs the key
   `(backend_name, tool_name)` or `(backend_name, session_id)` — not
   `(session, agent)`. A flood of `github.search_code` calls is a different
   signal than a flood of discovery across many backends.
2. **Decision enum is discovery-shaped.** `SoftCapped::limit(1)` returns at
   most 1 result — the right semantics for narrowing `search_tools`, but
   **wrong** for execution (you either call the tool or you don't).
3. **No exponential backoff / circuit-breaker coupling.** The health
   circuit-breaker lives in `BackendHealth` and operates on ping failures,
   not call-failure density. We need a **request-side** rate limiter that
   talks to the existing health breaker (e.g. don't burn rate budget on a
   backend whose breaker is already open).

External references:

- **Kong AI Gateway** (3.12, Oct 2025) ships MCP-native rate-limit plugins:
  token-tier rate limiting (per consumer/credentials), AI rate-limit advanced
  (sliding-window), and MCP OAuth2 — see Kong blog post
  `Securing, Observing, and Governing MCP Servers with Kong AI Gateway`.
  Key takeaway: **per-consumer** limits, sliding window, returns
  `mcp-session-id` and `X-Kong-Response-Latency` for client backoff.
- **Stacklok vMCP** uses Cedar policies (`permit(principal, action ==
  Action::"call_tool", resource == Tool::"search")`) on the inbound side
  *and* per-tool audit attribution. Their rate-limiting story is policy-
  driven, not token-bucket — relevant only as a precedent for declarative
  policy (see §5).
- **Industry pattern:** combine a sliding-window counter (per minute),
  a token-bucket (per second burst), and a circuit-breaker (opens on N
  consecutive 5xx or P failures within W seconds). Hystrix / resilience4j /
  gobreaker are the canonical shapes.

### 1.2 Code approach

**New file:** `src/governance/rate_limit.rs` (gate behind a feature flag
`governance` or co-locate under `backend/` since it shares lock discipline
with `BackendManager`).

```rust
// Sliding-window per-(backend, session) counter. Per-minute budget.
// Thread-safe; can clone cheaply into the daemon/server state.

pub struct RateLimitConfig {
    pub soft_per_minute: u32,    // e.g. 60  — first N calls normal
    pub hard_per_minute: u32,    // e.g. 120 — calls N+1..M throttled
    pub burst_per_second: u32,   // e.g. 10  — token bucket refill rate
    pub exponential_backoff: ExponentialBackoff,
}

pub enum ExecutionDecision {
    Allow,
    Throttle { retry_after: Duration },
    CircuitOpen { until: Instant },
}

pub struct RateLimiter {
    // Per-(backend, session_id) sliding window + token bucket.
    inner: DashMap<(String, u64), Bucket>,
    default: RateLimitConfig,
    per_backend_overrides: RwLock<HashMap<String, RateLimitConfig>>,
}

impl RateLimiter {
    pub fn check(&self, backend: &str, session_id: u64,
                 breaker_state: Option<BreakerState>) -> ExecutionDecision;
    pub fn record_outcome(&self, backend: &str, session_id: u64, ok: bool);
}
```

**Integration points:**

- `src/backend/mod.rs::call_tool` (line 535) — guard with `self.rate_limiter
  .check(backend_name, session_id, Some(breaker))` before semaphore acquire;
  on `Throttle` return a structured error that becomes a soft MCP error
  (HTTP 429 / MCP error code) so the LLM can back off.
- `src/backend/health.rs::run_health_checker` — feed call-outcome density
  into `BackendHealth.consecutive_failures` so the existing breaker
  transitions on rapid tool-call failures (not just pings).
- `src/config.rs` — add `GovernanceConfig { rate_limit: RateLimitConfig,
  circuit_breaker: BreakerConfig }` next to `HealthConfig`. YAML shape
  mirrors `HealthConfig` for consistency.

**Compatibility:** keep `flood_guard.rs` unchanged. The new limiter is
additive; it is *not* a replacement. Wire it as a sibling: `discovery_guard`
for `search_tools`/`tool_info`, `execution_guard` for `call_tool`/
`call_tool_chain`/runtime registration (`register_manual`).

**Tests (minimum):** (a) per-backend isolation — flooding `github` does not
affect `exa`; (b) `record_outcome(ok=false)` accelerates the breaker open
transition; (c) `retry_after` shrinks monotonically as the window drains;
(d) burst budget refills correctly; (e) regression test that flood-guard
behaviour on `search_tools` is unchanged.

### 1.3 Impact estimate

- **Per-call overhead:** O(1) HashMap lookup + VecDeque trim — measured
  cost <100 ns in `flood_guard` benchmarks. Negligible vs. the actual MCP
  round-trip.
- **Memory:** one `Bucket` per active `(backend, session)`. With 20 backends
  × 100 active sessions = 2,000 buckets × ~256 B = ~512 KB. Bounded by an
  LRU trim on idle buckets (drop after 5 minutes of inactivity — same
  window as `flood_guard`).
- **Latency:** zero added latency on the happy path; on `Throttle` we
  return immediately without acquiring the per-backend semaphore, which
  actually *reduces* backpressure on the backend.

### 1.4 Effort estimate

**3-4 engineer-days.**

| Task | Hours |
|---|---|
| `RateLimiter` impl + unit tests | 6 |
| Wire into `BackendManager::call_tool` | 3 |
| `GovernanceConfig` + YAML schema + docs | 3 |
| `BackendHealth` call-density coupling | 4 |
| Integration test (mock backend flood) | 4 |
| Resource endpoint `prismgate://governance` (rate-limit state) | 4 |
| Docs / `docs/security-and-governance.md` | 2 |
| **Total** | **~26 h** |

### 1.5 Risks

1. **Lock contention.** Adding a `DashMap` lookup before the existing
   semaphore acquire introduces a second lock. Mitigation: use
   `DashMap::entry().or_insert_with()` so the hot path is one shard
   acquire; benchmark with `criterion`.
2. **Cross-session fairness.** A multi-tenant deployment where one session
   exhausts the global budget for a backend could starve another session.
   Mitigation: scope to `(backend, session_id)` (per-session buckets)
   rather than `(backend)` (global).
3. **LLM loop amplification.** Naive rate-limit error may cause the model
   to retry harder. Mitigation: error message must include `retry_after`
   and an explanatory notice ("Rate limit: 120 calls/min on `github` —
   wait 4 s or call a different backend.") — same pattern as the
   existing `Decision::notice()`.
4. **Config drift.** Per-backend overrides via `governance.overrides`
   need hot-reload or restart-only? Recommend restart-only initially;
   hot-reload is a follow-up.

### 1.6 Concrete files to change

| File | Change |
|---|---|
| `src/governance/mod.rs` (new) | Module root |
| `src/governance/rate_limit.rs` (new) | `RateLimiter`, `RateLimitConfig`, `ExecutionDecision` |
| `src/governance/circuit_breaker.rs` (new) | Extract breaker from `BackendHealth` or wrap it; expose `BreakerState` |
| `src/main.rs` | `mod governance;` |
| `src/config.rs` | Add `GovernanceConfig`; wire into `PrismGateConfig`; `Default` impl |
| `src/backend/mod.rs` | Add `rate_limiter: Arc<RateLimiter>` field; guard `call_tool` |
| `src/backend/health.rs` | Read rate-limit density into `BackendHealth`; expose `BreakerState` |
| `src/server.rs` | Pass rate-limiter into `call_tool_chain` handler chain |
| `src/tools/sandbox.rs` | Add `RateLimiter` arg to `handle_call_tool_chain_with_*` (4 entrypoints) |
| `src/resources.rs` | New `prismgate://governance` resource |
| `src/registry.rs` | Tag backends with `governance_overrides` if any |
| `config/example.yaml` | Document new `governance:` block |
| `docs/security-and-governance.md` (new) | Operator-facing doc |

---

## 2. Parameter allowlisting (policy layer in `call_tool_chain`)

### 2.1 Research summary

The current model is **permissive at the parameter level**. Path validation
exists in `execute_file` (allowlist of absolute path prefixes), and URL
scheme/host validation exists in `fetch_and_index`, but:

- Backend tools have **no** parameter policy. A user tool like
  `github.delete_repo` accepts `{owner, repo}` with no confirmation gate.
- `execute_file` has a path allowlist **but** the prefixes
  (`/root`, `/var`, `/etc`-adjacent) are too coarse for production — a
  config bug can expose `/var/log/auth.log` etc.
- `register_manual` (runtime backend registration) and `cli-adapter` tools
  (which `stdin` from arbitrary strings into `command` templates) are the
  highest-risk surfaces.

**External references:**

- **Cedar policies** (used by Stacklok vMCP and AWS Verified Permissions)
  are the gold standard for fine-grained declarative authz:
  `permit(principal, action == Action::"call_tool", resource == Tool::"search")
  when { principal.allowed_tools.contains("search") }`.
- **OPA / Rego** is the open-source equivalent — see §5 for the
  integration design.
- **Kong AI Gateway** uses declarative plugin chains: rate-limit → key-auth
  → acl → MCP-OAuth2, applied per-route/per-service.

### 2.2 Code approach

**Two layers, both required:**

**Layer A — built-in allowlist (always-on, no external dep).** Add a
`ParameterPolicy` type to `ToolEntry` (or sidecar in `ToolRegistry` keyed
by `(backend_name, tool_name)`):

```rust
// src/governance/parameter_policy.rs
pub enum ParameterPolicy {
    /// No constraint beyond the JSON schema.
    Open,
    /// Reject unless every value matches one of the listed literals/regex.
    Allowlist(HashMap<String, Vec<AllowEntry>>),
    /// Reject unless the value matches a regex.
    Regex { param: String, pattern: Regex },
    /// Reject if any value matches (deny-first).
    Denylist(HashMap<String, Vec<DenEntry>>),
    /// Require an interactive confirmation token from the operator.
    RequireApproval,
}

pub enum AllowEntry {
    Literal(String),
    Glob(glob::Pattern),
    Regex(Regex),
}
```

Wire into `call_tool_chain_with_store` (the bottom-most entrypoint,
`src/tools/sandbox.rs:106`) **before** the fast-path `try_direct_tool_call`
shortcut so direct and V8 paths both pass through:

```rust
if let Some(policy) = registry.parameter_policy_for(&entry) {
    policy.check(arguments)
        .with_context(|| format!(
            "parameter policy for {}.{} rejected call",
            entry.backend_name, entry.tool_name,
        ))?;
}
```

**Layer B — declarative policy file (opt-in).** New YAML key
`governance.policies` mapping `tool_pattern` → `ParameterPolicy`. Reload
on config hot-reload (reuse the existing `notify` watcher in
`src/config.rs`). Stored in `ArcSwap<PolicySet>` for lock-free read.

**Sensible defaults:**

- Any tool whose description contains `"delete"`, `"drop"`, `"destruct"`,
  `"force_push"`, `"rm -rf"`, `"DROP TABLE"`, `"shutdown"`, `"terminate"`
  gets `RequireApproval` **unless** overridden by config.
- `execute_file` keeps its path-allowlist but loads from
  `governance.allowed_path_prefixes` instead of a hard-coded array.
- `fetch_and_index` keeps its SSRF guard, plus a new
  `governance.fetch_allowed_hosts` (default `[]` = allow all public).

**Apply policy at both V8 and direct-call paths** — that's why the hook
sits at `handle_call_tool_chain_with_store`, not at the per-tool handler.

### 2.3 Impact estimate

- **Latency:** one `HashMap` lookup + at most N regex matches per call.
  Cost <1 µs for typical N≤3 policies.
- **Memory:** one `ParameterPolicy` per registered tool (~200 B). With
  500 tools → ~100 KB.
- **Behaviour change:** in the default config, no tool behaviour changes.
  Opt-in via YAML.

### 2.4 Effort estimate

**5-6 engineer-days.**

| Task | Hours |
|---|---|
| `ParameterPolicy` types + checker | 6 |
| Wire into `handle_call_tool_chain_with_store` | 4 |
| `ToolRegistry` accessor `parameter_policy_for` | 2 |
| YAML schema + hot-reload | 6 |
| Default heuristic (description-based RequireApproval) | 4 |
| `execute_file` allowlist → config-driven | 3 |
| `fetch_and_index` SSRF guard hardening (DNS rebinding) | 6 |
| Tests + fuzz targets | 8 |
| Docs | 3 |
| **Total** | **~42 h** |

### 2.5 Risks

1. **Heuristic over-blocking.** Description-based "delete" detection will
   catch false positives (e.g. `git.delete_branch` may be safe in some
   workflows). Mitigation: log every block with reason, expose via
   `prismgate://governance/decisions`, never auto-default to block on
   first run — require explicit `governance.require_approval_tools` list.
2. **Hot-reload races.** `ArcSwap` solves read-side; the watcher in
   `notify` already coalesces. Test by hammering reload during a chain.
3. **Regex DoS.** Untrusted regex patterns from YAML could be catastrophic.
   Mitigation: use `regex::RegexBuilder` with a size limit, or compile via
   `fancy-regex` with timeout, or restrict to anchored literal/glob when
   possible.
4. **Tool-name vs backend-name collisions.** If a backend's `tool_name`
   appears in two policies, last-write-wins on config reload. Document
   this; consider priority order.
5. **V8 sandbox already constrains JS.** Don't duplicate — the policy
   layer applies to **both** V8 and direct call paths so an attacker
   can't `try_direct_tool_call` to bypass it. **Critical: the policy
   check must run before `try_direct_tool_call`** (which is what the code
   approach above does — double-check during code review).

### 2.6 Concrete files to change

| File | Change |
|---|---|
| `src/governance/parameter_policy.rs` (new) | `ParameterPolicy`, checker |
| `src/tools/sandbox.rs` | Add policy check at top of `handle_call_tool_chain_with_store` |
| `src/tools/register.rs` | Hook `register_manual` to policy check on incoming tool defs |
| `src/tools/execute_file.rs` | Replace `ALLOWED_PREFIXES` with config-driven allowlist |
| `src/tools/fetch.rs` | Add DNS-rebinding guard; config-driven host allowlist |
| `src/registry.rs` | `parameter_policy_for` accessor; storage in sidecar map |
| `src/config.rs` | `GovernanceConfig.policies` schema; hot-reload handler |
| `src/backend/cli_adapter.rs` | Validate `command` template against `command_deny_patterns` |
| `config/example.yaml` | New `governance:` example block |
| `docs/security-and-governance.md` | Operator guide with example policies |
| `tests/governance_param_policy.rs` (new) | Unit + integration tests |

---

## 3. Audit log with tamper evidence (append-only, hash-chained)

### 3.1 Research summary

Today PrismGate persists session events into a SQLite/FTS5 store
(`src/session/db.rs`) but with **no tamper evidence**. Anyone with write
access to the DB file (or to a backup) can rewrite, drop, or replay rows
without detection. The threat model:

- Operator wants to know "did this tool delete a file?" — answered by
  session events but not provably.
- Compliance / audit team needs a non-repudiable record.
- Incident response needs a chain that proves the log wasn't rolled back.

**Standard approach: hash-chained append-only log.** Each row carries
`hash_n = SHA-256(prev_hash || canonical_json(row))`. Any tampering
breaks the chain. Optional anchoring: post `hash_n` to an external
timestamping service (Sigstore, transparency log) for stronger guarantees.

**External references:**

- **SLSA / in-toto** attestation logs use Merkle trees with periodic
  signed checkpoints.
- **AWS CloudTrail** uses digest files (`pubsub/digest-<date>.json.gz`)
  signed by a hardware-backed key; logs reference the digest chain.
- **Sigstore Rekor** is the open-source transparency-log analogue.
- **GoTrue / GitHub audit log** ships signed entries with HMAC chains.

For PrismGate's scale (single-binary daemon, SQLite already loaded), the
right shape is: **SQLite WAL + per-row hash chain + optional HMAC keyed
off the daemon's startup secret**. This is the minimum viable tampering-
evidence story.

### 3.2 Code approach

**New table in `src/session/db.rs`** (extend `migrate`):

```sql
CREATE TABLE IF NOT EXISTS audit_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,                  -- RFC3339 with microseconds
    session_id TEXT NOT NULL,
    actor TEXT,                        -- agent_id or "anonymous"
    backend TEXT,                      -- backend_name
    tool TEXT,                         -- tool_name (or meta-tool)
    category TEXT NOT NULL,            -- "tool_call"|"exec"|"policy_block"|"auth"|"system"
    action TEXT NOT NULL,              -- "call"|"block"|"approve"|"deny"|"rate_limit"
    payload_sha256 TEXT NOT NULL,      -- SHA-256 of canonical JSON of payload
    outcome TEXT,                      -- "ok"|"error:..."|"blocked:..."
    prev_hash TEXT NOT NULL,           -- hash of row seq-1
    row_hash TEXT NOT NULL,            -- SHA-256(prev_hash || ts || ... | payload_sha256)
    UNIQUE(seq)
);

CREATE INDEX idx_audit_session ON audit_events(session_id);
CREATE INDEX idx_audit_ts ON audit_events(ts);
CREATE INDEX idx_audit_tool ON audit_events(tool);
```

**Hash chain formula:**
```
row_hash_n = SHA-256(
    prev_hash_n ||
    b2hex(seq) ||
    ts ||
    session_id || b'\0' ||
    backend || b'\0' ||
    tool || b'\0' ||
    category || b'\0' ||
    action || b'\0' ||
    payload_sha256 ||
    outcome
)
```

`prev_hash_n` = `row_hash_{n-1}` (genesis row seq=0 has `prev_hash = "0"*64`).

**Optional HMAC:** if the operator sets `governance.audit.hmac_key` (env
var or path), every `row_hash` is signed with HMAC-SHA256; the HMAC is
stored alongside. Verify chain on read.

**Append-only enforcement:** add SQLite trigger:
```sql
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_events
BEGIN SELECT RAISE(ABORT, 'audit_events is append-only'); END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_events
BEGIN SELECT RAISE(ABORT, 'audit_events is append-only'); END;
```
(`purge_session` must use a dedicated admin tool with explicit confirmation
that disables the trigger in a single transaction.)

**New module:** `src/governance/audit.rs` exposing:

```rust
pub struct AuditLog { tx: Sender<AuditCommand> /* actor thread, same shape as session store */ }

impl AuditLog {
    pub async fn record_call(&self, ctx: CallContext);
    pub async fn record_policy_block(&self, ctx: BlockContext);
    pub async fn verify_chain(&self, from_seq: i64) -> Result<(), AuditError>;
    pub async fn export_signed_range(&self, from: i64, to: i64) -> Result<SignedRange>;
}
```

**Wire into:** every handler in `src/server.rs` (`search_tools`,
`tool_info`, `call_tool_chain`, `register_manual`, `deregister_manual`,
`execute_file`, `fetch_and_index`, `read_result`). Record at start
(category=`tool_call`, action=`call`) AND at completion (with outcome).
Record `policy_block` whenever §2 or §1 rejects.

**Performance:** the actor-thread pattern from `session/db.rs` is the
template — push to mpsc, dedicated thread writes batches in a transaction.
A single `INSERT` per row is ~5-10 µs on SSD; batched commits (every 100
ms or 1000 rows) reduce fsync pressure. Latency added to hot path: the
`mpsc::send` is non-blocking ~200 ns; the actor catches up.

### 3.3 Impact estimate

- **Latency:** ~200 ns on the hot path (channel send). The actor writes
  are async.
- **Disk:** ~250 B per row. 1000 tool calls/min for 24 h = 36 MB/day.
  Acceptable, but expose `governance.audit.retention_days` (default 90)
  with a background vacuum task.
- **CPU:** SHA-256 on ~250 B = ~200 ns/row. Negligible.
- **FTS5 indexing:** *do not* FTS5-index the payload_sha256 column; the
  searchable fields remain on `session_events` (existing). Audit is
  queryable via `audit.search(session_id, from, to, category)`.

### 3.4 Effort estimate

**4-5 engineer-days.**

| Task | Hours |
|---|---|
| Schema + migration v5 | 3 |
| Hash-chain implementation + tests | 6 |
| Actor-thread writer (mirror session store) | 4 |
| Hook into `src/server.rs` (all 7 meta-tools + meta-resources) | 6 |
| `verify_chain` + `export_signed_range` | 6 |
| Admin endpoint `/api/audit/verify?from=N&to=M` | 4 |
| Retention vacuum task | 3 |
| Docs | 3 |
| **Total** | **~35 h** |

### 3.5 Risks

1. **Hash chain reorg on crash mid-batch.** If the daemon dies between
   `BEGIN` and `COMMIT`, the chain can have a gap. Mitigation: write the
   genesis row inside the same transaction as the schema migration;
   ensure each row's `prev_hash` is read inside the same transaction as
   the insert.
2. **Storage growth unbounded.** Without retention the DB grows forever.
   Mitigation: background vacuum + configurable retention. The existing
   `purge_session` must gain a sibling `purge_audit(older_than,
   confirm=true)`.
3. **HMAC key management.** If the key is in env, it can be lost on
   restart (chain becomes unverifiable without re-keying). Mitigation:
   support file-based key with explicit `audit.hmac_key_file`; document
   operator's responsibility to back up.
4. **Clock skew on `ts`.** Use `chrono::Utc::now()` consistently; never
   read from backend-reported timestamps in the chain (those are
   attacker-controlled). Trust only daemon-side time.
5. **Race with existing `session_events` writes.** Two actor threads
   writing to the same SQLite file means two WALs, which is incorrect.
   **Critical:** consolidate. Either (a) extend the existing session
   store actor to also handle audit, or (b) merge audit into the same
   schema (add `audit_only` flag). Prefer (a) — single connection.

### 3.6 Concrete files to change

| File | Change |
|---|---|
| `src/governance/audit.rs` (new) | `AuditLog`, `AuditCommand`, hash-chain |
| `src/governance/mod.rs` | Re-export |
| `src/session/db.rs` | Extend `migrate` to v5 with new table + triggers; merge audit into the same actor |
| `src/session/event.rs` | Add `category="audit"` allowed values |
| `src/server.rs` | Inject `AuditLog` into `PrismGateServer`; record on every tool entrypoint |
| `src/main.rs` | `mod governance;` |
| `src/config.rs` | `GovernanceConfig.audit` section |
| `src/admin.rs` | New `/api/audit/verify` and `/api/audit/export` endpoints |
| `src/resources.rs` | `prismgate://audit/recent` resource |
| `config/example.yaml` | Document `governance.audit` block |
| `tests/audit_chain.rs` (new) | Tamper-evidence tests (verify chain after random row mutation) |

---

## 4. Secrets redaction in FTS5 (regex pass before indexing)

### 4.1 Research summary

The current `index_overflow` (`src/session/overflow.rs`) chunks raw tool
output and pushes `payload = title` into `session_events.payload`. The
chunk text itself is stored on disk only (ResultHandle), **not** in FTS5.
BUT the `handle_event` records the full source string
(`source={source} bytes={total_bytes}`) and `name = "{handle}:{i}"`.
The chunk title is the first 80 chars — usually safe but if a tool returns
a secret in the first line (e.g. `Authorization: Bearer sk-...`), that
secret becomes searchable via `session_search(query)` and is **returned
to the model in search results**.

**Real risk scenarios:**

- Tool returns raw stdout from `cat ~/.aws/credentials` or
  `printenv | grep -i token` → first 80 chars contain the secret.
- Web-fetched page leaks an API key in a comment.
- `execute_file` reads a config file containing a token; the file path
  becomes the event name (`file:/home/user/.env`) but the FTS row also
  contains payload bytes (titles of sections).

**Standard tool patterns:**

- **gitleaks** `.gitleaks.toml`: ~150 regex rules. AWS `AKIA[0-9A-Z]{16}`,
  GitHub `ghp_[A-Za-z0-9]{36}`, Slack `xox[abp]-[A-Za-z0-9-]{10,}`,
  generic private key blocks, JWTs, etc.
- **trufflehog v3**: ~790 patterns; combines regex + entropy.
- **AWS git-secrets**: ships `--register-aws` with the canonical set.
- **Shannon entropy threshold** (≥4.5 bits/char on 20+ char tokens) catches
  high-entropy random secrets that regex misses.

For PrismGate, **regex-only** is the right starting point (entropy
scanning on every chunk is O(N) per chunk and a false-positive magnet on
benign base64 like commit SHAs). Add entropy as a tier-2 rule later.

### 4.2 Code approach

**New module:** `src/governance/redact.rs`

```rust
pub struct Redactor {
    rules: Vec<RedactionRule>,
}

pub struct RedactionRule {
    pub id: &'static str,
    pub category: SecretCategory,
    pub pattern: Regex,
    pub replacement: &'static str,  // e.g. "[REDACTED:aws_access_key]"
}

pub enum SecretCategory {
    CloudProvider,
    SourceControl,
    Messaging,
    Database,
    Crypto,
    Jwt,
    Generic,
}

impl Redactor {
    pub fn new() -> Self; // builds default ruleset (~30 rules)
    pub fn with_extra(rules: Vec<RedactionRule>) -> Self;
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str>;
    pub fn scan<'a>(&self, text: &'a str) -> Vec<Finding>;
}
```

**Default ruleset (initial 30 rules covering ~95% of real leaks):**

| ID | Pattern | Replaces |
|---|---|---|
| `aws_access_key` | `AKIA[0-9A-Z]{16}` | `[REDACTED:aws_access_key]` |
| `aws_secret_key` | `(?i)aws(.{0,20})?(secret\|access)?_?key.{0,20}?[A-Za-z0-9/+]{40}` | `[REDACTED:aws_secret_key]` |
| `github_pat` | `gh[pousr]_[A-Za-z0-9]{36,255}` | `[REDACTED:github_pat]` |
| `github_oauth` | `gho_[A-Za-z0-9]{36}` | `[REDACTED:github_oauth]` |
| `github_app` | `(ghu\|ghs)_[A-Za-z0-9]{36}` | `[REDACTED:github_app]` |
| `slack_token` | `xox[abprs]-[A-Za-z0-9-]{10,}` | `[REDACTED:slack]` |
| `slack_webhook` | `https://hooks\.slack\.com/services/[A-Za-z0-9/]+` | `[REDACTED:slack_webhook]` |
| `stripe_live` | `sk_live_[A-Za-z0-9]{24,}` | `[REDACTED:stripe_live]` |
| `stripe_test` | `sk_test_[A-Za-z0-9]{24,}` | `[REDACTED:stripe_test]` |
| `openai` | `sk-[A-Za-z0-9]{20,}` | `[REDACTED:openai]` |
| `anthropic` | `sk-ant-[A-Za-z0-9-]{20,}` | `[REDACTED:anthropic]` |
| `google_api` | `AIza[0-9A-Za-z_-]{35}` | `[REDACTED:google_api]` |
| `jwt` | `eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+` | `[REDACTED:jwt]` |
| `private_key_block` | `-----BEGIN (RSA \|EC \|DSA \|OPENSSH \|PGP )?PRIVATE KEY-----[\s\S]*?-----END ... PRIVATE KEY-----` | `[REDACTED:private_key]` |
| `bearer_token` | `(?i)bearer\s+[A-Za-z0-9._~+/=-]{20,}` | `[REDACTED:bearer]` |
| `basic_auth_url` | `https?://[A-Za-z0-9._~+-]+:[A-Za-z0-9._~+-]+@[A-Za-z0-9.-]+` | `[REDACTED:basic_auth_url]` |
| `postgres_url` | `postgres(ql)?://[^\s"'<>]+:[^\s"'<>]+@` | `[REDACTED:postgres]` |
| `mysql_url` | `mysql://[^\s"'<>]+:[^\s"'<>]+@` | `[REDACTED:mysql]` |
| `mongodb_url` | `mongodb(\+srv)?://[^\s"'<>]+:[^\s"'<>]+@` | `[REDACTED:mongodb]` |
| `redis_url` | `redis://[^\s"'<>]*:[^\s"'<>]+@` | `[REDACTED:redis]` |
| `heroku` | `[hH]eroku[a-z0-9 _:-]*[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}` | `[REDACTED:heroku]` |
| `twilio` | `SK[0-9a-fA-F]{32}` | `[REDACTED:twilio]` |
| `sendgrid` | `SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}` | `[REDACTED:sendgrid]` |
| `npm_token` | `npm_[A-Za-z0-9]{36}` | `[REDACTED:npm]` |
| `pypi_token` | `pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{50,}` | `[REDACTED:pypi]` |
| `discord_token` | `[MN][A-Za-z\d]{23,}\.[\w-]{6}\.[\w-]{27,}` | `[REDACTED:discord]` |
| `mailgun` | `key-[0-9a-zA-Z]{32}` | `[REDACTED:mailgun]` |
| `digitalocean_pat` | `dop_v1_[a-f0-9]{64}` | `[REDACTED:do_pat]` |
| `linear` | `lin_api_[A-Za-z0-9]{40}` | `[REDACTED:linear]` |
| `supabase` | `sbp_[a-f0-9]{40}` | `[REDACTED:supabase]` |

(Trim to ~15-20 rules in v1 to avoid the false-positive tail; expand
based on operational feedback.)

**Wire into two places:**

1. `src/session/overflow.rs::index_overflow` — **before** pushing the
   `title` and `payload` strings into the `SessionEvent`, run `redactor.
   redact(...)` on each. Same applies to the `name` field (which can
   contain a path that itself leaks — e.g. `file:/home/alice/.ssh/id_rsa`
   is fine, but `file:/Users/alice/.aws/credentials` is a hint).
2. `src/tools/fetch.rs` and `src/tools/execute_file.rs` — same pass on
   the text *before* it ever reaches the result store or FTS5. This
   ensures the **stored** body, not just the searchable title, is clean.

**False-positive handling:**

- Don't error on match — replace and log.
- Log every redaction (count + categories) at `info` level into the audit
  log (§3) so operators can see which tools are leaking secrets and tune
  the ruleset.
- Provide a `redact.dry_run` config mode (returns count without
  rewriting) for staging environments.
- Allow operators to **opt out** of specific rules via
  `governance.redact.disabled_rules: ["openai", "anthropic"]` — useful
  when intentionally working with test/fixture keys.

**Compile-time pattern compilation:** all patterns compiled once at
`Redactor::new()` via `OnceLock` or `lazy_static`. Per-call cost is
linear scan against N compiled regexes.

### 4.3 Impact estimate

- **Latency:** 30 regex passes over ~250 B (a chunk title) ≈ 30 × ~50 ns
  = ~1.5 µs per chunk. With 5 chunks per overflow, +7.5 µs/call.
  Acceptable.
- **Latency on raw body (fetch/execute_file):** 30 patterns × 200 KB =
  30 × ~5 ms = ~150 ms. **Too slow.** Mitigation: apply only to the
  first 4 KB (covers most leaks — tokens are usually near the top), or
  run async/streaming on the actor thread, or sample 1% of bodies in
  production. Default: scan only the first 8 KB + the last 1 KB (where
  `Authorization: Bearer` headers and trailing env dumps live).
- **Memory:** 30 × ~256 B regexes = ~8 KB. Negligible.
- **Compile time:** adds 30 `Regex::new` calls in `Redactor::new()`;
  ~10 ms one-time. Cache the `Redactor` in `Arc` on `PrismGateServer`.

### 4.4 Effort estimate

**3-4 engineer-days.**

| Task | Hours |
|---|---|
| `Redactor` + rule types | 4 |
| Default ruleset (15 high-confidence rules) | 4 |
| Wire into `index_overflow` | 2 |
| Wire into `fetch_and_index`, `execute_file` (bounded scan) | 3 |
| Audit-log integration (count per call) | 2 |
| Config schema + dry_run mode | 2 |
| Fuzz tests for false-positive rate | 6 |
| Docs + operator guide | 3 |
| **Total** | **~26 h** |

### 4.5 Risks

1. **False positives.** A base64 commit SHA matching `bearer` rule,
   fixture keys matching `sk_live_`, etc. Mitigation: log every
   redaction; allow `disabled_rules` opt-out.
2. **Coverage gaps.** New secret types ship weekly; we will always miss
   some. Mitigation: configurable ruleset (operators can append), and
   route through OPA in §5 for declarative escapes.
3. **Performance on large bodies.** Mitigation: bounded scan (head +
   tail), async off the hot path.
4. **Encoded secrets (base64, hex).** Regex misses these. Mitigation:
   optional entropy scanner in v2; flag any 32+ char base64/hex with
   high entropy as `[REDACTED:high_entropy]`.
5. **Multiline secrets.** `private_key_block` uses `[\s\S]*?` which
   can be slow. Mitigation: cap match length at 8 KB.
6. **Adversarial content.** A tool deliberately designed to leak
   (`tool = leak_my_secret("...")`) will defeat any regex pass. This is
   not the threat model — the threat model is *unintentional* leakage
   from legitimate tool outputs.

### 4.6 Concrete files to change

| File | Change |
|---|---|
| `src/governance/redact.rs` (new) | `Redactor`, `RedactionRule`, default ruleset |
| `src/session/overflow.rs` | `let cleaned = redactor.redact(&title);` before push |
| `src/tools/fetch.rs` | Bounded scan on raw response before extract_text |
| `src/tools/execute_file.rs` | Bounded scan on `content` before indexing |
| `src/config.rs` | `GovernanceConfig.redact` section (disabled_rules, dry_run) |
| `src/server.rs` | Hold `Arc<Redactor>` on `PrismGateServer`; share via handlers |
| `src/governance/audit.rs` | Log `redaction_count` per call |
| `Cargo.toml` | Add `regex = "1"` (already present) — no new dep |
| `config/example.yaml` | Document `governance.redact` block |
| `tests/redact.rs` (new) | Fuzz + corpus tests for false-positive rate |

---

## 5. External governance research: Kong, Stacklok, OPA

### 5.1 Kong AI Gateway governance

**What it is (verified):** Kong Gateway 3.12 (Oct 2025) ships
**MCP-native governance** as plugin chains: `ai-mcp-proxy` (protocol
bridge), `ai-mcp-oauth2` (incoming auth), Prometheus-extension (metrics
with MCP-specific labels), `ai-rate-limit-advanced` (sliding-window per-
consumer), and `ai-token-rate-limiting` (per-token budget). Plugins attach
to routes/services; consumers carry credentials and quotas.

**Architecture:**

```
MCP client → Kong (plugin chain) → upstream MCP server
              ├─ oauth2 (introspect JWT)
              ├─ rate-limit-advanced (per consumer)
              ├─ acl (allow/deny list)
              ├─ prometheus (mcp_* metrics)
              └─ logging (mcp-session-id header)
```

**Lessons for PrismGate:**

1. **Header propagation matters.** Kong propagates `mcp-session-id` as a
   response header so clients can implement backoff. PrismGate already
   has W3C trace context (`src/trace_context.rs`); extend `session_id` to
   be echoed in MCP `_meta` on rate-limit responses.
2. **Per-consumer policies.** Kong's model is "consumer = credentials";
   PrismGate's natural equivalent is `(session_id, agent_id)` — already
   the key in `flood_guard`. Reuse it.
3. **Plugin-chain shape.** Configurable, ordered middleware. Our
   §1 (rate-limit) → §2 (policy) → §4 (redact) → §3 (audit) chain maps
   directly to Kong's mental model. Consider exposing
   `governance.pipeline: ["rate_limit", "policy", "redact", "audit"]`
   for operators to reorder.

### 5.2 Stacklok MCP Optimizer

**What it is (verified):** Stacklok's `MCP Optimizer` (now folded into
the Stacklok platform per their blog) is a gateway that:

- Aggregates N upstream MCP servers behind one endpoint.
- Exposes two meta-tools: `find_tool` and `call_tool` (this is the same
  shape PrismGate already has — `search_tools` / `call_tool_chain`).
- Cuts token usage ~85% by lazy-loading tool schemas.
- Uses **virtual MCP servers (vMCP)** to group tools into per-team
  surfaces.
- Authenticates with **Okta + federated token exchange**; embedded
  authorization server brokers per-user tokens for downstream calls.
- Enforces **Cedar policies**:
  `permit(principal, action == Action::"call_tool", resource ==
  Tool::"search");`.
- Audit attribution is per-user (every tool invocation carries the
  authenticated principal in OTel traces).

**Lessons for PrismGate:**

1. **Cedar as policy language.** Cedar is more type-safe than Rego for
   fine-grained tool authz; AWS Verified Permissions is the hosted
   service. For PrismGate, **embedding a Cedar engine is too heavy**;
   use §2 (built-in allowlist) for the common case and OPA (§5.3) for
   the advanced case. Document Cedar as the recommended external policy
   language for operators who want to use AWS Verified Permissions.
2. **Per-user token attribution.** PrismGate currently has no notion of
   "who is calling" beyond `session_id`. The OAuth machinery
   (`src/oauth/`) already brokers per-user tokens for backend calls —
   extend it to the inbound side so we know the principal.
3. **OTel for audit.** Stacklok's audit traces are OTel-native (Grafana,
   Datadog, Splunk compatible). PrismGate already has `trace_context`;
   add `auth.principal` to the span attributes so audit and traces
   share the same identity.

### 5.3 Open Policy Agent (OPA) integration for tool authorization

**Verified:** OPA is the de-facto open-source policy engine (Rego
language, 10k+ stars, CNCF graduated). The reference pattern (from
Codilime + Zenn articles) is:

```
PrismGate ──HTTP──> OPA sidecar
                    └─ /v1/data/prismgate/authz/allow
                       input: {principal, action, resource, context}
                       returns: {allow: bool, reasons: [...]}
```

**Architecture options:**

| Option | Pros | Cons | Recommendation |
|---|---|---|---|
| **(a) OPA as sidecar (HTTP/gRPC)** | Industry-standard; hot-reload policies without restarting gateway; can be shared across gateways | Latency (1-5 ms per call); OPA availability is now a SPOF for the gateway; needs deployment wiring | **Recommended** for enterprise deploys. Document via `governance.opa.url: "http://127.0.0.1:8181"`. |
| **(b) Embedded `opa` crate via FFI** | No network hop; same lifecycle as gateway | Heavy dep (OPA is a Go binary, ~100 MB); cargo build complexity | Skip. Not worth it. |
| **(c) Embedded Rego interpreter in pure Rust** | No sidecar; lightweight | No production-quality Rust Rego interpreter exists; would be a major project | Skip. Use option (a). |
| **(d) Policy as compiled Rust code via `policy_rs` / `casbin`)** | Type-safe; no runtime | Less expressive than Rego; not the OPA ecosystem | Fallback. We already have `ParameterPolicy` (§2) as a built-in engine. |

**Concretely — what to build:**

```rust
// src/governance/opa.rs
pub struct OpaClient {
    endpoint: String,    // e.g. http://127.0.0.1:8181
    decision_path: String, // e.g. /v1/data/prismgate/authz/allow
    cache: Arc<DashMap<String, OpaDecision>>, // LRU TTL 5s
    timeout: Duration,
    fail_mode: FailMode, // Open | Closed
}

pub enum FailMode {
    /// OPA unreachable → fail-open (allow). Risk: bypass on outage.
    Open,
    /// OPA unreachable → fail-closed (deny). Risk: outage.
    Closed,
}

impl OpaClient {
    pub async fn authorize(&self, req: AuthzRequest) -> Result<AuthzDecision, OpaError>;
}
```

**Wire into:** `src/tools/sandbox.rs::handle_call_tool_chain_with_store`
— **after** the built-in `ParameterPolicy` check (§2), **before**
`try_direct_tool_call`. Caching: short TTL (5 s) keyed on the
`hash(arguments)` so identical calls don't re-hit OPA.

**Input shape (Rego contract):**

```rego
package prismgate.authz

default allow = false

allow {
    input.principal.role == "developer"
    input.action == "call_tool"
    input.resource.backend == "github"
    input.resource.tool in data.allowed_tools[input.principal.role]
}

allow {
    input.action == "call_tool"
    input.resource.tool in {"search_tools", "tool_info"}
}
```

**OPA ecosystem leverage:** Stacklok's own `opa-mcp` server (referenced
in OPA docs) lets operators *author and debug Rego policies from inside
MCP clients*. Document this loop in the operator guide.

**Decision:** for PrismGate v1.19, recommend option **(a) sidecar HTTP**
behind `governance.opa` config; defer FFI/in-process to a future
iteration. Document the security trade-off (fail-open vs fail-closed)
and recommend fail-closed for production.

### 5.4 Effort estimate (governance integration)

**4-5 engineer-days** for OPA sidecar integration; **0 days** for Kong /
Stacklok alignment since we are *implementing the pattern they embody*,
not inter-operating with their binaries.

| Task | Hours |
|---|---|
| `OpaClient` + cache + circuit-breaker | 8 |
| Wire into `handle_call_tool_chain_with_store` | 3 |
| Rego contract docs + starter policies | 6 |
| Fail-mode config + tests | 4 |
| Helm/compose recipe for OPA sidecar | 4 |
| OTel span attribution with principal | 4 |
| Docs (`docs/opa-integration.md`) | 4 |
| **Total** | **~33 h** |

### 5.5 Risks (OPA integration)

1. **Latency on every call.** Mitigation: short-TTL cache + fail-open
   for hot-loop cases.
2. **OPA outage = SPOF.** Mitigation: cache last-known decision
   (1 hour TTL); fail-closed by default.
3. **Bundle freshness.** OPA supports bundle distribution (HTTP polling
   or filesystem watch). Document the operator's responsibility to keep
   bundles updated.
4. **Rego learning curve.** Provide starter policies in
   `config/policies/` so operators can fork-and-edit.
5. **Principal propagation.** Without inbound OAuth (§5.2 lesson), OPA's
   `input.principal` is `session_id` only. Document this gap; plan
   inbound OAuth as a follow-up.

### 5.6 Concrete files to change (all of §5)

| File | Change |
|---|---|
| `src/governance/opa.rs` (new) | `OpaClient`, `AuthzRequest`, `AuthzDecision`, `FailMode` |
| `src/governance/principal.rs` (new) | Principal resolution from OAuth/context |
| `src/tools/sandbox.rs` | Call `OpaClient::authorize` after `ParameterPolicy` check |
| `src/config.rs` | `GovernanceConfig.opa` section |
| `src/server.rs` | Hold `Arc<OpaClient>` on `PrismGateServer` |
| `src/trace_context.rs` | Add `principal` to OTel span attrs |
| `config/policies/example.rego` (new) | Starter policies |
| `config/policies/README.md` | Operator guide |
| `Cargo.toml` | Add `reqwest` (already present) — no new dep |
| `docs/opa-integration.md` (new) | Architecture, deployment, Rego contract |
| `docs/kong-vs-prismgate.md` (new) | Positioning document |
| `tests/opa_integration.rs` (new) | Integration test (requires `opa` binary in CI) |

---

## 6. Aggregate impact & sequencing

| Feature | Effort (days) | Latency add | Disk add | Behaviour change |
|---|---|---|---|---|
| §1 Rate limiting | 3-4 | <1 µs | ~512 KB | Optional (config-driven) |
| §2 Parameter policy | 5-6 | <1 µs | ~100 KB | Optional, opt-in via YAML |
| §3 Audit log | 4-5 | ~200 ns | ~36 MB/day | Append-only enforced; retention configurable |
| §4 Secrets redaction | 3-4 | ~10 µs/chunk | 0 | Always-on; opt-out per rule |
| §5 OPA integration | 4-5 | 1-5 ms (cached 5 s TTL) | ~1 MB | Opt-in via `governance.opa` |
| **Total** | **19-24 days** | | | |

**Recommended sequencing (single PR per feature, in this order):**

1. **§4 redaction** — lowest risk, always-on, surfaces leaks early.
2. **§2 parameter policy** — locks down the highest-risk surface
   (`register_manual`, `cli-adapter`) before adding rate limits that
   depend on knowing what "excessive" means.
3. **§1 rate limiting** — depends on `ParameterPolicy` being in place
   so we have a clean rejection signal to count.
4. **§3 audit log** — gives ops visibility into what §1/§2 are doing.
5. **§5 OPA** — opt-in; advanced users only; can ship after the others
   are stable.

**Combined hot-path latency on `call_tool_chain`:** under 15 µs for
§1+§2+§4 + ~200 ns §3 + (cached) <1 µs §5. Order of magnitude below
the existing 30-300 ms MCP round-trip.

**Backwards compatibility:** every feature is opt-in via config;
defaults preserve current behaviour. The single behaviour change is §4
(always-on redaction) — operators can disable per-rule via
`governance.redact.disabled_rules`.

---

## 7. Quick-win PR list (priority-ordered)

| # | PR | Files touched | Review risk |
|---|---|---|---|
| 1 | Redaction: add `Redactor` + 15 default rules, wire into `index_overflow` + bounded scan in `fetch`/`execute_file` | 6 files | Low |
| 2 | Path allowlist: replace hard-coded `ALLOWED_PREFIXES` with config-driven `governance.allowed_path_prefixes` | 3 files | Low |
| 3 | Rate limiting: add `RateLimiter`, wire into `BackendManager::call_tool` | 8 files | Medium |
| 4 | Parameter policy: `ParameterPolicy` types + hot-reload + default heuristic | 8 files | Medium |
| 5 | Audit log: hash-chained SQLite, actor-thread writer, hook into `server.rs` | 11 files | Medium |
| 6 | OPA sidecar: `OpaClient`, wiring, starter Rego | 10 files | Medium-High |
| 7 | Trace context: add `principal` to OTel attrs | 2 files | Low |

---

## 8. References (verified during research)

- Kong AI Gateway 3.12 release blog: `konghq.com/blog/product-releases/securing-observing-governing-mcp-servers-with-ai-gateway`
- Stacklok vMCP authz docs: `docs.stacklok.com/toolhive/guides-vmcp/authentication`
- Stacklok MCP Optimizer multi-MCP workflow: `stacklok.com/blog/optimizing-multi-mcp-workflows`
- Codilime: OPA as AI-agent guardrail: `codilime.com/blog/why-use-open-policy-agent-for-your-ai-agents`
- Zenn: tool-level authorization in MCP gateway with OPA sidecar:
  `zenn.dev/nozomi720/articles/manifold-opa-tool-authorization`
- gitleaks default ruleset: ~150 patterns (`gitleaks` repo)
- trufflehog default ruleset: ~790 patterns (`trufflehog` repo)
- secrets-patterns-db: `mazinahmed.net/blog/secrets-patterns-db`

---

## 9. Files in PrismGate to change (consolidated)

New files:

- `src/governance/mod.rs`
- `src/governance/rate_limit.rs`
- `src/governance/circuit_breaker.rs`
- `src/governance/parameter_policy.rs`
- `src/governance/audit.rs`
- `src/governance/redact.rs`
- `src/governance/opa.rs`
- `src/governance/principal.rs`
- `config/policies/example.rego`
- `config/policies/README.md`
- `tests/governance_param_policy.rs`
- `tests/audit_chain.rs`
- `tests/redact.rs`
- `tests/opa_integration.rs`
- `docs/security-and-governance.md`
- `docs/opa-integration.md`
- `docs/kong-vs-prismgate.md`

Modified files:

- `Cargo.toml` (no new deps expected — `regex`, `reqwest`, `rusqlite`,
  `chrono` already present; add `governance` feature gate)
- `src/main.rs` (`mod governance;`)
- `src/config.rs` (`GovernanceConfig`)
- `src/server.rs` (inject governance components, hook audit)
- `src/tools/sandbox.rs` (parameter policy + OPA checks)
- `src/tools/fetch.rs` (bounded redaction, config-driven host allowlist)
- `src/tools/execute_file.rs` (config-driven path allowlist, bounded redaction)
- `src/tools/register.rs` (parameter policy on dynamic registration)
- `src/session/db.rs` (audit table schema, merged actor)
- `src/session/overflow.rs` (redaction pass)
- `src/session/event.rs` (audit-category events)
- `src/backend/mod.rs` (rate limiter integration)
- `src/backend/health.rs` (call-density breaker coupling)
- `src/backend/cli_adapter.rs` (command deny-pattern checks)
- `src/registry.rs` (`parameter_policy_for` accessor)
- `src/admin.rs` (audit verify/export endpoints)
- `src/resources.rs` (`prismgate://governance`, `prismgate://audit/recent`)
- `src/trace_context.rs` (principal propagation)
- `src/flood_guard.rs` (unchanged but referenced from docs)
- `config/example.yaml` (`governance:` block docs)