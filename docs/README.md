# Gatemini Documentation

This set of docs is aligned to the current Rust implementation in `src/`, not to historical marketing copy or one specific populated registry snapshot.

Naming note:

- `gatemini` is the binary, package, config directory, socket prefix, and MCP server name.
- `PrismGate` is the GitHub repository and release channel name.

## Start here

- [Architecture](architecture.md): daemon/proxy lifecycle, socket coordination, direct mode, restart flow
- [Codebase Map](codebase-map.md): end-to-end tour of the source tree and runtime ownership
- [Tool Discovery](tool-discovery.md): the 7 meta-tools, BM25 plus optional semantic search, RRF fusion
- [Backend Management](backend-management.md): backend states, transports, health checker, prerequisites, concurrency
- [Secrets & Config](secrets-and-config.md): `.env` load order, environment interpolation, secretref resolution, hot-reload boundaries
- [Resources & Prompts](resources-and-prompts.md): live `gatemini://` resources and MCP prompts
- [Sandbox](sandbox.md): `call_tool_chain` fast paths and V8 execution model
- [Token Efficiency](token-efficiency.md): where the context savings come from and how to measure them
- [Telemetry](telemetry-strategy.md): what the code already tracks and what OTEL work is still proposed

## Benchmarks and references

- [Search Quality](benchmarks/search-quality.md): code-backed search behavior plus a test methodology
- [Token Savings](benchmarks/token-savings.md): example measurement approaches, not a fixed source-of-truth registry size
- [MCP Best Practices](references/mcp-best-practices.md): external material mapped back to this project
- [Competing Gateways](references/competing-gateways.md): positioning context and tradeoffs

## Source orientation

If you are reading code and docs side by side, start with:

- `src/main.rs` for shared initialization
- `src/ipc/` for daemon and proxy behavior
- `src/server.rs` for the public MCP surface
- `src/backend/` for transport implementations and lifecycle management
- `src/config.rs` for defaults and hot-reload behavior
