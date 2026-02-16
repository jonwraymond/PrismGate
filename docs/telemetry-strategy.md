# Telemetry Strategy

This document outlines PrismGate's plan for extracting real effectiveness data using OpenTelemetry with GenAI semantic conventions.

## Current State

PrismGate already has basic observability through the `tracing` crate:

| Capability | Implementation | Location |
|------------|---------------|----------|
| Structured logging | `tracing` with `info!`, `warn!`, `error!` | All modules |
| In-flight call tracking | `CallGuard` RAII counter (AtomicUsize) | `src/backend/mod.rs` |
| Health check state | `consecutive_failures`, `restart_count`, `circuit_open_since` | `src/backend/health.rs` |
| Cache operations | Load/save event logging with tool counts | `src/cache.rs` |
| Backend lifecycle | Start/stop/restart event logging | `src/backend/mod.rs` |

### What's Missing

- **No token counting** -- cannot measure actual token savings from progressive disclosure
- **No call frequency tracking** -- don't know which tools agents use most
- **No latency histograms** -- no P50/P95/P99 for tool calls or discovery
- **No usage analytics** -- can't prove brief mode adoption or discovery patterns
- **No response size tracking** -- token savings are estimated, not measured

## OpenTelemetry Integration Plan

### Phase 1: Traces

Add spans to every tool call and discovery operation:

```
[daemon session]
  ├── [search_tools] query="web search" brief=true results=5 response_bytes=340
  ├── [tool_info] tool="web_search_exa" detail="brief" response_bytes=120
  ├── [tool_info] tool="web_search_exa" detail="full" response_bytes=3200
  └── [call_tool_chain]
       ├── [direct_parse] matched=true
       └── [backend.call_tool] backend="exa" tool="web_search_exa" duration_ms=450
```

**Span attributes** following [OpenTelemetry GenAI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/):

| Attribute | Type | Example |
|-----------|------|---------|
| `gatemini.tool.name` | string | `"search_tools"` |
| `gatemini.tool.backend` | string | `"exa"` |
| `gatemini.discovery.mode` | string | `"brief"` or `"full"` |
| `gatemini.discovery.result_count` | int | `5` |
| `gatemini.response.size_bytes` | int | `3200` |
| `gatemini.sandbox.execution_mode` | string | `"direct"` or `"v8"` |

### Phase 2: Metrics

Key metrics to track, using [GenAI conventions](https://opentelemetry.io/blog/2024/llm-observability/) where applicable:

**Discovery Metrics**:

| Metric | Type | Purpose |
|--------|------|---------|
| `gatemini.search.requests` | Counter | Total search_tools calls |
| `gatemini.search.brief_ratio` | Gauge | % of searches using brief mode |
| `gatemini.tool_info.requests` | Counter | Total tool_info calls |
| `gatemini.tool_info.brief_ratio` | Gauge | % of tool_info using brief mode |
| `gatemini.discovery.depth` | Histogram | Steps before execution (1-4) |
| `gatemini.response.size_bytes` | Histogram | Response sizes by tool type |

**Backend Metrics**:

| Metric | Type | Purpose |
|--------|------|---------|
| `gatemini.backend.call_duration` | Histogram | Tool call latency by backend |
| `gatemini.backend.calls_total` | Counter | Total calls by backend + tool |
| `gatemini.backend.errors_total` | Counter | Failed calls by backend + error type |
| `gatemini.backend.health_state` | Gauge | Current state (Healthy/Unhealthy/Stopped) |
| `gatemini.backend.restarts_total` | Counter | Auto-restart count by backend |
| `gatemini.backend.in_flight` | Gauge | Currently active calls |

**Sandbox Metrics**:

| Metric | Type | Purpose |
|--------|------|---------|
| `gatemini.sandbox.executions_total` | Counter | Total call_tool_chain invocations |
| `gatemini.sandbox.direct_parse_ratio` | Gauge | % resolved without V8 |
| `gatemini.sandbox.execution_duration` | Histogram | V8 execution time |
| `gatemini.sandbox.heap_usage_bytes` | Histogram | V8 heap consumption |

### Phase 3: Export

OTLP export to configurable backend:

```yaml
# gatemini.yaml
telemetry:
  enabled: true
  otlp_endpoint: "http://localhost:4317"  # Jaeger, Grafana, etc.
  export_interval: 30s
  service_name: "gatemini"
```

Compatible with:
- [Jaeger](https://www.jaegertracing.io/) -- distributed tracing
- [Grafana Cloud](https://grafana.com/docs/grafana-cloud/monitor-applications/ai-observability/mcp-observability/setup/) -- MCP-specific dashboards
- [SigNoz](https://signoz.io/blog/mcp-observability-with-otel/) -- open-source observability
- [Datadog](https://www.datadoghq.com/blog/mcp-client-monitoring/) -- LLM + MCP monitoring

## Metrics to Prove Progressive Disclosure Effectiveness

The key question: **do agents actually use progressive disclosure, and does it save tokens?**

### Adoption Metrics

```
brief_search_count / total_search_count = brief adoption rate
brief_tool_info_count / total_tool_info_count = brief adoption rate
```

Target: >90% of searches use brief mode, proving the default is effective.

### Token Savings Metrics

```
actual_response_bytes = sum of all search_tools + tool_info response sizes
counterfactual_bytes = result_count * average_full_tool_schema_size
savings_ratio = 1 - (actual_response_bytes / counterfactual_bytes)
```

### Discovery Depth Distribution

Track the step at which agents execute:

```
1-step: search_tools → call_tool_chain (agent already knows the tool)
2-step: search_tools → tool_info(brief) → call_tool_chain
3-step: search_tools → tool_info(brief) → tool_info(full) → call_tool_chain
4-step: full progressive disclosure flow
```

Shallower depth = more efficient discovery = better tool naming and descriptions.

## Reference Implementations

| Project | Approach | Relevance |
|---------|----------|-----------|
| [OpenLLMetry](https://github.com/traceloop/openllmetry) | OTel extensions for LLM calls | Semantic conventions for AI observability |
| [IBM Context Forge](https://github.com/IBM/mcp-context-forge) | OTLP instrumentation for MCP gateway | Production MCP gateway telemetry |
| [OpenLIT](https://docs.openlit.io/latest/openlit/quickstart-mcp-observability) | Single-line MCP instrumentation | Minimal-effort integration pattern |
| [Sentry MCP](https://blog.sentry.io/introducing-mcp-server-monitoring/) | Automatic span capture for tools | Error tracking integration |
| [SigNoz MCP](https://signoz.io/blog/mcp-observability-with-otel/) | Hierarchical span model for agents | Span hierarchy design reference |

## Proposed Span Hierarchy

Following [SigNoz's recommended model](https://signoz.io/blog/mcp-observability-with-otel/):

```
[session]                    -- Per-client daemon connection
  ├── [mcp.request]          -- Each MCP JSON-RPC request
  │   ├── [discovery]        -- search_tools, tool_info, list_tools_meta
  │   │   ├── [bm25_search]  -- BM25 scoring
  │   │   └── [semantic_search]  -- Embedding lookup (if enabled)
  │   ├── [tool_call]        -- call_tool_chain
  │   │   ├── [parse]        -- Direct parse or V8 sandbox
  │   │   └── [backend.call] -- Actual backend tool invocation
  │   └── [registration]     -- register_manual, deregister_manual
  └── [health_check]         -- Periodic backend pings
```

## Implementation Notes

### Rust Crates

| Crate | Purpose |
|-------|---------|
| `opentelemetry` | Core OTel API |
| `opentelemetry-otlp` | OTLP exporter |
| `tracing-opentelemetry` | Bridge `tracing` spans to OTel |
| `opentelemetry-semantic-conventions` | Standard attribute names |

PrismGate already uses `tracing` throughout, so the integration path is adding a `tracing-opentelemetry` layer that forwards spans to the OTLP exporter. Existing `info!`, `debug!`, and `warn!` calls become span events automatically.

### Minimal Code Changes

```rust
// In src/main.rs initialization:
let tracer = opentelemetry_otlp::new_pipeline()
    .tracing()
    .with_exporter(opentelemetry_otlp::new_exporter().tonic())
    .install_batch()?;

let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
// Add to existing tracing subscriber
```

```rust
// In tool call handlers:
#[tracing::instrument(fields(
    gatemini.tool.name = %tool_name,
    gatemini.tool.backend = %backend,
    gatemini.response.size_bytes = tracing::field::Empty,
))]
```

## Sources

- [OpenTelemetry AI Agent Observability](https://opentelemetry.io/docs/specs/semconv/gen-ai/) -- OTel conventions for AI
- [OTel LLM Observability Introduction](https://opentelemetry.io/blog/2024/llm-observability/) -- Foundational reference
- [Datadog MCP Monitoring](https://www.datadoghq.com/blog/mcp-client-monitoring/) -- End-to-end MCP tracing
- [Datadog OTel GenAI Conventions](https://www.datadoghq.com/blog/llm-otel-semantic-convention/) -- Standard schema
- [OpenLLMetry](https://github.com/traceloop/openllmetry) -- OTel extensions for LLM calls
- [IBM Context Forge](https://github.com/IBM/mcp-context-forge) -- MCP gateway with OTLP
- [SigNoz MCP Observability](https://signoz.io/blog/mcp-observability-with-otel/) -- Span hierarchy model
- [Grafana MCP Observability](https://grafana.com/docs/grafana-cloud/monitor-applications/ai-observability/mcp-observability/setup/) -- Dashboard setup
- [Sentry MCP Monitoring](https://blog.sentry.io/introducing-mcp-server-monitoring/) -- Automatic instrumentation
- [OpenLIT MCP Quickstart](https://docs.openlit.io/latest/openlit/quickstart-mcp-observability) -- Single-line setup
- [FinOps for AI](https://www.finops.org/wg/finops-for-ai-overview/) -- 30-200x cost variance
- [VictoriaMetrics AI Observability](https://victoriametrics.com/blog/ai-agents-observability/) -- Open-source metrics
- [`tracing` crate](https://docs.rs/tracing) -- Rust structured logging
- [`tracing-opentelemetry`](https://docs.rs/tracing-opentelemetry) -- OTel bridge
