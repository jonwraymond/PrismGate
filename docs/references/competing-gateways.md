# Competing MCP Gateways

Comparison of PrismGate with other MCP and AI gateways, based on research via deepwiki queries and web search.

## Overview

| Gateway | Architecture | Transport | Tool Discovery | Sandbox | Open Source |
|---------|-------------|-----------|----------------|---------|-------------|
| **PrismGate** | Unix daemon + proxies | Stdio + HTTP | BM25 + semantic hybrid | V8 TypeScript | Yes |
| **Kong AI** | Plugin-based proxy | HTTP | N/A (plugin routing) | No | Partial |
| **Envoy AI** | Sidecar proxy | HTTP | N/A (session routing) | No | Yes |
| **Microsoft** | K8s dual-plane | HTTP | Tool Gateway Router | No | Yes |
| **Lasso** | Security gateway | HTTP | N/A (pass-through) | No | Yes |
| **IBM Context Forge** | Federated registry | HTTP | OTLP-instrumented | No | Yes |

## Detailed Comparison

### PrismGate (gatemini)

**Architecture**: Shared daemon process managing all backends via Unix domain sockets. Multiple Claude Code sessions connect through lightweight proxy processes.

**Differentiators**:
- **Progressive disclosure**: 7 meta-tools + brief/full modes (82-98% token savings)
- **BM25 + semantic hybrid search**: Combines keyword and conceptual matching with RRF fusion
- **V8 TypeScript sandbox**: Multi-tool orchestration in a single execution context
- **Local-machine optimization**: Unix domain sockets (30-66% lower latency than TCP)
- **Tool cache**: Instant availability on restart without re-discovery

**Trade-offs**:
- Local-only (no remote/cloud deployment)
- Single-machine scaling (no horizontal distribution)
- Designed for developer workstation use with Claude Code

**Source**: [GitHub](https://github.com/jraymond/gatemini)

---

### Kong AI Gateway

**Architecture**: Plugin-based API gateway with AI-specific extensions. The AI-Proxy plugin routes MCP traffic to multiple LLM providers.

**Key features**:
- `llm_format` field with provider-specific routing logic
- `preserve` mode for direct SDK passthrough (no transformation)
- ACL features for per-tool access control
- OAuth 2.1 implementation for MCP authentication
- Protocol bridge: converts REST APIs into MCP tools

**Differentiators**:
- Enterprise-grade with existing Kong infrastructure integration
- Multi-provider LLM routing (OpenAI, Anthropic, Mistral, etc.)
- Production-hardened rate limiting and authentication

**Trade-offs**:
- No progressive tool discovery
- Requires Kong infrastructure
- Commercial licensing for full features

**Source**: [Kong MCP Blog](https://konghq.com/blog/engineering/ai-gateway-mcp-gateway-mcp-server-breakdown)

---

### Envoy AI Gateway

**Architecture**: Sidecar proxy using token-encoding for session management. Session state is encrypted into the client session ID rather than stored centrally.

**Key features**:
- **Stateless session management**: Client-encoded session IDs contain multiple backend session references
- **MCPProxy**: Multiplexes by initializing parallel upstream sessions
- Configurable encryption (tunable latency: 1-2ms at low security to tens of ms at high security)
- No central session store needed (horizontal scaling without shared state)
- All core MCP features: tool calls, notifications, prompts, resources

**Differentiators**:
- Horizontal scaling without shared state (session info encoded in client ID)
- Leverages Envoy's existing proxy infrastructure
- ~1-2ms per-session overhead with tuned encryption

**Trade-offs**:
- No tool discovery optimization
- Session encoding adds per-request latency
- Requires Envoy infrastructure

**Sources**: [Envoy AI Gateway MCP](https://aigateway.envoyproxy.io/blog/mcp-implementation/), [Tetrate Performance](https://tetrate.io/blog/envoy-ai-gateway-mcp-performance)

---

### Microsoft MCP Gateway

**Architecture**: Kubernetes-native with separate Control Plane and Data Plane.

**Key features**:
- **Control Plane**: RESTful APIs for tool registration and lifecycle management
- **Data Plane**: Runtime traffic routing with session affinity
- **StatefulSets**: Tool adapters deployed as Kubernetes StatefulSets with ClusterIP services
- **Tool Gateway Router**: Specialized pods maintaining tool awareness, using HttpToolExecutor for forwarding
- Two routing modes: direct adapter access and tool gateway routing

**Differentiators**:
- Full Kubernetes-native deployment model
- Separation of control and data planes
- Session-aware stateful routing to specialized pods

**Trade-offs**:
- Requires Kubernetes infrastructure
- Significant operational complexity (StatefulSets, services, routing rules)
- No token optimization or progressive disclosure
- Enterprise-focused, not developer workstation

**Source**: [GitHub](https://github.com/microsoft/mcp-gateway)

---

### Lasso Security MCP Gateway

**Architecture**: Security-first gateway with plugin-based guardrail system.

**Key features**:
- **BasicGuardrailPlugin**: Regex patterns for secret masking
- **LassoGuardrailPlugin**: External AI safety API integration with fail-open semantics
- **PresidioGuardrailPlugin**: Microsoft Presidio for PII detection
- Intercepts both requests and responses (bidirectional filtering)
- Plugin architecture for custom guardrails

**Differentiators**:
- First security-centric MCP gateway (launched April 2025)
- PII detection and secret masking built-in
- AI safety API integration for prompt injection detection
- Fail-open semantics (safety check failures don't block traffic)

**Trade-offs**:
- Security focus, not tool discovery optimization
- No progressive disclosure or token savings
- Requires external AI safety APIs for full functionality

**Source**: [GitHub](https://github.com/lasso-security/mcp-gateway)

---

### IBM Context Forge

**Architecture**: Federated gateway that aggregates multiple peer gateways into a unified registry.

**Key features**:
- **Federation**: Multiple gateways auto-discover and merge registries
- **Redis-backed syncing**: Multi-cluster deployments with consistent state
- **Virtual Servers**: Logical tool bundling across federated servers
- **OTLP instrumentation**: Full OpenTelemetry support for traces, metrics, and logs
- Token usage and cost tracking
- REST and MCP service federation

**Differentiators**:
- Multi-cluster federation (unique in the MCP gateway space)
- Built-in observability with OTLP
- Virtual servers for logical tool grouping
- LLM-specific metrics (token usage, costs)

**Trade-offs**:
- Requires Redis for federation
- Complex multi-cluster setup
- No progressive disclosure or tool search optimization

**Source**: [GitHub](https://github.com/IBM/mcp-context-forge)

## Feature Comparison Matrix

| Feature | PrismGate | Kong | Envoy | Microsoft | Lasso | IBM |
|---------|-----------|------|-------|-----------|-------|-----|
| Progressive disclosure | **Yes** | No | No | No | No | No |
| Token optimization | **82-98%** | No | No | No | No | No |
| Hybrid search (BM25+semantic) | **Yes** | No | No | No | No | No |
| Code execution sandbox | **V8** | No | No | No | No | No |
| Tool cache | **Yes** | No | No | No | No | No |
| Circuit breaker | **Yes** | Yes | Yes | Yes | No | Yes |
| Health checks | **Yes** | Yes | Yes | Yes | No | Yes |
| Hot-reload config | **Yes** | Yes | Yes | No | No | Yes |
| Secret management | **BWS** | Vault | N/A | K8s Secrets | N/A | N/A |
| PII detection | No | No | No | No | **Yes** | No |
| Federation | No | No | No | No | No | **Yes** |
| OTLP telemetry | Planned | Yes | No | No | No | **Yes** |
| Horizontal scaling | No | **Yes** | **Yes** | **Yes** | No | **Yes** |
| Auth (OAuth/JWT) | No | **Yes** | No | No | No | **Yes** |
| Multi-LLM routing | No | **Yes** | No | No | No | No |

## Architectural Patterns

### Session Management

| Gateway | Approach | Statefulness |
|---------|----------|-------------|
| PrismGate | Per-client daemon connection | Stateful (daemon manages sessions) |
| Kong | Plugin-based | Stateless (plugin per-request) |
| Envoy | Token-encoded session ID | Stateless (session in client ID) |
| Microsoft | StatefulSet pods | Stateful (K8s affinity) |
| Lasso | Pass-through | Stateless |
| IBM | Redis-backed | Stateful (Redis store) |

### Deployment Model

| Gateway | Target Environment | Scaling |
|---------|-------------------|---------|
| PrismGate | Developer workstation | Single-machine, shared daemon |
| Kong | Cloud/on-prem API gateway | Horizontal (Kong clustering) |
| Envoy | Cloud-native sidecar | Horizontal (Envoy mesh) |
| Microsoft | Kubernetes cluster | Horizontal (K8s scaling) |
| Lasso | Cloud security layer | Single instance |
| IBM | Multi-cluster enterprise | Horizontal (Redis federation) |

## Where PrismGate Fits

PrismGate occupies a unique niche: **local-machine MCP gateway optimized for AI agent context efficiency**. While other gateways focus on cloud deployment, authentication, or security, PrismGate focuses on the developer experience problem of managing 30+ MCP tools without overwhelming the AI agent's context window.

The progressive disclosure system, hybrid search, and V8 sandbox are capabilities no other gateway provides. However, PrismGate is intentionally local-only -- it's not designed for cloud deployment, multi-tenant access, or horizontal scaling. For those needs, Kong (enterprise), Envoy (cloud-native), or IBM (federated) are better choices.

## Sources

- [Kong AI Gateway](https://konghq.com/blog/engineering/ai-gateway-mcp-gateway-mcp-server-breakdown) -- Kong MCP architecture
- [Envoy AI Gateway](https://aigateway.envoyproxy.io/blog/mcp-implementation/) -- Session encoding
- [Envoy Performance](https://tetrate.io/blog/envoy-ai-gateway-mcp-performance) -- Latency benchmarks
- [Microsoft MCP Gateway](https://github.com/microsoft/mcp-gateway) -- K8s dual-plane
- [Lasso MCP Gateway](https://github.com/lasso-security/mcp-gateway) -- Security guardrails
- [IBM Context Forge](https://github.com/IBM/mcp-context-forge) -- Federated gateways
- [MCP Gateway Comparison (Moesif)](https://www.moesif.com/blog/monitoring/model-context-protocol/Comparing-MCP-Model-Context-Protocol-Gateways/) -- Feature comparison
- [MCP Gateways Explained](https://mcpmanager.ai/blog/mcp-gateway/) -- Gateway overview
- [MCP Server vs Gateway](https://skywork.ai/blog/mcp-server-vs-mcp-gateway-comparison-2025/) -- When to use each
- [Awesome MCP Gateways](https://github.com/e2b-dev/awesome-mcp-gateways) -- Curated list
- [Centralizing AI Tool Access (AIM)](https://research.aimultiple.com/mcp-gateway/) -- Market analysis
