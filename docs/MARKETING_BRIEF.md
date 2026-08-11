# PrismGate Marketing Brief

## Target Audience

### Primary
- **DevOps/SRE teams** running MCP servers at scale
- **AI/ML engineers** building agent workflows with multiple MCP tools
- **Enterprise teams** needing governance, audit trails, and secrets management for MCP

### Secondary
- **Open-source developers** building MCP server ecosystems
- **Platform teams** integrating MCP into existing infrastructure
- **Security-conscious organizations** requiring credential isolation

## Value Proposition

**PrismGate** is the open-source MCP gateway that makes tool discovery intelligent, sessions isolated, and secrets secure — without vendor lock-in.

### Tagline Options
1. "The gateway that refracts your MCP tools through a prism of intelligence."
2. "Intelligent tool discovery. Isolated sessions. Secure secrets."
3. "The open-source MCP gateway built for scale."

## Competitive Advantages

| Advantage | PrismGate | MCPX | Docker MCP | Microsoft |
|-----------|-----------|------|------------|-----------|
| **3-Tier Search** | ✅ BM25→trigram→fuzzy | Basic | Basic | Basic |
| **V8 Sandbox** | ✅ TypeScript execution | ❌ | ❌ | ❌ |
| **Session Isolation** | ✅ InstancePool | ❌ | ✅ Container | ✅ K8s |
| **Secrets Management** | ✅ BWS + env fallback | ✅ Vault | ❌ | Azure Key Vault |
| **Open Source** | ✅ MIT | MIT + Enterprise | ✅ | ❌ Azure-locked |
| **Rust Performance** | ✅ Low latency | Go | Docker | Azure-dependent |

## Messaging Framework

### For Developers
"PrismGate gives you intelligent tool discovery across hundreds of MCP tools, TypeScript execution in a V8 sandbox, and per-session isolation — all in a single Rust binary."

### For Enterprise
"PrismGate provides enterprise-grade MCP governance with BWS secrets management, structured audit logging, and tool-level access control — open-source, no vendor lock-in."

### For Security Teams
"PrismGate isolates credentials from config files using BWS reference-only patterns, provides structured audit trails for every tool call, and supports circuit breakers for graceful degradation."

## Launch Talking Points

1. **Intelligent Discovery** — "Unlike flat tool lists, PrismGate's 3-tier search engine finds the right tool in milliseconds, even across hundreds of registered MCP servers."

2. **TypeScript Sandbox** — "Composite tools run in an isolated V8 sandbox, enabling multi-step orchestrations without exposing your backend MCP servers."

3. **Session Isolation** — "Each MCP client gets its own dedicated backend instance, preventing cross-session contamination and enabling stateful tool interactions."

4. **Open Source** — "MIT licensed, no vendor lock-in, no enterprise tier gating. All features available to everyone."

5. **Rust Performance** — "Built in Rust for low latency, memory safety, and zero-copy serialization. ~4ms p99 for tool discovery."

## Metrics to Track

- GitHub stars and forks
- npm/cargo download counts
- Discord/community engagement
- Issue resolution time
- PR merge velocity
- Documentation page views
