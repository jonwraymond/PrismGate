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
## Full Heartbeat — 2026-06-17 13:57 UTC
- CI: Container ✅, Commit Lint ✅ — no cascade. CI ❌ (dead-code in trace_context.rs), Security ❌ (Cargo Deny bitwarden licenses + RUSTSEC-2026-0173)
- PRs open: 5 (#130 dead-code, #129 release/draft, #127 commitlint, #126 Container cascade, #125 cascade+fmt)
- Kanban: 3 ready (P1), 1 done (P2), 0 stalled, 0 running

---

## Market Research — 2026-06-17 (Competitive Analysis Cycle)

### Competitor Landscape Update

| Competitor | New Since Last Check | Threat Level | Key Differentiator |
|-----------|---------------------|-------------|-------------------|
| **MCPX (Lunar.dev)** | Gartner Representative Vendor, SOC 2 certified (Enterprise), ~4ms p99, full-chain immutable audit (User→Agent→MCP→Tool), risk scoring per server | 🔴 HIGH | Governance depth unmatched |
| **Bifrost (Maxim AI)** | Unified LLM+MCP gateway, 11μs overhead at 5k RPS, HashiCorp Vault integration, Google/GitHub SSO | 🟡 MEDIUM | Ultra-low latency + unified infra |
| **TrueFoundry** | 3-4ms latency, 350+ RPS on 1 vCPU, OAuth 2.0 OBO identity injection, hybrid cloud/on-prem | 🟡 MEDIUM | Performance + OAuth |
| **MintMCP** | **First SOC 2 Type II certified** MCP platform, one-click deployment, Cursor partnership | 🟡 MEDIUM | Managed compliance |
| **MCPJungle** | 52 releases, 1.1k stars, 274 commits, 24 contributors, MPL-2.0, Docker + brew, tool groups shipped | 🟡 MEDIUM | Developer adoption velocity |
| **IBM ContextForge** | 40+ plugins, multi-protocol (MCP, A2A, REST→MCP, gRPC), OpenTelemetry, Redis federation | 🟢 LOW (complexity) | Extensibility |
| **Kong AI Gateway** | MCP Proxy plugin for existing Kong deployments, enterprise pricing | 🟢 LOW (locked) | Existing API governance |
| **Traefik Hub** | Triple Gate Pattern, OBO OAuth 2.0, TBAC | 🟢 LOW (coupled) | Security architecture |
| **Docker MCP** | Container isolation only — no governance, no RBAC, no audit | 🟢 LOW | Recognized as insufficient |
| **Microsoft MCP** | Azure-only, APIM-based — no tool RBAC or agent-identity attribution | 🟢 LOW | Azure lock-in |

### Biggest New Threats
1. **MCPX Enterprise tier**: Now SOC 2 certified + Gartner recognized. Tool-level RBAC, immutable audit, credential isolation, automated risk scoring. The governance benchmark.
2. **MintMCP**: First SOC 2 Type II certification solves the compliance blocker instantly. One-click deployment removes infra barriers.
3. **MCPJungle velocity**: 52 releases, 1.1k stars, growing contributor base. Simple Docker deployment + tool groups = developer on-ramp that PrismGate lacks.

### PrismGate Competitive Position (June 2026)

| Dimension | PrismGate Status | vs Best Competitor | Gap |
|-----------|-----------------|-------------------|-----|
| **Tool Discovery** | ✅ 7 meta-tools, 3-tier search (BM25→trigram→fuzzy) | Unique advantage | Leading |
| **Resource Efficiency** | ✅ Shared daemon, proxy reconnect | Unique architecture | Leading |
| **Context Savings** | ✅ Progressive discovery vs upfront schema dump | Only solution addressing token overhead | Leading |
| **Tool-level RBAC** | ❌ None | MCPX: full RBAC with IDP integration | Critical gap |
| **Immutable Audit** | ❌ None | MCPX: full-chain User→Agent→MCP→Tool | Critical gap |
| **OAuth/OIDC** | ❌ None | zuplo checklist: RFC 9728, 8414, 7591, 8707 | Critical gap |
| **Tool Groups** | ❌ Flat search only | MCPX, MCPJungle: hierarchical | Important gap |
| **Docker/Deploy** | ❌ No Docker image | MCPJungle: 1-command docker compose | Adoption barrier |
| **Prometheus/OTel** | ❌ None | MCPX: Prometheus metrics; ContextForge: OpenTelemetry | Important gap |
| **SOC2/Compliance** | ❌ None | MCPX, MintMCP: SOC2 certified | Enterprise barrier |
| **Community/Stars** | ⚠️ Unknown | MCPJungle: 1.1k stars, 24 contributors | Growth gap |

### MCP Protocol: 2026-07-28 RC Impact

**Status**: Release Candidate locked May 21. Final ship: July 28, 2026. **10 weeks to compliance.**

Key changes affecting PrismGate:
1. **Stateless core**: No initialize handshake, no `Mcp-Session-Id`. Proxy reconnect's cached handshake replay becomes obsolete.
2. **New required headers**: `MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name` on Streamable HTTP. New code needed.
3. **MRTR elicitation**: `InputRequiredResult` + `requestState` pattern. State burden shifts to client/server, not transport.
4. **Cache hints**: `ttlMs` + `cacheScope` in list/read results. PrismGate's tool cache (v4) needs updating.
5. **Auth hardening**: RFC 9207 (`iss` validation), RFC 8707 (audience validation), PKCE S256 enforced. PrismGate has none.
6. **Deprecations**: Roots, Sampling, Logging deprecated (12-month window). Low impact for PrismGate.
7. **Full JSON Schema 2020-12**: `$ref`/`$defs`, `oneOf`/`anyOf`/`allOf`. Tool schema parsing needs updating.
8. **Extensions formalized**: MCP Apps (iframe UI, SEP-1865), Tasks (SEP-2663). New surface areas.

### User Pain Points (Community Research)

| Pain Point | Source | PrismGate Relevance |
|-----------|--------|--------------------|
| **Token overhead**: ~1000 tokens/tool/session | GitHub #2812 (May 28) | ✅ PrismGate's progressive discovery directly solves this |
| **Tool sprawl**: >30-60 tools degrades model accuracy | Reddit r/mcp, arXiv paper | ✅ 3-tier search (BM25→trigram→fuzzy) mitigates |
| **Supply chain**: STDIO servers = "reputational time bomb" | MCP maintainer meeting (#2547) | ⚠️ PrismGate supports STDIO backends — needs security posture |
| **Session confusion**: Transport vs application sessions conflated | Maintainer meeting (#2547) | ⚠️ Dedicated instance mode may need updates for stateless spec |
| **Client gap**: "Real lack of clients implementing full spec" | Core maintainer meeting (#2536) | ℹ️ Documentation opportunity |
| **No standard governance**: Each org builds own audit | Multiple discussions | ❌ PrismGate has no audit/RBAC — major gap |
| **MCP OAuth complexity**: IndieAuth, PKCE, dynamic registration | Discussions #2740, #2760 | ❌ PrismGate has no OAuth — must implement |

### Top 3 Insights (June 17, 2026)

1. **2026-07-28 stateless spec is an opportunity, not a threat**: PrismGate's shared-daemon + proxy model was always closer to stateless than competitors' session-heavy architectures. The cached-handshake replay needs replacement, but the architecture is well-positioned. Competitors shipping session state machines must do a full rewrite.

2. **Governance depth is the existential gap**: MCPX now has tool-level RBAC, immutable audit trails, credential isolation, and SOC 2 certification — all the things enterprises require. PrismGate has zero governance features. The progressive discovery advantage won't matter if organizations can't pass compliance review.

3. **Docker deployment unlocks adoption**: MCPJungle's 1.1k stars on a simpler product prove that developer on-ramp (1-command Docker, brew install, tool groups) drives adoption faster than architectural sophistication. PrismGate's "cargo install from source" is a 10x adoption barrier.

### Actionable Kanban Cards Created (7 new)
- t_483ad1f0: Research 2026-07-28 stateless spec impact (alan, P1) — RUNNING
- t_92b97b34: Implement tool groups (turing, P1) — RUNNING
- t_b41502a0: Implement OAuth 2.0/OIDC (turing, P1) — RUNNING
- t_e3f34480: Ship Docker image (turing, P1) — RUNNING
- t_b4d73497: Competitive parity gap analysis — audit/metrics (alan, P2)
- t_758a367d: Write competitive comparison page (mira, P2)
- t_57cb95d8: Research MCP user pain points (alan, P2)

