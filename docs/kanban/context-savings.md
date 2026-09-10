# PrismGate — Context Savings & Token Efficiency Board

Epic: Enhance PrismGate token savings and context management
Target branch: `feat/context-savings`
Upstream repo: `jonwraymond/PrismGate`

## Quick Wins
- [ ] 1. Wire telemetry: call `record_bytes` from `call_tool_chain` pipeline and expose via `gatemini://stats`
- [ ] 2. Per-agent search flood guard: rolling window keyed by `session_id` for `search_tools` / `tool_info`
- [ ] 3. Byte-budget truncation utilities: UTF-8-safe hard caps and JSON-aware truncation markers
- [ ] 4. Session purge command: `gatemini purge` + optional MCP tool

## Medium-Lift Enhancements
- [ ] 5. Persistent result handles: store raw output before destructive processing, return searchable handle + excerpts
- [ ] 6. Structured retrieval: JSON-path / Markdown-heading / text-boundary chunks + ranked multi-query retrieval with budgets
- [ ] 7. Sandbox data handles: extend bridge with `store/load/query` helpers for reuse across sandbox calls

## High-Lift / High-Reward
- [ ] 8. FTS5-backed session event store: file edits, tool calls, errors, decisions, tasks; new `gatemini://session/search` resource

## Cross-Cutting
- [ ] A. Tests for all new telemetry, flood guard, truncation, result store, session store
- [ ] B. Build verification: `cargo check/test/clippy/fmt` green on branch
- [ ] C. Container image build verification via GHCR PR image path
- [ ] D. Docs update: README + `llms.txt` + `gatemini://call_tool_chain` guide
- [ ] E. Open PR against upstream with changelog and rollback note
