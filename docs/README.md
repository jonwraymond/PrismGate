# PrismGate Documentation

PrismGate (gatemini) is a Rust MCP gateway that connects to 30+ backend MCP servers and exposes 7 meta-tools to AI agents via a shared daemon architecture.

## Quick Links

- **Discovering tools**: [Tool Discovery](tool-discovery.md) -- progressive disclosure workflow
- **Adding backends**: [Backend Management](backend-management.md) -- stdio/HTTP lifecycle
- **Understanding token savings**: [Token Efficiency](token-efficiency.md) -- measured 82-98% reduction
- **Configuration**: [Secrets & Config](secrets-and-config.md) -- YAML pipeline, hot-reload, BWS

## Documentation Index

### Core Architecture

| Document | Description |
|----------|-------------|
| [Architecture](architecture.md) | IPC daemon/proxy model, Unix socket coordination, process lifecycle |
| [Tool Discovery](tool-discovery.md) | Progressive disclosure, BM25+semantic hybrid search, brief/full modes |
| [Token Efficiency](token-efficiency.md) | Measured savings with real data, comparison tables, industry benchmarks |
| [Sandbox](sandbox.md) | V8 `call_tool_chain` execution, bridge preamble, direct parsing fast path |

### Operations

| Document | Description |
|----------|-------------|
| [Backend Management](backend-management.md) | Health checks, circuit breaker, auto-restart, stdio/HTTP backends |
| [Secrets & Config](secrets-and-config.md) | Secret resolution pipeline, config hot-reload, Bitwarden integration |
| [Resources & Prompts](resources-and-prompts.md) | MCP resources for @-mention discovery, guided workflow prompts |
| [Telemetry Strategy](telemetry-strategy.md) | OpenTelemetry integration plan with GenAI semantic conventions |

### Benchmarks

| Document | Description |
|----------|-------------|
| [Search Quality](benchmarks/search-quality.md) | BM25+semantic validation at scale, RRF fusion analysis |
| [Token Savings](benchmarks/token-savings.md) | Real measurements, before/after comparisons, test methodology |

### References

| Document | Description |
|----------|-------------|
| [MCP Best Practices](references/mcp-best-practices.md) | 34+ curated external sources on tool design, naming, progressive disclosure |
| [Competing Gateways](references/competing-gateways.md) | Kong, Envoy, Microsoft, Lasso, IBM architecture comparison |

## Building

```bash
cargo build                    # debug build
cargo build --release          # release build (includes V8 sandbox + semantic search)
cargo test                     # run unit tests
```

## Source Code Map

See [CLAUDE.md](../CLAUDE.md) for the complete module reference table.
