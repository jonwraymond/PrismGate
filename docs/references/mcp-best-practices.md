# MCP Best Practices Reference

Curated external research on building effective MCP servers, organized by topic. Sources include official MCP documentation, industry blog posts, academic papers, and competing implementations.

## Token Efficiency & Progressive Disclosure

### Speakeasy: Dynamic Toolsets V2
- **URL**: https://www.speakeasy.com/blog/how-we-reduced-token-usage-by-100x-dynamic-toolsets-v2
- **Key finding**: Dynamic Toolset approach reduces token usage by 96% for inputs and 90% for total consumption. Up to 160x reduction while maintaining 100% success rates.
- **Relevance**: Validates PrismGate's `search_tools` + `tool_info` meta-tool pattern.

### Speakeasy: Progressive Discovery vs Semantic Search
- **URL**: https://www.speakeasy.com/blog/100x-token-reduction-dynamic-toolsets
- **Key finding**: Progressive search uses 1,600-2,500 tokens regardless of toolset size (40-400 tools). Token usage remains constant as tools scale.
- **Relevance**: Confirms PrismGate's hybrid BM25+semantic approach achieves similar constant-token behavior.

### Speakeasy: Dynamic Tool Discovery in MCP
- **URL**: https://www.speakeasy.com/mcp/tool-design/dynamic-tool-discovery
- **Key finding**: Adding a separate `describe_tools` function reduces token usage since input schemas represent the largest portion.
- **Relevance**: Mirrors PrismGate's two-tier approach: `search_tools` for discovery, `tool_info` for schemas.

### Anthropic: Advanced Tool Use
- **URL**: https://www.anthropic.com/engineering/advanced-tool-use
- **Key finding**: `defer_loading: true` enables the Tool Search Tool. Only 3-5 most relevant tools are expanded. Up to 85% reduction; accuracy improves from 79.5% to 88.1%.
- **Relevance**: Official Anthropic validation of PrismGate's architecture.

### Anthropic: Tool Search Tool Documentation
- **URL**: https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool
- **Key finding**: Keep 3-5 tools always loaded, defer the rest. API returns `tool_reference` blocks automatically expanded.
- **Relevance**: PrismGate's 7 meta-tools serve as the always-loaded set.

### Layered.dev: Schema Bloat Token Tax
- **URL**: https://layered.dev/mcp-tool-schema-bloat-the-hidden-token-tax-and-how-to-fix-it/
- **Key finding**: A five-server setup with 58 tools consumes approximately 55K tokens before conversation starts.
- **Relevance**: Quantifies the problem PrismGate solves.

### RAG-MCP Academic Paper
- **URL**: https://arxiv.org/abs/2505.03275
- **Key finding**: RAG-MCP triples tool selection accuracy (43.13% vs 13.62%) and reduces prompt tokens by 50%+.
- **Relevance**: Academic validation of retrieval-first tool discovery.

### Huawei SEP-1576: Schema Redundancy
- **URL**: https://github.com/modelcontextprotocol/modelcontextprotocol/issues/1576
- **Key finding**: 60% of GitHub MCP server tools share identical field definitions. JSON `$ref` deduplication can cut 30-60%.
- **Relevance**: Complementary optimization PrismGate could add to full-mode responses.

### ProDisco: Progressive Disclosure for Kubernetes
- **URL**: https://github.com/harche/ProDisco
- **Key finding**: Indexes APIs for discovery, agents search + execute in sandbox. Prevents 30,000-50,000 token overhead.
- **Relevance**: Validates PrismGate's search+sandbox pattern.

### Meta-Tool Pattern: Bounded Context Packs
- **URL**: https://blog.synapticlabs.ai/bounded-context-packs-meta-tool-pattern
- **Key finding**: Meta-tools serve as the discovery interface, exposing searchTools and getTypeDefinitions instead of hundreds of narrow tools.
- **Relevance**: PrismGate's 7 meta-tools are a production implementation of this pattern.

### Progressive Disclosure in Agentic Workflows
- **URL**: https://medium.com/@prakashkop054/s01-mcp03-progressive-disclosure-for-knowledge-discovery-in-agentic-workflows-8fc0b2840d01
- **Key finding**: Two-stage discovery (minimal → full) achieves 96% token reduction in typical workflows.
- **Relevance**: Mirrors PrismGate's brief/full discovery modes.

### Progressive Tool Discovery Pattern
- **URL**: https://agentic-patterns.com/patterns/progressive-tool-discovery/
- **Key finding**: Recognized agentic pattern that scales to hundreds or thousands of tools.
- **Relevance**: PrismGate is a production implementation.

## Tool Scaling & Selection

### Microsoft: Tool-space Interference
- **URL**: https://www.microsoft.com/en-us/research/blog/tool-space-interference-in-the-mcp-era-designing-for-agent-compatibility-at-scale/
- **Key finding**: 1,500 MCP servers analyzed. Tool name collisions, semantic overlap, and long response context degrade performance. OpenAI recommends <20 tools.
- **Relevance**: PrismGate's namespaced registry prevents tool-space interference.

### Demiliani: Too Many Tools Problem
- **URL**: https://demiliani.com/2025/09/04/model-context-protocol-and-the-too-many-tools-problem/
- **Key finding**: Performance degrades after ~40 tools, falls off a cliff after 60. Cursor limits to 40 MCP tools.
- **Relevance**: PrismGate's 7 meta-tools stay well within the accuracy threshold.

### Jenova: AI Tool Overload
- **URL**: https://www.jenova.ai/en/resources/mcp-tool-scalability-problem
- **Key finding**: 5-7 tools is the practical upper limit for consistent accuracy. RAG-MCP approach triples accuracy.
- **Relevance**: PrismGate exposes exactly 7 meta-tools.

### The MCP Tool Trap
- **URL**: https://jentic.com/blog/the-mcp-tool-trap
- **Key finding**: Tool descriptions consume context needed for reasoning and task memory. Too many leads to hallucinated parameters.
- **Relevance**: PrismGate's meta-tool architecture addresses this directly.

### Tool-to-Agent Retrieval
- **URL**: https://arxiv.org/abs/2511.01854
- **Key finding**: 19.4% Recall@5 improvement via unified embedding space. MCP-specific LiveMCPBench benchmark.
- **Relevance**: Academic validation of embedding-based tool search.

### ToolLLM (ICLR 2024)
- **URL**: https://arxiv.org/abs/2307.16789
- **Key finding**: 16,464 APIs organized in tree structure with DFSDT planning. Foundational large-scale tool discovery paper.
- **Relevance**: Validates the need for structured tool discovery at scale.

## MCP Protocol & Primitives

### MCP Specification (2025-11-25)
- **URL**: https://modelcontextprotocol.io/specification/2025-11-25
- **Key finding**: Official spec defining tools/list, tools/call, pagination, listChanged notifications, and three primitives.
- **Relevance**: PrismGate implements all three MCP primitives.

### MCP Resources Concept
- **URL**: https://modelcontextprotocol.info/docs/concepts/resources/
- **Key finding**: Resources are application-controlled data. Templates use URI patterns for dynamic content.
- **Relevance**: PrismGate's resource system provides @-mention discovery.

### WorkOS MCP Features Guide
- **URL**: https://workos.com/blog/mcp-features-guide
- **Key finding**: Comprehensive guide to tools, resources, prompts, sampling, roots, and elicitation.
- **Relevance**: Reference for PrismGate's multi-primitive approach.

### Laurent Kubaski: Stdio Transport
- **URL**: https://medium.com/@laurentkubaski/understanding-mcp-stdio-transport-protocol-ae3d5daf64db
- **Key finding**: Client launches server as subprocess. Newline-delimited messages on stdin/stdout.
- **Relevance**: PrismGate's proxy mode bridges this protocol.

### MCP Message Types (Portkey)
- **URL**: https://portkey.ai/blog/mcp-message-types-complete-json-rpc-reference-guide/
- **Key finding**: JSON-RPC 2.0 with requests, notifications, and results. Transport-agnostic.
- **Relevance**: PrismGate bridges JSON-RPC over Unix domain sockets.

### CodeSignal: MCP Primitives
- **URL**: https://codesignal.com/learn/courses/developing-and-integrating-a-mcp-server-in-python/lessons/exploring-and-exposing-mcp-server-capabilities-tools-resources-and-prompts
- **Key finding**: Tools are model-driven, Resources are application-driven, Prompts are user-driven.
- **Relevance**: PrismGate implements all three control patterns.

## Search & Retrieval

### Okapi BM25 (Wikipedia)
- **URL**: https://en.wikipedia.org/wiki/Okapi_BM25
- **Key finding**: Standard k1=1.2, b=0.75 parameters. Bag-of-words retrieval function.
- **Relevance**: PrismGate's BM25 implementation uses these parameters.

### Elastic: BM25 Algorithm Variables
- **URL**: https://www.elastic.co/blog/practical-bm25-part-2-the-bm25-algorithm-and-its-variables
- **Key finding**: k1 limits term frequency impact. b controls length normalization.
- **Relevance**: Parameter tuning guidance for PrismGate's tool descriptions.

### Elastic: Picking b and k1
- **URL**: https://www.elastic.co/blog/practical-bm25-part-3-considerations-for-picking-b-and-k1-in-elasticsearch
- **Key finding**: Defaults work for most corpora. Short documents may benefit from lower b.
- **Relevance**: PrismGate's tool descriptions are short and uniform.

### Azure: Reciprocal Rank Fusion
- **URL**: https://learn.microsoft.com/en-us/azure/search/hybrid-search-ranking
- **Key finding**: RRF score = 1/(rank + k), k=60. Handles disparate scoring ranges without tuning.
- **Relevance**: PrismGate's hybrid search uses RRF with K=60.

### Model2Vec
- **URL**: https://github.com/MinishLab/model2vec
- **Key finding**: 50x size reduction, 500x faster. Vocabulary distillation + PCA + Zipf weighting.
- **Relevance**: Powers PrismGate's semantic search.

### Hybrid Search Recall
- **URL**: https://medium.com/thinking-sand/hybrid-search-with-bm25-and-rank-fusion-for-accurate-results-456a70305dc5
- **Key finding**: Hybrid achieves 53.4% passage recall vs BM25's 22.1%.
- **Relevance**: Quantifies the benefit of PrismGate's hybrid approach.

## Observability & Telemetry

### OpenTelemetry: AI Agent Observability
- **URL**: https://opentelemetry.io/docs/specs/semconv/gen-ai/
- **Key finding**: Standard schema for tracking prompts, model responses, token usage, tool calls.
- **Relevance**: Target standard for PrismGate's telemetry.

### Datadog: MCP Client Monitoring
- **URL**: https://www.datadoghq.com/blog/mcp-client-monitoring/
- **Key finding**: End-to-end tracing of MCP lifecycle with automatic span capture.
- **Relevance**: Target observability model.

### OpenLLMetry
- **URL**: https://github.com/traceloop/openllmetry
- **Key finding**: OTel extensions for LLM calls. Conventions merged into OpenTelemetry standard.
- **Relevance**: Reference implementation for AI observability.

### Sentry: MCP Server Monitoring
- **URL**: https://blog.sentry.io/introducing-mcp-server-monitoring/
- **Key finding**: Single line of code for full MCP observability. Automatic span capture.
- **Relevance**: Integration target for PrismGate.

## Security & Secrets

### OWASP: Secrets Management Cheat Sheet
- **URL**: https://cheatsheetseries.owasp.org/cheatsheets/Secrets_Management_Cheat_Sheet.html
- **Key finding**: Provide SDKs, CLI for local dev, self-service workflows. Audit access.
- **Relevance**: Best practices for PrismGate's secret management.

### 1Password: Secret References
- **URL**: https://developer.1password.com/docs/cli/secret-references/
- **Key finding**: URI-based references (op://vault/item/field). Config files with references safe to commit.
- **Relevance**: Pattern comparison for PrismGate's secretref: syntax.

### Bitwarden: Secrets Manager SDK
- **URL**: https://bitwarden.com/help/secrets-manager-sdk/
- **Key finding**: Rust-based SDK with machine account authentication.
- **Relevance**: PrismGate's BWS integration foundation.

## Code Execution & Sandboxing

### Glama: Code Execution with MCP
- **URL**: https://glama.ai/blog/2025-12-14-code-execution-with-mcp-architecting-agentic-efficiency
- **Key finding**: Code execution reduces token consumption by batching operations in a single context.
- **Relevance**: Validates `call_tool_chain` architecture.

### Block Goose: Code Mode + MCP
- **URL**: https://block.github.io/goose/blog/2025/12/21/code-mode-doesnt-replace-mcp/
- **Key finding**: Code execution and MCP are complementary, not competing approaches.
- **Relevance**: Validates PrismGate's combined meta-tool + sandbox design.

### V8 Sandbox
- **URL**: https://v8.dev/blog/sandbox
- **Key finding**: In-process memory isolation with ~1% performance overhead.
- **Relevance**: Security foundation for PrismGate's TypeScript execution.

### Deno Security
- **URL**: https://docs.deno.com/runtime/fundamentals/security/
- **Key finding**: No filesystem, network, env, or subprocess access by default.
- **Relevance**: PrismGate's sandbox inherits these restrictions.

## Process Management & IPC

### Baeldung: IPC Performance
- **URL**: https://www.baeldung.com/linux/ipc-performance-comparison
- **Key finding**: Unix domain sockets deliver 30-66% lower latency and 7x throughput vs TCP.
- **Relevance**: Validates PrismGate's UDS choice.

### flock(2) Man Page
- **URL**: https://man7.org/linux/man-pages/man2/flock.2.html
- **Key finding**: LOCK_EX for exclusive lock, LOCK_NB for non-blocking. Auto-released on process exit.
- **Relevance**: PrismGate's daemon coordination mechanism.

### Process Group Kill
- **URL**: https://www.baeldung.com/linux/kill-members-process-group
- **Key finding**: kill(-pgid, signal) terminates all group members. setpgid(0,0) creates new group.
- **Relevance**: PrismGate's backend process isolation.

### AWS: Circuit Breaker Pattern
- **URL**: https://docs.aws.amazon.com/prescriptive-guidance/latest/cloud-design-patterns/circuit-breaker.html
- **Key finding**: Closed → Open → Half-Open states. Prevents cascading failures.
- **Relevance**: PrismGate's health checker implements this pattern.

### AWS: Exponential Backoff with Jitter
- **URL**: https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/
- **Key finding**: Jitter prevents thundering herd on retries.
- **Relevance**: PrismGate uses exponential backoff for restarts.

## Rust Dependencies

### DashMap
- **URL**: https://github.com/xacrimon/dashmap
- **Key finding**: Lock-free concurrent HashMap with per-shard RwLocks.
- **Relevance**: PrismGate's tool registry and backend storage.

### rmcp
- **URL**: https://github.com/4t145/rmcp
- **Key finding**: Rust MCP SDK with tokio async, stdio/HTTP transports.
- **Relevance**: PrismGate's core MCP protocol implementation.

### rustyscript
- **URL**: https://github.com/rscarson/rustyscript
- **Key finding**: Deno-based V8 sandbox for Rust. TypeScript transpilation, sandboxed by default.
- **Relevance**: PrismGate's call_tool_chain execution engine.

### Tantivy
- **URL**: https://github.com/quickwit-oss/tantivy
- **Key finding**: Full-text search engine in Rust with BM25. Alternative for large-scale search.
- **Relevance**: Potential upgrade path for PrismGate's search at 10,000+ tools.

## Standards & Specifications

### IETF: Token-Efficient Agentic Communication
- **URL**: https://datatracker.ietf.org/doc/draft-chang-agent-token-efficient/
- **Key finding**: IETF draft proposing Agentic Data Object Layer (ADOL) to eliminate redundant definitions.
- **Relevance**: Standards-track validation of the token efficiency problem.

### MCP Schema Reference
- **URL**: https://modelcontextprotocol.io/specification/draft/schema
- **Key finding**: Official JSON schema definitions for MCP messages.
- **Relevance**: PrismGate's protocol compliance baseline.

### MCP Transports Specification
- **URL**: https://modelcontextprotocol.info/specification/draft/basic/transports/
- **Key finding**: Stdio (newline-delimited) and HTTP (Streamable HTTP + SSE) transports.
- **Relevance**: PrismGate supports both transport types.
