# PrismGate Reflection Metrics — 2026-05-25

## System Refresh — Full Status

### Build & Test
- ✅ Build: Clean (43.13s release)
- ✅ Tests: 334 passed, 0 failed, 3 ignored (17.42s)
- ✅ CI: All recent runs green (Security ×4, Release)
- ✅ Clippy: Clean (0 errors)
- ✅ GH Auth: Authenticated as jonwraymond

### Codebase
- Rust files: 48
- Rust lines: 21,902
- Doc files: 21
- Total commits: 204
- Commits this week: 0
- Latest: Merge PR #98 (release 1.14.4)

### Git Status
- 15 branches with commits ahead of main (0 PRs for any)
- Biggest: feat/process-supervision-overhaul (14 commits)
- 0 open PRs, 0 open issues

### Kanban
- Done: 8 (all P1)
- Ready P1: 24
- Ready P2: 64
- Ready P3: 18
- In Progress: 0
- In Review: 0
- **Total ready: 106**

### Agent Farm
- Alan gateway: Running (since May 24)
- CEARO gateway: Running (since May 24)
- Turing gateway: Running (since May 24)
- Mira gateway: Running (since May 25)
- Hermes WebUI: Running (since May 23)

### Infrastructure
- Paperclip API: ❌ Dead (404 since May 24)
- LiteLLM MCP: ❌ Unhealthy
- Kanban DB: ✅ Active (/root/.hermes/kanban/boards/prismgate/kanban.db)
- GH CLI: ✅ Authenticated (device code: jonwraymond)

### Dogfooding
- Phase: 0/5 (not started)
- Backends migrated: 0/4
- PrismGate daemon: Not deployed

### Reflection Summary
|**What worked**: Git auth fixed, Kanban DB intact, all 4 agent gateways running, build green.
|**What needs improvement**: 0 tasks in progress, 15 un-PR'd branches, Paperclip dead, no dogfooding.
|**Next priority**: Create PRs for 8 done tasks, activate P1 Kanban delegation, set up heartbeats.

---

## Market Research — 2026-05-25

### Competitor Changes
- **MCPX (Lunar.dev)**: Gartner Representative Vendor, SOC 2 certified (Enterprise), ~4ms p99, tool-level RBAC + immutable audit trails shipped; OSS tier covers basic audit + OAuth
- **Docker MCP Gateway**: No built-in governance; container isolation ≠ governance
- **Microsoft MCP**: Azure-native, session-aware routing; no tool-level RBAC or agent-identity attribution
- **IBM ContextForge**: 40+ plugins, multi-cluster federation; high deployment complexity, Cedar RBAC rule-based only
- **MCPJungle**: Single binary, registry discovery; light on governance features

### Protocol Updates
- **MCP 2026-07-28 RC (locked May 21, ships July 28)**: Largest revision since launch — stateless core, sessions removed from protocol, MRTR elicitation pattern (server rejects + client re-issues), `Mcp-Method` header routing, `ttlMs`/`cacheScope` caching, W3C Trace Context in `_meta`
- **Sessions removed from core**: Transport-level session IDs eliminated; applications must use explicit handle pattern (server-minted IDs passed as arguments); sessions extension published for feedback
- **Extensions first-class**: Reverse-DNS IDs, negotiated via capabilities map, version independently; MCP Apps (sandboxed iframe UI) and Tasks (graduated from experimental) shipped as official extensions
- **STDIO security concern**: Maintainers flagged high-popularity STDIO servers as reputational supply-chain risk; no protocol-level fix possible

### User Pain Points
- **Context window saturation**: 50 tools = 20K–25K tokens; tool discovery degrades at scale — PrismGate's three-tier search (BM25 → trigram → fuzzy) addresses this
- **No dynamic/lazy loading**: Backends load all tools upfront; PrismGate's meta-tool discovery model is ahead of competitors
- **Transport scaling gaps**: Stateful sessions break behind load balancers; MCP 2026-07-28 fixes this but backends/gateways must adapt
- **Credential isolation**: MCPX Enterprise enforces secrets-by-reference; PrismGate has env interpolation + Bitwarden but no enforcement model

### Top 3 Insights
1. **PrismGate's architecture is well-aligned with 2026-07-28 stateless spec**: Shared-daemon + proxy reconnect model maps cleanly to stateless HTTP transport. No sticky-session dependency.
2. **PrismGate's gap is governance depth**: MCPX ships tool-level RBAC, immutable audit trails, and SSO at Enterprise tier. PrismGate has none of these. Dogfooding (Phase 0/5) should be prioritized to surface real governance needs.
3. **MCP Apps (iframe-based UI extensions)** represent a new surface area PrismGate does not yet expose. This is a potential extension point.

### Actionable Kanban Items
- [ ] Investigate stateless HTTP backend support for MCP 2026-07-28 (SEP-2243 `Mcp-Method` header routing)
- [ ] Audit dedicated instance mode session_id threading — does it conflict with stateless spec?
- [ ] Add tool-level access control primitives (MVP: per-tool allow/deny in config)
- [ ] Add immutable audit log format for tool invocations (append-only, signed)
- [ ] Surface MCP Apps iframe extension support as a research spike
- [ ] Run PrismGate dogfooding: migrate 1 backend (e.g., filesystem) to live daemon

## Full Heartbeat — 2026-06-19 14:23 UTC

- **Main build**: ✅ cargo check (bin + all-features) clean
- **CI**: All non-Security workflows green; Security/Cargo Deny ❌ (persistent 3 issues)
- **Open PRs**: 5 (#131 CONFLICTING, #129 release-please, #127/#126/#125 stale)
- **Kanban**: 11 tasks (2 ready P1, 3 blocked, 6 done, 0 stalled)
- **Cargo Deny**: RUSTSEC-2026-0173 + aes yanked + bitwarden licenses
- **Pre-delegation audit**: clean (0 title-only tasks)

---

## Market Research — 2026-06-19

### New Competitor Entrants (since June 17 update)

| Entrant | Date | Key Differentiator | Threat Level |
|---------|------|-------------------|-------------|
| **NetFoundry MCP Gateway** | June 3, 2026 | Zero-trust cryptographic identity per agent; denied tools REMOVED from registry entirely (not just runtime-checked); unified MCP+LLM gateway; 50% token cost savings claim | **HIGH** |
| **Envoy AI Gateway MCP** | June 2026 | Built on Envoy Proxy; OAuth enforcement; tool filtering via includeRegex; MCPRoute API; Streamable HTTP transport; upstream auth | **MEDIUM** |
| **AWS MCP Gateway & Registry** | June 2026 | Apache 2.0; React UI; Cisco AI Defense scanning (YARA + LLM); hybrid search (RRF — vector + lexical); OIDC (Entra, Okta, Auth0, Cognito, Keycloak, PingFederate); Fortnightly releases | **HIGH** |
| **Speakeasy MCP Gateway** | March 2026 | One-entry-point model; focused on API contract generation | **LOW** |
| **Kong Konnect MCP** | June 2026 | Existing API governance extended to MCP; plugin-based | **MEDIUM** |

### MCPJungle Update (June 2026)
- **52 releases** (v0.4.5 latest), 1.1k stars, 24 contributors
- Go-based (84.4%) + TypeScript (9.3%)
- Tool groups, enterprise mode (auth + ACLs + observability)
- PostgreSQL support for production
- Brew install + Docker Compose 1-cmd deploy
- Growing fast: 274 commits

### Protocol: 2026-07-28 RC — Impact Deep-Dive
- **Stateless core locked** (May 21). Ships July 28 — 39 days.
- **PrismGate impact analysis:**
  - ❌ Proxy reconnect's cached-handshake replay becomes **OBSOLETE** (no `initialize` to replay)
  - ❌ New `Mcp-Method`/`Mcp-Name` headers required for Streamable HTTP — routing code needed
  - ❌ Tool cache v4 needs `ttlMs`/`cacheScope` support
  - ❌ OAuth/RFC 9207/8707/PKCE S256 compliance gap is now **protocol-required**, not optional
  - ✅ PrismGate shared-daemon architecture is **well-aligned** with stateless spec
  - ✅ `_meta` field already used in PrismGate — easy to add clientInfo there
- **Deprecations**: Roots, Sampling, Logging (12-month removal window)
- **Extensions formalized**: MCP Apps (sandboxed iframes), Tasks (long-running)
- **W3C Trace Context** in `_meta` — needs `traceparent`/`tracestate` headers (already noted as gap in `trace_context` module)

### Security Landscape (NEW)
- **30+ MCP-related CVEs** filed Jan-Feb 2026, including RCE rated **9.6 CVSS**
- **CVE-2026-32211**: Azure DevOps MCP server missing-auth vulnerability — **9.1 CVSS**
- **Tool Description Poisoning**: Invariant Labs demonstrated malicious descriptions inject directly into model context window; no network anomaly, no auth failure, no intrusion log
- **Dead Server Epidemic**: 52% of public MCP servers effectively dead (Rapid Claw audit) — no commits, no CI, no CVE patching
- **Auth complexity**: 4 specs required for production MCP (OAuth 2.1, RFC 9728, PKCE, additional implementation requirements). "No partial credit."

### User Pain Points (Updated)
| Pain Point | Severity | PrismGate Response |
|-----------|----------|-------------------|
| Tool description poisoning | CRITICAL | PrismGate can sanitize/normalize tool descriptions before they reach model |
| Dead server epidemic (52%) | HIGH | Health checks + stderr capture already in PrismGate — extensible to server vetting |
| Auth tax (4-spec stack) | HIGH | PrismGate needs OAuth/RFC 9207/8707/PKCE — gap is now mandatory |
| Credential sprawl | HIGH | BWS + env fallback exists; no credential isolation enforcement |
| Tool sprawl / context saturation | MEDIUM | 3-tier search already addresses this — continued differentiator |
| No standard governance framework | HIGH | Audit + RBAC remain PrismGate gaps |
| 5 silent failure modes after 100 requests | MEDIUM | Health checker + memory limits partially address |

### PrismGate Feature Parity Update

| Capability | PrismGate | Best-in-Class |
|-----------|-----------|--------------|
| Tool discovery | ✅ 3-tier progressive | ✅ PrismGate leads |
| Context efficiency | ✅ Progressive + intent filtering | ✅ PrismGate leads |
| TS sandbox | ✅ V8 isolated | ✅ PrismGate leads (unique) |
| Zero-trust identity | ❌ | ✅ NetFoundry (cryptographic identity per agent) |
| MCP security scanning | ❌ | ✅ AWS/Cisco AI Defense (YARA + LLM) |
| Tool RBAC + IDP | ❌ | ✅ MCPX (Gartner rep), MintMCP (SOC2 Type II) |
| Immutable audit | ❌ | ✅ MCPX (full-chain User→Agent→MCP→Tool) |
| OAuth/OIDC compliance | ❌ | ✅ Multiple (spec-required July 28) |
| Docker deploy | ❌ | ✅ MCPJungle (1-cmd), MCPX |
| Dead server detection | ⚠️ Health checks | ❌ Nobody has this yet |
| Supply chain vetting | ❌ | ✅ AWS (Cisco scanners) |

### Top 3 Actionable Insights

1. **2026-07-28 deadline is 39 days away.** Proxy reconnect is dead code after July 28. `Mcp-Method`/`Mcp-Name` header routing must be implemented. Cache v4 needs `ttlMs`/`cacheScope`. OAuth/RFC compliance becomes mandatory — not a differentiator, but table stakes.

2. **Tool description poisoning is a CRITICAL unsolved problem** with no standard solution. PrismGate's position as an intermediary gateway means it CAN sanitize/normalize/inspect tool descriptions before they reach the model. This is a unique potential differentiator — neither MCPX, MCPJungle, nor Envoy claim to do this.

3. **Dead server detection (52% of ecosystem) is a problem NOBODY has addressed.** PrismGate already captures backend stderr, health status, RSS, and peak RSS. Extending this to a "server vetting score" (last commit date, CVE patch status, CI status) would be a first-in-market feature.

### Actionable Kanban Items
- [ ] P1: Implement 2026-07-28 stateless protocol support (Mcp-Method/Mcp-Name headers, remove initialize replay, ttlMs/cacheScope in cache v4)
- [ ] P1: Add OAuth 2.1 / RFC 9207 / RFC 8707 / PKCE S256 compliance (protocol-required by July 28)
- [ ] P1: Tool description sanitization/normalization layer (poisoning defense)
- [ ] P2: Dead server detection — extend health checker with GitHub commit-age, CVE-patch, CI-status scoring
- [ ] P2: W3C Trace Context (traceparent/tracestate) in `_meta` — add `trace_context` module
- [ ] P2: Audit immutable tool invocation log (append-only, signed)
- [ ] P3: Add NetFoundry, Envoy AI Gateway, AWS MCP Gateway to competitive landscape monitoring
- [ ] P3: Docker deployment option (docker-compose / container image)
- [ ] P3: MCP Apps iframe extension research spike

---

## Full Self-Reflection Loop — 2026-06-19 Market Research Cycle

### Step 1: Outcome Review
- **Goal**: Competitive analysis cycle on MCP ecosystem — research new entrants, protocol changes, pain points, and create actionable Kanban tasks.
- **Delivered**: 
  - 5 new competitor entrants identified (NetFoundry, AWS, Envoy AI Gateway, Speakeasy, Kong)
  - 2026-07-28 RC impact analysis on PrismGate architecture
  - Updated competitive landscape reference with new threat matrix
  - 4 new Kanban cards (1 P1, 2 P2, 1 P3)
  - Updated REFLECTION_METRICS.md with full market research section
  - Security landscape analysis: 30+ CVEs, tool poisoning, dead server epidemic
- **Worked well**: Parallel web searches covering 5 angles simultaneously; comprehensive source extraction
- **Blockers**: None — all tool calls succeeded; repo stayed on feature branch

### Step 2: PrismGate-Specific Technical Analysis
- **Daemon stability**: ✅ Build green (cargo check 3.66s), tests 308 pass / 0 fail / 3 ignored (4.84s)
- **Tool discovery**: ✅ PrismGate's 3-tier search remains unique differentiator — competitors (MCPJungle, MCPX) do flat lists
- **MCP compatibility**: ⚠️ Proxy reconnect obsolete after 2026-07-28 (no `initialize` to replay); `Mcp-Method`/`Mcp-Name` headers needed for Streamable HTTP
- **Secrets management**: ✅ BWS + env fallback exists; ❌ No credential isolation enforcement (MCPX Enterprise does secrets-by-reference)
- **TypeScript execution**: ✅ V8 sandbox unique to PrismGate
- **Regressions**: None — all 308 tests pass, clippy clean

### Step 3: Dogfooding & Migration Status
- **Current status**: Phase 0/5 — not started. No backends migrated.
- No PrismGate daemon deployed for dogfooding. This remains the biggest operational gap. Without live dogfooding, governance pain points (RBAC, audit, auth) can only be inferred from competitor analysis, not validated against real usage.
- **Recommendation**: Prioritize 1-backend migration (e.g., filesystem) to surface real-world needs.

### Step 4: Market & Feature Parity Insights
- **New competitive intelligence**: 3 high-threat entrants in June alone — NetFoundry (zero-trust identity), AWS (supply chain scanning), Envoy (enterprise proxy). The MCP gateway market is consolidating fast.
- **PrismGate differentiation opportunities**:
  1. Tool description sanitization (poisoning defense) — NO competitor claims this
  2. Dead server detection / vetting score — NO competitor has this
  3. Progressive discovery + context efficiency — PrismGate already leads
- **Urgent gaps**: OAuth/RFC compliance becomes mandatory July 28 (39 days); proxy reconnect becomes dead code same date.
- **Feature parity score**: PrismGate leads on 3 dimensions (tool discovery, context efficiency, TS sandbox), lags on 7 (RBAC, audit, OAuth, Docker, OTel, SOC2, supply chain scanning). Net parity: -4.

### Step 5: Learning Extraction & Skill Update
- **Updated skills**: `prismgate-competitor-feature-review` competitive landscape reference updated with 5 new entrants, security landscape, and pain points.
- **New Kanban cards created**: 4 (t_db020195, t_a0bdf603, t_598972b1, t_092b5249)
- **Lessons for future cycles**: 
  - Parallel web searches (5 at once) are efficient for market research
  - Build/test pre-check is essential — caught green state this cycle
  - Existing Kanban cards should be checked before creating duplicates (3 of 7 proposed items already existed)
- **Skill gaps identified**: No competitor has tool sanitization or dead server detection — these are first-in-market opportunities for PrismGate.

### Step 6: Automated Reflection Metrics (PrismGate Edition)

#### MCP Migration Progress
- **Total backends**: 4
- **Migrated**: 0 (0%)
- **Current phase**: Phase 0/5
- **Next migration**: None scheduled

#### Dogfooding Stability Score
- **Uptime**: N/A (daemon not deployed)
- **Error rate**: N/A
- **Latency overhead**: N/A

#### Tool Discovery Performance
- **BM25 search**: Not measured in this cycle
- **Trigram search**: Not measured in this cycle
- **Fuzzy search**: Not measured in this cycle
- **Success rate**: Not measured

#### Test Coverage & Pass Rate
- **Total tests**: 311 (308 pass + 3 ignored)
- **Passing**: 308 (99.0%)
- **New tests added**: 0 this cycle (research only)
- **Regressions**: 0

#### Daemon Resource Impact
- **Memory**: N/A
- **CPU**: N/A
- **Uptime**: N/A

#### Features/Improvements Delivered
- Competitive landscape updated (5 new entrants)
- Market research findings added to REFLECTION_METRICS.md
- 4 Kanban cards for actionable findings
- Security landscape analysis (30+ CVEs, tool poisoning, dead servers)

#### Documentation & Branding Updates
- REFLECTION_METRICS.md updated with full market research section
- Competitive landscape reference updated in competitor-feature-review skill

#### Reflection Summary
**What worked**: Parallel research across 15+ sources, comprehensive competitor coverage, build + test verification green. Discovered 3 first-in-market opportunities for PrismGate.
**What needs improvement**: Dogfooding remains at 0% — no live validation of governance features. Feature parity gap widening as more enterprise-grade competitors enter market. OAuth compliance deadline (July 28) is 39 days away with blocked Kanban card.
**Next priority**: Escalate OAuth card (t_b41502a0) from blocked to active — protocol-required by July 28. Begin tool description sanitization implementation (t_db020195, P1). Start 1-backend dogfooding migration.
## Full Heartbeat — 2026-06-20 02:28 UTC

- **Main**: ✅ builds clean @ 62609a8
- **PR #130**: ✅ merged today (trace_context fix)
- **Kanban**: 15 total, 2 ready P1, 6 blocked, 7 done, 0 running
- **CI**: mixed — CI/Test/Clippy pass on recent PRs; Cargo Deny fails across all runs
- **Security**: 3 persistent Cargo Deny failures (RUSTSEC-2026-0173, bitwarden licenses, aes yanked)
- **Stale PRs**: #125 (24d), #126 (23d) — Container cascade, suggest close
- **Next**: denom.toml single-fix unblocks all 5 open PRs simultaneously
